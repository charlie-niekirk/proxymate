//! Stream adapters for the CONNECT path.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use bytes::{Buf, Bytes};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Replays `prefix` before reading from `inner`: lets us look at the first
/// bytes of a tunnel (TLS or not?) without losing them.
pub struct Prefixed<S> {
    prefix: Bytes,
    inner: S,
}

impl<S> Prefixed<S> {
    pub fn new(prefix: Bytes, inner: S) -> Self {
        Prefixed { prefix, inner }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Prefixed<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        if !self.prefix.is_empty() {
            let n = self.prefix.len().min(buf.remaining());
            buf.put_slice(&self.prefix[..n]);
            self.prefix.advance(n);
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Prefixed<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

/// Bytes read from and written to a stream, shared with whoever watches it.
#[derive(Debug, Default)]
pub struct Counters {
    pub read: AtomicU64,
    pub written: AtomicU64,
}

impl Counters {
    pub fn read(&self) -> u64 {
        self.read.load(Ordering::Relaxed)
    }

    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }
}

/// Counts the bytes going through `inner`.
pub struct Counting<S> {
    inner: S,
    counters: Arc<Counters>,
}

impl<S> Counting<S> {
    pub fn new(inner: S, counters: Arc<Counters>) -> Self {
        Counting { inner, counters }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Counting<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        let n = (buf.filled().len() - before) as u64;
        self.counters.read.fetch_add(n, Ordering::Relaxed);
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Counting<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = result {
            self.counters.written.fetch_add(n as u64, Ordering::Relaxed);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        if let Poll::Ready(Ok(n)) = result {
            self.counters.written.fetch_add(n as u64, Ordering::Relaxed);
        }
        result
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn prefix_is_read_first_and_bytes_are_counted() {
        let (a, mut b) = tokio::io::duplex(64);
        let counters = Arc::new(Counters::default());
        let mut stream = Counting::new(Prefixed::new(Bytes::from_static(b"he"), a), counters.clone());
        b.write_all(b"llo").await.unwrap();
        drop(b);
        let mut out = String::new();
        stream.read_to_string(&mut out).await.unwrap();
        assert_eq!(out, "hello");
        assert_eq!(counters.read(), 5);
    }
}
