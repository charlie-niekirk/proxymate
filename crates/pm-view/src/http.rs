//! HTTP helpers of the inspector: the port of `lib/http.ts` (display-cased
//! header names, query pairs, telling text from binary, the Raw tab's
//! rebuilt message) with its tests.

use pm_model::Header;

/// Title-cases a lower-case header name (`content-type` → `Content-Type`),
/// as h2 sends them; a name with capitals is kept as it is.
pub fn display_header_name(name: &str) -> String {
    if name.bytes().any(|b| b.is_ascii_uppercase()) {
        return name.to_owned();
    }
    let mut out = String::with_capacity(name.len());
    let mut start = true;
    for c in name.chars() {
        if start {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        start = c == '-';
    }
    out
}

/// The first value of a header, case-insensitively.
pub fn header_value<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

/// The headers as display-cased `(name, value)` rows.
pub fn header_rows(headers: &[Header]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|h| (display_header_name(&h.name), h.value.clone()))
        .collect()
}

/// A URL's query string as ordered, decoded pairs, repeated keys kept
/// (`URLSearchParams`: `+` is a space, empty pieces are skipped).
pub fn query_pairs(url: &str) -> Vec<(String, String)> {
    let Some(q) = url.find('?') else {
        return Vec::new();
    };
    let rest = &url[q + 1..];
    let query = rest.split_once('#').map_or(rest, |(q, _)| q);
    query
        .split('&')
        .filter(|piece| !piece.is_empty())
        .map(|piece| {
            let (name, value) = piece.split_once('=').unwrap_or((piece, ""));
            (form_decode(name), form_decode(value))
        })
        .collect()
}

/// `application/x-www-form-urlencoded` decoding: `+` is a space, `%XX` a
/// byte; bytes that aren't UTF-8 become U+FFFD, as in a browser.
pub fn form_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if let Some((hi, lo)) = bytes
                .get(i + 1)
                .and_then(|&a| Some((hex(a)?, hex(*bytes.get(i + 2)?)?))) =>
            {
                out.push(hi << 4 | lo);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// The UTF-8 text of a body, or `None` if it looks binary (a NUL in the
/// first 8 KB, or bytes that aren't UTF-8).
pub fn body_text(bytes: &[u8]) -> Option<&str> {
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

/// An HTTP message rebuilt from its parsed parts, for the Raw tab. Not the
/// exact wire bytes: header names are display-cased and framing may differ.
pub fn raw_message(start_line: &str, headers: &[Header], body: Option<&str>) -> String {
    let mut out = String::with_capacity(start_line.len() + headers.len() * 32 + body.map_or(0, str::len) + 2);
    out.push_str(start_line);
    for h in headers {
        out.push('\n');
        out.push_str(&display_header_name(&h.name));
        out.push_str(": ");
        out.push_str(&h.value);
    }
    out.push_str("\n\n");
    if let Some(body) = body {
        out.push_str(body);
    }
    out
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
    fn title_cases_lower_case_header_names_only() {
        assert_eq!(display_header_name("content-type"), "Content-Type");
        assert_eq!(display_header_name("x-request-id"), "X-Request-Id");
        assert_eq!(display_header_name("X-API-Key"), "X-API-Key");
        assert_eq!(display_header_name(":authority"), ":authority");
    }

    #[test]
    fn finds_header_values_case_insensitively() {
        assert_eq!(
            header_value(&[h("content-type", "text/plain")], "Content-Type"),
            Some("text/plain")
        );
        assert_eq!(header_value(&[], "host"), None);
    }

    #[test]
    fn parses_query_pairs_in_order_with_repeats() {
        let pairs = query_pairs("http://a/b?x=1&y=two%20words&x=3#frag");
        assert_eq!(
            pairs,
            [
                ("x".into(), "1".into()),
                ("y".into(), "two words".into()),
                ("x".into(), "3".into())
            ]
        );
        assert!(query_pairs("http://a/b").is_empty());
        assert_eq!(
            query_pairs("http://a/?q=a+b&&flag&e=%E2%9C%93&bad=%zz"),
            [
                ("q".into(), "a b".into()),
                ("flag".into(), String::new()),
                ("e".into(), "✓".into()),
                ("bad".into(), "%zz".into())
            ]
        );
    }

    #[test]
    fn tells_text_from_binary() {
        assert_eq!(body_text("héllo".as_bytes()), Some("héllo"));
        assert_eq!(body_text(&[0x1f, 0x8b, 0x08, 0x00]), None);
        assert_eq!(body_text(&[0xff, 0xfe, 0x41]), None);
    }

    #[test]
    fn rebuilds_a_raw_message() {
        assert_eq!(
            raw_message("GET / HTTP/1.1", &[h("host", "a")], None),
            "GET / HTTP/1.1\nHost: a\n\n"
        );
        assert_eq!(
            raw_message("HTTP/1.1 200 OK", &[h("Content-Type", "text/plain")], Some("hi")),
            "HTTP/1.1 200 OK\nContent-Type: text/plain\n\nhi"
        );
    }
}
