//! Per-flow message logs: WebSocket messages and server-sent events. They
//! sit beside the flow detail rather than in it, so a long-lived connection
//! with many messages never makes loading its detail slow. Logs are in
//! memory; WebSocket payloads above a small size spill to the segment files.
//! A saved session's logs are rebuilt in memory, payloads inline, when a
//! flow is first inspected. An edited message (M19) keeps what arrived
//! beside what was forwarded.

use std::collections::{HashSet, VecDeque};
use std::io;

use parking_lot::RwLock;
use pm_model::{FlowId, SseEvent, SsePage, WsClose, WsDirection, WsMessage, WsOpcode, WsPage, WsQuery};

use super::blob::{BlobRef, CAPTURE_MAX};
use super::filter::contains_ignore_ascii_case;
use super::segments::SegmentStore;
use crate::io::sse::{ParsedEvent, SseParser};

/// Recorded bytes per WebSocket message; the rest is forwarded, not kept.
pub(crate) const PAYLOAD_MAX: usize = 1024 * 1024;
/// Payloads up to this size stay in memory.
const PAYLOAD_INLINE: usize = 1024;
/// Characters of a text payload shown in the message list.
const PREVIEW_CHARS: usize = 256;

pub(crate) struct WsLog {
    /// The flow, for the segment files' bookkeeping.
    owner: FlowId,
    inner: RwLock<WsInner>,
}

#[derive(Default)]
struct WsInner {
    records: VecDeque<WsRecord>,
    /// Messages dropped from the front by the limit; the first kept
    /// message has this index.
    dropped: u32,
    open: bool,
    close: Option<WsClose>,
    sent: u64,
    received: u64,
}

struct WsRecord {
    meta: WsMessage,
    payload: BlobRef,
    /// What arrived, for an edited message.
    original: Option<BlobRef>,
}

/// One message to record.
#[derive(Clone, Copy)]
pub(crate) struct WsEntry<'a> {
    pub direction: WsDirection,
    pub opcode: WsOpcode,
    /// The whole message as forwarded (as it arrived, for a dropped one);
    /// the close reason for a close frame.
    pub payload: &'a [u8],
    pub close_code: Option<u16>,
    /// What arrived, when a rule, script or breakpoint changed it.
    pub original: Option<&'a [u8]>,
    pub injected: bool,
    pub dropped: bool,
}

impl<'a> WsEntry<'a> {
    pub fn new(direction: WsDirection, opcode: WsOpcode, payload: &'a [u8]) -> WsEntry<'a> {
        WsEntry {
            direction,
            opcode,
            payload,
            close_code: None,
            original: None,
            injected: false,
            dropped: false,
        }
    }
}

impl WsLog {
    pub fn new(owner: FlowId) -> Self {
        WsLog {
            owner,
            inner: RwLock::new(WsInner {
                open: true,
                ..Default::default()
            }),
        }
    }

    /// Appends one message, dropping the oldest past `max` (0 keeps all).
    /// Payloads are cut at [`PAYLOAD_MAX`]; `len` is the full size.
    pub fn push(&self, segments: &SegmentStore, entry: WsEntry<'_>, time_ms: f64, max: usize) {
        let payload = entry.payload;
        let len = payload.len() as u64;
        let kept = &payload[..payload.len().min(PAYLOAD_MAX)];
        let preview = match entry.opcode {
            WsOpcode::Binary => String::new(),
            _ => preview(kept),
        };
        let mut truncated = kept.len() < payload.len();
        let stored = match self.store(segments, kept) {
            Some(blob) => blob,
            None => {
                truncated = true;
                BlobRef::Empty
            }
        };
        let original = entry.original.map(|o| {
            self.store(segments, &o[..o.len().min(PAYLOAD_MAX)])
                .unwrap_or(BlobRef::Empty)
        });
        let mut inner = self.inner.write();
        match entry.direction {
            WsDirection::Sent => inner.sent += len,
            WsDirection::Received => inner.received += len,
        }
        let index = inner.dropped + inner.records.len() as u32;
        if max > 0 {
            while inner.records.len() >= max {
                inner.records.pop_front();
                inner.dropped += 1;
            }
        }
        inner.records.push_back(WsRecord {
            meta: WsMessage {
                index,
                direction: entry.direction,
                opcode: entry.opcode,
                len,
                time_ms,
                truncated,
                close_code: entry.close_code,
                preview,
                edited: original.is_some(),
                injected: entry.injected,
                dropped: entry.dropped,
            },
            payload: stored,
            original,
        });
    }

