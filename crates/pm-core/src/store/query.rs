//! The filter DSL, shared by the search box and MCP. Terms are separated by
//! spaces and must all match; `-` in front of a term negates it.
//!
//! ```text
//! host:*.example.com method:POST,PUT status:4xx ct:json app:Safari
//! dur>500 size>1mb -host:*.google.com path:/api/* state:failed "two words"
//! ```
//!
//! - `host:` exact, or a glob (`*.example.com` also matches `example.com`).
//! - `path:` a glob over the path (without the query); `*` crosses `/`.
//! - `method:` and `scheme:` take comma-separated values.
//! - `status:` `404`, `4xx`, `400-499` or a list of those.
//! - `ct:` a content chip (`json form xml js css graphql document media
//!   other`) or kind (`html image video font pdf binary …`).
//! - `app:` a case-insensitive substring of the client app.
//! - `state:` `active paused completed failed blocked tunnel`; `is:edited`.
//! - `dur` (ms, or with `s`), `size` (response), `reqsize`, `id`: compare with
//!   `> >= < <= =`; sizes take `b kb mb gb`.
//! - `is:pinned`.
//! - Deep terms, which need the flow's detail or bodies: `header:` (a
//!   substring of any `name: value` line; `reqheader:`/`resheader:` for one
//!   side) and `body:` (a substring of the decoded body; `reqbody:`,
//!   `resbody:`). [`FlowQuery::matches`] can't judge them from a summary
//!   and passes them; [`FlowQuery::deep`] has them for a deep search.
//! - Anything else (or a quoted phrase) is a case-insensitive substring of
//!   `host[:port]/path?query`.
//!
//! In lenient mode (the search box) a term that doesn't parse is searched
//! as text; in strict mode (MCP) it is an error, so agents learn about typos.

use globset::{GlobBuilder, GlobMatcher};
use pm_model::{ContentGroup, ContentKind, FlowId, FlowState, FlowSummary, Scheme};

use super::deep::{DeepQuery, DeepTerm, Side};
use super::filter::contains_ignore_ascii_case;

#[derive(Debug, Clone, Default)]
pub struct FlowQuery {
    terms: Vec<(bool, Term)>,
}

#[derive(Debug, Clone)]
enum Term {
    /// Lower-cased.
    Text(Vec<u8>),
    Host(HostMatch),
    Path(GlobMatcher),
    Method(Vec<String>),
    Scheme(Vec<Scheme>),
    Status(Vec<(u16, u16)>),
    Content(Vec<ContentKind>),
    /// Lower-cased.
    App(Vec<u8>),
    State(Vec<FlowState>),
    Edited,
    Pinned,
    Num(Field, Cmp, f64),
    /// Needs the detail or a body: judged by a deep search.
    Deep(DeepTerm),
}

#[derive(Debug, Clone)]
enum HostMatch {
    Exact(String),
    Domain(String),
    Glob(GlobMatcher),
}

#[derive(Debug, Clone, Copy)]
enum Field {
    Duration,
    Size,
    ReqSize,
    Id,
}

#[derive(Debug, Clone, Copy)]
enum Cmp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
}

const KEYS: &str = "host, path, method, scheme, status, ct, app, state, is, dur, size, reqsize, id, header, reqheader, resheader, body, reqbody, resbody";

impl FlowQuery {
    /// Parses a query; a term that doesn't parse is an error naming it.
    pub fn parse(input: &str) -> Result<FlowQuery, String> {
        let mut terms = Vec::new();
        for raw in tokens(input) {
            terms.push(parse_term(&raw)?);
        }
        Ok(FlowQuery { terms })
    }

