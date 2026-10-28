//! A body wrapper that hands every data frame (and trailers) to the flow
//! recorder as it passes through. Frames are forwarded unchanged and immediately, so
//! streaming responses (SSE, LLM tokens) stay live end to end.

use std::pin::Pin;
use std::task::{Context, Poll, ready};

use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};
use pm_model::BodyPart;

use crate::recorder::{BodyEnd, FlowRecorder};

// The `Body` bound lets `Drop` ask whether the stream was finished.
pub struct TeeBody<B: Body> {
    inner: B,
    tap: Option<Tap>,
}

struct Tap {
    recorder: FlowRecorder,
    part: BodyPart,
}

impl<B: Body> TeeBody<B> {
    /// Tees `inner` into `recorder`; with `None` it is a plain passthrough.
    pub fn new(inner: B, recorder: Option<FlowRecorder>, part: BodyPart) -> Self {
        TeeBody {
            inner,
            tap: recorder.map(|recorder| Tap { recorder, part }),
        }
    }

    fn end(&mut self, end: BodyEnd) {
        if let Some(tap) = self.tap.take() {
            tap.recorder.body_end(tap.part, end);
        }
    }
}

impl<B> Body for TeeBody<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: std::fmt::Display,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, B::Error>>> {
        let this = &mut *self;
        let frame = ready!(Pin::new(&mut this.inner).poll_frame(cx));
        match &frame {
            Some(Ok(frame)) => {
                if let Some(tap) = &this.tap {
                    if let Some(data) = frame.data_ref() {
                        tap.recorder.body_data(tap.part, data);
                    } else if let Some(trailers) = frame.trailers_ref() {
                        tap.recorder.trailers(tap.part, trailers);
                    }
                }
            }
            Some(Err(err)) => this.end(BodyEnd::Error(err.to_string())),
            None => this.end(BodyEnd::Complete),
        }
        Poll::Ready(frame)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl<B: Body> Drop for TeeBody<B> {
    fn drop(&mut self) {
        // hyper may drop a body without polling its end once it knows the
        // stream is over (empty bodies, HEAD and 204 responses).
        let end = if self.inner.is_end_stream() {
            BodyEnd::Complete
        } else {
            BodyEnd::Aborted
        };
        self.end(end);
    }
}