    /// Keeps `bytes` inline or in the segment files; `None` if writing
    /// them failed.
    fn store(&self, segments: &SegmentStore, bytes: &[u8]) -> Option<BlobRef> {
        if bytes.is_empty() {
            Some(BlobRef::Empty)
        } else if bytes.len() <= PAYLOAD_INLINE {
            Some(BlobRef::Inline(bytes.to_vec()))
        } else {
            segments
                .append(self.owner, bytes)
                .ok()
                .map(|extent| BlobRef::Segments(vec![extent]))
        }
    }

    /// A finished log rebuilt from saved messages and their payloads, and
    /// the originals of edited messages by index.
    pub fn restore(
        messages: Vec<(WsMessage, Vec<u8>)>,
        mut originals: std::collections::HashMap<u32, Vec<u8>>,
        close: Option<WsClose>,
    ) -> Self {
        let mut inner = WsInner {
            open: false,
            close,
            // A log saved after the limit dropped its oldest messages.
            dropped: messages.first().map_or(0, |(m, _)| m.index),
            ..Default::default()
        };
        for (meta, payload) in messages {
            match meta.direction {
                WsDirection::Sent => inner.sent += meta.len,
                WsDirection::Received => inner.received += meta.len,
            }
            let inline = |bytes: Vec<u8>| {
                if bytes.is_empty() {
                    BlobRef::Empty
                } else {
                    BlobRef::Inline(bytes)
                }
            };
            let original = originals.remove(&meta.index).map(inline);
            inner.records.push_back(WsRecord {
                meta,
                payload: inline(payload),
                original,
            });
        }
        WsLog {
            owner: 0,
            inner: RwLock::new(inner),
        }
    }

    /// Payload bytes sent by the client and by the server so far.
    pub fn totals(&self) -> (u64, u64) {
        let inner = self.inner.read();
        (inner.sent, inner.received)
    }

    pub fn close(&self, close: Option<WsClose>) {
        let mut inner = self.inner.write();
        inner.open = false;
        inner.close = close;
    }

    pub fn page(&self, segments: Option<&SegmentStore>, query: &WsQuery, start: u32, count: u32) -> io::Result<WsPage> {
        let inner = self.inner.read();
        let needle = query.text.trim().to_ascii_lowercase().into_bytes();
        let window = start as usize..(start as usize).saturating_add(count as usize);
        let mut len = 0usize;
        let mut messages = Vec::new();
        if needle.is_empty() && query.direction.is_none() {
            len = inner.records.len();
            messages = inner
                .records
                .range(window.start.min(len)..window.end.min(len))
                .map(|r| r.meta.clone())
                .collect();
        } else {
            for record in &inner.records {
                if query.direction.is_some_and(|d| d != record.meta.direction) {
                    continue;
                }
                if !needle.is_empty() {
                    let spilled;
                    let bytes = match &record.payload {
                        BlobRef::Empty => &[][..],
                        BlobRef::Inline(bytes) => bytes.as_slice(),
                        blob => {
                            spilled = blob.read_from(segments)?;
                            spilled.as_slice()
                        }
                    };
                    if !contains_ignore_ascii_case(bytes, &needle) {
                        continue;
                    }
                }
                if window.contains(&len) {
                    messages.push(record.meta.clone());
                }
                len += 1;
            }
        }
        Ok(WsPage {
            total: inner.records.len() as u32,
            len: len as u32,
            start,
            messages,
            open: inner.open,
            close: inner.close.clone(),
            dropped: inner.dropped,
        })
    }

    /// The recorded payload of message `index` (gone if it was dropped).
    pub fn payload(&self, segments: Option<&SegmentStore>, index: u32) -> io::Result<Option<Vec<u8>>> {
        let inner = self.inner.read();
        let Some(pos) = index.checked_sub(inner.dropped) else {
            return Ok(None);
        };
        match inner.records.get(pos as usize) {
            Some(record) => record.payload.read_from(segments).map(Some),
            None => Ok(None),
        }
    }

    /// What arrived for message `index`, if a rule, script or breakpoint
    /// edited it.
    pub fn original(&self, segments: Option<&SegmentStore>, index: u32) -> io::Result<Option<Vec<u8>>> {
        let inner = self.inner.read();
        let Some(pos) = index.checked_sub(inner.dropped) else {
            return Ok(None);
        };
        match inner.records.get(pos as usize).and_then(|r| r.original.as_ref()) {
            Some(blob) => blob.read_from(segments).map(Some),
            None => Ok(None),
        }
    }

