//! Autocomplete for the search box's filter DSL (parsed by the engine:
//! `pm-core/src/store/query.rs`), ported from `lib/dsl.ts`. It suggests
//! keys, then values for the key being typed: known hosts, domains and apps
//! come from the sidebar snapshot.
//!
//! Positions are byte offsets into the text (the input's caret), always on
//! a character boundary.

/// One entry of the suggestion list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// What the term becomes.
    pub insert: String,
    /// Shown in the list.
    pub label: String,
    pub detail: Option<&'static str>,
    /// A whole term: a space follows it. A key (`host:`) waits for its value.
    pub complete: bool,
}

/// What the box knows about the capture, for value suggestions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DslContext {
    pub hosts: Vec<String>,
    /// Registrable domains, suggested as `*.domain`.
    pub domains: Vec<String>,
    pub apps: Vec<String>,
}

impl DslContext {
    /// From the sidebar: domains with hosts under them are suggested as
    /// `*.domain`, every host (or a lone domain) as itself.
    pub fn from_sidebar(snapshot: &pm_model::SidebarSnapshot) -> DslContext {
        let domains = snapshot.domains.iter().filter(|d| !d.hosts.is_empty());
        DslContext {
            domains: domains.map(|d| d.domain.clone()).collect(),
            hosts: snapshot
                .domains
                .iter()
                .flat_map(|d| {
                    if d.hosts.is_empty() {
                        vec![d.domain.clone()]
                    } else {
                        d.hosts.iter().map(|h| h.host.clone()).collect()
                    }
                })
                .collect(),
            apps: snapshot.apps.iter().map(|a| a.name.clone()).collect(),
        }
    }
}

/// A DSL key and its help.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub key: &'static str,
    pub detail: &'static str,
    /// Numeric keys take an operator instead of `:`.
    pub numeric: bool,
}

const fn key(key: &'static str, detail: &'static str) -> Key {
    Key {
        key,
        detail,
        numeric: false,
    }
}

const fn numeric(key: &'static str, detail: &'static str) -> Key {
    Key {
        key,
        detail,
        numeric: true,
    }
}

pub const DSL_KEYS: [Key; 23] = [
    key("host", "exact or glob; *.example.com includes example.com"),
    key("path", "glob over the path, * crosses /"),
    key("method", "GET, POST…; comma-separated"),
    key("status", "404, 4xx or 400-499"),
    key(
        "ct",
        "content type: json, form, xml, js, css, graphql, document, media…",
    ),
    key("grpc", "gRPC status: 0, not_found; !0 for failed calls"),
    key("app", "client app (substring)"),
    key("scheme", "http, https, ws, wss"),
    key("state", "active, paused, completed, failed, blocked, tunnel"),
    key("is", "edited, pinned, labelled, commented or a state"),
    key("label", "red, orange, yellow, green, blue, purple or any"),
    key("comment", "the flow's comment contains…"),
    key(
        "op",
        "OpenAPI operationId (glob); op:* any operation of an imported spec",
    ),
    key("header", "any header line contains… (deep search)"),
    key("reqheader", "a request header contains… (deep search)"),
    key("resheader", "a response header contains… (deep search)"),
    key("body", "the decoded request or response body contains… (deep search)"),
    key("reqbody", "the request body contains… (deep search)"),
    key("resbody", "the response body contains… (deep search)"),
    numeric("dur", "duration in ms (or 2s): dur>500"),
    numeric("size", "response size: size>1mb"),
    numeric("reqsize", "request size: reqsize>10kb"),
    numeric("id", "flow number: id>=100"),
];

/// The fixed value lists.
fn values(key: &str) -> &'static [&'static str] {
    match key {
        "method" => &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "CONNECT"],
        "status" => &[
            "2xx", "3xx", "4xx", "5xx", "200", "301", "304", "400", "401", "403", "404", "500", "502", "503",
        ],
        "ct" => &[
            "json", "form", "xml", "js", "css", "graphql", "document", "media", "other", "html", "image", "video",
            "font", "pdf", "binary", "grpc", "protobuf", "msgpack", "cbor",
        ],
        "grpc" => &[
            "!0",
            "0",
            "ok",
            "not_found",
            "unavailable",
            "internal",
            "unauthenticated",
            "permission_denied",
            "deadline_exceeded",
        ],
        "scheme" => &["http", "https", "ws", "wss"],
        "state" => &["active", "paused", "completed", "failed", "blocked", "tunnel"],
        "is" => &[
            "edited",
            "pinned",
            "labelled",
            "commented",
            "failed",
            "active",
            "paused",
            "blocked",
            "tunnel",
            "completed",
        ],
        "label" => &["red", "orange", "yellow", "green", "blue", "purple", "any"],
        _ => &[],
    }
}

