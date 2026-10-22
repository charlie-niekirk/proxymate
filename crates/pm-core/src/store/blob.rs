//! Body capture. A body is tee'd chunk by chunk, still content-encoded:
//! small bodies stay inline in the flow detail, larger ones spill to the
//! segment files in bounded chunks, and anything past the capture cap is
//! counted but not kept. None of this can slow or alter forwarding.

use std::io;

use pm_model::FlowId;
use serde::{Deserialize, Serialize};

use super::segments::{Extent, SegmentStore};
use crate::io::content::SNIFF_LEN;

/// Bodies up to this size are stored inside the flow detail.
pub(crate) const INLINE_MAX: usize = 8 * 1024;
/// Recorded bytes per body; the rest is forwarded but not kept.
pub(crate) const CAPTURE_MAX: u64 = 64 * 1024 * 1024;
/// A spilling body is written out in chunks of this size, which bounds the
/// memory a streaming body holds.
const SPILL_CHUNK: usize = 256 * 1024;

/// Where a captured body's bytes live.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum BlobRef {
    #[default]
    Empty,
    Inline(Vec<u8>),
    Segments(Vec<Extent>),
}

impl BlobRef {
    /// Like [`BlobRef::read`], for stores that may have no segment files
    /// (a saved session keeps everything inline).
    pub fn read_from(&self, segments: Option<&SegmentStore>) -> io::Result<Vec<u8>> {
        match (self, segments) {
            (_, Some(segments)) => self.read(segments),
            (BlobRef::Segments(_), None) => Err(io::Error::other("this store has no segment files")),
            (BlobRef::Empty, None) => Ok(Vec::new()),
            (BlobRef::Inline(bytes), None) => Ok(bytes.clone()),
        }
    }

    /// Adds the segment files this blob lives in.
    pub fn segments_used(&self, out: &mut std::collections::HashSet<u32>) {
        if let BlobRef::Segments(extents) = self {
            out.extend(extents.iter().map(|e| e.seg));
        }
    }

    pub fn read(&self, segments: &SegmentStore) -> io::Result<Vec<u8>> {
        match self {
            BlobRef::Empty => Ok(Vec::new()),
            BlobRef::Inline(bytes) => Ok(bytes.clone()),
            BlobRef::Segments(extents) => {
                let mut out = Vec::with_capacity(extents.iter().map(|e| e.len as usize).sum());
                segments.read_into(extents, &mut out)?;
                Ok(out)
            }
        }
    }
}

/// One body being captured.
#[derive(Debug, Default)]
pub(crate) struct Capture {
    /// The flow the body belongs to; segment files track their owners.
    owner: FlowId,
    /// Bytes not yet spilled.
    buf: Vec<u8>,
    extents: Vec<Extent>,
    /// Bytes seen on the wire.
    pub size: u64,
    /// Bytes kept.
    pub captured: u64,
    /// Bytes were dropped: over the cap, or a segment write failed.
    pub truncated: bool,
    /// The first [`SNIFF_LEN`] bytes, kept in memory for content detection.
    pub head: Vec<u8>,
}

impl Capture {
    pub fn new(owner: FlowId) -> Self {
        Capture {
            owner,
            ..Default::default()
        }
    }

    pub fn push(&mut self, data: &[u8], segments: &SegmentStore) {
        self.size += data.len() as u64;
        if self.truncated {
            return;
        }
        let room = (CAPTURE_MAX - self.captured) as usize;
        let keep = &data[..data.len().min(room)];
        if keep.len() < data.len() {
            self.truncated = true;
        }
        if self.head.len() < SNIFF_LEN {
            let n = keep.len().min(SNIFF_LEN - self.head.len());
            self.head.extend_from_slice(&keep[..n]);
        }
        self.buf.extend_from_slice(keep);
        self.captured += keep.len() as u64;
        if self.buf.len() >= SPILL_CHUNK {
            self.spill(segments);
        }
    }

    /// Writes out what is left and returns where the body lives.
    pub fn finish(&mut self, segments: &SegmentStore) -> BlobRef {
        self.head = Vec::new();
        if self.extents.is_empty() {
            if self.buf.is_empty() {
                return BlobRef::Empty;
            }
            if self.buf.len() <= INLINE_MAX {
                return BlobRef::Inline(std::mem::take(&mut self.buf));
            }
        }
        self.spill(segments);
        BlobRef::Segments(std::mem::take(&mut self.extents))
    }

    /// Adds the segment files spilled to so far.
    pub fn segments_used(&self, out: &mut std::collections::HashSet<u32>) {
        out.extend(self.extents.iter().map(|e| e.seg));
    }

    /// The bytes captured so far, for inspecting a body still streaming.
    pub fn snapshot(&self, segments: &SegmentStore) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(self.captured as usize);
        segments.read_into(&self.extents, &mut out)?;
        out.extend_from_slice(&self.buf);
        Ok(out)
    }

    fn spill(&mut self, segments: &SegmentStore) {
        if self.buf.is_empty() {
            return;
        }
        match segments.append(self.owner, &self.buf) {
            Ok(extent) => self.extents.push(extent),
            Err(_) => {
                // Keep forwarding; the recording just ends here.
                self.truncated = true;
                self.captured -= self.buf.len() as u64;
            }
        }
        self.buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segments() -> (SegmentStore, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("pm-blob-test-{}", uuid::Uuid::new_v4()));
        (SegmentStore::new(&root), root)
    }

    #[test]
    fn small_bodies_stay_inline() {
        let (segs, root) = segments();
        let mut capture = Capture::default();
        capture.push(b"{\"ok\":", &segs);
        capture.push(b"true}", &segs);
        assert_eq!(capture.head, b"{\"ok\":true}");
        assert_eq!(capture.finish(&segs), BlobRef::Inline(b"{\"ok\":true}".to_vec()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn large_bodies_spill_and_read_back_in_order() {
        let (segs, root) = segments();
        let mut capture = Capture::default();
        let body: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
        for chunk in body.chunks(10_000) {
            capture.push(chunk, &segs);
        }
        assert_eq!(capture.snapshot(&segs).unwrap(), body);
        assert_eq!(capture.head, &body[..SNIFF_LEN]);
        let blob = capture.finish(&segs);
        assert!(matches!(&blob, BlobRef::Segments(e) if e.len() == 4));
        assert_eq!(blob.read(&segs).unwrap(), body);
        assert_eq!(
            (capture.size, capture.captured, capture.truncated),
            (1_000_000, 1_000_000, false)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bodies_over_the_cap_are_truncated_but_counted() {
        let (segs, root) = segments();
        let mut capture = Capture::default();
        let chunk = vec![7u8; 1024 * 1024];
        for _ in 0..(CAPTURE_MAX / chunk.len() as u64 + 2) {
            capture.push(&chunk, &segs);
        }
        assert!(capture.truncated);
        assert_eq!(capture.captured, CAPTURE_MAX);
        assert_eq!(capture.size, CAPTURE_MAX + 2 * 1024 * 1024);
        assert_eq!(capture.finish(&segs).read(&segs).unwrap().len() as u64, CAPTURE_MAX);
        let _ = std::fs::remove_dir_all(root);
    }
}
