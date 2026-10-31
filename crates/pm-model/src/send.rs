//! Requests Proxymate sends itself: Replay, Edit & Replay, Compose, and the
//! MCP `replay_flow` / `send_request` tools (M7). All of them go through
//! `Engine::send(RequestSpec, SendOpts)`, and the flow they make is
//! recorded like proxied traffic, marked with [`SentBy`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::detail::Header;

/// A request to send. `url` is absolute (`http://` or `https://`); the
/// headers go out as given, except that hop-by-hop headers are dropped and
/// `Content-Length` is set from the body.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type, JsonSchema)]
pub struct RequestSpec {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<Header>,
    /// The body; `None` sends none.
    #[serde(default)]
    pub body: Option<String>,
    /// True if `body` is base64 (a binary body), false if it is the text
    /// itself (sent as UTF-8).
    #[serde(default)]
    pub body_base64: bool,
}

/// How [`RequestSpec`]s are sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct SendOpts {
    /// Run the request (and its response) through the rules, as if a
    /// client had sent it through the proxy: rewrites, Map Local,
    /// breakpoints, the throttle. Off, it goes straight to the upstream.
    pub apply_rules: bool,
    /// Resolve only once the response has finished (or failed). Off, the
    /// call returns as soon as the flow is recorded and the exchange
    /// carries on in the background.
    pub wait: bool,
    /// Who asked; shown in the flow table and the Client column.
    pub origin: SendOrigin,
}

impl Default for SendOpts {
    fn default() -> Self {
        SendOpts {
            apply_rules: true,
            wait: false,
            origin: SendOrigin::User,
        }
    }
}

/// Who sent a request through `Engine::send`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SendOrigin {
    /// Replay or Compose in the app.
    User,
    /// An MCP client; `client` is its name, shown in the Client column.
    Agent { client: String },
}

impl SendOrigin {
    pub fn sent_by(&self) -> SentBy {
        match self {
            SendOrigin::User => SentBy::User,
            SendOrigin::Agent { .. } => SentBy::Agent,
        }
    }
}

/// The flow-table marker of a flow Proxymate sent itself (the summary's
/// `sent_by`); `None` there for traffic that came through the proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SentBy {
    User,
    Agent,
    /// One of a Repeat's requests (M17).
    Repeat,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_default_their_optional_fields() {
        let spec: RequestSpec = serde_json::from_str(r#"{"method":"GET","url":"http://a/"}"#).unwrap();
        assert_eq!((spec.headers.len(), spec.body, spec.body_base64), (0, None, false));
        let origin: SendOrigin = serde_json::from_str(r#"{"kind":"agent","client":"claude"}"#).unwrap();
        assert_eq!(origin.sent_by(), SentBy::Agent);
    }
}
