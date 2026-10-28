//! Content-kind detection: the `Content-Type` header first, then magic-byte
//! sniffing, then a text heuristic (JSON or XML by the first non-space byte).
//!
//! Detection looks only at the head of a body ([`SNIFF_LEN`] bytes, decoded
//! if the body is content-encoded), so it is cheap enough to run on every
//! flow as it finishes. GraphQL requests are recognised from their JSON
//! envelope, which may need the whole (small) request body.

use pm_model::ContentKind;

/// Bytes of a body that sniffing looks at.
pub const SNIFF_LEN: usize = 16 * 1024;
/// GraphQL envelopes are parsed only up to this size.
pub const GRAPHQL_MAX: usize = 1024 * 1024;

/// What detection concluded about a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub kind: ContentKind,
    /// The MIME type to serve the body as, e.g. `image/webp`.
    pub mime: Option<String>,
}

impl Detected {
    fn new(kind: ContentKind, mime: impl Into<String>) -> Detected {
        Detected {
            kind,
            mime: Some(mime.into()),
        }
    }
}

/// A GraphQL operation found in a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphQlRequest {
    /// `operationName`, or the name in the query text; `None` if anonymous.
    pub operation: Option<String>,
}

/// The kind a `Content-Type` value denotes, ignoring parameters.
pub fn kind_from_content_type(value: &str) -> ContentKind {
    let mime = essence(value);
    let (top, sub) = mime.split_once('/').unwrap_or((mime.as_str(), ""));
    match (top, sub) {
        ("", _) => ContentKind::None,
        // Before `+json`: `application/grpc+json` is still gRPC framing.
        ("application", s) if is_grpc_subtype(s) => ContentKind::Grpc,
        ("application", "x-protobuf" | "protobuf" | "vnd.google.protobuf" | "x-google-protobuf") => {
            ContentKind::Protobuf
        }
        ("application", "msgpack" | "x-msgpack" | "vnd.msgpack") => ContentKind::MessagePack,
        ("application", "cbor") => ContentKind::Cbor,
        (_, "json") | (_, "graphql-response+json") => ContentKind::Json,
        (_, "graphql") | (_, "graphql+json") => ContentKind::GraphQl,
        (_, s) if s.ends_with("+json") => ContentKind::Json,
        ("text", "html") | ("application", "xhtml+xml") => ContentKind::Html,
        ("image", "svg+xml") => ContentKind::Svg,
        (_, "xml") => ContentKind::Xml,
        (_, s) if s.ends_with("+xml") => ContentKind::Xml,
        ("application", "x-www-form-urlencoded") => ContentKind::FormUrlEncoded,
        ("multipart", _) => ContentKind::Multipart,
        (_, "javascript" | "ecmascript" | "x-javascript") => ContentKind::Javascript,
        ("text", "css") => ContentKind::Css,
        ("text", _) => ContentKind::Text,
        ("image", _) => ContentKind::Image,
        ("video", _) => ContentKind::Video,
        ("audio", _) => ContentKind::Audio,
        ("font", _) | ("application", "font-woff" | "font-woff2" | "x-font-ttf" | "vnd.ms-fontobject") => {
            ContentKind::Font
        }
        ("application", "pdf") => ContentKind::Pdf,
        _ => ContentKind::Binary,
    }
}

/// `grpc`, `grpc+proto`, `grpc-web`, `grpc-web-text+proto`, …
fn is_grpc_subtype(sub: &str) -> bool {
    let base = sub.split_once('+').map_or(sub, |(base, _)| base);
    matches!(base, "grpc" | "grpc-web" | "grpc-web-text")
}

