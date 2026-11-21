//! The Breakpoint Editor's logic (`Breakpoints/BreakpointEditor.tsx` and
//! `store/breakpoints.ts`), with no gpui: the queue's lines and countdowns,
//! the queue menu as data, which paused flow is shown, the edits an HTTP
//! form makes (`MessageEdits`), and a held WebSocket message edited as
//! text, a JSON tree or hex.

use pm_model::{
    BreakpointId, BreakpointInfo, BreakpointResolution, Header, MessageEdits, Phase, WsDirection, WsOpcode,
};

use crate::copy_formats::{base64, hex_bytes};
use crate::http::body_text;
use crate::kv::{self, KvRow};
use crate::stream::parse_hex;

pub const DESCRIPTION: &str = "Paused flows wait here until you execute, abort or answer them, or until they time out \
and continue unmodified.";
pub const EMPTY_QUEUE: &str = "Nothing is paused.";
pub const EMPTY: &str = "Add a Breakpoint action to a rule (or use “Breakpoint…” on a flow) to pause matching flows.";
pub const MESSAGE_WAITS: &str = "Later messages this way wait behind it.";

pub fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Request => "Request",
        Phase::Response => "Response",
    }
}

/// Seconds until the flow continues on its own, as `m:ss` ("no timeout"
/// without a deadline).
pub fn countdown(deadline_ms: Option<f64>, now_ms: f64) -> String {
    let Some(deadline) = deadline_ms else {
        return "no timeout".into();
    };
    let left = ((deadline - now_ms) / 1000.).round().max(0.) as u64;
    format!("{}:{:02}", left / 60, left % 60)
}

/// `#4` for flow id 3 (the table numbers from 1).
pub fn flow_number(bp: &BreakpointInfo) -> Option<String> {
    bp.flow_id.map(|id| format!("#{}", id + 1))
}

/// The queue row's accessible name.
pub fn queue_label(bp: &BreakpointInfo, now_ms: f64) -> String {
    let what = match &bp.message {
        Some(m) => format!("WebSocket message {}", direction_label(m.direction)),
        None => bp.request.method.clone(),
    };
    format!(
        "{} {what} {} · {} · {}",
        phase_label(bp.phase),
        bp.request.url,
        bp.rule_name,
        countdown(bp.deadline_ms, now_ms)
    )
}

pub fn direction_label(direction: WsDirection) -> &'static str {
    match direction {
        WsDirection::Sent => "Sent by the client",
        WsDirection::Received => "From the server",
    }
}

/// Which paused flow to show: the chosen one while it waits, else the
/// oldest.
pub fn shown(queue: &[BreakpointInfo], chosen: Option<BreakpointId>) -> Option<&BreakpointInfo> {
    chosen
        .and_then(|id| queue.iter().find(|b| b.id == id))
        .or_else(|| queue.first())
}

/// What the editor window does when the queue changes: it opens itself
/// when the queue goes from empty to not, and closes once it is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueChange {
    Open,
    Close,
    Stay,
}

pub fn queue_change(before: usize, after: usize) -> QueueChange {
    match (before, after) {
        (0, n) if n > 0 => QueueChange::Open,
        (n, 0) if n > 0 => QueueChange::Close,
        _ => QueueChange::Stay,
    }
}

