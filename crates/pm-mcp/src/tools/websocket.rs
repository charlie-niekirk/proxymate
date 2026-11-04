//! WebSocket tools: messages, sending and closing (M19).

use super::*;

#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(super) enum Direction {
    /// Client to server.
    Sent,
    /// Server to client.
    Received,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct SendWsMessage {
    /// An open WebSocket flow.
    pub(super) id: FlowId,
    pub(super) direction: Direction,
    /// Text as is, or base64 with `binary`.
    pub(super) payload: String,
    /// Send a binary message; `payload` is base64.
    #[serde(default)]
    pub(super) binary: bool,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct GetWsMessages {
    pub(super) id: FlowId,
    #[serde(default)]
    pub(super) direction: Option<Direction>,
    /// Case-insensitive text the payload must contain.
    #[serde(default)]
    pub(super) text: String,
    /// 1–500, default 50.
    #[serde(default)]
    pub(super) limit: Option<u32>,
    #[serde(default)]
    pub(super) cursor: Option<String>,
    /// Payload bytes per message, default 4096 (0 leaves payloads out).
    #[serde(default)]
    pub(super) payload_max_bytes: Option<u32>,
}

pub(super) fn get_ws_messages(ctx: &Ctx<'_>, p: GetWsMessages) -> ToolResult {
    let query = WsQuery {
        direction: p.direction.map(ws_direction),
        text: p.text,
    };
    let start = cursor_offset(&p.cursor)? as u32;
    let count = limit(p.limit, 50, 500) as u32;
    let page = ctx.engine.websocket_messages(p.id, &query, start, count)?;
    let payload_max = p.payload_max_bytes.map_or(4096, |m| (m as usize).min(BODY_MAX));
    let messages: Vec<Value> = page
        .messages
        .iter()
        .map(|m| {
            let mut v = json!(m);
            if payload_max > 0 {
                if let Ok(payload) = ctx.engine.websocket_payload(p.id, m.index) {
                    let (text, encoding, cut) = payload_json(&payload, m.opcode, payload_max);
                    v["payload"] = json!(text);
                    v["payload_encoding"] = json!(encoding);
                    v["payload_cut"] = json!(cut);
                }
                if m.edited
                    && let Ok(Some(original)) = ctx.engine.websocket_original_in(LIVE_STORE, p.id, m.index)
                {
                    let (text, encoding, _) = payload_json(&original, m.opcode, payload_max);
                    v["original"] = json!(text);
                    v["original_encoding"] = json!(encoding);
                }
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("preview");
                }
            }
            v
        })
        .collect();
    let end = page.start + page.messages.len() as u32;
    Ok(json!({
        "total": page.total,
        "matched": page.len,
        "messages": messages,
        "open": page.open,
        "close": page.close,
        "next_cursor": (end < page.len).then(|| end.to_string()),
    }))
}

pub(super) fn ws_direction(direction: Direction) -> WsDirection {
    match direction {
        Direction::Sent => WsDirection::Sent,
        Direction::Received => WsDirection::Received,
    }
}

/// A payload cut at `max`: UTF-8 text, or base64 for binary; and whether
/// it was cut.
pub(super) fn payload_json(payload: &[u8], opcode: pm_model::WsOpcode, max: usize) -> (String, &'static str, bool) {
    let cut = &payload[..payload.len().min(max)];
    let (text, encoding) = match std::str::from_utf8(cut) {
        Ok(s) if opcode != pm_model::WsOpcode::Binary => (s.to_owned(), "utf8"),
        _ => (base64::engine::general_purpose::STANDARD.encode(cut), "base64"),
    };
    (text, encoding, payload.len() > max)
}