/// Detects what a body holds from its `Content-Type` (if any) and its first
/// bytes, already decoded. An empty `head` means there is no body.
///
/// The header wins unless the bytes clearly say otherwise: a binary
/// signature (PNG, PDF, MP4, ...) beats any header, a generic or missing
/// header falls back to the text heuristic, and a text type whose bytes
/// are binary becomes [`ContentKind::Binary`].
pub fn detect(content_type: Option<&str>, head: &[u8]) -> Detected {
    if head.is_empty() {
        return Detected {
            kind: ContentKind::None,
            mime: None,
        };
    }
    let header = content_type.map_or(ContentKind::None, kind_from_content_type);
    let header_mime = content_type.map(essence).filter(|m| !m.is_empty());
    // Binary formats without a signature: a frame or a field tag can look
    // like a magic number, so their header wins outright.
    if matches!(
        header,
        ContentKind::Grpc | ContentKind::Protobuf | ContentKind::MessagePack | ContentKind::Cbor
    ) {
        return Detected::new(header, header_mime.unwrap_or_default());
    }
    if let Some(sniffed) = sniff_magic(head) {
        return sniffed;
    }
    let generic = matches!(header, ContentKind::None | ContentKind::Text)
        || header_mime.as_deref().is_some_and(is_generic_binary);
    if generic {
        if let Some(text) = sniff_text(head) {
            // A text/* header keeps its own type unless the heuristic
            // found structure (JSON, XML, ...) in it.
            return match (&header_mime, text.kind) {
                (Some(mime), ContentKind::Text) if header == ContentKind::Text => Detected::new(header, mime.clone()),
                _ => text,
            };
        }
        return Detected::new(ContentKind::Binary, "application/octet-stream");
    }
    let mime = header_mime.unwrap_or_default();
    if is_textual(header) {
        if looks_binary(head) {
            return Detected::new(ContentKind::Binary, "application/octet-stream");
        }
        // HTML error pages served as JSON (or JSON served as HTML) are
        // common enough to correct.
        if let Some(text) = sniff_text(head)
            && matches!(header, ContentKind::Json | ContentKind::Html | ContentKind::Xml)
            && matches!(text.kind, ContentKind::Json | ContentKind::Html | ContentKind::Svg)
            && text.kind != header
        {
            return text;
        }
    }
    Detected::new(header, mime)
}

/// Recognises a GraphQL request from its body (`{"query": ...}` or a batch
/// of them) or, for GET requests, from its query string.
pub fn graphql_request(content_type: Option<&str>, body: &[u8], query_string: &str) -> Option<GraphQlRequest> {
    let kind = content_type.map_or(ContentKind::None, kind_from_content_type);
    if kind == ContentKind::GraphQl && !body.is_empty() {
        let text = std::str::from_utf8(body).ok()?;
        return Some(GraphQlRequest {
            operation: operation_in_query(text),
        });
    }
    if !body.is_empty() && body.len() <= GRAPHQL_MAX && matches!(kind, ContentKind::Json | ContentKind::None) {
        let value: serde_json::Value = serde_json::from_slice(body).ok()?;
        let first = match &value {
            serde_json::Value::Array(batch) => batch.first()?,
            other => other,
        };
        return graphql_envelope(first);
    }
    if body.is_empty() {
        let mut query = None;
        let mut name = None;
        for (key, value) in form_pairs(query_string) {
            match key.as_str() {
                "query" => query = Some(value),
                "operationName" if !value.is_empty() => name = Some(value),
                _ => {}
            }
        }
        let query = query.filter(|q| looks_like_graphql(q))?;
        return Some(GraphQlRequest {
            operation: name.or_else(|| operation_in_query(&query)),
        });
    }
    None
}

fn graphql_envelope(value: &serde_json::Value) -> Option<GraphQlRequest> {
    let object = value.as_object()?;
    let query = object.get("query")?.as_str()?;
    let persisted = object.get("extensions").and_then(|e| e.get("persistedQuery")).is_some();
    if !looks_like_graphql(query) && !(query.is_empty() && persisted) {
        return None;
    }
    let name = object
        .get("operationName")
        .and_then(|n| n.as_str())
        .filter(|n| !n.is_empty());
    Some(GraphQlRequest {
        operation: name.map(str::to_owned).or_else(|| operation_in_query(query)),
    })
}

fn looks_like_graphql(query: &str) -> bool {
    let q = query.trim_start();
    q.starts_with('{')
        || ["query", "mutation", "subscription", "fragment"]
            .iter()
            .any(|kw| q.starts_with(kw) && q[kw.len()..].starts_with(|c: char| !is_name_char(c)))
}