/// Other spellings of keys.
pub const ALIASES: [(&str, &str); 9] = [
    ("domain", "host"),
    ("client", "app"),
    ("type", "ct"),
    ("content", "ct"),
    ("proto", "scheme"),
    ("respbody", "resbody"),
    ("respheader", "resheader"),
    ("note", "comment"),
    ("operation", "op"),
];

/// The key an alias stands for (or the key itself).
pub fn canonical(key: &str) -> &str {
    ALIASES.iter().find(|(a, _)| *a == key).map_or(key, |(_, k)| k)
}

const MAX: usize = 8;

/// The term the caret is in: where it starts and its text up to the caret.
/// Inside a quoted value the term runs back to the space before its quote.
pub fn term_at(text: &str, caret: usize) -> (usize, &str) {
    let caret = caret.min(text.len());
    let head = &text[..caret];
    let mut quotes = head.matches('"').count() as i64;
    let mut start = caret;
    for (i, c) in head.char_indices().rev() {
        if c == '"' {
            quotes -= 1;
        }
        if c.is_whitespace() && quotes % 2 == 0 {
            break;
        }
        start = i;
    }
    (start, &text[start..caret])
}

/// Suggestions for the term at `caret`.
pub fn suggest(text: &str, caret: usize, ctx: &DslContext) -> Vec<Suggestion> {
    let (_, term) = term_at(text, caret);
    let negate = if term.starts_with('-') { "-" } else { "" };
    let body = &term[negate.len()..];
    let Some(colon) = body.find(':') else {
        if body.contains(['<', '>', '=']) {
            return Vec::new();
        }
        let typed = body.to_lowercase();
        return DSL_KEYS
            .iter()
            .filter(|k| k.key.starts_with(&typed))
            .flat_map(|k| {
                let ops: &[&str] = if k.numeric { &[">", "<"] } else { &[":"] };
                ops.iter().map(move |op| Suggestion {
                    insert: format!("{negate}{}{op}", k.key),
                    label: format!("{}{op}", k.key),
                    detail: Some(k.detail),
                    complete: false,
                })
            })
            .take(MAX)
            .collect();
    };
    let raw_key = body[..colon].to_lowercase();
    let key = canonical(&raw_key);
    let value = &body[colon + 1..];
    // A comma list: complete its last item.
    let done_len = value.rfind(',').map_or(0, |c| c + 1);
    let typed = value[done_len..].to_lowercase();
    let prefix = format!("{negate}{}{}", &body[..=colon], &value[..done_len]);
    let options: Vec<(String, Option<&'static str>)> = match key {
        "host" => ctx
            .domains
            .iter()
            .map(|d| (format!("*.{d}"), Some("the domain and its hosts")))
            .chain(ctx.hosts.iter().map(|h| (h.clone(), None)))
            .collect(),
        "app" => ctx
            .apps
            .iter()
            .map(|a| (if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }, None))
            .collect(),
        _ => values(key).iter().map(|v| ((*v).to_owned(), None)).collect(),
    };
    let plain = |v: &str| {
        let v = v.strip_prefix('"').or_else(|| v.strip_prefix("*.")).unwrap_or(v);
        v.to_lowercase()
    };
    let mut seen = std::collections::HashSet::new();
    let mut matching: Vec<(String, Option<&'static str>)> = options
        .into_iter()
        .filter(|(value, _)| {
            let v = plain(value);
            let hit = v.starts_with(&typed) || (typed.chars().count() > 1 && v.contains(&typed));
            hit && value.to_lowercase() != typed && seen.insert(value.clone())
        })
        .collect();
    // What starts with the typed text first; the sort is stable.
    matching.sort_by_key(|(value, _)| !plain(value).starts_with(&typed));
    matching
        .into_iter()
        .take(MAX)
        .map(|(value, detail)| Suggestion {
            insert: format!("{prefix}{value}"),
            label: value,
            detail,
            complete: true,
        })
        .collect()
}

/// `text` with the term at `caret` replaced by `s`: the new text and caret.
pub fn accept(text: &str, caret: usize, s: &Suggestion) -> (String, usize) {
    let (start, _) = term_at(text, caret);
    let caret = caret.min(text.len());
    let end = text[caret..]
        .char_indices()
        .find(|(_, c)| c.is_whitespace())
        .map_or(text.len(), |(i, _)| caret + i);
    let before = &text[..start];
    let after = text[end..].trim_start();
    let insert = if s.complete {
        format!("{} ", s.insert)
    } else {
        s.insert.clone()
    };
    let rest = match (after.is_empty(), s.complete) {
        (true, _) => String::new(),
        (false, true) => after.to_owned(),
        (false, false) => format!(" {after}"),
    };
    let caret = before.len() + insert.len();
    (format!("{before}{insert}{rest}"), caret)
}

