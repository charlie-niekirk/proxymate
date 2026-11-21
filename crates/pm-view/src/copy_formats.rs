//! The text forms the context menus copy (`lib/copyFormats.ts`): header
//! lines, query strings, cookies, hex dumps, Base64, SSE events, filter
//! terms. Pure, so they're tested without a clipboard.

/// `Name: value`.
pub fn header_line(name: &str, value: &str) -> String {
    format!("{name}: {value}")
}

/// One `Name: value` line per header (repeats kept).
pub fn headers_block<S: AsRef<str>>(rows: &[(S, S)]) -> String {
    rows.iter()
        .map(|(k, v)| header_line(k.as_ref(), v.as_ref()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `k: v` lines, for plain tables (TLS, certificate fields).
pub fn plain_block<S: AsRef<str>>(rows: &[(S, S)]) -> String {
    headers_block(rows)
}

/// `application/x-www-form-urlencoded`, as `URLSearchParams` writes it:
/// spaces as `+`, `*-._` and alphanumerics as they are, the rest
/// percent-encoded as UTF-8.
fn form_encode(s: &str, out: &mut String) {
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            b'*' | b'-' | b'.' | b'_' => out.push(b as char),
            b if b.is_ascii_alphanumeric() => out.push(b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
}

/// One `k=v` pair, URL-encoded as a form or query sends it.
pub fn form_pair(name: &str, value: &str) -> String {
    let mut out = String::new();
    form_encode(name, &mut out);
    out.push('=');
    form_encode(value, &mut out);
    out
}

/// `a=1&b=2`, re-encoded.
pub fn query_string<S: AsRef<str>>(rows: &[(S, S)]) -> String {
    rows.iter()
        .map(|(k, v)| form_pair(k.as_ref(), v.as_ref()))
        .collect::<Vec<_>>()
        .join("&")
}

fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// Pairs as a JSON object in their order (two-space indent, as
/// `JSON.stringify(…, null, 2)`); a repeated name becomes an array.
pub fn pairs_json<S: AsRef<str>>(rows: &[(S, S)]) -> String {
    let mut keys: Vec<(&str, Vec<&str>)> = Vec::new();
    for (k, v) in rows {
        let (k, v) = (k.as_ref(), v.as_ref());
        match keys.iter_mut().find(|(name, _)| *name == k) {
            Some((_, values)) => values.push(v),
            None => keys.push((k, vec![v])),
        }
    }
    if keys.is_empty() {
        return "{}".into();
    }
    let fields: Vec<String> = keys
        .iter()
        .map(|(k, values)| {
            let value = match values.as_slice() {
                [one] => json_string(one),
                many => format!(
                    "[\n{}\n  ]",
                    many.iter()
                        .map(|v| format!("    {}", json_string(v)))
                        .collect::<Vec<_>>()
                        .join(",\n")
                ),
            };
            format!("  {}: {value}", json_string(k))
        })
        .collect();
    format!("{{\n{}\n}}", fields.join(",\n"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
}

fn cookie_pair(part: &str) -> Option<Cookie> {
    let (name, value) = match part.split_once('=') {
        Some((name, value)) => (name.trim(), value.trim()),
        None => (part.trim(), ""),
    };
    (!name.is_empty()).then(|| Cookie {
        name: name.into(),
        value: value.into(),
    })
}

/// The cookies of a `Cookie` header (`a=1; b=2`).
pub fn parse_cookie(header: &str) -> Vec<Cookie> {
    header.split(';').filter_map(cookie_pair).collect()
}

/// The cookie a `Set-Cookie` header sets: its first pair, before the
/// attributes.
pub fn parse_set_cookie(header: &str) -> Option<Cookie> {
    cookie_pair(header.split(';').next().unwrap_or(""))
}

/// The credentials of an `Authorization` value without its scheme
/// (`Bearer abc` → `abc`).
pub fn auth_token(value: &str) -> Option<String> {
    let s = value.trim_start();
    let mut chars = s.char_indices();
    if !chars.next().is_some_and(|(_, c)| c.is_ascii_alphabetic()) {
        return None;
    }
    let scheme_end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(s.len());
    let after = &s[scheme_end..];
    if !after.starts_with(char::is_whitespace) {
        return None;
    }
    let token = after.trim_start();
    // `.` stops at a line break, and the pattern is anchored at the end.
    if token.is_empty() || token.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return None;
    }
    Some(token.trim().to_owned())
}

/// Bytes per hex dump row, as the Hex viewer shows them.
pub const HEX_WIDTH: usize = 16;

/// `de ad be ef`.
pub fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

/// Width of the offset column for a body of `length` bytes.
pub fn offset_digits(length: usize) -> usize {
    format!("{length:x}").len().max(8)
}

/// One row of a hex dump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HexRow {
    pub offset: String,
    /// The bytes, with a gap after eight.
    pub hex: String,
    /// Printable ASCII, `.` for the rest.
    pub ascii: String,
}

pub fn hex_row(bytes: &[u8], row: usize) -> HexRow {
    let start = row * HEX_WIDTH;
    let slice = bytes
        .get(start..(start + HEX_WIDTH).min(bytes.len()))
        .unwrap_or_default();
    let hex = slice
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if i == 8 {
                format!(" {b:02x}")
            } else {
                format!("{b:02x}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let ascii = slice
        .iter()
        .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' })
        .collect();
    HexRow {
        offset: format!("{start:0width$x}", width = offset_digits(bytes.len())),
        hex,
        ascii,
    }
}

/// An `xxd`-style dump of the whole body.
pub fn hex_dump(bytes: &[u8]) -> String {
    let rows = bytes.len().div_ceil(HEX_WIDTH);
    // 16 × "xx", 15 separating spaces and the gap after eight.
    let width = HEX_WIDTH * 3;
    (0..rows)
        .map(|r| {
            let HexRow { offset, hex, ascii } = hex_row(bytes, r);
            format!("{offset}  {hex:<width$}  {ascii}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Standard Base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A server-sent event as it travels: `event:`, `id:`, `retry:` and one
/// `data:` per line.
pub fn sse_wire(event: Option<&str>, id: Option<&str>, data: &str, retry: Option<u64>) -> String {
    let mut lines = Vec::new();
    if let Some(event) = event {
        lines.push(format!("event: {event}"));
    }
    if let Some(id) = id {
        lines.push(format!("id: {id}"));
    }
    if let Some(retry) = retry {
        lines.push(format!("retry: {retry}"));
    }
    lines.extend(data.split('\n').map(|line| format!("data: {line}")));
    lines.join("\n")
}

/// Tab-separated `label  value` lines, which paste into a spreadsheet as
/// two columns.
pub fn table_text<S: AsRef<str>>(rows: &[(S, S)]) -> String {
    rows.iter()
        .map(|(k, v)| format!("{}\t{}", k.as_ref(), v.as_ref()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A filter DSL term. Values with spaces are quoted; the DSL has no escape
/// for `"`, so a value is cut at its first one, which still matches as a
/// substring.
pub fn dsl_term(key: &str, value: &str) -> String {
    let v = value.split('"').next().unwrap_or("");
    if v.is_empty() || v.contains(char::is_whitespace) {
        format!("{key}:\"{v}\"")
    } else {
        format!("{key}:{v}")
    }
}

/// `text` narrowed by `term`, unless it already has it.
pub fn with_term(text: &str, term: &str) -> String {
    let current = text.trim();
    if current.split_whitespace().any(|t| t == term) {
        return current.to_owned();
    }
    if current.is_empty() {
        term.to_owned()
    } else {
        format!("{current} {term}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_header_lines_and_blocks() {
        assert_eq!(
            header_line("Content-Type", "text/html; charset=utf-8"),
            "Content-Type: text/html; charset=utf-8"
        );
        assert_eq!(
            headers_block(&[("Accept", "*/*"), ("Set-Cookie", "a=1"), ("Set-Cookie", "b=2")]),
            "Accept: */*\nSet-Cookie: a=1\nSet-Cookie: b=2"
        );
        assert_eq!(plain_block(&[("Issuer", "R3")]), "Issuer: R3");
    }

    #[test]
    fn re_encodes_query_and_form_pairs() {
        assert_eq!(form_pair("q", "two words&more"), "q=two+words%26more");
        assert_eq!(query_string(&[("x", "1"), ("y", "é"), ("x", "3")]), "x=1&y=%C3%A9&x=3");
    }

    #[test]
    fn turns_pairs_into_json_repeats_as_arrays() {
        let json = pairs_json(&[("a", "1"), ("b", "2"), ("a", "3"), ("a", "4")]);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, serde_json::json!({"a": ["1", "3", "4"], "b": "2"}));
        assert!(json.find("\"a\"").unwrap() < json.find("\"b\"").unwrap(), "order kept");
        assert_eq!(pairs_json(&[("k", "v")]), "{\n  \"k\": \"v\"\n}");
        assert_eq!(pairs_json::<&str>(&[]), "{}");
    }

    #[test]
    fn reads_cookies() {
        let c = |n: &str, v: &str| Cookie {
            name: n.into(),
            value: v.into(),
        };
        assert_eq!(
            parse_cookie("session=abc; theme=dark;  empty=; flag"),
            [c("session", "abc"), c("theme", "dark"), c("empty", ""), c("flag", "")]
        );
        assert_eq!(parse_cookie(""), []);
        assert_eq!(
            parse_set_cookie("id=a3fWa=; Expires=Thu, 21 Oct 2021 07:28:00 GMT; Secure"),
            Some(c("id", "a3fWa="))
        );
        assert_eq!(parse_set_cookie("; Secure"), None);
    }

    #[test]
    fn takes_the_scheme_off_an_authorization_value() {
        assert_eq!(
            auth_token("Bearer eyJhbGciOi.payload.sig").as_deref(),
            Some("eyJhbGciOi.payload.sig")
        );
        assert_eq!(auth_token("Basic dXNlcjpwYXNz").as_deref(), Some("dXNlcjpwYXNz"));
        assert_eq!(auth_token("opaque-token"), None);
    }

    #[test]
    fn dumps_hex_like_the_hex_viewer() {
        let mut bytes = b"Hello, hex view!".to_vec();
        bytes.extend([0x00, 0xff]);
        assert_eq!(hex_bytes(&bytes[..4]), "48 65 6c 6c");
        assert_eq!(
            hex_row(&bytes, 0),
            HexRow {
                offset: "00000000".into(),
                hex: "48 65 6c 6c 6f 2c 20 68  65 78 20 76 69 65 77 21".into(),
                ascii: "Hello, hex view!".into(),
            }
        );
        assert_eq!(
            hex_row(&bytes, 1),
            HexRow {
                offset: "00000010".into(),
                hex: "00 ff".into(),
                ascii: "..".into(),
            }
        );
        let dump = hex_dump(&bytes);
        let lines: Vec<_> = dump.lines().collect();
        assert_eq!(
            lines,
            [
                "00000000  48 65 6c 6c 6f 2c 20 68  65 78 20 76 69 65 77 21  Hello, hex view!".to_owned(),
                format!("00000010  {:<48}  ..", "00 ff"),
            ]
        );
    }

    #[test]
    fn encodes_base64_of_any_size() {
        assert_eq!(base64(b"hi"), "aGk=");
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        let big = vec![0x41; 100_000];
        let encoded = base64(&big);
        assert_eq!(encoded.len(), 100_000usize.div_ceil(3) * 4);
        assert!(encoded.starts_with("QUFB"));
    }

    #[test]
    fn writes_sse_events_as_they_travel() {
        assert_eq!(
            sse_wire(Some("update"), Some("7"), "line 1\nline 2", None),
            "event: update\nid: 7\ndata: line 1\ndata: line 2"
        );
        assert_eq!(sse_wire(None, None, "{}", Some(3000)), "retry: 3000\ndata: {}");
    }

    #[test]
    fn writes_tab_separated_tables() {
        assert_eq!(
            table_text(&[("DNS", "3 ms"), ("Total", "40 ms")]),
            "DNS\t3 ms\nTotal\t40 ms"
        );
    }

    #[test]
    fn builds_filter_terms_the_dsl_reads() {
        assert_eq!(dsl_term("host", "example.com"), "host:example.com");
        assert_eq!(dsl_term("app", "Google Chrome"), "app:\"Google Chrome\"");
        assert_eq!(dsl_term("reqheader", "etag: W/\"abc\""), "reqheader:\"etag: W/\"");
        assert_eq!(dsl_term("app", ""), "app:\"\"");
    }

    #[test]
    fn narrows_the_search_text_without_repeating_a_term() {
        assert_eq!(with_term("", "status:404"), "status:404");
        assert_eq!(with_term("  host:a.com ", "status:404"), "host:a.com status:404");
        assert_eq!(
            with_term("host:a.com status:404", "status:404"),
            "host:a.com status:404"
        );
    }
}
