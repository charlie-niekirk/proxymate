//! gRPC and gRPC-web framing: a body is a series of length-prefixed
//! messages (a 1-byte flags field, a 4-byte big-endian length). A set
//! compressed flag means the message is compressed with the call's
//! `grpc-encoding`. In gRPC-web, a frame with flag `0x80` carries the
//! trailers as an HTTP/1-style header block, and `-text` bodies are Base64
//! (possibly one padded chunk per message).
//!
//! Splitting works on any prefix of a body, so a live server stream decodes
//! as far as it has arrived; the rest is reported as pending.

use base64::Engine as _;
use pm_model::{GrpcStatus, Header};

use super::decode;
use super::wire;

/// The trailer frame's flag bit in gRPC-web.
const TRAILER_FLAG: u8 = 0x80;
/// Messages past this many are left undecoded.
pub const MAX_MESSAGES: usize = 10_000;

/// Which flavour of gRPC a `Content-Type` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Grpc,
    Web,
    WebText,
}

impl Flavor {
    /// From a `Content-Type` already known to be gRPC.
    pub fn of(content_type: Option<&str>) -> Flavor {
        let mime = content_type
            .and_then(|v| v.split(';').next())
            .map(|m| m.trim().to_ascii_lowercase())
            .unwrap_or_default();
        let sub = mime.strip_prefix("application/").unwrap_or(&mime);
        let base = sub.split_once('+').map_or(sub, |(base, _)| base);
        match base {
            "grpc-web-text" => Flavor::WebText,
            "grpc-web" => Flavor::Web,
            _ => Flavor::Grpc,
        }
    }

    pub fn is_web(self) -> bool {
        self != Flavor::Grpc
    }
}

/// One message as framed.
#[derive(Debug, Clone, PartialEq)]
pub struct RawMessage {
    pub compressed: bool,
    /// Payload bytes on the wire.
    pub size: u32,
    /// The payload, decompressed; or why it couldn't be.
    pub payload: Result<Vec<u8>, String>,
}

/// A body split into messages.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Framed {
    pub messages: Vec<RawMessage>,
    /// gRPC-web: the trailer frame's headers, names lower-cased.
    pub trailers: Vec<Header>,
    /// Bytes after the last whole frame.
    pub pending: usize,
    pub error: Option<String>,
}

/// Splits `body` (as recorded, after any `Content-Encoding`) into messages,
/// decompressing each with `encoding` (`grpc-encoding`) when flagged.
pub fn split(body: &[u8], flavor: Flavor, encoding: Option<&str>) -> Framed {
    let mut framed = Framed::default();
    let decoded;
    let mut b = body;
    if flavor == Flavor::WebText {
        let (bytes, undecoded) = match base64_chunks(body) {
            Ok(ok) => ok,
            Err(e) => {
                framed.error = Some(e);
                return framed;
            }
        };
        decoded = bytes;
        b = &decoded;
        framed.pending = undecoded;
    }
    while !b.is_empty() && framed.messages.len() < MAX_MESSAGES {
        if b.len() < 5 {
            framed.pending += b.len();
            break;
        }
        let flags = b[0];
        let len = u32::from_be_bytes([b[1], b[2], b[3], b[4]]);
        let Some(payload) = b.get(5..5 + len as usize) else {
            framed.pending += b.len();
            break;
        };
        b = &b[5 + len as usize..];
        if flavor.is_web() && flags & TRAILER_FLAG != 0 {
            framed.trailers = parse_trailer_block(payload);
            continue;
        }
        let compressed = flags & 1 == 1;
        let payload = if compressed {
            match encoding.map(str::trim).filter(|e| !e.is_empty() && *e != "identity") {
                Some(coding) => decode::decode(coding, payload).map_err(|e| e.to_string()),
                None => Err("flagged as compressed, but the call has no grpc-encoding".into()),
            }
        } else {
            Ok(payload.to_vec())
        };
        framed.messages.push(RawMessage {
            compressed,
            size: len,
            payload,
        });
    }
    if !b.is_empty() && framed.messages.len() >= MAX_MESSAGES {
        framed.error = Some(format!("only the first {MAX_MESSAGES} messages are decoded"));
    }
    framed
}

/// `-text` bodies: Base64, where each message may be its own padded chunk.
/// Returns the decoded bytes and how many trailing characters were left
/// for later (a chunk still arriving).
fn base64_chunks(body: &[u8]) -> Result<(Vec<u8>, usize), String> {
    let text: Vec<u8> = body.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < text.len() {
        // A chunk ends after its padding.
        if text[i] == b'=' {
            while i < text.len() && text[i] == b'=' {
                i += 1;
            }
            decode_chunk(&text[start..i], &mut out)?;
            start = i;
        } else {
            i += 1;
        }
    }
    let rest = &text[start..];
    let whole = rest.len() / 4 * 4;
    decode_chunk(&rest[..whole], &mut out)?;
    Ok((out, rest.len() - whole))
}