    /// Adds the segment files the kept payloads live in.
    pub fn segments_used(&self, out: &mut HashSet<u32>) {
        for record in &self.inner.read().records {
            record.payload.segments_used(out);
            if let Some(original) = &record.original {
                original.segments_used(out);
            }
        }
    }
}

/// The start of a text payload, for the list.
fn preview(bytes: &[u8]) -> String {
    let head = &bytes[..bytes.len().min(PREVIEW_CHARS * 4)];
    let text = match std::str::from_utf8(head) {
        Ok(text) => text,
        // Cut mid-character, or not text at all.
        Err(err) if err.error_len().is_none() => std::str::from_utf8(&head[..err.valid_up_to()]).unwrap_or(""),
        Err(_) => return String::new(),
    };
    text.chars().take(PREVIEW_CHARS).collect()
}

pub(crate) struct SseLog {
    inner: RwLock<SseInner>,
}

#[derive(Default)]
struct SseInner {
    parser: SseParser,
    events: Vec<SseEvent>,
    open: bool,
    /// Data bytes kept so far; parsing stops at the body capture cap.
    kept: u64,
}

impl SseLog {
    pub fn new() -> Self {
        SseLog {
            inner: RwLock::new(SseInner {
                open: true,
                ..Default::default()
            }),
        }
    }

    /// A finished log of saved events.
    pub fn restore(events: Vec<SseEvent>) -> Self {
        SseLog {
            inner: RwLock::new(SseInner {
                events,
                ..Default::default()
            }),
        }
    }

    /// Every event so far, for saving.
    pub fn events(&self) -> Vec<SseEvent> {
        self.inner.read().events.clone()
    }

    /// Parses one body chunk; events it completes are stamped `time_ms`.
    pub fn push(&self, chunk: &[u8], time_ms: f64) {
        let mut inner = self.inner.write();
        if inner.kept >= CAPTURE_MAX {
            return;
        }
        let SseInner {
            parser, events, kept, ..
        } = &mut *inner;
        parser.push(chunk, &mut |event| {
            *kept += event.data.len() as u64;
            let index = events.len() as u32;
            events.push(sse_event(index, Some(time_ms), event));
        });
    }

    /// The response ended. An event without its terminating blank line is
    /// dropped, as a browser would.
    pub fn finish(&self) {
        self.inner.write().open = false;
    }