/// Header and body keys: their terms run as a deep search.
const DEEP_KEYS: [&str; 13] = [
    "header",
    "headers",
    "reqheader",
    "resheader",
    "respheader",
    "req.header",
    "res.header",
    "body",
    "reqbody",
    "resbody",
    "respbody",
    "req.body",
    "res.body",
];

/// True if the query has header or body terms, which run as a deep search.
pub fn has_deep_terms(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        let token = token.strip_prefix('-').unwrap_or(token);
        let Some((key, value)) = token.split_once(':') else {
            return false;
        };
        !key.is_empty() && !value.is_empty() && DEEP_KEYS.iter().any(|k| k.eq_ignore_ascii_case(key))
    })
}

/// One match of the search text's term pattern
/// (`(-?)(?:([\w.]+)(:|[<>=]+))?("([^"]*)"|\S+)`).
struct Term<'a> {
    negate: bool,
    key: Option<(&'a str, &'a str)>,
    raw: &'a str,
    quoted: Option<&'a str>,
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.'
}

/// The value at the start of `s`: a quoted phrase, else a run of
/// non-space. `None` at a space or the end.
fn value_at(s: &str) -> Option<(&str, Option<&str>, usize)> {
    if let Some(inner) = s.strip_prefix('"')
        && let Some(close) = inner.find('"')
    {
        return Some((&s[..close + 2], Some(&inner[..close]), close + 2));
    }
    let len = s.find(char::is_whitespace).unwrap_or(s.len());
    (len > 0).then(|| (&s[..len], None, len))
}

fn terms(text: &str) -> Vec<Term<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let Some(c) = rest.chars().next() else { break };
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let try_at = |negate: bool| -> Option<(Term<'_>, usize)> {
            let start = usize::from(negate);
            let s = &rest[start..];
            let word = s.find(|c: char| !is_word(c)).unwrap_or(s.len());
            if word > 0 {
                let after = &s[word..];
                let op = if after.starts_with(':') {
                    1
                } else {
                    after
                        .find(|c: char| !matches!(c, '<' | '>' | '='))
                        .unwrap_or(after.len())
                };
                if op > 0
                    && let Some((raw, quoted, len)) = value_at(&after[op..])
                {
                    let term = Term {
                        negate,
                        key: Some((&s[..word], &after[..op])),
                        raw,
                        quoted,
                    };
                    return Some((term, start + word + op + len));
                }
            }
            let (raw, quoted, len) = value_at(s)?;
            Some((
                Term {
                    negate,
                    key: None,
                    raw,
                    quoted,
                },
                start + len,
            ))
        };
        let found = if c == '-' {
            try_at(true).or_else(|| try_at(false))
        } else {
            try_at(false)
        };
        match found {
            Some((term, len)) => {
                out.push(term);
                i += len;
            }
            None => i += c.len_utf8(),
        }
    }
    out
}

/// Which side a body key searches.
fn body_side(key: &str) -> Option<Option<pm_model::BodyPart>> {
    use pm_model::BodyPart::{Request, Response};
    match key {
        "body" => Some(None),
        "reqbody" | "req.body" => Some(Some(Request)),
        "resbody" | "respbody" | "res.body" => Some(Some(Response)),
        _ => None,
    }
}

/// Keys whose values aren't body text, so a term with one isn't a search
/// for body text.
fn not_body(key: &str) -> bool {
    DSL_KEYS.iter().any(|k| k.key == key)
        || ALIASES.iter().any(|(a, _)| *a == key)
        || matches!(key, "headers" | "req.header" | "res.header")
}

/// What the body viewers highlight: the values of `body:`,
/// `reqbody:`/`resbody:` (for that side) and bare words or quoted phrases,
/// lower-cased, longest first. Negated terms don't count.
pub fn body_search_terms(text: &str, part: pm_model::BodyPart) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    for term in terms(text) {
        if term.negate {
            continue;
        }
        let value = term.quoted.unwrap_or(term.raw).trim();
        if value.is_empty() {
            continue;
        }
        let Some((key, op)) = term.key else {
            // A bare key with no value yet (`body:`) isn't text.
            let bare_key = value
                .strip_suffix(':')
                .is_some_and(|k| !k.is_empty() && k.chars().all(is_word));
            if !bare_key {
                add(value.to_lowercase());
            }
            continue;
        };
        if op != ":" {
            continue;
        }
        let key = key.to_lowercase();
        match body_side(&key) {
            Some(None) => add(value.to_lowercase()),
            Some(Some(side)) if side == part => add(value.to_lowercase()),
            Some(Some(_)) => {}
            None if !not_body(&key) => add(format!("{key}:{value}").to_lowercase()),
            None => {}
        }
    }
    // Longest first; the sort is stable.
    out.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
    out
}

#[cfg(test)]
mod tests {
    use pm_model::BodyPart;

    use super::*;