/// An entry of the queue row's menu.
#[derive(Clone, Debug, PartialEq)]
pub enum QueueItem {
    Copy {
        label: &'static str,
        text: String,
        what: &'static str,
    },
    ShowFlow(pm_model::FlowId),
    OpenRule {
        id: String,
        name: String,
    },
    Separator,
    /// Continue unmodified.
    Continue,
    /// "Abort", or "Drop the message" for a WebSocket message.
    Abort(&'static str),
}

pub fn queue_menu(bp: &BreakpointInfo) -> Vec<QueueItem> {
    let mut items = vec![
        QueueItem::Copy {
            label: "Copy URL",
            text: bp.request.url.clone(),
            what: "URL",
        },
        QueueItem::Copy {
            label: "Copy method",
            text: bp.request.method.clone(),
            what: "method",
        },
        QueueItem::Separator,
    ];
    if let Some(id) = bp.flow_id {
        items.push(QueueItem::ShowFlow(id));
    }
    items.push(QueueItem::OpenRule {
        id: bp.rule_id.clone(),
        name: bp.rule_name.clone(),
    });
    items.push(QueueItem::Separator);
    items.push(QueueItem::Continue);
    items.push(QueueItem::Abort(if bp.message.is_some() {
        "Drop the message"
    } else {
        "Abort"
    }));
    items
}

/// The HTTP form's draft: the head, headers and body as edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpDraft {
    pub method: String,
    pub url: String,
    /// The status line's code as typed (a response).
    pub status: String,
    pub headers: Vec<KvRow>,
    /// The headers as paused, with the rows' ids (for the tints).
    pub original_headers: Vec<KvRow>,
    /// The body as edited; `None` until it changes.
    pub body: Option<String>,
}

impl HttpDraft {
    pub fn of(bp: &BreakpointInfo) -> Self {
        let headers = match (bp.phase, &bp.response) {
            (Phase::Response, Some(r)) => kv::from_headers(&r.headers),
            (Phase::Response, None) => Vec::new(),
            (Phase::Request, _) => kv::from_headers(&bp.request.headers),
        };
        HttpDraft {
            method: bp.request.method.clone(),
            url: bp.request.url.clone(),
            status: bp.response.as_ref().map_or(200, |r| r.status).to_string(),
            original_headers: headers.clone(),
            headers,
            body: None,
        }
    }

    /// The status typed, if it is one (100–999).
    pub fn status_code(&self) -> Option<u16> {
        parse_status(&self.status)
    }

    /// The edits to send with Execute: only what differs from the paused
    /// message. A status that isn't a number is an error.
    pub fn edits(&self, bp: &BreakpointInfo, original_body: &str) -> Result<MessageEdits, String> {
        let request = bp.phase == Phase::Request;
        let mut out = MessageEdits::default();
        if request && self.method != bp.request.method {
            out.method = Some(self.method.clone());
        }
        if request && self.url != bp.request.url {
            out.url = Some(self.url.clone());
        }
        if !request {
            let status = self.status_code().ok_or("The status must be a number from 100 to 999")?;
            if Some(status) != bp.response.as_ref().map(|r| r.status) {
                out.status = Some(status);
            }
        }
        let headers = kv::to_headers(&self.headers);
        let paused = match (request, &bp.response) {
            (true, _) => &bp.request.headers,
            (false, Some(r)) => &r.headers,
            (false, None) => &Vec::new(),
        };
        if &headers != paused {
            out.headers = Some(headers);
        }
        if bp.body.editable
            && let Some(body) = &self.body
            && body != original_body
        {
            out.body = Some(body.clone());
        }
        Ok(out)
    }
}

pub fn parse_status(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|s| (100..=999).contains(s))
}

/// The Respond… form: what the client gets instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: String,
    pub headers: Vec<KvRow>,
    pub body: String,
}

impl Default for Answer {
    fn default() -> Self {
        Answer {
            status: "200".into(),
            headers: vec![KvRow::new("content-type", "application/json")],
            body: String::new(),
        }
    }
}

impl Answer {
    pub fn resolution(&self) -> Result<BreakpointResolution, String> {
        Ok(BreakpointResolution::Respond {
            status: parse_status(&self.status).ok_or("The status must be a number from 100 to 999")?,
            headers: kv::to_headers(&self.headers),
            body: self.body.clone(),
        })
    }
}

/// Execute with the form's edits.
pub fn execute(edits: MessageEdits) -> BreakpointResolution {
    BreakpointResolution::Continue { edits: Some(edits) }
}

/// Merges `extra` (the dev remote's edits) over the form's.
pub fn merge_edits(mut base: MessageEdits, extra: MessageEdits) -> MessageEdits {
    base.method = extra.method.or(base.method);
    base.url = extra.url.or(base.url);
    base.status = extra.status.or(base.status);
    base.headers = extra.headers.or(base.headers);
    base.body = extra.body.or(base.body);
    base.body_base64 = extra.body_base64.or(base.body_base64);
    base
}

