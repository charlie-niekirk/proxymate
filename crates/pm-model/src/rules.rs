//! The unified rules system: a [`Rule`] is a [`Matcher`] plus an ordered
//! list of [`Action`]s. Map Local, rewrites, breakpoints, Map Remote,
//! throttling and blocking are all actions; the "Map Local…", "Breakpoint…"
//! and "Rewrite…" presets only pre-fill rules.
//!
//! These types also derive `schemars::JsonSchema`, so the MCP tools (M7)
//! take and return exactly these shapes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::detail::{Header, RequestHead, ResponseHead};
use crate::flow::{FlowId, Scheme};
use crate::stream::{WsDirection, WsOpcode};

/// A rule's id: a UUID, or `builtin-…` for rules the app creates itself.
pub type RuleId = String;

/// The id of the global throttle rule behind the toolbar's network
/// conditions menu. It matches everything and is disabled until a
/// profile is picked.
pub const GLOBAL_THROTTLE_RULE_ID: &str = "builtin-throttle";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Rule {
    pub id: RuleId,
    pub name: String,
    pub enabled: bool,
    /// Rules run in ascending `order`; every matching rule runs.
    pub order: i64,
    pub origin: RuleOrigin,
    pub matcher: Matcher,
    pub actions: Vec<Action>,
}

/// Who created a rule. Agent rules show an "AI · client" badge and can be
/// removed together.
///
/// Serialized by hand: JSON (Tauri, MCP, `app.sqlite`) gets the tagged
/// object the attributes below describe, while postcard (flow details in
/// segments and sessions) gets the tag and then the fields in order, the
/// bytes serde's internally tagged derive writes there. postcard can't
/// read that derive back (it needs `deserialize_any`), which made every
/// flow a rule changed unreadable from disk before M14.
#[derive(Debug, Clone, PartialEq, Type, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleOrigin {
    User,
    /// Pre-filled from a flow by a preset (or built in, like the throttle).
    Preset {
        preset: PresetKind,
    },
    /// Created by an MCP client.
    Agent {
        client: String,
        session: Option<String>,
        /// Milliseconds since the Unix epoch.
        created_at_ms: f64,
    },
}

/// [`RuleOrigin`]'s JSON form.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OriginJson {
    User,
    Preset {
        preset: PresetKind,
    },
    Agent {
        client: String,
        session: Option<String>,
        created_at_ms: f64,
    },
}

impl Serialize for RuleOrigin {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        if serializer.is_human_readable() {
            let json = match self.clone() {
                RuleOrigin::User => OriginJson::User,
                RuleOrigin::Preset { preset } => OriginJson::Preset { preset },
                RuleOrigin::Agent {
                    client,
                    session,
                    created_at_ms,
                } => OriginJson::Agent {
                    client,
                    session,
                    created_at_ms,
                },
            };
            return json.serialize(serializer);
        }
        match self {
            RuleOrigin::User => {
                let mut t = serializer.serialize_tuple(1)?;
                t.serialize_element("user")?;
                t.end()
            }
            RuleOrigin::Preset { preset } => {
                let mut t = serializer.serialize_tuple(2)?;
                t.serialize_element("preset")?;
                t.serialize_element(preset)?;
                t.end()
            }
            RuleOrigin::Agent {
                client,
                session,
                created_at_ms,
            } => {
                let mut t = serializer.serialize_tuple(4)?;
                t.serialize_element("agent")?;
                t.serialize_element(client)?;
                t.serialize_element(session)?;
                t.serialize_element(created_at_ms)?;
                t.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for RuleOrigin {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            return Ok(match OriginJson::deserialize(deserializer)? {
                OriginJson::User => RuleOrigin::User,
                OriginJson::Preset { preset } => RuleOrigin::Preset { preset },
                OriginJson::Agent {
                    client,
                    session,
                    created_at_ms,
                } => RuleOrigin::Agent {
                    client,
                    session,
                    created_at_ms,
                },
            });
        }
        struct Compact;
        impl<'de> serde::de::Visitor<'de> for Compact {
            type Value = RuleOrigin;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a rule origin")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<RuleOrigin, A::Error> {
                use serde::de::Error;
                let missing = || A::Error::custom("a rule origin is cut short");
                let tag: String = seq.next_element()?.ok_or_else(missing)?;
                Ok(match tag.as_str() {
                    "user" => RuleOrigin::User,
                    "preset" => RuleOrigin::Preset {
                        preset: seq.next_element()?.ok_or_else(missing)?,
                    },
                    "agent" => RuleOrigin::Agent {
                        client: seq.next_element()?.ok_or_else(missing)?,
                        session: seq.next_element()?.ok_or_else(missing)?,
                        created_at_ms: seq.next_element()?.ok_or_else(missing)?,
                    },
                    other => return Err(A::Error::unknown_variant(other, &["user", "preset", "agent"])),
                })
            }
        }
        deserializer.deserialize_tuple(4, Compact)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PresetKind {
    MapLocal,
    Breakpoint,
    Rewrite,
    MapRemote,
    Block,
    Throttle,
}

/// What a rule applies to. Every field that is set must match; empty lists
/// and `None` match anything. The request part decides whether the rule
/// runs at all; `response` only gates its response-phase actions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Matcher {
    /// Upper-case methods, e.g. `["GET", "POST"]`. Empty matches any.
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub scheme: Option<Scheme>,
    /// Matched against the host without the port. A glob `*.example.com`
    /// also matches `example.com`.
    #[serde(default)]
    pub host: Option<Pattern>,
    #[serde(default)]
    pub port: Option<u16>,
    /// Matched against the path without the query, e.g. `/api/users`. In
    /// globs `*` also crosses `/`.
    #[serde(default)]
    pub path: Option<Pattern>,
    #[serde(default)]
    pub query: Vec<KvMatch>,
    /// Header names are case-insensitive.
    #[serde(default)]
    pub headers: Vec<KvMatch>,
    /// The client app's name (e.g. `Safari`, `curl`).
    #[serde(default)]
    pub client_app: Option<Pattern>,
    #[serde(default)]
    pub response: Option<ResponseMatcher>,
}

/// A string pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Pattern {
    pub kind: PatternKind,
    pub value: String,
    #[serde(default)]
    pub case_insensitive: bool,
}

