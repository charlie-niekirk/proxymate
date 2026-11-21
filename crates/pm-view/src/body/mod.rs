//! The body viewers' logic (G4; `components/BodyViewer/` and `lib/body/`):
//! the viewer [`registry`], content detection for parts ([`content`]), the
//! lossless JSON reader ([`json_doc`]), pretty print ([`json_format`]),
//! paths ([`json_path`]) and tree rows ([`json_tree`]), markup formatting
//! ([`markup`]), [`multipart`] and [`graphql`] parsing, JS/CSS
//! [`beautify`], body [`search`] and the CodeView's line [`highlight`]ing,
//! with the limits and helpers the Body tab shares.

pub mod beautify;
pub mod content;
pub mod graphql;
pub mod highlight;
pub mod json_doc;
pub mod json_format;
pub mod json_path;
pub mod json_tree;
pub mod markup;
pub mod multipart;
pub mod registry;
pub mod search;

use pm_model::BodyPart;

/// Above this, only the head of a body is loaded until the user asks for
/// all of it ("Load all").
pub const LOAD_LIMIT: u64 = 20 * 1024 * 1024;
/// Above this, code viewers show the CodeView without the editor's
/// highlighting until the user asks ("Highlight anyway").
pub const HIGHLIGHT_LIMIT: usize = 2 * 1024 * 1024;
/// XML and HTML above this aren't re-indented.
pub const FORMAT_MAX: usize = 2 * 1024 * 1024;
/// Minified JS or CSS below this starts beautified.
pub const AUTO_BEAUTIFY_MAX: usize = 512 * 1024;

/// A body's text: UTF-8 with no NUL in its first 8 KB. A multi-byte
/// character cut at the end (a capped load) is dropped, not fatal.
pub fn text_of(bytes: &[u8]) -> Option<&str> {
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(e) if e.error_len().is_none() => std::str::from_utf8(&bytes[..e.valid_up_to()]).ok(),
        Err(_) => None,
    }
}

/// The same check the menu's "Copy body" uses: a NUL near the start.
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(8192)].contains(&0)
}

/// 1234567 → "1,234,567".
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Minified code reads better beautified: long average lines.
pub fn looks_minified(text: &str) -> bool {
    if text.len() >= AUTO_BEAUTIFY_MAX {
        return false;
    }
    let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
    text.len() / lines > 200
}

/// URL-encoded form fields, in order, repeated names kept.
pub fn form_fields(text: &str) -> Vec<(String, String)> {
    text.trim()
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|piece| {
            let (name, value) = piece.split_once('=').unwrap_or((piece, ""));
            (crate::http::form_decode(name), crate::http::form_decode(value))
        })
        .collect()
}

const EXTENSIONS: [(&str, &str); 25] = [
    ("application/json", "json"),
    ("text/html", "html"),
    ("application/xml", "xml"),
    ("text/xml", "xml"),
    ("text/plain", "txt"),
    ("text/css", "css"),
    ("text/javascript", "js"),
    ("application/javascript", "js"),
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/gif", "gif"),
    ("image/webp", "webp"),
    ("image/avif", "avif"),
    ("image/svg+xml", "svg"),
    ("image/x-icon", "ico"),
    ("image/bmp", "bmp"),
    ("video/mp4", "mp4"),
    ("video/webm", "webm"),
    ("audio/mpeg", "mp3"),
    ("audio/ogg", "ogg"),
    ("application/pdf", "pdf"),
    ("font/woff2", "woff2"),
    ("font/woff", "woff"),
    ("font/ttf", "ttf"),
    ("font/otf", "otf"),
];

/// A file name for "Save body…": the URL's last path segment, with an
/// extension that matches the body.
pub fn suggested_file_name(url: &str, mime: &str, part: BodyPart) -> String {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = after_scheme
        .find('/')
        .map_or("", |i| &after_scheme[i..])
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    let last = path
        .split('/')
        .rfind(|s| !s.is_empty())
        .map(crate::http::form_decode)
        .unwrap_or_else(|| "body".into());
    let mut last: String = last
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "_.@+-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if last.is_empty() {
        last = "body".into();
    }
    let base = match part {
        BodyPart::Request => format!("{last}-request"),
        BodyPart::Response => last,
    };
    match EXTENSIONS.iter().find(|(m, _)| *m == mime) {
        Some((_, ext)) if !base.to_lowercase().ends_with(&format!(".{ext}")) => format!("{base}.{ext}"),
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_survives_a_cut_character_but_not_binary() {
        let s = "aé";
        assert_eq!(text_of(&s.as_bytes()[..2]), Some("a"));
        assert_eq!(text_of(b"a\0b"), None);
        assert_eq!(text_of(b"\xff\xfeab"), None);
    }

    #[test]
    fn groups_digits() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn suggests_file_names() {
        assert_eq!(
            suggested_file_name(
                "https://a.example/v1/users.json?x=1",
                "application/json",
                BodyPart::Response
            ),
            "users.json"
        );
        assert_eq!(
            suggested_file_name("https://a.example/v1/users", "application/json", BodyPart::Request),
            "users-request.json"
        );
        assert_eq!(
            suggested_file_name("https://a.example/", "image/png", BodyPart::Response),
            "body.png"
        );
        assert_eq!(
            suggested_file_name("https://a.example/a%20b", "application/x-thing", BodyPart::Response),
            "a_b"
        );
    }

    #[test]
    fn reads_form_fields_and_minified_code() {
        assert_eq!(
            form_fields("a=1&b=x+y&a=%C3%A9&flag\n"),
            [
                ("a".into(), "1".into()),
                ("b".into(), "x y".into()),
                ("a".into(), "é".into()),
                ("flag".into(), String::new())
            ]
        );
        assert!(looks_minified(&"x".repeat(1000)));
        assert!(!looks_minified("a\nb\nc"));
    }
}