/// How a held WebSocket message is edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageMode {
    Text,
    Tree,
    Hex,
}

impl MessageMode {
    pub fn label(self) -> &'static str {
        match self {
            MessageMode::Text => "Text",
            MessageMode::Tree => "Tree",
            MessageMode::Hex => "Hex",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            MessageMode::Text => "text",
            MessageMode::Tree => "tree",
            MessageMode::Hex => "hex",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [MessageMode::Text, MessageMode::Tree, MessageMode::Hex]
            .into_iter()
            .find(|m| m.id() == id)
    }
}

/// A held message's payload as edited: text (or a tree over it) or hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageDraft {
    pub original: Vec<u8>,
    /// The original as text, if it is UTF-8.
    pub original_text: Option<String>,
    pub opcode: WsOpcode,
    pub mode: MessageMode,
    /// The text as edited (`None`: unchanged).
    pub text: Option<String>,
    /// The hex as edited (`None`: not opened as hex yet).
    pub hex: Option<String>,
}

pub fn is_json(text: &str) -> bool {
    crate::body::json_doc::parse(text).is_ok()
}

impl MessageDraft {
    pub fn new(original: Vec<u8>, opcode: WsOpcode) -> Self {
        let original_text = body_text(&original).map(str::to_owned);
        // Binary that isn't UTF-8 can only be edited as hex.
        let mode = match &original_text {
            None => MessageMode::Hex,
            Some(_) if opcode == WsOpcode::Binary => MessageMode::Hex,
            Some(t) if is_json(t) => MessageMode::Tree,
            Some(_) => MessageMode::Text,
        };
        MessageDraft {
            original,
            original_text,
            opcode,
            mode,
            text: None,
            hex: None,
        }
    }

    /// The modes offered.
    pub fn modes(&self) -> Vec<MessageMode> {
        match &self.original_text {
            None => vec![MessageMode::Hex],
            Some(t) if is_json(t) => vec![MessageMode::Text, MessageMode::Tree, MessageMode::Hex],
            Some(_) => vec![MessageMode::Text, MessageMode::Hex],
        }
    }

    pub fn current_text(&self) -> String {
        self.text
            .clone()
            .or_else(|| self.original_text.clone())
            .unwrap_or_default()
    }

    pub fn current_hex(&self) -> String {
        self.hex.clone().unwrap_or_else(|| hex_bytes(&self.original))
    }

    /// Switches mode, carrying the edits over; an error leaves it as is.
    pub fn switch_to(&mut self, next: MessageMode) -> Result<(), String> {
        if next == self.mode {
            return Ok(());
        }
        let mut next_text = self.current_text();
        if next == MessageMode::Hex {
            self.hex = Some(match &self.text {
                None => hex_bytes(&self.original),
                Some(t) => hex_bytes(t.as_bytes()),
            });
        }
        if self.mode == MessageMode::Hex
            && let Some(hex) = &self.hex
        {
            let decoded = parse_hex(hex).and_then(|b| body_text(&b).map(str::to_owned));
            let Some(decoded) = decoded else {
                return Err("The bytes aren't UTF-8 text: keep editing them as hex.".into());
            };
            next_text = decoded.clone();
            self.text = Some(decoded);
        }
        if next == MessageMode::Tree && !is_json(&next_text) {
            return Err("The text isn't valid JSON.".into());
        }
        self.mode = next;
        Ok(())
    }

    /// The edits Send sends: hex as bytes, text as text, nothing if unchanged.
    pub fn edits(&self, editable: bool) -> Result<MessageEdits, String> {
        if !editable {
            return Ok(MessageEdits::default());
        }
        if self.mode == MessageMode::Hex {
            let Some(hex) = &self.hex else {
                return Ok(MessageEdits::default());
            };
            let bytes = parse_hex(hex).ok_or("That isn't hex: use pairs of digits like 0a ff 3c.")?;
            if bytes == self.original {
                return Ok(MessageEdits::default());
            }
            return Ok(MessageEdits {
                body_base64: Some(base64(&bytes)),
                ..Default::default()
            });
        }
        Ok(match &self.text {
            Some(t) if Some(t) != self.original_text.as_ref() => MessageEdits {
                body: Some(t.clone()),
                ..Default::default()
            },
            _ => MessageEdits::default(),
        })
    }
}