    fn ctx() -> DslContext {
        DslContext {
            hosts: vec!["api.github.com".into(), "github.com".into(), "localhost".into()],
            domains: vec!["github.com".into()],
            apps: vec!["Safari".into(), "Google Chrome".into()],
        }
    }

    fn labels(s: Vec<Suggestion>) -> Vec<String> {
        s.into_iter().map(|s| s.label).collect()
    }

    #[test]
    fn finds_the_term_at_the_caret_quotes_included() {
        assert_eq!(term_at("host:a sta", 10), (7, "sta"));
        assert_eq!(term_at("app:\"Google Ch", 14), (0, "app:\"Google Ch"));
    }

    #[test]
    fn suggests_keys_numeric_ones_with_operators() {
        assert_eq!(labels(suggest("st", 2, &ctx())), ["status:", "state:"]);
        assert_eq!(labels(suggest("du", 2, &ctx())), ["dur>", "dur<"]);
        assert_eq!(suggest("-ho", 3, &ctx())[0].insert, "-host:");
        assert_eq!(suggest("dur>5", 5, &ctx()), []);
    }

    #[test]
    fn suggests_known_hosts_domains_apps_and_fixed_values() {
        assert_eq!(
            labels(suggest("host:git", 8, &ctx())),
            ["*.github.com", "github.com", "api.github.com"]
        );
        assert!(labels(suggest("host:", 5, &ctx())).contains(&"localhost".into()));
        let apps: Vec<_> = suggest("app:goo", 7, &ctx()).into_iter().map(|s| s.insert).collect();
        assert_eq!(apps, ["app:\"Google Chrome\""]);
        let methods: Vec<_> = suggest("method:get,po", 13, &ctx())
            .into_iter()
            .map(|s| s.insert)
            .collect();
        assert_eq!(methods, ["method:get,POST"]);
        assert_eq!(labels(suggest("status:4", 8, &ctx()))[..2], ["4xx", "400"]);
        assert_eq!(labels(suggest("is:p", 4, &ctx())), ["pinned", "paused"]);
    }

    #[test]
    fn aliases_suggest_their_keys_values() {
        assert_eq!(labels(suggest("domain:loc", 10, &ctx())), ["localhost"]);
        assert_eq!(labels(suggest("client:saf", 10, &ctx())), ["Safari"]);
        assert_eq!(labels(suggest("type:gra", 8, &ctx())), ["graphql"]);
        assert_eq!(canonical("respbody"), "resbody");
        assert_eq!(canonical("host"), "host");
    }

    #[test]
    fn replaces_the_term_and_moves_the_caret() {
        let key = suggest("ho", 2, &ctx()).remove(0);
        assert_eq!(accept("ho api", 2, &key), ("host: api".into(), 5));
        let value = suggest("x host:gi", 9, &ctx()).remove(0);
        assert_eq!(accept("x host:gi", 9, &value), ("x host:*.github.com ".into(), 20));
    }

    #[test]
    fn spots_deep_terms() {
        assert!(has_deep_terms("host:a body:token"));
        assert!(has_deep_terms("-reqheader:x-debug"));
        assert!(has_deep_terms("RESBODY:x"));
        assert!(!has_deep_terms("somebody:x body"));
        assert!(!has_deep_terms("body: x"));
    }

    #[test]
    fn body_search_terms_take_the_right_side_and_bare_text() {
        assert_eq!(
            body_search_terms(
                "host:*.example.com body:needle resbody:\"two words\" -body:skip token",
                BodyPart::Response
            ),
            ["two words", "needle", "token"]
        );
        assert_eq!(body_search_terms("reqbody:abc resbody:xyz", BodyPart::Request), ["abc"]);
        assert!(body_search_terms("status:4xx dur>500 label:red is:commented", BodyPart::Response).is_empty());
        assert!(body_search_terms("body:", BodyPart::Response).is_empty());
    }

    #[test]
    fn the_context_comes_from_the_sidebar() {
        use pm_model::{AppCount, DomainNode, HostCount, SidebarSnapshot};
        let snapshot = SidebarSnapshot {
            store: 0,
            total: 3,
            apps: vec![AppCount {
                name: "curl".into(),
                bundle_id: None,
                has_icon: false,
                remote: false,
                count: 3,
            }],
            domains: vec![
                DomainNode {
                    domain: "github.com".into(),
                    count: 2,
                    hosts: vec![HostCount {
                        host: "api.github.com".into(),
                        count: 2,
                    }],
                },
                DomainNode {
                    domain: "localhost".into(),
                    count: 1,
                    hosts: vec![],
                },
            ],
        };
        let ctx = DslContext::from_sidebar(&snapshot);
        assert_eq!(ctx.domains, ["github.com"]);
        assert_eq!(ctx.hosts, ["api.github.com", "localhost"]);
        assert_eq!(ctx.apps, ["curl"]);
    }
}