/// The name of the first named operation in a GraphQL document.
fn operation_in_query(query: &str) -> Option<String> {
    let mut rest = query;
    while let Some(pos) = ["query", "mutation", "subscription"]
        .iter()
        .filter_map(|kw| rest.find(kw).map(|p| (p, kw.len())))
        .min()
    {
        let (at, len) = pos;
        let before_ok = rest[..at].chars().next_back().is_none_or(|c| !is_name_char(c));
        let after = &rest[at + len..];
        if before_ok && after.starts_with(|c: char| c.is_whitespace()) {
            let name: String = after.trim_start().chars().take_while(|&c| is_name_char(c)).collect();
            return (!name.is_empty()).then_some(name);
        }
        rest = after;
    }
    None
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `key=value` pairs of a query string, percent-decoded.
fn form_pairs(query: &str) -> impl Iterator<Item = (String, String)> + '_ {
    query.split('&').filter(|p| !p.is_empty()).map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(k), percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi << 4 | lo);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The summary's kind for a flow: GraphQL operations and form submissions
/// are classified by their request, everything else by the response,
/// falling back to the request.
pub fn flow_kind(request: ContentKind, response: ContentKind) -> ContentKind {
    match (request, response) {
        (request @ (ContentKind::GraphQl | ContentKind::FormUrlEncoded | ContentKind::Multipart), _) => request,
        (request, ContentKind::None) => request,
        (_, response) => response,
    }
}