fn decode_chunk(chunk: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
    if chunk.is_empty() {
        return Ok(());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(chunk)
        .map_err(|e| format!("grpc-web-text body isn't Base64: {e}"))?;
    out.extend_from_slice(&bytes);
    Ok(())
}

/// The gRPC-web trailer frame: `name: value` lines separated by CRLF.
fn parse_trailer_block(payload: &[u8]) -> Vec<Header> {
    String::from_utf8_lossy(payload)
        .split("\r\n")
        .flat_map(|line| line.split('\n'))
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            let name = name.trim();
            (!name.is_empty()).then(|| Header::new(name.to_ascii_lowercase(), value.trim()))
        })
        .collect()
}

/// The canonical name of a status code.
pub fn code_name(code: u8) -> String {
    const NAMES: [&str; 17] = [
        "OK",
        "CANCELLED",
        "UNKNOWN",
        "INVALID_ARGUMENT",
        "DEADLINE_EXCEEDED",
        "NOT_FOUND",
        "ALREADY_EXISTS",
        "PERMISSION_DENIED",
        "RESOURCE_EXHAUSTED",
        "FAILED_PRECONDITION",
        "ABORTED",
        "OUT_OF_RANGE",
        "UNIMPLEMENTED",
        "INTERNAL",
        "UNAVAILABLE",
        "DATA_LOSS",
        "UNAUTHENTICATED",
    ];
    NAMES
        .get(code as usize)
        .map_or_else(|| format!("UNKNOWN({code})"), |n| (*n).to_owned())
}

/// The code a status name stands for (`NOT_FOUND`, `not_found`).
pub fn code_of_name(name: &str) -> Option<u8> {
    (0..=16u8).find(|&c| code_name(c).eq_ignore_ascii_case(name))
}

fn header<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

/// The `grpc-status` code in a header list.
pub fn status_code(headers: &[Header]) -> Option<u8> {
    header(headers, "grpc-status")?.trim().parse().ok()
}

/// The call's status from its trailers (or a trailers-only response's
/// headers): the code, the percent-decoded message, and the details.
pub fn status(headers: &[Header]) -> Option<GrpcStatus> {
    let code = status_code(headers)?;
    let message = header(headers, "grpc-message")
        .map(percent_decode)
        .filter(|m| !m.is_empty());
    let details = header(headers, "grpc-status-details-bin").and_then(status_details);
    Some(GrpcStatus {
        code,
        name: code_name(code),
        message,
        details,
    })
}

/// `grpc-message` is percent-encoded UTF-8 (lenient: bad escapes stay).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(hi), Some(lo)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `grpc-status-details-bin`: unpadded (or padded) Base64 of a
/// `google.rpc.Status`, shown as JSON. Each `Any` detail keeps its type
/// URL; its value is decoded schema-less.
pub fn status_details(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('=');
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(value)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value))
        .ok()?;
    let status = decode_rpc_status(&bytes)?;
    serde_json::to_string_pretty(&status).ok()
}

fn decode_rpc_status(mut b: &[u8]) -> Option<serde_json::Value> {
    let mut code = 0u64;
    let mut message = String::new();
    let mut details = Vec::new();
    while !b.is_empty() {
        let (tag, rest) = wire::varint(b)?;
        b = rest;
        match (tag >> 3, tag & 7) {
            (1, 0) => {
                let (v, rest) = wire::varint(b)?;
                code = v;
                b = rest;
            }
            (2 | 3, 2) => {
                let (len, rest) = wire::varint(b)?;
                let data = rest.get(..len as usize)?;
                b = &rest[len as usize..];
                if tag >> 3 == 2 {
                    message = String::from_utf8_lossy(data).into_owned();
                } else {
                    details.push(decode_any(data)?);
                }
            }
            _ => return None,
        }
    }
    Some(serde_json::json!({ "code": code, "message": message, "details": details }))
}

fn decode_any(mut b: &[u8]) -> Option<serde_json::Value> {
    let mut type_url = String::new();
    let mut value: &[u8] = &[];
    while !b.is_empty() {
        let (tag, rest) = wire::varint(b)?;
        if tag & 7 != 2 {
            return None;
        }
        let (len, rest) = wire::varint(rest)?;
        let data = rest.get(..len as usize)?;
        b = &rest[len as usize..];
        match tag >> 3 {
            1 => type_url = String::from_utf8_lossy(data).into_owned(),
            2 => value = data,
            _ => {}
        }
    }
    let decoded = wire::decode(value)
        .ok()
        .and_then(|(node, _)| serde_json::from_str::<serde_json::Value>(&node.to_json()).ok())
        .unwrap_or_else(|| base64::engine::general_purpose::STANDARD.encode(value).into());
    Some(serde_json::json!({ "@type": type_url, "value": decoded }))
}

