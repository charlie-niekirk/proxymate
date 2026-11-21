//! The WebSocket and SSE panels' logic (`lib/stream.ts` and the pure parts
//! of `WebSocketPanel.tsx` and `SsePanel.tsx`): close codes, the header's
//! status and count lines, a row's preview and flags, the composer's and
//! Resend's messages, the message detail's lines, the rows' menus and an
//! event's fields. Which panel a flow gets is [`crate::inspector::stream_kind`].

use pm_model::{SseEvent, WsClose, WsDirection, WsMessage, WsOpcode, WsSend};

use crate::copy_formats::{base64, sse_wire};
use crate::flow::{format_size, format_time};

/// Rows a list page holds (`usePagedList`'s `LIST_PAGE`): a page is
/// fetched once full; only the tail page refetches as the list grows.
pub const LIST_PAGE: u32 = 128;

/// The page holding row `index`.
pub fn page_of(index: usize) -> u32 {
    (index / LIST_PAGE as usize) as u32
}

const CLOSE_NAMES: [(u16, &str); 14] = [
    (1000, "Normal Closure"),
    (1001, "Going Away"),
    (1002, "Protocol Error"),
    (1003, "Unsupported Data"),
    (1005, "No Status"),
    (1006, "Abnormal Closure"),
    (1007, "Invalid Payload"),
    (1008, "Policy Violation"),
    (1009, "Message Too Big"),
    (1010, "Missing Extension"),
    (1011, "Internal Error"),
    (1012, "Service Restart"),
    (1013, "Try Again Later"),
    (1015, "TLS Handshake"),
];

/// `1000 Normal Closure`, or the bare code for application codes.
pub fn close_code_label(code: u16) -> String {
    match CLOSE_NAMES.iter().find(|(c, _)| *c == code) {
        Some((_, name)) => format!("{code} {name}"),
        None => code.to_string(),
    }
}

/// One line for the panel header: who closed, how and why.
pub fn describe_close(close: &WsClose) -> String {
    let by = match close.by {
        WsDirection::Sent => "client",
        WsDirection::Received => "server",
    };
    let mut parts = vec![format!("Closed by {by}"), close_code_label(close.code)];
    if !close.reason.is_empty() {
        parts.push(close.reason.clone());
    }
    parts.join(" · ")
}

/// The WebSocket header's status: Open, how it closed, or Closed.
pub fn ws_status(open: bool, close: Option<&WsClose>) -> String {
    match (open, close) {
        (true, _) => "Open".into(),
        (false, Some(close)) => describe_close(close),
        (false, None) => "Closed".into(),
    }
}

/// `12 messages`, or `3 of 12 messages` while a filter hides some.
pub fn count_label(len: u32, total: u32, noun: &str) -> String {
    if len == total {
        format!("{total} {noun}")
    } else {
        format!("{len} of {total} {noun}")
    }
}

pub fn opcode_label(opcode: WsOpcode) -> &'static str {
    match opcode {
        WsOpcode::Text => "Text",
        WsOpcode::Binary => "Binary",
        WsOpcode::Ping => "Ping",
        WsOpcode::Pong => "Pong",
        WsOpcode::Close => "Close",
    }
}

/// Bytes from hex text (`de ad be ef`, `0xdead`, any spacing), or `None`
/// if it isn't hex.
pub fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let lower = text.to_ascii_lowercase().replace("0x", "");
    let digits: Vec<u8> = lower
        .bytes()
        .filter(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b',' | b':'))
        .collect();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            Some((hi * 16 + lo) as u8)
        })
        .collect()
}

/// How the composer reads what was typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeMode {
    Text,
    Hex,
}

/// What the composer says when its hex isn't hex.
pub const NOT_HEX: &str = "That isn't hex: use pairs of digits like 0a ff 3c.";

/// A message for `Engine::ws_send` from the composer, or why it can't go.
pub fn ws_send_of(direction: WsDirection, mode: ComposeMode, input: &str) -> Result<WsSend, &'static str> {
    match mode {
        ComposeMode::Text => Ok(WsSend {
            direction,
            payload: input.to_owned(),
            binary: false,
        }),
        ComposeMode::Hex => {
            let bytes = parse_hex(input).ok_or(NOT_HEX)?;
            Ok(WsSend {
                direction,
                payload: base64(&bytes),
                binary: true,
            })
        }
    }
}

