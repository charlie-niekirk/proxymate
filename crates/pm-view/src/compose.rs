//! Compose and Edit & Replay (`lib/compose.ts`): the draft the window
//! edits, its content-type picker, the editor language for a content type,
//! and the `RequestSpec` it sends.

use pm_model::{Header, RequestSpec, Scheme};

use crate::kv::{self, KvRow};

/// The Content-Type choices of the body's picker (`None`: no header).
pub const CONTENT_TYPES: [(&str, Option<&str>); 8] = [
    ("None", None),
    ("JSON", Some("application/json")),
    ("Form", Some("application/x-www-form-urlencoded")),
    ("Text", Some("text/plain")),
    ("XML", Some("application/xml")),
    ("HTML", Some("text/html")),
    ("JavaScript", Some("application/javascript")),
    ("GraphQL", Some("application/graphql")),
];

pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// Replay needs an HTTP request that isn't a tunnel or a WebSocket.
pub fn is_sendable(method: &str, scheme: Scheme) -> bool {
    method != "CONNECT" && !matches!(scheme, Scheme::Ws | Scheme::Wss)
}

/// The toast for Replay on a flow it can't send.
pub const NOT_SENDABLE: (&str, &str) = ("Can't replay this flow", "tunnels and WebSockets can't be sent again yet");
pub const NOTHING_TO_REPLAY: (&str, &str) = ("Nothing to replay", "select a flow first");

/// Compose's window description.
pub const DESCRIPTION: &str =
    "Sent from Proxymate and recorded as a new flow. Host and Content-Length are set from the URL and the body.";

/// What Compose edits. A binary body (from Edit & Replay) stays base64.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposeDraft {
    pub method: String,
    pub url: String,
    pub headers: Vec<KvRow>,
    pub body: String,
    pub body_base64: bool,
}

impl Default for ComposeDraft {
    /// A new request.
    fn default() -> Self {
        ComposeDraft {
            method: "GET".into(),
            url: "https://".into(),
            headers: vec![KvRow::new("accept", "*/*")],
            body: String::new(),
            body_base64: false,
        }
    }
}

impl ComposeDraft {
    /// A recorded request (Edit & Replay) as a draft.
    pub fn of(spec: &RequestSpec) -> Self {
        ComposeDraft {
            method: spec.method.clone(),
            url: spec.url.clone(),
            headers: kv::from_headers(&spec.headers),
            body: spec.body.clone().unwrap_or_default(),
            body_base64: spec.body_base64,
        }
    }

    /// What Send sends: the method upper-cased, the URL trimmed, nameless
    /// headers dropped, an empty body as none.
    pub fn spec(&self) -> RequestSpec {
        RequestSpec {
            method: self.method.trim().to_uppercase(),
            url: self.url.trim().to_owned(),
            headers: kv::to_headers(&self.headers),
            body: (!self.body.is_empty()).then(|| self.body.clone()),
            body_base64: self.body_base64,
        }
    }

