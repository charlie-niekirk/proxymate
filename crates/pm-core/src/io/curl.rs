//! Copy as cURL: a recorded request as a POSIX shell command that sends it
//! again.
//!
//! - Everything is single-quoted, so nothing in a URL, header or body is
//!   expanded by the shell.
//! - Headers curl sets itself (Host, Content-Length, hop-by-hop) are left
//!   out; an `Accept-Encoding` that curl can decode becomes `--compressed`.
//! - A text body goes inline with `--data-raw`. A binary (or
//!   content-encoded) body is piped in with `--data-binary @-`, from base64
//!   in the command itself, or from a file the user saves first when it is
//!   too large to paste.

use std::fmt::Write as _;

use base64::Engine as _;
use pm_model::{FlowDetail, HttpVersion};

/// Binary bodies up to this size are embedded as base64.
pub const INLINE_BINARY_MAX: usize = 256 * 1024;

/// The file a large binary body is read from.
pub const BODY_FILE: &str = "request-body.bin";

/// Headers curl derives or that only apply to one hop.
const SKIPPED: &[&str] = &[
    "content-length",
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authorization",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
];

/// Content codings `--compressed` asks for and decodes.
const CURL_CODINGS: &[&str] = &["gzip", "x-gzip", "deflate", "br", "zstd", "identity", "*"];

/// The command for a recorded request. `body` is the request body as
/// recorded (still content-encoded).
pub fn command(detail: &FlowDetail, body: &[u8]) -> String {
    let request = &detail.request;
    let method = request.method.to_ascii_uppercase();
    let mut args: Vec<String> = Vec::new();

    let text_body = (!body.is_empty())
        .then(|| std::str::from_utf8(body).ok())
        .flatten()
        .filter(|text| is_plain_text(text) && detail.request_body.encoding.is_none());
    match method.as_str() {
        "GET" if body.is_empty() => {}
        "POST" if !body.is_empty() => {}
        "HEAD" if body.is_empty() => args.push("--head".into()),
        _ => args.push(format!("-X {}", quote(&method))),
    }

    let authority = authority_of(&request.url);
    let mut compressed = false;
    for header in &request.headers {
        let name = header.name.to_ascii_lowercase();
        if SKIPPED.contains(&name.as_str()) || name.starts_with(':') {
            continue;
        }
        if name == "host" && authority.is_some_and(|a| a.eq_ignore_ascii_case(header.value.trim())) {
            continue;
        }
        if name == "accept-encoding" && curl_decodes(&header.value) {
            compressed = true;
            continue;
        }
        args.push(format!("-H {}", quote(&format!("{}: {}", header.name, header.value))));
    }
    if compressed {
        args.push("--compressed".into());
    }
    if request.version == HttpVersion::Http10 {
        args.push("--http1.0".into());
    }
    if request.url.contains(['[', ']', '{', '}']) {
        // Brackets and braces are curl URL globs (and IPv6 hosts).
        args.push("--globoff".into());
    }

    let mut prefix = String::new();
    if !body.is_empty() {
        match text_body {
            Some(text) => args.push(format!("--data-raw {}", quote(text))),
            None if body.len() <= INLINE_BINARY_MAX => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(body);
                prefix = format!("printf '%s' {} | base64 --decode | ", quote(&encoded));
                args.push("--data-binary @-".into());
            }
            None => {
                let _ = writeln!(
                    prefix,
                    "# The {} request body is binary: save it as {BODY_FILE} first (Save body to file…).",
                    size(body.len())
                );
                args.push(format!("--data-binary @{BODY_FILE}"));
            }
        }
    }

    let mut out = format!("{prefix}curl {}", quote(&request.url));
    for arg in args {
        out.push_str(" \\\n  ");
        out.push_str(&arg);
    }
    out
}

