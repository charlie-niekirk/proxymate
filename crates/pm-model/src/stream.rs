//! Messages inside a flow: WebSocket frames and server-sent events. Both
//! are paged through the engine (`websocket_messages`, `sse_events`), never
//! sent whole, since a long-lived connection can carry any number of them.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Which way a WebSocket message went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum WsDirection {
    /// Client to server.
    Sent,
    /// Server to client.
    Received,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum WsOpcode {
    Text,
    Binary,
    Ping,
    Pong,
    Close,
}

/// One WebSocket message (fragmented messages are reassembled). The payload
/// itself is fetched with `websocket_payload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WsMessage {
    /// Position in the flow's message list, from 0.
    pub index: u32,
    pub direction: WsDirection,
    pub opcode: WsOpcode,
    /// Payload size in bytes. For close frames, the reason only.
    pub len: u64,
    /// When the proxy received it, milliseconds since the Unix epoch.
    pub time_ms: f64,
    /// True if only the first part of the payload was recorded (the cap is
    /// 1 MB per message). Forwarding is unaffected.
    pub truncated: bool,
    /// Status code of a close frame, if it carried one.
    pub close_code: Option<u16>,
    /// The start of the payload for the list: text as is, binary empty.
    pub preview: String,
}

/// How a WebSocket connection ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct WsClose {
    /// The close code, or `1005` for a close frame without one and `1006` for
    /// a connection that dropped without any close frame (neither is ever
    /// sent on the wire).
    pub code: u16,
    pub reason: String,
    /// The side that closed first.
    pub by: WsDirection,
}

/// Which messages of a flow to list. The default lists everything.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct WsQuery {
    pub direction: Option<WsDirection>,
    /// Case-insensitive substring of the payload; empty matches everything.
    pub text: String,
}

/// A window of a flow's WebSocket messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WsPage {
    /// Messages in the flow.
    pub total: u32,
    /// Messages matching the query.
    pub len: u32,
    pub start: u32,
    pub messages: Vec<WsMessage>,
    /// False once the connection has closed.
    pub open: bool,
    pub close: Option<WsClose>,
    /// Older messages dropped by the per-connection limit; `total` and the
    /// page count from what is kept.
    #[serde(default)]
    pub dropped: u32,
}

/// One event of a `text/event-stream` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SseEvent {
    /// Position in the stream, from 0.
    pub index: u32,
    /// When the event was complete, milliseconds since the Unix epoch.
    /// `None` when the stream was parsed after the fact (a compressed body).
    pub time_ms: Option<f64>,
    /// The `id` field, if the event set one.
    pub id: Option<String>,
    /// The `event` field; `None` means the default type, `message`.
    pub event: Option<String>,
    /// The `data` lines, joined with `\n`.
    pub data: String,
    /// The `retry` field, in milliseconds.
    pub retry: Option<u32>,
    /// True if `data` was cut at the 1 MB cap.
    pub truncated: bool,
}

/// A window of a flow's server-sent events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SsePage {
    /// Events in the stream so far.
    pub total: u32,
    /// Events matching the search text.
    pub len: u32,
    pub start: u32,
    pub events: Vec<SseEvent>,
    /// False once the response has ended.
    pub open: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_serialize_lowercase() {
        assert_eq!(serde_json::to_string(&WsDirection::Received).unwrap(), r#""received""#);
        assert_eq!(serde_json::to_string(&WsOpcode::Pong).unwrap(), r#""pong""#);
        let query: WsQuery = serde_json::from_str(r#"{"direction":"sent","text":"a"}"#).unwrap();
        assert_eq!(query.direction, Some(WsDirection::Sent));
    }
}