/// Resend: the recorded message again, its payload as it was forwarded.
pub fn resend_of(direction: WsDirection, opcode: WsOpcode, payload: &[u8]) -> WsSend {
    match opcode {
        WsOpcode::Text => WsSend {
            direction,
            payload: String::from_utf8_lossy(payload).into_owned(),
            binary: false,
        },
        _ => WsSend {
            direction,
            payload: base64(payload),
            binary: true,
        },
    }
}

/// True for messages Resend can send again.
pub fn can_resend(m: &WsMessage) -> bool {
    matches!(m.opcode, WsOpcode::Text | WsOpcode::Binary) && !m.truncated
}

/// "Resend to the server" or "Resend to the client".
pub fn resend_label(direction: WsDirection) -> &'static str {
    match direction {
        WsDirection::Sent => "Resend to the server",
        WsDirection::Received => "Resend to the client",
    }
}

/// A badge on a message row (M19).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Injected,
    Edited,
    Dropped,
}

impl Flag {
    pub fn label(self) -> &'static str {
        match self {
            Flag::Injected => "Injected",
            Flag::Edited => "Edited",
            Flag::Dropped => "Dropped",
        }
    }

    /// The badge's tooltip.
    pub fn title(self) -> &'static str {
        match self {
            Flag::Injected => "Sent by Proxymate (the composer, Resend or an agent)",
            Flag::Edited => "A rule, script or breakpoint changed it; the payload is what was forwarded",
            Flag::Dropped => "A rule, script or breakpoint dropped it: it was never forwarded",
        }
    }
}

/// The badges a message row shows.
pub fn message_flags(m: &WsMessage) -> Vec<Flag> {
    let mut out = Vec::new();
    if m.injected {
        out.push(Flag::Injected);
    }
    if m.edited {
        out.push(Flag::Edited);
    }
    if m.dropped {
        out.push(Flag::Dropped);
    }
    out
}

/// A close frame's code and reason: `1000 Normal Closure · bye`.
pub fn close_text(m: &WsMessage) -> String {
    let mut parts = Vec::new();
    if let Some(code) = m.close_code {
        parts.push(close_code_label(code));
    }
    if !m.preview.is_empty() {
        parts.push(m.preview.clone());
    }
    parts.join(" · ")
}

/// A row's Payload column.
pub fn row_preview(m: &WsMessage) -> String {
    match m.opcode {
        WsOpcode::Close => close_text(m),
        WsOpcode::Binary => format!("Binary data · {} bytes", m.len),
        _ => m.preview.clone(),
    }
}

/// A row's Size column: "0 B" rather than the table's dash.
pub fn size_label(len: u64) -> String {
    match len {
        0 => "0 B".into(),
        n => format_size(n),
    }
}

/// A row's text for assistive technology.
pub fn row_label(m: &WsMessage) -> String {
    let way = match m.direction {
        WsDirection::Sent => "Sent",
        WsDirection::Received => "Received",
    };
    let mut label = format!(
        "{way} {} message {}, {}",
        opcode_label(m.opcode),
        m.index + 1,
        size_label(m.len)
    );
    for flag in message_flags(m) {
        label.push_str(", ");
        label.push_str(flag.label());
    }
    label
}

/// The message detail's lines: who sent it, its type, size and time; its
/// close code, a truncation and a drop. `recorded` is the payload's
/// recorded length, once known.
pub fn detail_lines(m: &WsMessage, recorded: Option<usize>) -> Vec<String> {
    let who = match (m.injected, m.direction) {
        (true, WsDirection::Sent) => "Sent to the server by Proxymate",
        (true, WsDirection::Received) => "Sent to the client by Proxymate",
        (false, WsDirection::Sent) => "Sent by the client",
        (false, WsDirection::Received) => "Received from the server",
    };
    let mut lines = vec![format!(
        "{who} · {} · {} bytes · {}",
        opcode_label(m.opcode),
        m.len,
        format_time(Some(m.time_ms))
    )];
    if let Some(code) = m.close_code {
        lines.push(format!("Close code {}", close_code_label(code)));
    }
    if m.truncated {
        lines.push(format!(
            "Recorded the first {} of {}",
            format_size(recorded.unwrap_or(0) as u64),
            format_size(m.len)
        ));
    }
    if m.dropped {
        lines.push("Dropped by a rule, script or breakpoint: it was never forwarded.".into());
    }
    lines
}