impl Pattern {
    pub fn exact(value: impl Into<String>) -> Pattern {
        Pattern {
            kind: PatternKind::Exact,
            value: value.into(),
            case_insensitive: false,
        }
    }

    pub fn glob(value: impl Into<String>) -> Pattern {
        Pattern {
            kind: PatternKind::Glob,
            value: value.into(),
            case_insensitive: true,
        }
    }

    pub fn regex(value: impl Into<String>) -> Pattern {
        Pattern {
            kind: PatternKind::Regex,
            value: value.into(),
            case_insensitive: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    Exact,
    /// `*` any run of characters, `?` one, `[abc]` a class, `{a,b}` either.
    Glob,
    /// Rust `regex` syntax, unanchored. Compiled with a size limit.
    Regex,
}

/// A query parameter or header condition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct KvMatch {
    pub name: String,
    /// `None`: the name only has to be present. Otherwise some value of it
    /// must match.
    #[serde(default)]
    pub value: Option<Pattern>,
    /// Inverts the condition: the name is absent, or no value matches.
    #[serde(default)]
    pub negate: bool,
}

/// Gates the response phase of a rule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ResponseMatcher {
    /// Inclusive ranges, e.g. `[{from: 400, to: 599}]`. Empty matches any.
    #[serde(default)]
    pub status: Vec<StatusRange>,
    /// Matched against the `Content-Type` header (with parameters).
    #[serde(default)]
    pub content_type: Option<Pattern>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct StatusRange {
    pub from: u16,
    pub to: u16,
}

/// Which message an action changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Request,
    Response,
}