    pub fn page(&self, text: &str, start: u32, count: u32) -> SsePage {
        let inner = self.inner.read();
        page_events(&inner.events, text, start, count, inner.open)
    }
}

pub(crate) fn sse_event(index: u32, time_ms: Option<f64>, event: ParsedEvent) -> SseEvent {
    SseEvent {
        index,
        time_ms,
        id: event.id,
        event: event.event,
        data: event.data,
        retry: event.retry,
        truncated: event.truncated,
    }
}

/// The window `start..start + count` of the events matching `text` (in
/// the data, the event type or the id, ignoring ASCII case).
pub(crate) fn page_events(events: &[SseEvent], text: &str, start: u32, count: u32, open: bool) -> SsePage {
    let needle = text.trim().to_ascii_lowercase().into_bytes();
    let window = start as usize..(start as usize).saturating_add(count as usize);
    let matches = |e: &&SseEvent| {
        needle.is_empty()
            || [Some(e.data.as_str()), e.event.as_deref(), e.id.as_deref()]
                .into_iter()
                .flatten()
                .any(|s| contains_ignore_ascii_case(s.as_bytes(), &needle))
    };
    let mut len = 0usize;
    let mut page = Vec::new();
    for event in events.iter().filter(matches) {
        if window.contains(&len) {
            page.push(event.clone());
        }
        len += 1;
    }
    SsePage {
        total: events.len() as u32,
        len: len as u32,
        start,
        events: page,
        open,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segments() -> (SegmentStore, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("pm-messages-test-{}", uuid::Uuid::new_v4()));
        (SegmentStore::new(&root), root)
    }

    #[test]
    fn ws_pages_filter_by_direction_and_text() {
        let (segs, root) = segments();
        let log = WsLog::new(0);
        log.push(&segs, WsEntry::new(WsDirection::Sent, WsOpcode::Text, b"hello"), 1.0, 0);
        log.push(
            &segs,
            WsEntry::new(WsDirection::Received, WsOpcode::Text, b"HELLO back"),
            2.0,
            0,
        );
        let big: Vec<u8> = b"needle ".iter().copied().cycle().take(5_000).collect();
        log.push(
            &segs,
            WsEntry::new(WsDirection::Received, WsOpcode::Binary, &big),
            3.0,
            0,
        );
        let close = WsEntry {
            close_code: Some(1000),
            ..WsEntry::new(WsDirection::Sent, WsOpcode::Close, b"bye")
        };
        log.push(&segs, close, 4.0, 0);

        let all = log.page(Some(&segs), &WsQuery::default(), 1, 2).unwrap();
        assert_eq!((all.total, all.len), (4, 4));
        assert_eq!(all.messages.iter().map(|m| m.index).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(all.messages[1].preview, "");

        let received = WsQuery {
            direction: Some(WsDirection::Received),
            text: "hello".into(),
        };
        let page = log.page(Some(&segs), &received, 0, 10).unwrap();
        assert_eq!(page.messages.iter().map(|m| m.index).collect::<Vec<_>>(), vec![1]);

        // Spilled payloads are searched too.
        let spilled = WsQuery {
            direction: None,
            text: "NEEDLE".into(),
        };
        assert_eq!(log.page(Some(&segs), &spilled, 0, 10).unwrap().len, 1);
        assert_eq!(log.payload(Some(&segs), 2).unwrap().unwrap(), big);
        assert_eq!(log.totals(), (8, 5_010));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn payloads_are_capped() {
        let (segs, root) = segments();
        let log = WsLog::new(0);
        let big = vec![1u8; PAYLOAD_MAX + 1];
        log.push(&segs, WsEntry::new(WsDirection::Sent, WsOpcode::Binary, &big), 1.0, 0);
        let page = log.page(Some(&segs), &WsQuery::default(), 0, 1).unwrap();
        assert!(page.messages[0].truncated);
        assert_eq!(page.messages[0].len, PAYLOAD_MAX as u64 + 1);
        assert_eq!(log.payload(Some(&segs), 0).unwrap().unwrap().len(), PAYLOAD_MAX);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_oldest_messages_go_past_the_limit() {
        let (segs, root) = segments();
        let log = WsLog::new(0);
        for i in 0..10u8 {
            log.push(
                &segs,
                WsEntry::new(WsDirection::Sent, WsOpcode::Text, &[b'a' + i]),
                1.0,
                4,
            );
        }
        let page = log.page(Some(&segs), &WsQuery::default(), 0, 10).unwrap();
        assert_eq!((page.total, page.len, page.dropped), (4, 4, 6));
        assert_eq!(
            page.messages.iter().map(|m| m.index).collect::<Vec<_>>(),
            vec![6, 7, 8, 9]
        );
        assert_eq!(log.payload(Some(&segs), 9).unwrap().unwrap(), b"j");
        assert_eq!(log.payload(Some(&segs), 2).unwrap(), None);
        // Totals still count every message.
        assert_eq!(log.totals(), (10, 0));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn edited_messages_keep_their_original() {
        let (segs, root) = segments();
        let log = WsLog::new(0);
        let big = vec![b'x'; 4_000];
        let edited = WsEntry {
            original: Some(&big),
            ..WsEntry::new(WsDirection::Sent, WsOpcode::Text, b"short")
        };
        log.push(&segs, edited, 1.0, 0);
        let dropped = WsEntry {
            dropped: true,
            ..WsEntry::new(WsDirection::Received, WsOpcode::Text, b"gone")
        };
        log.push(&segs, dropped, 2.0, 0);
        let injected = WsEntry {
            injected: true,
            ..WsEntry::new(WsDirection::Received, WsOpcode::Binary, b"\x01")
        };
        log.push(&segs, injected, 3.0, 0);
        let page = log.page(Some(&segs), &WsQuery::default(), 0, 10).unwrap();
        let flags: Vec<_> = page
            .messages
            .iter()
            .map(|m| (m.edited, m.dropped, m.injected))
            .collect();
        assert_eq!(
            flags,
            vec![(true, false, false), (false, true, false), (false, false, true)]
        );
        assert_eq!(log.payload(Some(&segs), 0).unwrap().unwrap(), b"short");
        assert_eq!(log.original(Some(&segs), 0).unwrap().unwrap(), big);
        assert_eq!(log.original(Some(&segs), 1).unwrap(), None);
        let mut used = HashSet::new();
        log.segments_used(&mut used);
        assert_eq!(used.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn previews_stop_at_character_boundaries() {
        let text = "é".repeat(PREVIEW_CHARS * 2);
        assert_eq!(preview(text.as_bytes()).chars().count(), PREVIEW_CHARS);
        assert_eq!(preview(&[0xff, 0xfe]), "");
    }

    #[test]
    fn sse_events_arrive_as_they_complete() {
        let log = SseLog::new();
        log.push(b"data: one\n\nda", 1.0);
        assert_eq!(log.page("", 0, 10).total, 1);
        log.push(b"ta: two\nevent: tick\n\n", 2.0);
        let page = log.page("TICK", 0, 10);
        assert_eq!((page.total, page.len), (2, 1));
        assert_eq!(page.events[0].data, "two");
        assert_eq!(page.events[0].time_ms, Some(2.0));
        assert!(page.open);
        log.finish();
        assert!(!log.page("", 0, 0).open);
    }
}