/// One entry of a stream row's menu: a label and what it copies, or an
/// action the panel runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    /// Copies the text now.
    Copy {
        label: &'static str,
        text: String,
        what: &'static str,
    },
    /// Copies the payload in `format` once it is fetched.
    CopyPayload {
        label: &'static str,
        format: PayloadFormat,
        enabled: bool,
    },
    Separator,
    Resend(WsDirection),
}

/// How a payload is copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PayloadFormat {
    /// UTF-8 text; refused for binary bytes.
    Text,
    Base64,
    Hex,
}

impl PayloadFormat {
    /// The toast's name for what was copied.
    pub fn what(self) -> &'static str {
        match self {
            PayloadFormat::Text => "payload",
            PayloadFormat::Base64 => "payload as Base64",
            PayloadFormat::Hex => "payload as hex",
        }
    }
}

/// What "Copy payload" refuses binary bytes with.
pub const NOT_TEXT: &str = "The payload isn't UTF-8 text: copy it as Base64 or hex instead";

/// The payload in `format`, or why it can't be.
pub fn payload_as(bytes: &[u8], format: PayloadFormat) -> Result<String, &'static str> {
    match format {
        PayloadFormat::Text => crate::http::body_text(bytes).map(str::to_owned).ok_or(NOT_TEXT),
        PayloadFormat::Base64 => Ok(base64(bytes)),
        PayloadFormat::Hex => Ok(crate::copy_formats::hex_bytes(bytes)),
    }
}

/// A message row's menu (`MessageItems`); payloads are fetched whole (the
/// row only has a preview).
pub fn ws_menu(m: &WsMessage, sendable: bool) -> Vec<MenuItem> {
    let empty = m.len == 0;
    let mut items = vec![match m.opcode {
        WsOpcode::Close => MenuItem::Copy {
            label: "Copy close code and reason",
            text: close_text(m),
            what: "close reason",
        },
        _ => MenuItem::CopyPayload {
            label: "Copy payload",
            format: PayloadFormat::Text,
            enabled: m.opcode != WsOpcode::Binary && !empty,
        },
    }];
    items.push(MenuItem::CopyPayload {
        label: "Copy payload as Base64",
        format: PayloadFormat::Base64,
        enabled: !empty,
    });
    items.push(MenuItem::CopyPayload {
        label: "Copy payload as hex",
        format: PayloadFormat::Hex,
        enabled: !empty,
    });
    if sendable && can_resend(m) {
        items.push(MenuItem::Separator);
        items.push(MenuItem::Resend(m.direction));
    }
    items
}

/// An event's name: the default type is `message`.
pub fn event_name(e: &SseEvent) -> &str {
    e.event.as_deref().unwrap_or("message")
}

/// An SSE row's menu (`EventItems`).
pub fn sse_menu(e: &SseEvent) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::Copy {
            label: "Copy data",
            text: e.data.clone(),
            what: "event data",
        },
        MenuItem::Copy {
            label: "Copy as event-stream text",
            text: sse_wire(e.event.as_deref(), e.id.as_deref(), &e.data, e.retry.map(u64::from)),
            what: "event",
        },
        MenuItem::Copy {
            label: "Copy event name",
            text: event_name(e).to_owned(),
            what: "event name",
        },
    ];
    if let Some(id) = &e.id {
        items.push(MenuItem::Copy {
            label: "Copy id",
            text: id.clone(),
            what: "event id",
        });
    }
    items
}

/// The SSE detail's fields: event, id, retry, time.
pub fn sse_fields(e: &SseEvent) -> Vec<(String, String)> {
    let mut rows = vec![(
        "event".to_owned(),
        e.event.clone().unwrap_or_else(|| "message (default)".into()),
    )];
    if let Some(id) = &e.id {
        rows.push(("id".into(), id.clone()));
    }
    if let Some(retry) = e.retry {
        rows.push(("retry".into(), format!("{retry} ms")));
    }
    if e.time_ms.is_some() {
        rows.push(("time".into(), format_time(e.time_ms)));
    }
    rows
}