/// One step of a rule. Actions of all matching rules run in rule order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "type")]
pub enum Action {
    /// Replaces every value of the header (adds it if missing).
    SetHeader {
        phase: Phase,
        name: String,
        value: String,
    },
    RemoveHeader {
        phase: Phase,
        name: String,
    },
    /// Rewrites the matching part of each value of a header.
    ReplaceHeaderValue {
        phase: Phase,
        name: String,
        pattern: Pattern,
        replacement: String,
    },
    /// Replaces every value of a query parameter (adds it if missing).
    SetQuery {
        name: String,
        value: String,
    },
    RemoveQuery {
        name: String,
    },
    /// Find and replace in a text body (decoded first).
    ReplaceBody {
        phase: Phase,
        pattern: Pattern,
        replacement: String,
    },
    /// Replaces the whole body.
    SetBody {
        phase: Phase,
        body: String,
        /// Also sets `Content-Type`.
        #[serde(default)]
        content_type: Option<String>,
    },
    /// Sets the value at a JSON pointer (RFC 6901) in a JSON body. Missing
    /// object members are created; `-` appends to an array. Only the bytes
    /// of that value change: key order, number text and formatting
    /// elsewhere are kept.
    JsonSet {
        phase: Phase,
        pointer: String,
        value: serde_json::Value,
        /// The value as exact JSON text, e.g. `12345678901234567890` or
        /// `1.50`, which `value` can't always hold. Takes precedence.
        #[serde(default)]
        raw: Option<String>,
    },
    /// Removes the value at a JSON pointer (every member of that name, if
    /// an object repeats it).
    JsonRemove {
        phase: Phase,
        pointer: String,
    },
    /// Sets a field of a form-urlencoded body (every occurrence replaced,
    /// or appended).
    FormSet {
        phase: Phase,
        name: String,
        value: String,
    },
    FormRemove {
        phase: Phase,
        name: String,
    },
    /// Appends a header value, keeping the existing ones; repeated headers
    /// are a `RemoveHeader` followed by several of these.
    AddHeader {
        phase: Phase,
        name: String,
        value: String,
    },
    /// Response only.
    SetStatus {
        status: u16,
    },
    /// Answers without contacting the upstream. Response-phase actions
    /// still run on the answer.
    MapLocal {
        source: MapLocalSource,
        status: u16,
        #[serde(default)]
        headers: Vec<Header>,
        /// `Content-Type` of the answer; guessed from a file's extension if
        /// unset.
        #[serde(default)]
        content_type: Option<String>,
    },
    /// Sends the request to another origin (and optionally path). If
    /// several rules map a request, the last one wins.
    MapRemote {
        #[serde(default)]
        scheme: Option<Scheme>,
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        port: Option<u16>,
        /// Replaces the path (the query is kept).
        #[serde(default)]
        path: Option<String>,
        /// Keep the original `Host` header instead of the new origin's.
        #[serde(default)]
        preserve_host: bool,
    },
    /// Pauses the flow for editing in the Breakpoint Editor (or by an
    /// agent). The request breakpoint fires after every request rewrite.
    Breakpoint {
        request: bool,
        response: bool,
    },
    Delay {
        phase: Phase,
        ms: u32,
    },
    /// Shapes bandwidth with a token bucket; 0 means unlimited. Latency is
    /// added before the response.
    Throttle {
        download_kbps: u32,
        upload_kbps: u32,
        latency_ms: u32,
    },
    /// Stops the flow.
    Block {
        mode: BlockMode,
    },
    /// Runs a script (M14): a rewrite script's `onRequest`/`onResponse` in
    /// its phase, or a mock script's `onMock`, which answers like Map
    /// Local (the first answer wins). A script that throws or times out is
    /// skipped and reported on the flow. A rewrite script's
    /// `onWsMessage` runs on the messages of a matching WebSocket.
    Script {
        script_id: String,
    },
    /// Acts on the messages of a WebSocket whose handshake the rule
    /// matches (M19): text and binary messages, never control frames.
    /// Keep this the last variant.
    WsMessage {
        /// `None` acts on both directions.
        #[serde(default)]
        direction: Option<WsDirection>,
        /// Only messages whose payload contains a match (exact text is a
        /// substring, a glob must match the whole payload, a regex
        /// anywhere). `None` matches every message.
        #[serde(default)]
        filter: Option<Pattern>,
        op: WsOp,
    },
}

/// What an [`Action::WsMessage`] does to a message. Ops run in rule and
/// action order on the message as the earlier ones left it; a drop ends
/// the run, and the breakpoint fires after every rewrite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WsOp {
    /// The message is never forwarded (it is recorded as dropped).
    Drop,
    /// Find and replace in the payload.
    Replace {
        pattern: Pattern,
        replacement: String,
    },
    /// Sets the value at a JSON pointer in a text message, changing only
    /// those bytes (as `Action::JsonSet`).
    JsonSet {
        pointer: String,
        value: serde_json::Value,
        #[serde(default)]
        raw: Option<String>,
    },
    JsonRemove {
        pointer: String,
    },
    /// Holds the message (and the ones after it in its direction).
    Delay {
        ms: u32,
    },
    /// Pauses the message for editing; later messages in its direction
    /// wait behind it.
    Breakpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MapLocalSource {
    Inline {
        body: String,
    },
    File {
        path: String,
    },
    /// Serves files under `root`. The request path is tried in full, then
    /// with leading segments dropped (`/api/v1/a.json` → `v1/a.json` →
    /// `a.json`); a path ending in `/` serves `index.html`.
    Directory {
        root: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlockMode {
    /// Answers with this status and a short text body.
    Status { status: u16 },
    /// Closes the connection (h1) or resets the stream (h2).
    Reset,
}

/// A rule that changed a flow, recorded on the flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct AppliedRule {
    pub rule_id: RuleId,
    pub name: String,
    pub origin: RuleOrigin,
    pub phase: Phase,
}

/// What rules did to one flow. Drives the Edited column and the
/// inspector's rule badges.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct FlowRules {
    pub applied: Vec<AppliedRule>,
    /// Set when a body action or breakpoint couldn't touch a body (over
    /// the buffering cap, or undecodable); the body was forwarded as is.
    pub rewrite_skipped: Option<String>,
    /// A breakpoint timed out and the flow continued unmodified.
    pub breakpoint_timeout: bool,
    /// The response came from Map Local, a mock script or a breakpoint,
    /// not the upstream.
    pub mocked: bool,
    /// Scripts that failed on this flow (M14). Keep this the last field:
    /// a session written before it decodes with one `0` byte appended.
    #[serde(default)]
    pub scripts: Vec<crate::scripts::ScriptIssue>,
}