/// Frames `messages` as a gRPC body (uncompressed), for tests and fixtures.
pub fn frame(messages: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for m in messages {
        out.push(0);
        out.extend_from_slice(&(m.len() as u32).to_be_bytes());
        out.extend_from_slice(m);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn splits_messages_and_keeps_the_partial_tail() {
        let mut body = frame(&[b"\x08\x01", b""]);
        body.extend_from_slice(&[0, 0, 0, 0, 9, 1, 2]);
        let framed = split(&body, Flavor::Grpc, None);
        assert_eq!(framed.messages.len(), 2);
        assert_eq!(framed.messages[0].payload, Ok(vec![8, 1]));
        assert_eq!(framed.messages[1].size, 0);
        assert_eq!(framed.pending, 7);
        assert_eq!(framed.error, None);
    }

    #[test]
    fn decompresses_flagged_messages() {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(b"\x08\x96\x01").unwrap();
        let gz = gz.finish().unwrap();
        let mut body = vec![1];
        body.extend_from_slice(&(gz.len() as u32).to_be_bytes());
        body.extend_from_slice(&gz);
        let framed = split(&body, Flavor::Grpc, Some("gzip"));
        assert_eq!(framed.messages[0].payload, Ok(vec![0x08, 0x96, 0x01]));
        assert!(framed.messages[0].compressed);
        assert!(split(&body, Flavor::Grpc, None).messages[0].payload.is_err());
        let zstd = zstd::bulk::compress(b"\x08\x02", 1).unwrap();
        let mut body = vec![1];
        body.extend_from_slice(&(zstd.len() as u32).to_be_bytes());
        body.extend_from_slice(&zstd);
        assert_eq!(
            split(&body, Flavor::Grpc, Some("zstd")).messages[0].payload,
            Ok(vec![8, 2])
        );
    }

    #[test]
    fn grpc_web_trailers_and_text() {
        let mut body = frame(&[b"\x08\x01"]);
        let trailer = b"grpc-status: 5\r\ngrpc-message: no%20such%20thing\r\n";
        body.push(0x80);
        body.extend_from_slice(&(trailer.len() as u32).to_be_bytes());
        body.extend_from_slice(trailer);
        let framed = split(&body, Flavor::Web, None);
        assert_eq!(framed.messages.len(), 1);
        let status = status(&framed.trailers).unwrap();
        assert_eq!((status.code, status.name.as_str()), (5, "NOT_FOUND"));
        assert_eq!(status.message.as_deref(), Some("no such thing"));

        // Each frame its own padded chunk, as browsers' grpc-web-text sends.
        let b64 = base64::engine::general_purpose::STANDARD;
        let text = format!("{}{}", b64.encode(frame(&[b"\x08\x01"])), b64.encode(&body[7..]));
        let framed = split(text.as_bytes(), Flavor::WebText, None);
        assert_eq!(framed.messages.len(), 1);
        assert_eq!(status_code(&framed.trailers), Some(5));
        assert!(split(b"!!!!", Flavor::WebText, None).error.is_some());
    }

    #[test]
    fn flavors_and_names() {
        assert_eq!(Flavor::of(Some("application/grpc+proto")), Flavor::Grpc);
        assert_eq!(Flavor::of(Some("application/grpc-web+proto")), Flavor::Web);
        assert_eq!(Flavor::of(Some("application/grpc-web-text")), Flavor::WebText);
        assert_eq!(code_name(16), "UNAUTHENTICATED");
        assert_eq!(code_name(40), "UNKNOWN(40)");
        assert_eq!(code_of_name("not_found"), Some(5));
    }

    #[test]
    fn status_details_decode_as_google_rpc_status() {
        // Status{code: 3, message: "bad", details: [Any{type_url: "t/x.Y", value: {1: 7}}]}.
        let any = [&[0x0a, 5][..], b"t/x.Y", &[0x12, 2, 0x08, 7]].concat();
        let status = [&[0x08, 3, 0x12, 3][..], b"bad", &[0x1a, any.len() as u8], &any].concat();
        let header = base64::engine::general_purpose::STANDARD_NO_PAD.encode(&status);
        let json: serde_json::Value = serde_json::from_str(&status_details(&header).unwrap()).unwrap();
        assert_eq!(json["code"], 3);
        assert_eq!(json["message"], "bad");
        assert_eq!(json["details"][0]["@type"], "t/x.Y");
        assert_eq!(json["details"][0]["value"]["1"], 7);
    }
}