/// The SSE header's status.
pub fn sse_status(open: bool) -> &'static str {
    if open { "Streaming" } else { "Stream ended" }
}

/// The SSE list's empty text.
pub fn sse_empty(total: u32, open: bool) -> &'static str {
    match (total, open) {
        (0, true) => "Waiting for events…",
        (0, false) => "No events",
        _ => "No events match",
    }
}

/// The WebSocket list's empty text.
pub fn ws_empty(total: u32) -> &'static str {
    if total == 0 {
        "No messages yet"
    } else {
        "No messages match"
    }
}

/// An SSE row's Data column: the first 300 characters, on one line.
pub fn data_preview(data: &str) -> String {
    data.chars()
        .take(300)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(opcode: WsOpcode) -> WsMessage {
        WsMessage {
            index: 4,
            direction: WsDirection::Sent,
            opcode,
            len: 5,
            time_ms: 0.,
            truncated: false,
            close_code: None,
            preview: "hello".into(),
            edited: false,
            injected: false,
            dropped: false,
        }
    }

    #[test]
    fn close_descriptions_name_standard_codes_and_keep_application_ones_bare() {
        assert_eq!(close_code_label(1000), "1000 Normal Closure");
        assert_eq!(close_code_label(4000), "4000");
        let close = |code, reason: &str, by| WsClose {
            code,
            reason: reason.into(),
            by,
        };
        assert_eq!(
            describe_close(&close(1001, "bye", WsDirection::Received)),
            "Closed by server · 1001 Going Away · bye"
        );
        assert_eq!(
            describe_close(&close(1006, "", WsDirection::Sent)),
            "Closed by client · 1006 Abnormal Closure"
        );
        assert_eq!(ws_status(true, None), "Open");
        assert_eq!(ws_status(false, None), "Closed");
        assert_eq!(count_label(3, 3, "messages"), "3 messages");
        assert_eq!(count_label(1, 3, "events"), "1 of 3 events");
    }

    #[test]
    fn reads_hex_in_the_usual_spellings() {
        assert_eq!(parse_hex("de ad BE ef"), Some(vec![0xde, 0xad, 0xbe, 0xef]));
        assert_eq!(parse_hex("0x00ff,0x10"), Some(vec![0, 255, 16]));
        assert_eq!(parse_hex(""), Some(vec![]));
        assert_eq!(parse_hex("abc"), None);
        assert_eq!(parse_hex("zz"), None);
    }

    #[test]
    fn builds_composer_messages() {
        assert_eq!(
            ws_send_of(WsDirection::Sent, ComposeMode::Text, "hi"),
            Ok(WsSend {
                direction: WsDirection::Sent,
                payload: "hi".into(),
                binary: false
            })
        );
        assert_eq!(
            ws_send_of(WsDirection::Received, ComposeMode::Hex, "00 01 02"),
            Ok(WsSend {
                direction: WsDirection::Received,
                payload: "AAEC".into(),
                binary: true
            })
        );
        assert_eq!(ws_send_of(WsDirection::Sent, ComposeMode::Hex, "nope"), Err(NOT_HEX));
    }

    #[test]
    fn resends_text_as_text_and_binary_as_base64() {
        assert_eq!(
            resend_of(WsDirection::Sent, WsOpcode::Text, "é".as_bytes()),
            WsSend {
                direction: WsDirection::Sent,
                payload: "é".into(),
                binary: false
            }
        );
        assert_eq!(
            resend_of(WsDirection::Received, WsOpcode::Binary, &[0, 1, 2]),
            WsSend {
                direction: WsDirection::Received,
                payload: "AAEC".into(),
                binary: true
            }
        );
        assert!(!can_resend(&message(WsOpcode::Ping)));
        let mut cut = message(WsOpcode::Binary);
        cut.truncated = true;
        assert!(!can_resend(&cut));
        assert!(can_resend(&message(WsOpcode::Text)));
    }

    #[test]
    fn lists_a_messages_flags() {
        let mut m = message(WsOpcode::Text);
        m.edited = true;
        m.dropped = true;
        assert_eq!(message_flags(&m), vec![Flag::Edited, Flag::Dropped]);
        m = message(WsOpcode::Text);
        m.injected = true;
        assert_eq!(message_flags(&m), vec![Flag::Injected]);
        assert_eq!(row_label(&m), "Sent Text message 5, 5 B, Injected");
    }

    #[test]
    fn previews_close_and_binary_frames() {
        let mut close = message(WsOpcode::Close);
        close.close_code = Some(1000);
        close.preview = "bye".into();
        assert_eq!(row_preview(&close), "1000 Normal Closure · bye");
        assert_eq!(row_preview(&message(WsOpcode::Binary)), "Binary data · 5 bytes");
        assert_eq!(size_label(0), "0 B");
        assert_eq!(data_preview("a\nb"), "a b");
    }

    #[test]
    fn menus_offer_the_right_copies_and_resend() {
        let labels = |items: Vec<MenuItem>| -> Vec<String> {
            items
                .into_iter()
                .map(|i| match i {
                    MenuItem::Copy { label, .. } => label.to_owned(),
                    MenuItem::CopyPayload { label, enabled, .. } => {
                        format!("{label}{}", if enabled { "" } else { " (off)" })
                    }
                    MenuItem::Separator => "—".into(),
                    MenuItem::Resend(d) => resend_label(d).into(),
                })
                .collect()
        };
        assert_eq!(
            labels(ws_menu(&message(WsOpcode::Text), true)),
            [
                "Copy payload",
                "Copy payload as Base64",
                "Copy payload as hex",
                "—",
                "Resend to the server"
            ]
        );
        assert_eq!(
            labels(ws_menu(&message(WsOpcode::Binary), false)),
            ["Copy payload (off)", "Copy payload as Base64", "Copy payload as hex"]
        );
        let mut close = message(WsOpcode::Close);
        close.close_code = Some(1001);
        close.preview = String::new();
        let items = ws_menu(&close, true);
        assert_eq!(
            items[0],
            MenuItem::Copy {
                label: "Copy close code and reason",
                text: "1001 Going Away".into(),
                what: "close reason"
            }
        );
        assert_eq!(payload_as(&[0xff, 0], PayloadFormat::Text), Err(NOT_TEXT));
        assert_eq!(payload_as(b"hi", PayloadFormat::Hex), Ok("68 69".into()));
    }

    #[test]
    fn events_have_fields_and_copies() {
        let e = SseEvent {
            index: 0,
            time_ms: None,
            id: Some("7".into()),
            event: None,
            data: "a\nb".into(),
            retry: Some(500),
            truncated: false,
        };
        assert_eq!(
            sse_fields(&e),
            vec![
                ("event".into(), "message (default)".into()),
                ("id".into(), "7".into()),
                ("retry".into(), "500 ms".into())
            ]
        );
        let items = sse_menu(&e);
        assert_eq!(items.len(), 4);
        assert_eq!(
            items[1],
            MenuItem::Copy {
                label: "Copy as event-stream text",
                text: "id: 7\nretry: 500\ndata: a\ndata: b".into(),
                what: "event"
            }
        );
        assert_eq!(sse_empty(0, true), "Waiting for events…");
        assert_eq!(sse_empty(2, false), "No events match");
        assert_eq!(sse_status(false), "Stream ended");
        assert_eq!(ws_empty(0), "No messages yet");
    }

    #[test]
    fn detail_lines_say_who_and_what() {
        let mut m = message(WsOpcode::Close);
        m.injected = true;
        m.direction = WsDirection::Received;
        m.close_code = Some(1000);
        m.truncated = true;
        m.dropped = true;
        let lines = detail_lines(&m, Some(2));
        assert!(lines[0].starts_with("Sent to the client by Proxymate · Close · 5 bytes · "));
        assert_eq!(lines[1], "Close code 1000 Normal Closure");
        assert_eq!(lines[2], "Recorded the first 2 B of 5 B");
        assert_eq!(
            lines[3],
            "Dropped by a rule, script or breakpoint: it was never forwarded."
        );
        assert_eq!(page_of(127), 0);
        assert_eq!(page_of(128), 1);
    }
}
