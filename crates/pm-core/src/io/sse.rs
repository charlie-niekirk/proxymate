//! Incremental `text/event-stream` parsing, as the HTML standard defines
//! it. Fed body chunks as they stream through the tee, so events show up
//! while the response is still open.

/// `data` past this is dropped from the recorded event.
pub const DATA_MAX: usize = 1024 * 1024;
/// A single line past this is cut (a stream that never sends a newline).
const LINE_MAX: usize = DATA_MAX + 1024;

/// One dispatched event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedEvent {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: String,
    pub retry: Option<u32>,
    pub truncated: bool,
}

/// Turns stream bytes into events. Lines may end in CRLF, LF or CR, and a
/// line or a CRLF pair may be split across chunks.
#[derive(Debug, Default)]
pub struct SseParser {
    line: Vec<u8>,
    line_cut: bool,
    /// The previous chunk ended in CR, so a leading LF belongs to it.
    after_cr: bool,
    started: bool,
    /// The first bytes of the stream, while they may still be a split BOM.
    head: Vec<u8>,
    /// The event being built.
    current: ParsedEvent,
    /// Set once a `data` field was seen: an event without one isn't dispatched.
    has_data: bool,
}

impl SseParser {
    /// Parses `chunk`, calling `emit` for each event it completes.
    pub fn push(&mut self, chunk: &[u8], emit: &mut impl FnMut(ParsedEvent)) {
        if self.started {
            return self.push_lines(chunk, emit);
        }
        // A leading UTF-8 BOM is not part of the first line; it may arrive
        // split across chunks.
        const BOM: &[u8] = b"\xEF\xBB\xBF";
        self.head.extend_from_slice(chunk);
        if self.head.len() < BOM.len() && BOM.starts_with(&self.head) {
            return;
        }
        self.started = true;
        let head = std::mem::take(&mut self.head);
        self.push_lines(head.strip_prefix(BOM).unwrap_or(&head), emit);
    }

    fn push_lines(&mut self, mut chunk: &[u8], emit: &mut impl FnMut(ParsedEvent)) {
        if self.after_cr {
            self.after_cr = false;
            chunk = chunk.strip_prefix(b"\n").unwrap_or(chunk);
        }
        while let Some(end) = chunk.iter().position(|&b| b == b'\n' || b == b'\r') {
            self.extend_line(&chunk[..end]);
            let cr = chunk[end] == b'\r';
            chunk = &chunk[end + 1..];
            if cr {
                match chunk.first() {
                    Some(b'\n') => chunk = &chunk[1..],
                    None => self.after_cr = true,
                    Some(_) => {}
                }
            }
            let line = std::mem::take(&mut self.line);
            self.line_cut = false;
            self.process_line(&line, emit);
        }
        self.extend_line(chunk);
    }

    fn extend_line(&mut self, bytes: &[u8]) {
        let room = LINE_MAX.saturating_sub(self.line.len());
        if bytes.len() > room {
            self.line_cut = true;
            self.current.truncated = true;
        }
        self.line.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }

    fn process_line(&mut self, line: &[u8], emit: &mut impl FnMut(ParsedEvent)) {
        if line.is_empty() {
            self.dispatch(emit);
            return;
        }
        if line[0] == b':' {
            return;
        }
        let (field, value) = match line.iter().position(|&b| b == b':') {
            Some(colon) => {
                let value = &line[colon + 1..];
                (&line[..colon], value.strip_prefix(b" ").unwrap_or(value))
            }
            None => (line, &[][..]),
        };
        let value = String::from_utf8_lossy(value);
        match field {
            b"event" => self.current.event = Some(value.into_owned()),
            b"data" => {
                let data = &mut self.current.data;
                if self.has_data {
                    data.push('\n');
                }
                self.has_data = true;
                let room = DATA_MAX.saturating_sub(data.len());
                if value.len() > room {
                    self.current.truncated = true;
                    let mut cut = room;
                    while !value.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    data.push_str(&value[..cut]);
                } else {
                    data.push_str(&value);
                }
            }
            b"id" if !value.contains('\0') => self.current.id = Some(value.into_owned()),
            b"retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                self.current.retry = value.parse().ok();
            }
            _ => {}
        }
    }

    fn dispatch(&mut self, emit: &mut impl FnMut(ParsedEvent)) {
        let event = std::mem::take(&mut self.current);
        if std::mem::take(&mut self.has_data) {
            emit(event);
        }
    }
}

/// Every complete event of a whole stream.
pub fn parse_all(body: &[u8]) -> Vec<ParsedEvent> {
    let mut events = Vec::new();
    SseParser::default().push(body, &mut |e| events.push(e));
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(events: &[ParsedEvent]) -> Vec<&str> {
        events.iter().map(|e| e.data.as_str()).collect()
    }

    #[test]
    fn parses_fields_and_skips_comments() {
        let events = parse_all(
            b": keep-alive\n\nid: 7\nevent: delta\ndata: {\"a\":1}\nretry: 1500\n\ndata:no space\ndata:  two\n\n",
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id.as_deref(), Some("7"));
        assert_eq!(events[0].event.as_deref(), Some("delta"));
        assert_eq!(events[0].data, r#"{"a":1}"#);
        assert_eq!(events[0].retry, Some(1500));
        assert_eq!(events[1].data, "no space\n two");
        assert_eq!(events[1].event, None);
    }

    #[test]
    fn events_without_data_are_not_dispatched() {
        let events = parse_all(b"id: 1\n\nevent: x\n\ndata\n\n");
        assert_eq!(data(&events), vec![""]);
        assert_eq!(events[0].id, None);
    }

    #[test]
    fn handles_any_line_ending_split_anywhere() {
        let stream = b"\xEF\xBB\xBFdata: a\r\n\r\ndata: b\r\rdata: c\n\ndata: unterminated";
        for split in 0..stream.len() {
            let mut events = Vec::new();
            let mut parser = SseParser::default();
            for part in [&stream[..split], &stream[split..]] {
                parser.push(part, &mut |e| events.push(e));
            }
            assert_eq!(data(&events), vec!["a", "b", "c"], "split at {split}");
        }
    }

    #[test]
    fn huge_data_is_capped() {
        let mut stream = b"data: ".to_vec();
        stream.extend(std::iter::repeat_n(b'x', DATA_MAX + 10));
        stream.extend_from_slice(b"\n\n");
        let events = parse_all(&stream);
        assert_eq!(events[0].data.len(), DATA_MAX);
        assert!(events[0].truncated);
    }
}