/// The headers of a paused head, for comparing.
pub fn paused_headers(bp: &BreakpointInfo) -> &[Header] {
    match (bp.phase, &bp.response) {
        (Phase::Response, Some(r)) => &r.headers,
        (Phase::Response, None) => &[],
        (Phase::Request, _) => &bp.request.headers,
    }
}

/// The body's heading: "Body · 1,234 bytes".
pub fn body_heading(size: u64) -> String {
    if size == 0 {
        "Body".into()
    } else {
        format!("Body · {} bytes", thousands(size))
    }
}

/// "Sent by the client · Text · 12 bytes".
pub fn message_heading(direction: WsDirection, opcode: WsOpcode, size: u64) -> String {
    format!(
        "{} · {} · {} bytes",
        direction_label(direction),
        if opcode == WsOpcode::Text { "Text" } else { "Binary" },
        thousands(size)
    )
}

pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;

    #[test]
    fn counts_down_in_minutes_and_seconds() {
        assert_eq!(countdown(None, 0.), "no timeout");
        assert_eq!(countdown(Some(125_400.), 0.), "2:05");
        assert_eq!(countdown(Some(1_000.), 5_000.), "0:00");
    }

    #[test]
    fn opens_on_the_first_pause_and_closes_when_empty() {
        assert_eq!(queue_change(0, 1), QueueChange::Open);
        assert_eq!(queue_change(1, 2), QueueChange::Stay);
        assert_eq!(queue_change(2, 0), QueueChange::Close);
        assert_eq!(queue_change(0, 0), QueueChange::Stay);
    }

    #[test]
    fn shows_the_chosen_flow_while_it_waits() {
        let a = sample::breakpoint(1, Phase::Request);
        let b = sample::breakpoint(2, Phase::Response);
        let queue = [a, b];
        assert_eq!(shown(&queue, Some(2)).unwrap().id, 2);
        assert_eq!(shown(&queue, Some(9)).unwrap().id, 1);
        assert!(shown(&[], Some(1)).is_none());
    }

    #[test]
    fn the_queue_menu_names_the_flow_and_rule() {
        let bp = sample::breakpoint(1, Phase::Request);
        let menu = queue_menu(&bp);
        assert!(menu.contains(&QueueItem::ShowFlow(bp.flow_id.unwrap())));
        assert_eq!(menu.last(), Some(&QueueItem::Abort("Abort")));
        let mut ws = bp.clone();
        ws.message = Some(pm_model::HeldMessage {
            direction: WsDirection::Sent,
            opcode: WsOpcode::Text,
        });
        ws.flow_id = None;
        let menu = queue_menu(&ws);
        assert!(!menu.iter().any(|i| matches!(i, QueueItem::ShowFlow(_))));
        assert_eq!(menu.last(), Some(&QueueItem::Abort("Drop the message")));
    }

    #[test]
    fn a_request_form_sends_only_what_changed() {
        let bp = sample::breakpoint(1, Phase::Request);
        let mut draft = HttpDraft::of(&bp);
        assert_eq!(draft.edits(&bp, "{}").unwrap(), MessageEdits::default());
        draft.method = "PUT".into();
        draft.url = format!("{}?x=1", bp.request.url);
        draft.headers.push(KvRow::new("x-added", "1"));
        draft.headers.push(KvRow::new(" ", "dropped"));
        draft.body = Some("{\"a\":2}".into());
        let edits = draft.edits(&bp, "{}").unwrap();
        assert_eq!(edits.method.as_deref(), Some("PUT"));
        assert!(edits.url.unwrap().ends_with("?x=1"));
        let headers = edits.headers.unwrap();
        assert_eq!(headers.last().unwrap().name, "x-added");
        assert_eq!(headers.len(), bp.request.headers.len() + 1);
        assert_eq!(edits.body.as_deref(), Some("{\"a\":2}"));
        assert_eq!(edits.status, None);
        // A body that isn't editable is never sent.
        let mut locked = bp.clone();
        locked.body.editable = false;
        assert_eq!(draft.edits(&locked, "{}").unwrap().body, None);
    }

    #[test]
    fn a_response_form_sends_its_status() {
        let bp = sample::breakpoint(1, Phase::Response);
        let mut draft = HttpDraft::of(&bp);
        assert_eq!(draft.status, "200");
        assert_eq!(draft.edits(&bp, "").unwrap(), MessageEdits::default());
        draft.status = "503".into();
        draft.method = "DELETE".into();
        let edits = draft.edits(&bp, "").unwrap();
        assert_eq!(edits.status, Some(503));
        assert_eq!(edits.method, None, "a response can't change the request");
        draft.status = "abc".into();
        assert!(draft.edits(&bp, "").is_err());
    }

    #[test]
    fn respond_answers_with_the_form() {
        let mut answer = Answer::default();
        answer.body = "{}".into();
        assert_eq!(
            answer.resolution().unwrap(),
            BreakpointResolution::Respond {
                status: 200,
                headers: vec![Header {
                    name: "content-type".into(),
                    value: "application/json".into()
                }],
                body: "{}".into(),
            }
        );
        answer.status = "42".into();
        assert!(answer.resolution().is_err());
    }

    #[test]
    fn merges_the_remotes_edits_over_the_forms() {
        let base = MessageEdits {
            method: Some("PUT".into()),
            body: Some("a".into()),
            ..Default::default()
        };
        let merged = merge_edits(
            base,
            MessageEdits {
                body: Some("b".into()),
                ..Default::default()
            },
        );
        assert_eq!((merged.method.as_deref(), merged.body.as_deref()), (Some("PUT"), Some("b")));
    }

    #[test]
    fn a_json_message_opens_as_a_tree_and_round_trips_through_hex() {
        let mut m = MessageDraft::new(br#"{"a":1}"#.to_vec(), WsOpcode::Text);
        assert_eq!(m.mode, MessageMode::Tree);
        assert_eq!(m.modes(), [MessageMode::Text, MessageMode::Tree, MessageMode::Hex]);
        assert_eq!(m.edits(true).unwrap(), MessageEdits::default());
        m.text = Some(r#"{"a":2}"#.into());
        m.switch_to(MessageMode::Hex).unwrap();
        assert_eq!(m.current_hex(), hex_bytes(br#"{"a":2}"#));
        assert_eq!(m.edits(true).unwrap().body_base64, Some(base64(br#"{"a":2}"#)));
        m.hex = Some("7b 7d".into());
        m.switch_to(MessageMode::Tree).unwrap();
        assert_eq!(m.current_text(), "{}");
        assert_eq!(m.edits(true).unwrap().body.as_deref(), Some("{}"));
        assert_eq!(m.edits(false).unwrap(), MessageEdits::default());
    }

    #[test]
    fn binary_messages_stay_hex_until_they_are_text() {
        let mut m = MessageDraft::new(vec![0, 0xff], WsOpcode::Binary);
        assert_eq!(m.mode, MessageMode::Hex);
        assert_eq!(m.modes(), [MessageMode::Hex]);
        m.hex = Some("00 ff".into());
        assert_eq!(m.edits(true).unwrap(), MessageEdits::default(), "unchanged bytes");
        assert!(m.switch_to(MessageMode::Text).is_err());
        m.hex = Some("zz".into());
        assert!(m.edits(true).is_err());

        let mut t = MessageDraft::new(b"hello".to_vec(), WsOpcode::Text);
        assert_eq!(t.mode, MessageMode::Text);
        assert!(t.switch_to(MessageMode::Tree).is_err(), "not JSON");
        t.text = Some("bye".into());
        assert_eq!(t.edits(true).unwrap().body.as_deref(), Some("bye"));
    }

    #[test]
    fn headings_count_bytes() {
        assert_eq!(body_heading(0), "Body");
        assert_eq!(body_heading(1234567), "Body · 1,234,567 bytes");
        assert_eq!(
            message_heading(WsDirection::Received, WsOpcode::Binary, 12),
            "From the server · Binary · 12 bytes"
        );
    }
}