    /// Parses a query from the search box: terms that don't parse are
    /// searched as text.
    pub fn parse_lenient(input: &str) -> FlowQuery {
        let terms = tokens(input)
            .into_iter()
            .map(|raw| {
                parse_term(&raw).unwrap_or_else(|_| (false, Term::Text(raw.text.to_ascii_lowercase().into_bytes())))
            })
            .collect();
        FlowQuery { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Judges every term a summary can; deep terms pass.
    pub fn matches(&self, f: &FlowSummary) -> bool {
        self.terms
            .iter()
            .all(|(negate, term)| matches!(term, Term::Deep(_)) || term.matches(f) != *negate)
    }

    /// The header and body terms, if there are any.
    pub fn deep(&self) -> Option<DeepQuery> {
        let terms: Vec<(bool, DeepTerm)> = self
            .terms
            .iter()
            .filter_map(|(negate, term)| match term {
                Term::Deep(deep) => Some((*negate, deep.clone())),
                _ => None,
            })
            .collect();
        (!terms.is_empty()).then(|| DeepQuery::new(terms))
    }
}

struct Token {
    text: String,
    quoted: bool,
}

/// Splits on whitespace; double quotes group a phrase (`"a b"`, or
/// `host:"a b"` for a value).
fn tokens(input: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    for c in input.chars() {
        match c {
            '"' => {
                // Only a phrase opened at the start of a term is pure text;
                // `host:"a b"` is still a key.
                if !in_quotes && current.is_empty() {
                    quoted = true;
                }
                in_quotes = !in_quotes;
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    out.push(Token {
                        text: std::mem::take(&mut current),
                        quoted,
                    });
                }
                quoted = false;
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(Token { text: current, quoted });
    }
    out.retain(|t| !t.text.is_empty() && t.text != "-");
    out
}

fn parse_term(token: &Token) -> Result<(bool, Term), String> {
    let (negate, body) = match token.text.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, token.text.as_str()),
    };
    let text = || Term::Text(body.to_ascii_lowercase().into_bytes());
    if token.quoted {
        return Ok((negate, text()));
    }
    // `dur>500`, `size<=1mb`, `id=4`.
    if let Some(at) = body.find(['>', '<', '=']) {
        let (key, rest) = body.split_at(at);
        if let Some(field) = numeric_field(key) {
            let (cmp, value) = match rest.as_bytes() {
                [b'>', b'=', ..] => (Cmp::Ge, &rest[2..]),
                [b'<', b'=', ..] => (Cmp::Le, &rest[2..]),
                [b'>', ..] => (Cmp::Gt, &rest[1..]),
                [b'<', ..] => (Cmp::Lt, &rest[1..]),
                _ => (Cmp::Eq, &rest[1..]),
            };
            let value = parse_number(field, value).ok_or_else(|| format!("{body:?}: can't read {value:?}"))?;
            return Ok((negate, Term::Num(field, cmp, value)));
        }
    }
    let Some((key, value)) = body.split_once(':') else {
        return Ok((negate, text()));
    };
    let key = key.to_ascii_lowercase();
    if value.is_empty() {
        return Err(format!("{body:?}: {key}: needs a value"));
    }
    let bad = |what: &str| format!("{body:?}: {what}");
    let list = || value.split(',').map(str::trim).filter(|v| !v.is_empty());
    let term = match key.as_str() {
        "host" | "domain" => Term::Host(host_match(value).map_err(|e| bad(&e))?),
        "path" => Term::Path(glob(value).map_err(|e| bad(&e))?),
        "method" => Term::Method(list().map(str::to_ascii_uppercase).collect()),
        "scheme" | "proto" => Term::Scheme(
            list()
                .map(|s| match s.to_ascii_lowercase().as_str() {
                    "http" => Ok(Scheme::Http),
                    "https" => Ok(Scheme::Https),
                    "ws" => Ok(Scheme::Ws),
                    "wss" => Ok(Scheme::Wss),
                    other => Err(bad(&format!("unknown scheme {other:?} (http, https, ws, wss)"))),
                })
                .collect::<Result<_, _>>()?,
        ),
        "status" => Term::Status(
            list()
                .map(status_range)
                .collect::<Option<_>>()
                .ok_or_else(|| bad("status takes 404, 4xx or 400-499 (comma-separated)"))?,
        ),
        "ct" | "type" | "content" => Term::Content(
            list()
                .map(|c| content_kinds(c).ok_or_else(|| bad(&format!("unknown content type {c:?}"))))
                .collect::<Result<Vec<_>, _>>()?
                .concat(),
        ),
        "app" | "client" => Term::App(value.to_ascii_lowercase().into_bytes()),
        "state" => Term::State(
            list()
                .map(|s| state(s).ok_or_else(|| bad(&format!("unknown state {s:?}"))))
                .collect::<Result<_, _>>()?,
        ),
        "is" => match value.to_ascii_lowercase().as_str() {
            "edited" => Term::Edited,
            "pinned" => Term::Pinned,
            other => match state(other) {
                Some(s) => Term::State(vec![s]),
                None => return Err(bad("is: takes edited, pinned or a state")),
            },
        },
        "header" | "headers" => Term::Deep(DeepTerm::header(Side::Both, value)),
        "reqheader" | "req.header" => Term::Deep(DeepTerm::header(Side::Request, value)),
        "resheader" | "respheader" | "res.header" => Term::Deep(DeepTerm::header(Side::Response, value)),
        "body" => Term::Deep(DeepTerm::body(Side::Both, value)),
        "reqbody" | "req.body" => Term::Deep(DeepTerm::body(Side::Request, value)),
        "resbody" | "respbody" | "res.body" => Term::Deep(DeepTerm::body(Side::Response, value)),
        _ => return Err(format!("{body:?}: unknown key {key:?} (known: {KEYS})")),
    };
    Ok((negate, term))
}

fn numeric_field(key: &str) -> Option<Field> {
    match key.to_ascii_lowercase().as_str() {
        "dur" | "duration" | "time" => Some(Field::Duration),
        "size" | "respsize" => Some(Field::Size),
        "reqsize" => Some(Field::ReqSize),
        "id" => Some(Field::Id),
        _ => None,
    }
}

fn parse_number(field: Field, value: &str) -> Option<f64> {
    let value = value.trim().to_ascii_lowercase();
    let split = value
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number: f64 = number.parse().ok()?;
    let scale = match (field, unit) {
        (_, "") => 1.0,
        (Field::Duration, "ms") => 1.0,
        (Field::Duration, "s") => 1000.0,
        (Field::Size | Field::ReqSize, "b") => 1.0,
        (Field::Size | Field::ReqSize, "k" | "kb") => 1024.0,
        (Field::Size | Field::ReqSize, "m" | "mb") => 1024.0 * 1024.0,
        (Field::Size | Field::ReqSize, "g" | "gb") => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some(number * scale)
}

fn glob(pattern: &str) -> Result<GlobMatcher, String> {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(false)
        .build()
        .map(|g| g.compile_matcher())
        .map_err(|e| e.to_string())
}

fn host_match(value: &str) -> Result<HostMatch, String> {
    let value = value.to_ascii_lowercase();
    if let Some(domain) = value.strip_prefix("*.")
        && !domain.contains(['*', '?', '['])
    {
        return Ok(HostMatch::Domain(domain.to_owned()));
    }
    if value.contains(['*', '?', '[', '{']) {
        return glob(&value).map(HostMatch::Glob);
    }
    Ok(HostMatch::Exact(value))
}

fn status_range(s: &str) -> Option<(u16, u16)> {
    let lower = s.to_ascii_lowercase();
    if let Some(class) = lower.strip_suffix("xx") {
        let d: u16 = class.parse().ok().filter(|d| (1..=5).contains(d))?;
        return Some((d * 100, d * 100 + 99));
    }
    if let Some((from, to)) = lower.split_once('-') {
        return Some((from.parse().ok()?, to.parse().ok()?));
    }
    let code = lower.parse().ok()?;
    Some((code, code))
}

fn state(s: &str) -> Option<FlowState> {
    Some(match s.to_ascii_lowercase().as_str() {
        "active" => FlowState::Active,
        "paused" => FlowState::Paused,
        "completed" | "done" => FlowState::Completed,
        "failed" | "error" => FlowState::Failed,
        "blocked" => FlowState::Blocked,
        "tunnel" => FlowState::Tunnel,
        _ => return None,
    })
}

const ALL_KINDS: [ContentKind; 17] = [
    ContentKind::None,
    ContentKind::Json,
    ContentKind::Xml,
    ContentKind::Html,
    ContentKind::FormUrlEncoded,
    ContentKind::Multipart,
    ContentKind::GraphQl,
    ContentKind::Javascript,
    ContentKind::Css,
    ContentKind::Text,
    ContentKind::Image,
    ContentKind::Svg,
    ContentKind::Video,
    ContentKind::Audio,
    ContentKind::Font,
    ContentKind::Pdf,
    ContentKind::Binary,
];

/// A chip name covers its group's kinds; a kind name just itself.
fn content_kinds(name: &str) -> Option<Vec<ContentKind>> {
    let group = match name.to_ascii_lowercase().as_str() {
        "json" => Some(ContentGroup::Json),
        "form" => Some(ContentGroup::Form),
        "xml" => Some(ContentGroup::Xml),
        "js" | "javascript" => Some(ContentGroup::Js),
        "css" => Some(ContentGroup::Css),
        "graphql" => Some(ContentGroup::GraphQl),
        "document" | "doc" => Some(ContentGroup::Document),
        "media" => Some(ContentGroup::Media),
        "other" => Some(ContentGroup::Other),
        _ => None,
    };
    if let Some(group) = group {
        return Some(
            ALL_KINDS
                .into_iter()
                .filter(|k| ContentGroup::of(*k) == group)
                .collect(),
        );
    }
    let kind = match name.to_ascii_lowercase().as_str() {
        "html" => ContentKind::Html,
        "text" => ContentKind::Text,
        "image" | "img" => ContentKind::Image,
        "svg" => ContentKind::Svg,
        "video" => ContentKind::Video,
        "audio" => ContentKind::Audio,
        "font" => ContentKind::Font,
        "pdf" => ContentKind::Pdf,
        "binary" => ContentKind::Binary,
        "multipart" => ContentKind::Multipart,
        "none" => ContentKind::None,
        _ => return None,
    };
    Some(vec![kind])
}

impl Term {
    fn matches(&self, f: &FlowSummary) -> bool {
        match self {
            Term::Text(needle) => url_contains(f, needle),
            Term::Host(HostMatch::Exact(host)) => f.host.eq_ignore_ascii_case(host),
            Term::Host(HostMatch::Domain(domain)) => super::in_domain(&f.host, domain),
            Term::Host(HostMatch::Glob(glob)) => glob.is_match(&*f.host),
            Term::Path(glob) => glob.is_match(&f.path),
            Term::Method(methods) => methods.iter().any(|m| m.eq_ignore_ascii_case(&f.method)),
            Term::Scheme(schemes) => schemes.contains(&f.scheme),
            Term::Status(ranges) => f
                .status
                .is_some_and(|s| ranges.iter().any(|(from, to)| (*from..=*to).contains(&s))),
            Term::Content(kinds) => kinds.contains(&f.content_kind),
            Term::App(needle) => f
                .client
                .as_deref()
                .is_some_and(|c| needle.is_empty() || contains_ignore_ascii_case(c.as_bytes(), needle)),
            Term::State(states) => states.contains(&f.state),
            Term::Edited => f.edited,
            Term::Pinned => f.pinned,
            Term::Deep(_) => true,
            Term::Num(field, cmp, value) => {
                let actual = match field {
                    Field::Duration => match f.duration_ms {
                        Some(d) => f64::from(d),
                        None => return false,
                    },
                    Field::Size => f.resp_size as f64,
                    Field::ReqSize => f.req_size as f64,
                    Field::Id => f64::from(f.id as FlowId),
                };
                match cmp {
                    Cmp::Gt => actual > *value,
                    Cmp::Ge => actual >= *value,
                    Cmp::Lt => actual < *value,
                    Cmp::Le => actual <= *value,
                    Cmp::Eq => actual == *value,
                }
            }
        }
    }
}

/// `host[:port]path[?query]` contains `needle` (lower-case), case-insensitively.
fn url_contains(f: &FlowSummary, needle: &[u8]) -> bool {
    use std::cell::RefCell;
    thread_local! {
        static URL: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }
    if needle.is_empty() {
        return true;
    }
    URL.with_borrow_mut(|url| {
        url.clear();
        url.extend_from_slice(f.host.as_bytes());
        if !f.is_default_port() {
            url.push(b':');
            url.extend_from_slice(f.port.to_string().as_bytes());
        }
        url.extend_from_slice(f.path.as_bytes());
        if !f.query.is_empty() {
            url.push(b'?');
            url.extend_from_slice(f.query.as_bytes());
        }
        contains_ignore_ascii_case(url, needle)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::{HttpVersion, TlsStatus};

    fn flow() -> FlowSummary {
        FlowSummary {
            id: 7,
            state: FlowState::Completed,
            scheme: Scheme::Https,
            method: "POST".into(),
            host: "api.Example.com".into(),
            port: 443,
            path: "/v1/users/42".into(),
            query: "expand=1".into(),
            status: Some(404),
            content_kind: ContentKind::Json,
            client: Some("Google Chrome".into()),
            started_at_ms: 0.0,
            duration_ms: Some(750),
            req_size: 10,
            resp_size: 2 * 1024 * 1024,
            edited: true,
            http_version: HttpVersion::Http2,
            tls: TlsStatus::Intercepted,
            graphql_op: None,
            sent_by: None,
            pinned: false,
        }
    }

    fn matches(q: &str) -> bool {
        FlowQuery::parse(q)
            .unwrap_or_else(|e| panic!("{q}: {e}"))
            .matches(&flow())
    }

    #[test]
    fn keys_match_their_fields() {
        for q in [
            "host:api.example.com",
            "host:*.example.com",
            "host:*.EXAMPLE.com",
            "host:api.*",
            "path:/v1/users/*",
            "path:/v1/*/42",
            "method:get,post",
            "scheme:https",
            "status:4xx",
            "status:400-499",
            "status:200,404",
            "ct:json",
            "app:chrome",
            "state:completed",
            "is:edited",
            "dur>500",
            "dur<=1s",
            "size>1mb",
            "reqsize<1kb",
            "id=7",
            "users/42",
            "\"example.com/v1\"",
            "-host:*.google.com",
            "-status:5xx host:*.example.com dur>=750",
        ] {
            assert!(matches(q), "{q} should match");
        }
        for q in [
            "host:example.com",
            "path:/v2/*",
            "method:GET",
            "status:2xx",
            "ct:media",
            "app:safari",
            "state:failed",
            "dur>1s",
            "size<1mb",
            "-ct:json",
            "orders",
        ] {
            assert!(!matches(q), "{q} should not match");
        }
    }

    #[test]
    fn strict_parsing_names_the_bad_term() {
        assert!(FlowQuery::parse("hots:x").unwrap_err().contains("unknown key \"hots\""));
        assert!(FlowQuery::parse("status:abc").unwrap_err().contains("status"));
        assert!(FlowQuery::parse("dur>fast").is_err());
        assert!(FlowQuery::parse("").unwrap().is_empty());
    }

    #[test]
    fn deep_terms_pass_the_summary_and_are_kept_for_a_deep_search() {
        let q = FlowQuery::parse("host:*.example.com body:needle -reqheader:\"x-debug: 1\"").unwrap();
        assert!(q.matches(&flow()));
        assert_eq!(q.deep().map(|d| d.len()), Some(2));
        assert!(FlowQuery::parse("host:*.example.com").unwrap().deep().is_none());
        assert!(!FlowQuery::parse("host:other.com body:x").unwrap().matches(&flow()));
        assert!(matches("is:pinned") == flow().pinned);
    }

    #[test]
    fn lenient_parsing_falls_back_to_text() {
        let f = FlowSummary {
            host: "localhost".into(),
            port: 8080,
            scheme: Scheme::Http,
            ..flow()
        };
        assert!(FlowQuery::parse_lenient("localhost:8080").matches(&f));
        assert!(FlowQuery::parse_lenient("status:4xx localhost").matches(&f));
    }
}