/// Rule-related settings, kept in `app.sqlite`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct RuleSettings {
    /// How long a breakpoint waits before the flow continues unmodified;
    /// 0 waits forever.
    pub breakpoint_timeout_secs: u32,
}

impl Default for RuleSettings {
    fn default() -> Self {
        RuleSettings {
            breakpoint_timeout_secs: 300,
        }
    }
}

/// A paused flow, waiting for a [`BreakpointResolution`].
pub type BreakpointId = u32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct BreakpointInfo {
    pub id: BreakpointId,
    /// The flow, if it is being recorded.
    pub flow_id: Option<FlowId>,
    pub phase: Phase,
    pub rule_id: RuleId,
    pub rule_name: String,
    /// Milliseconds since the Unix epoch.
    pub paused_at_ms: f64,
    /// When the flow continues on its own; `None` waits forever.
    pub deadline_ms: Option<f64>,
    /// The request as it will be sent (after request rewrites).
    pub request: RequestHead,
    /// The response as it will be returned (response phase only).
    pub response: Option<ResponseHead>,
    /// The paused message's body (decoded), fetched separately.
    pub body: BreakpointBody,
    /// Set for a held WebSocket message (M19): `request` is the handshake
    /// and the body is the payload. `phase` is `request` for a message the
    /// client sent, `response` for one the server sent.
    #[serde(default)]
    pub message: Option<HeldMessage>,
}

/// A WebSocket message held at a breakpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct HeldMessage {
    pub direction: WsDirection,
    /// `text` or `binary`.
    pub opcode: WsOpcode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct BreakpointBody {
    /// Bytes held, decoded.
    pub size: u64,
    /// False above 16 MB, for a body that wasn't buffered, or one that
    /// couldn't be decoded: then only the head can be edited.
    pub editable: bool,
    /// Why the body can't be edited, if it can't.
    pub note: Option<String>,
}

/// How to let a paused flow go on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BreakpointResolution {
    /// Send it on, with any edits.
    Continue {
        #[serde(default)]
        edits: Option<MessageEdits>,
    },
    /// Close the connection (h1) or reset the stream (h2). A held
    /// WebSocket message is dropped instead; the connection stays open.
    Abort,
    /// Answer the client with this response instead (not for WebSocket
    /// messages).
    Respond {
        status: u16,
        #[serde(default)]
        headers: Vec<Header>,
        #[serde(default)]
        body: String,
    },
}

/// Edits to a paused message. Unset fields stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct MessageEdits {
    /// Request only.
    #[serde(default)]
    pub method: Option<String>,
    /// Request only: an absolute URL. Another origin is dialled directly.
    #[serde(default)]
    pub url: Option<String>,
    /// Response only.
    #[serde(default)]
    pub status: Option<u16>,
    /// Replaces every header.
    #[serde(default)]
    pub headers: Option<Vec<Header>>,
    /// Replaces the body (sent identity-encoded). A text WebSocket message
    /// stays text.
    #[serde(default)]
    pub body: Option<String>,
    /// WebSocket messages only: replaces the payload with these bytes
    /// (base64), e.g. a binary message edited as hex. Wins over `body`.
    #[serde(default)]
    pub body_base64: Option<String>,
}

/// The dry run of one rule against a recorded flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct RuleTestResult {
    /// The request matcher matched.
    pub matched: bool,
    /// Why the request matcher didn't match, if it didn't.
    pub mismatch: Option<String>,
    /// Whether the response matcher matched (`None` without a response).
    pub response_matched: Option<bool>,
    /// The request after the rule's request phase.
    pub request: RequestHead,
    pub request_body: Option<String>,
    /// The response after the rule's response phase, or Map Local's.
    pub response: Option<ResponseHead>,
    pub response_body: Option<String>,
    /// What else would happen: blocked, mapped, paused, delayed…
    pub notes: Vec<String>,
    /// `request_body` was cut at the dry run's body limit.
    #[serde(default)]
    pub request_body_truncated: bool,
    #[serde(default)]
    pub response_body_truncated: bool,
}
