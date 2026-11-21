//! Content detection for bodies the engine never sees on their own
//! (multipart parts), and the Content-Type fallback for flows recorded
//! before detection ran (`lib/body/content.ts`). A simplified mirror of
//! `pm-core/src/io/content.rs`, which stays the source of truth for flows.

use pm_model::ContentKind;

/// `type/subtype` of a Content-Type value, lower-cased.
pub fn essence(content_type: Option<&str>) -> String {
    content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

pub fn kind_from_content_type(content_type: Option<&str>) -> ContentKind {
    let mime = essence(content_type);
    if mime.is_empty() {
        return ContentKind::None;
    }
    let (top, sub) = mime.split_once('/').unwrap_or((mime.as_str(), ""));
    // Before `+json`: `application/grpc+json` is still gRPC framing.
    let sub_base = sub.split('+').next().unwrap_or("");
    if top == "application" && matches!(sub_base, "grpc" | "grpc-web" | "grpc-web-text") {
        return ContentKind::Grpc;
    }
    match mime.as_str() {
        "application/x-protobuf"
        | "application/protobuf"
        | "application/vnd.google.protobuf"
        | "application/x-google-protobuf" => return ContentKind::Protobuf,
        "application/msgpack" | "application/x-msgpack" | "application/vnd.msgpack" => {
            return ContentKind::MessagePack;
        }
        "application/cbor" => return ContentKind::Cbor,
        _ => {}
    }
    if sub == "json" || sub == "graphql-response+json" {
        return ContentKind::Json;
    }
    if sub == "graphql" || sub == "graphql+json" {
        return ContentKind::GraphQl;
    }
    if sub.ends_with("+json") {
        return ContentKind::Json;
    }
    if mime == "text/html" || mime == "application/xhtml+xml" {
        return ContentKind::Html;
    }
    if mime == "image/svg+xml" {
        return ContentKind::Svg;
    }
    if sub == "xml" || sub.ends_with("+xml") {
        return ContentKind::Xml;
    }
    if mime == "application/x-www-form-urlencoded" {
        return ContentKind::FormUrlEncoded;
    }
    if top == "multipart" {
        return ContentKind::Multipart;
    }
    if matches!(sub, "javascript" | "ecmascript" | "x-javascript") {
        return ContentKind::Javascript;
    }
    if mime == "text/css" {
        return ContentKind::Css;
    }
    match top {
        "text" => ContentKind::Text,
        "image" => ContentKind::Image,
        "video" => ContentKind::Video,
        "audio" => ContentKind::Audio,
        "font" => ContentKind::Font,
        _ if matches!(
            mime.as_str(),
            "application/font-woff" | "application/font-woff2" | "application/x-font-ttf"
        ) =>
        {
            ContentKind::Font
        }
        _ if mime == "application/pdf" => ContentKind::Pdf,
        _ => ContentKind::Binary,
    }
}

/// The kind a body falls back to when the engine has none (recorded before
/// detection, or still streaming): its header's, or text.
pub fn fallback_kind(content_type: Option<&str>) -> ContentKind {
    match kind_from_content_type(content_type) {
        ContentKind::None => ContentKind::Text,
        kind => kind,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sniffed {
    pub kind: ContentKind,
    pub mime: String,
}

fn sniffed(kind: ContentKind, mime: &str) -> Option<Sniffed> {
    Some(Sniffed {
        kind,
        mime: mime.to_owned(),
    })
}

fn at(bytes: &[u8], offset: usize, sig: &[u8]) -> bool {
    bytes.get(offset..offset + sig.len()) == Some(sig)
}

fn sniff_magic(b: &[u8]) -> Option<Sniffed> {
    const MAGIC: [(&[u8], ContentKind, &str); 11] = [
        (b"\x89PNG\r\n\x1a\n", ContentKind::Image, "image/png"),
        (b"\xff\xd8\xff", ContentKind::Image, "image/jpeg"),
        (b"GIF8", ContentKind::Image, "image/gif"),
        (b"%PDF-", ContentKind::Pdf, "application/pdf"),
        (b"\x1a\x45\xdf\xa3", ContentKind::Video, "video/webm"),
        (b"OggS", ContentKind::Audio, "audio/ogg"),
        (b"ID3", ContentKind::Audio, "audio/mpeg"),
        (b"wOFF", ContentKind::Font, "font/woff"),
        (b"wOF2", ContentKind::Font, "font/woff2"),
        (b"\x1f\x8b", ContentKind::Binary, "application/gzip"),
        (b"PK\x03\x04", ContentKind::Binary, "application/zip"),
    ];
    for (sig, kind, mime) in MAGIC {
        if at(b, 0, sig) {
            return sniffed(kind, mime);
        }
    }
    if at(b, 0, b"RIFF") && at(b, 8, b"WEBP") {
        return sniffed(ContentKind::Image, "image/webp");
    }
    if at(b, 0, b"RIFF") && at(b, 8, b"WAVE") {
        return sniffed(ContentKind::Audio, "audio/wav");
    }
    if at(b, 4, b"ftyp") {
        let brand = b.get(8..12).unwrap_or_default();
        if brand == b"avif" || brand == b"avis" {
            return sniffed(ContentKind::Image, "image/avif");
        }
        if brand.starts_with(b"M4A") {
            return sniffed(ContentKind::Audio, "audio/mp4");
        }
        return sniffed(ContentKind::Video, "video/mp4");
    }
    if at(b, 0, &[0, 0, 1, 0]) && b.get(4).is_some_and(|n| *n > 0) {
        return sniffed(ContentKind::Image, "image/x-icon");
    }
    if at(b, 0, b"BM") && b.len() > 18 {
        return sniffed(ContentKind::Image, "image/bmp");
    }
    None
}

/// True if the bytes are not printable UTF-8 text.
pub fn looks_binary(head: &[u8]) -> bool {
    if head.contains(&0) {
        return true;
    }
    if let Err(e) = std::str::from_utf8(head) {
        // Allow a multi-byte sequence cut at the end.
        if e.error_len().is_some() || head.len() - e.valid_up_to() > 3 {
            return true;
        }
    }
    let controls = head
        .iter()
        .filter(|c| **c < 0x20 && !matches!(**c, 9 | 10 | 13 | 12 | 27))
        .count();
    controls * 100 > head.len()
}

fn sniff_text(head: &[u8]) -> Option<Sniffed> {
    if looks_binary(head) {
        return None;
    }
    let lossy = String::from_utf8_lossy(&head[..head.len().min(1024)]);
    let text = lossy.trim_start_matches('\u{feff}').trim_start();
    let lower: String = text.chars().take(256).collect::<String>().to_lowercase();
    let after = |open: char| text[open.len_utf8()..].trim_start().chars().next();
    let json = match text.chars().next() {
        Some('{') => matches!(after('{'), Some('"' | '}') | None),
        Some('[') => matches!(
            after('['),
            Some('"' | '{' | '[' | ']' | '-' | '0'..='9' | 't' | 'f' | 'n') | None
        ),
        _ => false,
    };
    if json {
        return sniffed(ContentKind::Json, "application/json");
    }
    if ["<!doctype html", "<html", "<head", "<body"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        return sniffed(ContentKind::Html, "text/html");
    }
    if lower.starts_with("<svg") || (lower.starts_with("<?xml") && text.to_lowercase().contains("<svg")) {
        return sniffed(ContentKind::Svg, "image/svg+xml");
    }
    if lower.starts_with("<?xml") || (lower.starts_with('<') && lower[1..].starts_with(|c: char| c.is_ascii_lowercase()))
    {
        return sniffed(ContentKind::Xml, "application/xml");
    }
    sniffed(ContentKind::Text, "text/plain")
}

/// Detects a body from its Content-Type and first bytes, like the engine.
pub fn sniff(bytes: &[u8], content_type: Option<&str>) -> Sniffed {
    let head = &bytes[..bytes.len().min(16 * 1024)];
    let mime = essence(content_type);
    if head.is_empty() {
        return Sniffed {
            kind: ContentKind::None,
            mime,
        };
    }
    let header = kind_from_content_type(content_type);
    // Binary formats without a signature: their header wins, as in Rust.
    if matches!(
        header,
        ContentKind::Grpc | ContentKind::Protobuf | ContentKind::MessagePack | ContentKind::Cbor
    ) {
        return Sniffed { kind: header, mime };
    }
    if let Some(magic) = sniff_magic(head) {
        return magic;
    }
    if matches!(header, ContentKind::None | ContentKind::Text) || mime == "application/octet-stream" {
        return match sniff_text(head) {
            None => Sniffed {
                kind: ContentKind::Binary,
                mime: "application/octet-stream".into(),
            },
            Some(text) if header == ContentKind::Text && text.kind == ContentKind::Text => Sniffed {
                kind: ContentKind::Text,
                mime,
            },
            Some(text) => text,
        };
    }
    Sniffed { kind: header, mime }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_content_types() {
        let k = |ct: &str| kind_from_content_type(Some(ct));
        assert_eq!(k("application/json; charset=utf-8"), ContentKind::Json);
        assert_eq!(k("application/problem+json"), ContentKind::Json);
        assert_eq!(k("application/grpc+json"), ContentKind::Grpc);
        assert_eq!(k("application/graphql"), ContentKind::GraphQl);
        assert_eq!(k("application/atom+xml"), ContentKind::Xml);
        assert_eq!(k("image/svg+xml"), ContentKind::Svg);
        assert_eq!(k("multipart/form-data; boundary=x"), ContentKind::Multipart);
        assert_eq!(k("text/javascript"), ContentKind::Javascript);
        assert_eq!(k("font/woff2"), ContentKind::Font);
        assert_eq!(k("application/octet-stream"), ContentKind::Binary);
        assert_eq!(kind_from_content_type(None), ContentKind::None);
        assert_eq!(fallback_kind(None), ContentKind::Text);
    }

    #[test]
    fn sniffs_parts() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest", Some("text/plain")).kind, ContentKind::Image);
        assert_eq!(sniff(b"{\"a\":1}", None).kind, ContentKind::Json);
        assert_eq!(sniff(b"hello", Some("text/plain")).mime, "text/plain");
        assert_eq!(sniff(b"<html><body>", None).kind, ContentKind::Html);
        assert_eq!(sniff(b"<feed>", Some("application/octet-stream")).kind, ContentKind::Xml);
        assert_eq!(sniff(b"\x00\x01\x02", None).kind, ContentKind::Binary);
        assert_eq!(sniff(b"", Some("text/plain")).kind, ContentKind::None);
        assert_eq!(sniff(b"abc", Some("application/x-protobuf")).kind, ContentKind::Protobuf);
    }

    #[test]
    fn a_cut_multibyte_character_is_still_text() {
        let text = "é".repeat(10);
        assert!(!looks_binary(&text.as_bytes()[..text.len() - 1]));
        assert!(looks_binary(b"\xff\xfe\x00"));
    }
}