/// `s` single-quoted for a POSIX shell: `'` becomes `'\''`.
pub fn quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./:=@,+%".contains(&b))
    {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Text that survives a copy and paste: no control characters other than
/// tabs and line breaks.
fn is_plain_text(text: &str) -> bool {
    !text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

/// True if every coding in an `Accept-Encoding` value is one curl decodes.
fn curl_decodes(value: &str) -> bool {
    let mut any = false;
    for coding in value.split(',') {
        let coding = coding.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
        if coding.is_empty() {
            continue;
        }
        if !CURL_CODINGS.contains(&coding.as_str()) {
            return false;
        }
        any = true;
    }
    any
}

fn authority_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some(&rest[..end])
}

fn size(len: usize) -> String {
    if len >= 1024 * 1024 {
        format!("{:.1} MB", len as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KB", len as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::{BodyInfo, Header, RequestHead, Timing};

    fn detail(method: &str, url: &str, headers: &[(&str, &str)], encoding: Option<&str>) -> FlowDetail {
        FlowDetail {
            id: 0,
            request: RequestHead {
                method: method.into(),
                url: url.into(),
                version: HttpVersion::Http11,
                headers: headers.iter().map(|(n, v)| Header::new(*n, *v)).collect(),
            },
            response: None,
            request_body: BodyInfo {
                encoding: encoding.map(str::to_owned),
                ..Default::default()
            },
            response_body: BodyInfo::default(),
            error: None,
            tls: None,
            timing: Timing::default(),
            rules: Default::default(),
            trailers: None,
            via: None,
        }
    }

    /// Compares with `tests/golden/curl/<name>.sh`; `UPDATE_GOLDEN=1`
    /// rewrites the file instead.
    fn golden(name: &str, actual: &str) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/curl")
            .join(format!("{name}.sh"));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("{actual}\n")).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            actual,
            expected.trim_end_matches('\n'),
            "golden file {}",
            path.display()
        );
    }

    #[test]
    fn get_with_headers() {
        let d = detail(
            "GET",
            "https://api.example.com/v1/users?page=2&sort=name",
            &[
                ("host", "api.example.com"),
                ("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"),
                ("accept", "application/json"),
                ("accept-encoding", "gzip, deflate, br"),
                ("cookie", "session=abc; theme=dark"),
                ("connection", "keep-alive"),
            ],
            None,
        );
        golden("get_with_headers", &command(&d, b""));
    }

    #[test]
    fn post_json_with_quotes() {
        let d = detail(
            "POST",
            "https://api.example.com/v1/notes",
            &[
                ("content-type", "application/json"),
                ("content-length", "52"),
                ("x-note", "it's \"quoted\""),
            ],
            None,
        );
        let body = br#"{"title":"Rock 'n' roll","body":"line one\nline two"}"#;
        golden("post_json_with_quotes", &command(&d, body));
    }

    #[test]
    fn multiline_form_body() {
        let d = detail(
            "PUT",
            "http://localhost:8080/form",
            &[("content-type", "application/x-www-form-urlencoded")],
            None,
        );
        golden("multiline_form_body", &command(&d, b"a=1&b=two%20words\nc=$HOME `id`"));
    }

    #[test]
    fn binary_body_is_piped_from_base64() {
        let d = detail(
            "POST",
            "https://upload.example.com/put",
            &[("content-type", "application/octet-stream")],
            None,
        );
        golden("binary_body", &command(&d, &[0u8, 1, 2, 0xff, 0xfe, b'\'', b'\n']));
    }

    #[test]
    fn gzipped_body_is_binary() {
        let d = detail(
            "POST",
            "https://api.example.com/ingest",
            &[("content-type", "application/json"), ("content-encoding", "gzip")],
            Some("gzip"),
        );
        // A gzip body that happens to be valid UTF-8 still goes as bytes.
        golden("gzipped_body", &command(&d, b"not really gzip"));
    }

    #[test]
    fn large_binary_body_is_read_from_a_file() {
        let d = detail("POST", "https://upload.example.com/big", &[], None);
        golden("large_binary_body", &command(&d, &vec![0u8; INLINE_BINARY_MAX + 1]));
    }

    #[test]
    fn other_methods_and_versions() {
        let mut d = detail(
            "DELETE",
            "http://[::1]:8080/items/{id}",
            &[("host", "other.test")],
            None,
        );
        d.request.version = HttpVersion::Http10;
        golden("delete_http10_globoff", &command(&d, b""));
        let head = detail(
            "HEAD",
            "https://example.com/",
            &[("accept-encoding", "gzip, sdch")],
            None,
        );
        golden("head_unknown_coding", &command(&head, b""));
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("plain-word"), "plain-word");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote("$HOME"), "'$HOME'");
    }
}
