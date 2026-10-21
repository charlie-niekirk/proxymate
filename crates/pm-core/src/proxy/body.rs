//! The body a rule-aware exchange sends on: the original stream, a
//! rewritten whole body, or a buffered prefix followed by the rest of the
//! stream (a body over the rewrite cap). Any of them can be throttled by a
//! token bucket.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::{Duration, Instant};

use bytes::{Buf, Bytes};
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use tokio::time::Sleep;

pub struct RuleBody {
    source: Source,
    throttle: Option<Bucket>,
    /// Data taken from `source` that the bucket hasn't let through yet.
    held: Option<Bytes>,
}

enum Source {
    Stream(Incoming),
    Whole(Option<Bytes>),
    /// A buffered prefix, then the rest of the stream.
    Chain(VecDeque<Bytes>, Incoming),
}

impl RuleBody {
    pub fn stream(body: Incoming) -> RuleBody {
        RuleBody::new(Source::Stream(body))
    }

    pub fn whole(bytes: impl Into<Bytes>) -> RuleBody {
        let bytes = bytes.into();
        RuleBody::new(Source::Whole((!bytes.is_empty()).then_some(bytes)))
    }

    pub fn empty() -> RuleBody {
        RuleBody::new(Source::Whole(None))
    }

    pub fn chain(prefix: VecDeque<Bytes>, rest: Incoming) -> RuleBody {
        RuleBody::new(Source::Chain(prefix, rest))
    }

    fn new(source: Source) -> RuleBody {
        RuleBody {
            source,
            throttle: None,
            held: None,
        }
    }

    /// Limits the body to `bytes_per_sec` (0: unlimited).
    pub fn throttled(mut self, bytes_per_sec: u64) -> RuleBody {
        if bytes_per_sec > 0 {
            self.throttle = Some(Bucket::new(bytes_per_sec));
        }
        self
    }

    fn poll_source(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        match &mut self.source {
            Source::Stream(body) => Pin::new(body).poll_frame(cx),
            Source::Whole(bytes) => Poll::Ready(bytes.take().map(|b| Ok(Frame::data(b)))),
            Source::Chain(prefix, rest) => match prefix.pop_front() {
                Some(chunk) => Poll::Ready(Some(Ok(Frame::data(chunk)))),
                None => Pin::new(rest).poll_frame(cx),
            },
        }
    }
}

impl Body for RuleBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        let this = &mut *self;
        if this.throttle.is_none() {
            return this.poll_source(cx);
        }
        loop {
            if this.held.is_none() {
                match ready!(this.poll_source(cx)) {
                    Some(Ok(frame)) => match frame.into_data() {
                        Ok(data) if data.is_empty() => continue,
                        Ok(data) => this.held = Some(data),
                        Err(frame) => return Poll::Ready(Some(Ok(frame))),
                    },
                    other => return Poll::Ready(other),
                }
            }
            let bucket = this.throttle.as_mut().expect("throttled");
            let held = this.held.as_mut().expect("held data");
            let n = ready!(bucket.poll_take(cx, held.len()));
            let chunk = held.split_to(n);
            if held.is_empty() {
                this.held = None;
            }
            return Poll::Ready(Some(Ok(Frame::data(chunk))));
        }
    }

    fn is_end_stream(&self) -> bool {
        if self.held.is_some() {
            return false;
        }
        match &self.source {
            Source::Stream(body) => body.is_end_stream(),
            Source::Whole(bytes) => bytes.is_none(),
            Source::Chain(prefix, rest) => prefix.is_empty() && rest.is_end_stream(),
        }
    }

    fn size_hint(&self) -> SizeHint {
        let held = self.held.as_ref().map_or(0, |b| b.len() as u64);
        let mut hint = match &self.source {
            Source::Stream(body) => body.size_hint(),
            Source::Whole(bytes) => SizeHint::with_exact(bytes.as_ref().map_or(0, |b| b.len() as u64)),
            Source::Chain(prefix, rest) => {
                let buffered: u64 = prefix.iter().map(|b| b.len() as u64).sum();
                let rest = rest.size_hint();
                let mut hint = SizeHint::new();
                hint.set_lower(rest.lower() + buffered);
                if let Some(upper) = rest.upper() {
                    hint.set_upper(upper + buffered);
                }
                hint
            }
        };
        if held > 0 {
            let lower = hint.lower() + held;
            let upper = hint.upper().map(|u| u + held);
            hint = SizeHint::new();
            hint.set_lower(lower);
            if let Some(upper) = upper {
                hint.set_upper(upper);
            }
        }
        hint
    }
}

/// A token bucket refilled at `rate` bytes a second, holding at most a
/// tenth of a second's worth (so throughput stays smooth).
struct Bucket {
    rate: u64,
    tokens: f64,
    capacity: f64,
    last: Instant,
    sleep: Option<Pin<Box<Sleep>>>,
}

impl Bucket {
    fn new(rate: u64) -> Bucket {
        let capacity = (rate as f64 / 10.0).max(1024.0);
        Bucket {
            rate,
            tokens: capacity,
            capacity,
            last: Instant::now(),
            sleep: None,
        }
    }

    /// Waits until at least one byte may go, then takes up to `want`.
    fn poll_take(&mut self, cx: &mut Context<'_>, want: usize) -> Poll<usize> {
        loop {
            if let Some(sleep) = &mut self.sleep {
                ready!(sleep.as_mut().poll(cx));
                self.sleep = None;
            }
            let now = Instant::now();
            let elapsed = now.duration_since(self.last).as_secs_f64();
            self.last = now;
            self.tokens = (self.tokens + elapsed * self.rate as f64).min(self.capacity);
            if self.tokens >= 1.0 {
                let n = (self.tokens as usize).min(want).max(1);
                self.tokens -= n as f64;
                return Poll::Ready(n);
            }
            let wait = (1.0 - self.tokens) / self.rate as f64;
            // Wake for a reasonable chunk rather than every byte.
            let wait = wait.max(0.01);
            self.sleep = Some(Box::pin(tokio::time::sleep(Duration::from_secs_f64(wait))));
        }
    }
}

/// A body read up to a cap.
pub enum Buffered {
    Whole(Bytes),
    /// Over the cap: what was read, and the rest still to come.
    Over(VecDeque<Bytes>, Incoming),
}

/// Reads `body` until it ends or passes `cap` bytes. Trailers are dropped.
pub async fn buffer(mut body: Incoming, cap: usize) -> Result<Buffered, hyper::Error> {
    let mut chunks = VecDeque::new();
    let mut len = 0usize;
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame?.into_data() {
            len += data.len();
            chunks.push_back(data);
            if len > cap {
                return Ok(Buffered::Over(chunks, body));
            }
        }
    }
    let mut whole = Vec::with_capacity(len);
    for chunk in chunks {
        whole.extend_from_slice(chunk.chunk());
    }
    Ok(Buffered::Whole(Bytes::from(whole)))
}