/// `type/subtype` of a `Content-Type` value, lower-cased.
fn essence(value: &str) -> String {
    value.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

fn is_generic_binary(mime: &str) -> bool {
    matches!(
        mime,
        "application/octet-stream" | "binary/octet-stream" | "application/x-binary" | "application/unknown"
    )
}

fn is_textual(kind: ContentKind) -> bool {
    matches!(
        kind,
        ContentKind::Json
            | ContentKind::Xml
            | ContentKind::Html
            | ContentKind::FormUrlEncoded
            | ContentKind::GraphQl
            | ContentKind::Javascript
            | ContentKind::Css
            | ContentKind::Svg
    )
}

/// Binary formats with an unambiguous signature.
fn sniff_magic(b: &[u8]) -> Option<Detected> {
    use ContentKind::{Audio, Binary, Font, Image, Pdf, Video};
    let at = |offset: usize, sig: &[u8]| b.get(offset..offset + sig.len()) == Some(sig);
    let found = |kind, mime: &str| Some(Detected::new(kind, mime));
    if at(0, b"\x89PNG\r\n\x1a\n") {
        return found(Image, "image/png");
    }
    if at(0, b"\xFF\xD8\xFF") {
        return found(Image, "image/jpeg");
    }
    if at(0, b"GIF87a") || at(0, b"GIF89a") {
        return found(Image, "image/gif");
    }
    if at(0, b"RIFF") && at(8, b"WEBP") {
        return found(Image, "image/webp");
    }
    if at(0, b"RIFF") && at(8, b"WAVE") {
        return found(Audio, "audio/wav");
    }
    if at(0, b"RIFF") && at(8, b"AVI ") {
        return found(Video, "video/x-msvideo");
    }
    if at(4, b"ftyp") {
        let brand = b.get(8..12)?;
        return match brand {
            b"avif" | b"avis" => found(Image, "image/avif"),
            b"heic" | b"heix" | b"mif1" | b"msf1" => found(Image, "image/heic"),
            b"M4A " | b"M4B " => found(Audio, "audio/mp4"),
            b"qt  " => found(Video, "video/quicktime"),
            _ => found(Video, "video/mp4"),
        };
    }
    if at(0, b"\x00\x00\x01\x00") && b.len() >= 6 && b[4] > 0 {
        return found(Image, "image/x-icon");
    }
    // `BM` alone is too weak: also require a known DIB header size.
    if at(0, b"BM")
        && b.len() >= 18
        && matches!(
            u32::from_le_bytes([b[14], b[15], b[16], b[17]]),
            12 | 40 | 52 | 56 | 108 | 124
        )
    {
        return found(Image, "image/bmp");
    }
    if at(0, b"%PDF-") {
        return found(Pdf, "application/pdf");
    }
    if at(0, b"\x1A\x45\xDF\xA3") {
        return found(Video, "video/webm");
    }
    if at(0, b"OggS") {
        return found(Audio, "audio/ogg");
    }
    if at(0, b"fLaC") {
        return found(Audio, "audio/flac");
    }
    if at(0, b"ID3") || (b.len() >= 2 && b[0] == 0xFF && matches!(b[1], 0xFB | 0xF3 | 0xF2)) {
        return found(Audio, "audio/mpeg");
    }
    if at(0, b"wOFF") {
        return found(Font, "font/woff");
    }
    if at(0, b"wOF2") {
        return found(Font, "font/woff2");
    }
    if at(0, b"OTTO") {
        return found(Font, "font/otf");
    }
    if (at(0, b"\x00\x01\x00\x00") || at(0, b"true")) && b.len() >= 12 && b[4] == 0 && (1..=64).contains(&b[5]) {
        return found(Font, "font/ttf");
    }
    if at(0, b"\x1F\x8B") {
        return found(Binary, "application/gzip");
    }
    if at(0, b"PK\x03\x04") {
        return found(Binary, "application/zip");
    }
    if at(0, b"\x00asm") {
        return found(Binary, "application/wasm");
    }
    None
}

/// The text heuristic: JSON or XML by the first non-space byte, then plain
/// text if the bytes are printable UTF-8. `None` means binary.
fn sniff_text(head: &[u8]) -> Option<Detected> {
    if looks_binary(head) {
        return None;
    }
    let text = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let start = text.iter().position(|b| !b.is_ascii_whitespace())?;
    let text = &text[start..];
    let lower: Vec<u8> = text.iter().take(256).map(u8::to_ascii_lowercase).collect();
    let starts = |p: &[u8]| lower.starts_with(p);
    match text[0] {
        b'{' | b'[' if looks_like_json(text) => return Some(Detected::new(ContentKind::Json, "application/json")),
        b'<' => {
            if starts(b"<!doctype html") || starts(b"<html") || starts(b"<head") || starts(b"<body") {
                return Some(Detected::new(ContentKind::Html, "text/html"));
            }
            if starts(b"<svg") || (starts(b"<?xml") && contains(&lower_head(text, 1024), b"<svg")) {
                return Some(Detected::new(ContentKind::Svg, "image/svg+xml"));
            }
            if starts(b"<?xml") || lower.get(1).is_some_and(|c| c.is_ascii_alphabetic()) {
                let tag: Vec<u8> = lower[1..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .copied()
                    .collect();
                if HTML_TAGS.contains(&tag.as_slice()) {
                    return Some(Detected::new(ContentKind::Html, "text/html"));
                }
                return Some(Detected::new(ContentKind::Xml, "application/xml"));
            }
            if starts(b"<!--") {
                return Some(Detected::new(ContentKind::Xml, "application/xml"));
            }
        }
        _ => {}
    }
    Some(Detected::new(ContentKind::Text, "text/plain"))
}

const HTML_TAGS: &[&[u8]] = &[
    b"div", b"p", b"span", b"a", b"table", b"form", b"script", b"style", b"meta", b"link", b"title", b"h1", b"h2",
    b"ul", b"br", b"img", b"section", b"main", b"article", b"iframe",
];

fn lower_head(text: &[u8], n: usize) -> Vec<u8> {
    text.iter().take(n).map(u8::to_ascii_lowercase).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// After the opening bracket, the next token must be one JSON allows there.
fn looks_like_json(text: &[u8]) -> bool {
    let open = text[0];
    let Some(next) = text[1..].iter().find(|b| !b.is_ascii_whitespace()) else {
        // `{` or `[` alone at the end of a short head: assume a document.
        return true;
    };
    match open {
        b'{' => matches!(next, b'"' | b'}'),
        _ => matches!(
            next,
            b'"' | b'{' | b'[' | b']' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n'
        ),
    }
}

/// NUL bytes, invalid UTF-8 (allowing a sequence cut at the end of the
/// head) or many control characters mean binary.
fn looks_binary(head: &[u8]) -> bool {
    if head.contains(&0) {
        return true;
    }
    let valid = match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && head.len() - e.valid_up_to() < 4,
    };
    if !valid {
        return true;
    }
    let controls = head
        .iter()
        .filter(|&&b| b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t' | 0x0C | 0x1B))
        .count();
    controls * 100 > head.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ContentKind::{
        Audio, Binary, Css, Font, FormUrlEncoded, GraphQl, Html, Image, Javascript, Json, Multipart, Pdf, Svg, Text,
        Video, Xml,
    };

    #[test]
    fn maps_common_content_types() {
        let cases = [
            ("application/json; charset=utf-8", Json),
            ("application/problem+json", Json),
            ("application/graphql", GraphQl),
            ("text/html", Html),
            ("application/xml", Xml),
            ("application/atom+xml", Xml),
            ("image/svg+xml", Svg),
            ("application/x-www-form-urlencoded", FormUrlEncoded),
            ("multipart/form-data; boundary=x", Multipart),
            ("text/javascript", Javascript),
            ("TEXT/CSS", Css),
            ("text/plain", Text),
            ("image/png", Image),
            ("video/mp4", Video),
            ("font/woff2", Font),
            ("application/pdf", Pdf),
            ("application/octet-stream", Binary),
            ("", ContentKind::None),
            ("application/grpc", ContentKind::Grpc),
            ("application/grpc+proto", ContentKind::Grpc),
            ("application/grpc+json", ContentKind::Grpc),
            ("application/grpc-web+proto", ContentKind::Grpc),
            ("application/grpc-web-text", ContentKind::Grpc),
            ("application/x-protobuf; messageType=x.Y", ContentKind::Protobuf),
            ("application/vnd.google.protobuf", ContentKind::Protobuf),
            ("application/x-msgpack", ContentKind::MessagePack),
            ("application/msgpack", ContentKind::MessagePack),
            ("application/cbor", ContentKind::Cbor),
        ];
        for (value, kind) in cases {
            assert_eq!(kind_from_content_type(value), kind, "{value}");
        }
    }

    #[test]
    fn signatureless_binary_headers_beat_magic_bytes() {
        // A gRPC frame whose length bytes read like an ICO header.
        let frame = [0u8, 0, 1, 0, 4, 8, 1, 16, 2];
        assert_eq!(kind(Some("application/grpc"), &frame).0, ContentKind::Grpc);
        assert_eq!(kind(Some("application/x-protobuf"), &png()).0, ContentKind::Protobuf);
    }

    fn kind(ct: Option<&str>, body: &[u8]) -> (ContentKind, Option<String>) {
        let d = detect(ct, body);
        (d.kind, d.mime)
    }

    fn png() -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
        b
    }

    #[test]
    fn header_kinds_are_kept_when_the_bytes_agree() {
        let cases: [(&str, &[u8], ContentKind, &str); 9] = [
            ("application/json", br#"{"a":1}"#, Json, "application/json"),
            (
                "application/vnd.api+json",
                br#"[1,2]"#,
                Json,
                "application/vnd.api+json",
            ),
            ("text/html; charset=utf-8", b"<!doctype html><p>hi", Html, "text/html"),
            ("application/xml", b"<?xml version='1.0'?><a/>", Xml, "application/xml"),
            (
                "application/x-www-form-urlencoded",
                b"a=1&b=2",
                FormUrlEncoded,
                "application/x-www-form-urlencoded",
            ),
            (
                "multipart/form-data; boundary=x",
                b"--x\r\n",
                Multipart,
                "multipart/form-data",
            ),
            (
                "application/javascript",
                b"export const a = 1;",
                Javascript,
                "application/javascript",
            ),
            ("text/css", b"body { color: red }", Css, "text/css"),
            (
                "image/svg+xml",
                b"<svg xmlns='http://www.w3.org/2000/svg'/>",
                Svg,
                "image/svg+xml",
            ),
        ];
        for (ct, body, want, mime) in cases {
            assert_eq!(kind(Some(ct), body), (want, Some(mime.into())), "{ct}");
        }
    }

    #[test]
    fn empty_bodies_are_none() {
        assert_eq!(kind(Some("application/json"), b""), (ContentKind::None, None));
    }

    #[test]
    fn headerless_bodies_are_sniffed() {
        let cases: Vec<(Vec<u8>, ContentKind, &str)> = vec![
            (br#"  {"user": {"id": 1}}"#.to_vec(), Json, "application/json"),
            (b"[1, 2, 3]".to_vec(), Json, "application/json"),
            (b"\xEF\xBB\xBF{\"bom\":true}".to_vec(), Json, "application/json"),
            (b"<?xml version=\"1.0\"?><feed/>".to_vec(), Xml, "application/xml"),
            (b"<rss version=\"2.0\"></rss>".to_vec(), Xml, "application/xml"),
            (b"<!DOCTYPE html><html></html>".to_vec(), Html, "text/html"),
            (b"<div>fragment</div>".to_vec(), Html, "text/html"),
            (
                b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
                Svg,
                "image/svg+xml",
            ),
            (b"hello, world\n".to_vec(), Text, "text/plain"),
            (b"[INFO] not json".to_vec(), Text, "text/plain"),
            (png(), Image, "image/png"),
            (b"\xFF\xD8\xFF\xE0\0\x10JFIF".to_vec(), Image, "image/jpeg"),
            (b"GIF89a\x01\0\x01\0".to_vec(), Image, "image/gif"),
            (b"RIFF\x24\0\0\0WEBPVP8 ".to_vec(), Image, "image/webp"),
            (b"\0\0\0\x1cftypavif\0\0\0\0".to_vec(), Image, "image/avif"),
            (b"\0\0\x01\0\x01\0\x10\x10".to_vec(), Image, "image/x-icon"),
            (b"BM\x46\0\0\0\0\0\0\0\x36\0\0\0\x28\0\0\0".to_vec(), Image, "image/bmp"),
            (b"%PDF-1.7\n".to_vec(), Pdf, "application/pdf"),
            (b"\0\0\0\x20ftypisom\0\0\x02\0".to_vec(), Video, "video/mp4"),
            (b"\x1A\x45\xDF\xA3\x9f".to_vec(), Video, "video/webm"),
            (b"ID3\x04\0\0".to_vec(), Audio, "audio/mpeg"),
            (b"OggS\0\x02".to_vec(), Audio, "audio/ogg"),
            (b"wOF2\0\x01\0\0".to_vec(), Font, "font/woff2"),
            (b"\0\x01\0\0\0\x0c\0\x80\0\x03\0\x40".to_vec(), Font, "font/ttf"),
            (b"\x1F\x8B\x08\0".to_vec(), Binary, "application/gzip"),
            (vec![0x13, 0x00, 0xFE, 0x80, 0x01], Binary, "application/octet-stream"),
        ];
        for (body, want, mime) in cases {
            assert_eq!(
                kind(None, &body),
                (want, Some(mime.into())),
                "{:?}",
                String::from_utf8_lossy(&body)
            );
        }
    }

    #[test]
    fn mislabelled_bodies_follow_the_bytes() {
        // A PNG served as JSON, octet-stream or the wrong image type.
        assert_eq!(kind(Some("application/json"), &png()).0, Image);
        assert_eq!(kind(Some("image/jpeg"), &png()), (Image, Some("image/png".into())));
        assert_eq!(kind(Some("application/octet-stream"), &png()).0, Image);
        // JSON served as text/plain or octet-stream.
        assert_eq!(kind(Some("text/plain"), br#"{"a":1}"#).0, Json);
        assert_eq!(kind(Some("application/octet-stream"), br#"{"a":1}"#).0, Json);
        // An HTML error page served as JSON.
        assert_eq!(
            kind(Some("application/json"), b"<!DOCTYPE html><title>502</title>").0,
            Html
        );
        // Binary served as text.
        assert_eq!(kind(Some("text/plain"), &[0, 1, 2, 3, 255]).0, Binary);
        assert_eq!(kind(Some("application/json"), &[0x80, 0x81, 0x00]).0, Binary);
        // Plain text keeps a specific text/* type.
        assert_eq!(kind(Some("text/csv"), b"a,b\n1,2\n"), (Text, Some("text/csv".into())));
    }

    #[test]
    fn utf8_cut_at_the_end_of_the_head_is_still_text() {
        let mut head = "café ".repeat(10).into_bytes();
        head.extend_from_slice(&"é".as_bytes()[..1]);
        assert_eq!(kind(None, &head).0, Text);
    }

    #[test]
    fn compressed_bodies_are_detected_after_decoding() {
        let json = br#"{"compressed": true, "items": [1, 2, 3]}"#;
        let gzip = {
            use std::io::Write;
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(json).unwrap();
            enc.finish().unwrap()
        };
        // As stored: gzip bytes, with or without a Content-Encoding.
        assert_eq!(kind(None, &gzip).0, Binary);
        let head = crate::io::decode::decode_prefix("gzip", &gzip, SNIFF_LEN);
        assert_eq!(kind(Some("application/json"), &head).0, Json);
        assert_eq!(kind(None, &head).0, Json);
        // A truncated stream still yields its head.
        let head = crate::io::decode::decode_prefix("gzip", &gzip[..gzip.len() - 8], SNIFF_LEN);
        assert!(json.starts_with(&head[..head.len().min(json.len())]));
    }

    #[test]
    fn graphql_requests_are_recognised() {
        let body = br#"{"query":"query GetUser($id: ID!) { user(id: $id) { name } }","variables":{"id":"1"}}"#;
        assert_eq!(
            graphql_request(Some("application/json"), body, ""),
            Some(GraphQlRequest {
                operation: Some("GetUser".into())
            })
        );
        let named = br#"{"operationName":"Feed","query":"{ feed { id } }"}"#;
        assert_eq!(
            graphql_request(None, named, "").unwrap().operation.as_deref(),
            Some("Feed")
        );
        let anonymous = br#"{"query":"{ viewer { login } }"}"#;
        assert_eq!(
            graphql_request(Some("application/json"), anonymous, "")
                .unwrap()
                .operation,
            None
        );
        let batch = br#"[{"query":"mutation AddTodo { add { id } }"},{"query":"{ a }"}]"#;
        assert_eq!(
            graphql_request(Some("application/json"), batch, "")
                .unwrap()
                .operation
                .as_deref(),
            Some("AddTodo")
        );
        let persisted = br#"{"operationName":"P","query":"","extensions":{"persistedQuery":{"version":1}}}"#;
        assert_eq!(
            graphql_request(None, persisted, "").unwrap().operation.as_deref(),
            Some("P")
        );
        let raw = b"subscription OnEvent { event { id } }";
        assert_eq!(
            graphql_request(Some("application/graphql"), raw, "")
                .unwrap()
                .operation
                .as_deref(),
            Some("OnEvent")
        );
        let get = "query=query%20Search%20%7B%20s%20%7D&variables=%7B%7D";
        assert_eq!(
            graphql_request(None, b"", get).unwrap().operation.as_deref(),
            Some("Search")
        );

        // Ordinary JSON with a "query" field is not GraphQL.
        assert_eq!(
            graphql_request(Some("application/json"), br#"{"query":"shoes","page":2}"#, ""),
            None
        );
        assert_eq!(graphql_request(None, b"", "query=shoes"), None);
        assert_eq!(graphql_request(Some("text/plain"), body, ""), None);
    }

    #[test]
    fn operation_names_skip_keywords_inside_names() {
        assert_eq!(operation_in_query("{ myquery { id } }"), None);
        assert_eq!(operation_in_query("# comment\nquery  Q_1 { a }"), Some("Q_1".into()));
    }

    #[test]
    fn flow_kind_prefers_graphql_and_forms_then_the_response() {
        assert_eq!(flow_kind(GraphQl, Json), GraphQl);
        assert_eq!(flow_kind(FormUrlEncoded, Html), FormUrlEncoded);
        assert_eq!(flow_kind(Json, Html), Html);
        assert_eq!(flow_kind(Multipart, ContentKind::None), Multipart);
        assert_eq!(flow_kind(ContentKind::None, ContentKind::None), ContentKind::None);
    }
}