    pub fn content_type(&self) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.trim().eq_ignore_ascii_case("content-type"))
            .map(|h| h.value.as_str())
    }

    /// The picker's choice: a listed type, or the header's own value.
    pub fn content_type_choice(&self) -> ContentTypeChoice {
        let current = self.content_type();
        let mime = current.map(|c| c.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
        match CONTENT_TYPES.iter().find(|(_, v)| v.map(str::to_owned) == mime) {
            Some((label, _)) => ContentTypeChoice::Listed(label),
            None => ContentTypeChoice::Custom(current.unwrap_or_default().to_owned()),
        }
    }

    /// Sets (or, with `None`, removes) the Content-Type header in place.
    pub fn set_content_type(&mut self, value: Option<&str>) {
        let at = self
            .headers
            .iter()
            .position(|h| h.name.trim().eq_ignore_ascii_case("content-type"));
        match (at, value) {
            (Some(i), None) => {
                self.headers.remove(i);
            }
            (Some(i), Some(v)) => self.headers[i].value = v.to_owned(),
            (None, Some(v)) => self.headers.push(KvRow::new("content-type", v)),
            (None, None) => {}
        }
    }

    /// The binary body's size, from its base64.
    pub fn binary_size(&self) -> usize {
        self.body.len() * 3 / 4
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentTypeChoice {
    Listed(&'static str),
    Custom(String),
}

/// `withContentType` over plain headers.
pub fn with_content_type(headers: &[Header], value: Option<&str>) -> Vec<Header> {
    let at = headers
        .iter()
        .position(|h| h.name.trim().eq_ignore_ascii_case("content-type"));
    let mut out = headers.to_vec();
    match (at, value) {
        (Some(i), None) => {
            out.remove(i);
        }
        (Some(i), Some(v)) => out[i].value = v.to_owned(),
        (None, Some(v)) => out.push(Header {
            name: "content-type".into(),
            value: v.to_owned(),
        }),
        (None, None) => {}
    }
    out
}

/// The editor grammar for a Content-Type (`languageFor`, with XML through
/// the HTML grammar as G4's viewers do).
pub fn language_for(content_type: Option<&str>) -> &'static str {
    let mime = content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime == "application/json" || mime.ends_with("+json") {
        "json"
    } else if mime == "application/graphql" {
        "graphql"
    } else if mime.ends_with("/xml") || mime.ends_with("+xml") || mime == "text/html" {
        "html"
    } else if mime.ends_with("javascript") {
        "javascript"
    } else if mime == "text/css" {
        "css"
    } else {
        "text"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(name: &str, value: &str) -> Header {
        Header {
            name: name.into(),
            value: value.into(),
        }
    }

    #[test]
    fn sets_replaces_and_removes_the_content_type_in_place() {
        let headers = vec![h("accept", "*/*"), h("Content-Type", "text/plain")];
        assert_eq!(
            with_content_type(&headers, Some("application/json")),
            vec![h("accept", "*/*"), h("Content-Type", "application/json")]
        );
        assert_eq!(with_content_type(&headers, None), vec![h("accept", "*/*")]);
        assert_eq!(
            with_content_type(&[], Some("text/plain")),
            vec![h("content-type", "text/plain")]
        );
        let mut draft = ComposeDraft::default();
        draft.set_content_type(Some("text/plain"));
        assert_eq!(draft.content_type(), Some("text/plain"));
        assert_eq!(draft.content_type_choice(), ContentTypeChoice::Listed("Text"));
        draft.set_content_type(Some("application/json; charset=utf-8"));
        assert_eq!(draft.content_type_choice(), ContentTypeChoice::Listed("JSON"));
        draft.set_content_type(Some("image/png"));
        assert_eq!(draft.content_type_choice(), ContentTypeChoice::Custom("image/png".into()));
        draft.set_content_type(None);
        assert_eq!(draft.content_type_choice(), ContentTypeChoice::Listed("None"));
        assert_eq!(draft.headers.len(), 1);
    }

    #[test]
    fn picks_the_editor_language_from_the_content_type() {
        assert_eq!(language_for(Some("application/json; charset=utf-8")), "json");
        assert_eq!(language_for(Some("application/vnd.api+json")), "json");
        assert_eq!(language_for(Some("application/soap+xml")), "html");
        assert_eq!(language_for(Some("application/graphql")), "graphql");
        assert_eq!(language_for(Some("text/javascript")), "javascript");
        assert_eq!(language_for(None), "text");
    }

    #[test]
    fn replays_only_plain_requests() {
        assert!(is_sendable("POST", Scheme::Https));
        assert!(!is_sendable("CONNECT", Scheme::Https));
        assert!(!is_sendable("GET", Scheme::Wss));
    }

    #[test]
    fn turns_a_draft_into_a_spec() {
        let draft = ComposeDraft {
            method: " post ".into(),
            headers: vec![KvRow::new(" ", "x"), KvRow::new("x-a", "1")],
            ..Default::default()
        };
        assert_eq!(
            draft.spec(),
            RequestSpec {
                method: "POST".into(),
                url: "https://".into(),
                headers: vec![h("x-a", "1")],
                body: None,
                body_base64: false,
            }
        );
        let binary = ComposeDraft::of(&RequestSpec {
            method: "PUT".into(),
            url: "http://a/".into(),
            headers: Vec::new(),
            body: Some("AAE=".into()),
            body_base64: true,
        });
        let spec = binary.spec();
        assert_eq!((spec.body.as_deref(), spec.body_base64), (Some("AAE="), true));
        assert_eq!(binary.binary_size(), 3);
    }
}
