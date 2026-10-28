//! Schema-less protobuf: the wire format decoded to field numbers, wire
//! types and values, the way `protoc --decode_raw` does.
//!
//! Object keys are field numbers; a field seen more than once becomes an
//! array. Each value is badged with its wire type and the other readings
//! of its bits (a varint's zigzag value, a fixed64's double). A
//! length-delimited field is printable text if it reads as such, else a
//! nested message if every byte parses, else bytes (Base64), noting when
//! they also read as packed varints.

use base64::Engine as _;

use super::tree::{Badges, Node, child};

/// Nested messages deeper than this stay bytes.
const MAX_DEPTH: usize = 48;
/// Packed varints are suggested in a badge only up to this many.
const PACKED_HINT_MAX: usize = 16;

/// A decoded message: the tree and its badges.
pub fn decode(bytes: &[u8]) -> Result<(Node, Vec<pm_model::Badge>), String> {
    let fields = parse(bytes, 0).map_err(|e| format!("not a protobuf message: {e}"))?;
    let mut badges = Badges::default();
    let node = to_node(fields, "", &mut badges, 0);
    Ok((node, badges.into_vec()))
}

#[derive(Debug, Clone, PartialEq)]
enum Value<'a> {
    Varint(u64),
    Fixed64(u64),
    Fixed32(u32),
    Len(&'a [u8]),
    Group(Vec<(u32, Value<'a>)>),
}

/// Parses every field of a message; any leftover or malformed byte fails.
fn parse(mut b: &[u8], depth: usize) -> Result<Vec<(u32, Value<'_>)>, String> {
    let mut fields = Vec::new();
    while !b.is_empty() {
        let (field, value, rest) = field(b, depth)?;
        match value {
            Some(value) => fields.push((field, value)),
            None => return Err(format!("unexpected end of group {field}")),
        }
        b = rest;
    }
    Ok(fields)
}

/// One field; `None` for an end-group tag (only valid inside a group).
#[allow(clippy::type_complexity)]
fn field(b: &[u8], depth: usize) -> Result<(u32, Option<Value<'_>>, &[u8]), String> {
    let (tag, rest) = varint(b).ok_or("truncated tag")?;
    let number = tag >> 3;
    if number == 0 || number > 0x1FFF_FFFF {
        return Err(format!("field number {number} is out of range"));
    }
    let number = number as u32;
    match tag & 7 {
        0 => {
            let (v, rest) = varint(rest).ok_or("truncated varint")?;
            Ok((number, Some(Value::Varint(v)), rest))
        }
        1 => {
            let bytes: [u8; 8] = rest.get(..8).ok_or("truncated fixed64")?.try_into().unwrap_or_default();
            Ok((number, Some(Value::Fixed64(u64::from_le_bytes(bytes))), &rest[8..]))
        }
        2 => {
            let (len, rest) = varint(rest).ok_or("truncated length")?;
            let len = usize::try_from(len).map_err(|_| "length out of range")?;
            let data = rest.get(..len).ok_or("length past the end")?;
            Ok((number, Some(Value::Len(data)), &rest[len..]))
        }
        3 => {
            if depth >= MAX_DEPTH {
                return Err("groups nested too deeply".into());
            }
            let mut fields = Vec::new();
            let mut b = rest;
            loop {
                if b.is_empty() {
                    return Err(format!("group {number} isn't closed"));
                }
                let (inner, value, rest) = field(b, depth + 1)?;
                b = rest;
                match value {
                    Some(value) => fields.push((inner, value)),
                    None if inner == number => break,
                    None => return Err(format!("group {number} closed as {inner}")),
                }
            }
            Ok((number, Some(Value::Group(fields)), b))
        }
        4 => Ok((number, None, rest)),
        5 => {
            let bytes: [u8; 4] = rest.get(..4).ok_or("truncated fixed32")?.try_into().unwrap_or_default();
            Ok((number, Some(Value::Fixed32(u32::from_le_bytes(bytes))), &rest[4..]))
        }
        w => Err(format!("wire type {w} doesn't exist")),
    }
}

/// A base-128 varint and the bytes after it.
pub(crate) fn varint(b: &[u8]) -> Option<(u64, &[u8])> {
    let mut value = 0u64;
    for (i, &byte) in b.iter().enumerate().take(10) {
        value |= u64::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((value, &b[i + 1..]));
        }
    }
    None
}

fn to_node(fields: Vec<(u32, Value<'_>)>, pointer: &str, badges: &mut Badges, depth: usize) -> Node {
    // Group repeats by field number, in order of first appearance.
    let mut grouped: Vec<(u32, Vec<Value<'_>>)> = Vec::new();
    for (number, value) in fields {
        match grouped.iter_mut().find(|(n, _)| *n == number) {
            Some((_, values)) => values.push(value),
            None => grouped.push((number, vec![value])),
        }
    }
    let mut out = Vec::with_capacity(grouped.len());
    for (number, values) in grouped {
        let key = number.to_string();
        let field_ptr = child(pointer, &key);
        let node = if values.len() == 1 {
            let value = values.into_iter().next().unwrap_or(Value::Varint(0));
            value_node(value, &field_ptr, badges, depth)
        } else {
            badges.add(&field_ptr, "repeated");
            Node::Arr(
                values
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| value_node(v, &child(&field_ptr, &i.to_string()), badges, depth))
                    .collect(),
            )
        };
        out.push((key, node));
    }
    Node::Obj(out)
}

fn value_node(value: Value<'_>, pointer: &str, badges: &mut Badges, depth: usize) -> Node {
    match value {
        Value::Varint(v) => {
            let zigzag = ((v >> 1) as i64) ^ -((v & 1) as i64);
            if v > i64::MAX as u64 {
                // A negative int32/int64 is ten bytes of two's complement.
                badges.add(pointer, format!("varint · uint64 {v}"));
                Node::I64(v as i64)
            } else {
                let label = if v & 1 == 1 && v > 1 {
                    format!("varint · sint {zigzag}")
                } else {
                    "varint".to_owned()
                };
                badges.add(pointer, label);
                Node::U64(v)
            }
        }
        Value::Fixed64(v) => {
            let f = f64::from_bits(v);
            if plausible_float(f) {
                badges.add(pointer, format!("fixed64 · double · int {}", v as i64));
                Node::F64(f)
            } else {
                badges.add(pointer, "fixed64");
                Node::I64(v as i64)
            }
        }
        Value::Fixed32(v) => {
            let f = f32::from_bits(v);
            if plausible_float(f64::from(f)) {
                badges.add(pointer, format!("fixed32 · float · int {}", v as i32));
                // Through its shortest decimal text, not f64's longer one.
                Node::F64(f.to_string().parse().unwrap_or(f64::from(f)))
            } else {
                badges.add(pointer, "fixed32");
                Node::I64(i64::from(v as i32))
            }
        }
        Value::Group(fields) => {
            badges.add(pointer, "group");
            to_node(fields, pointer, badges, depth + 1)
        }
        Value::Len(data) => len_node(data, pointer, badges, depth),
    }
}

fn len_node(data: &[u8], pointer: &str, badges: &mut Badges, depth: usize) -> Node {
    if data.is_empty() {
        badges.add(pointer, "len 0");
        return Node::Str(String::new());
    }
    if let Some(text) = printable(data) {
        badges.add(pointer, "string");
        return Node::Str(text.to_owned());
    }
    if depth < MAX_DEPTH
        && let Ok(fields) = parse(data, depth + 1)
    {
        badges.add(pointer, "message");
        return to_node(fields, pointer, badges, depth + 1);
    }
    let mut label = format!("bytes · {}", data.len());
    if let Some(packed) = packed_varints(data) {
        let list: Vec<String> = packed.iter().map(u64::to_string).collect();
        label.push_str(&format!(" · packed? {}", list.join(", ")));
    }
    badges.add(pointer, label);
    Node::Str(base64::engine::general_purpose::STANDARD.encode(data))
}

/// Text a person would read: UTF-8 without control characters other than
/// whitespace.
fn printable(data: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(data).ok()?;
    text.chars()
        .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        .then_some(text)
}

/// Doubles that are likely meant as numbers: zero or a magnitude a person
/// would write. Other bit patterns are more likely integers.
fn plausible_float(f: f64) -> bool {
    f == 0.0 || (f.is_finite() && (1e-9..1e15).contains(&f.abs()))
}

fn packed_varints(mut data: &[u8]) -> Option<Vec<u64>> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let (v, rest) = varint(data)?;
        out.push(v);
        if out.len() > PACKED_HINT_MAX {
            return None;
        }
        data = rest;
    }
    (out.len() > 1).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn badge(badges: &[pm_model::Badge], pointer: &str) -> String {
        badges
            .iter()
            .find(|b| b.pointer == pointer)
            .map(|b| b.label.clone())
            .unwrap_or_default()
    }

    #[test]
    fn decodes_fields_with_wire_types() {
        // 1: 150 (varint), 2: "testing", 3: {1: 1}, 4: 1.5 (double),
        // 5: 2.5f (float), 1 again, 6: -3 as sint (zigzag 5).
        let mut b = vec![0x08, 0x96, 0x01, 0x12, 0x07];
        b.extend_from_slice(b"testing");
        b.extend_from_slice(&[0x1a, 0x02, 0x08, 0x01]);
        b.push(0x21);
        b.extend_from_slice(&1.5f64.to_le_bytes());
        b.push(0x2d);
        b.extend_from_slice(&2.5f32.to_le_bytes());
        b.extend_from_slice(&[0x08, 0x02, 0x30, 0x05]);
        let (node, badges) = decode(&b).unwrap();
        assert_eq!(
            node.to_json(),
            r#"{
  "1": [
    150,
    2
  ],
  "2": "testing",
  "3": {
    "1": 1
  },
  "4": 1.5,
  "5": 2.5,
  "6": 5
}"#
        );
        assert_eq!(badge(&badges, "/1"), "repeated");
        assert_eq!(badge(&badges, "/1/0"), "varint");
        assert_eq!(badge(&badges, "/2"), "string");
        assert_eq!(badge(&badges, "/3"), "message");
        assert_eq!(badge(&badges, "/3/1"), "varint");
        assert!(badge(&badges, "/4").starts_with("fixed64 · double"));
        assert!(badge(&badges, "/5").starts_with("fixed32 · float"));
        assert_eq!(badge(&badges, "/6"), "varint · sint -3");
    }

    #[test]
    fn negative_int64_and_bytes() {
        let mut b = vec![0x08];
        b.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]);
        // Not text, not a message (field 0): bytes, which also read as packed.
        b.extend_from_slice(&[0x12, 0x03, 0x00, 0x01, 0x02]);
        let (node, badges) = decode(&b).unwrap();
        assert_eq!(
            node,
            Node::Obj(vec![
                ("1".into(), Node::I64(-1)),
                ("2".into(), Node::Str("AAEC".into()))
            ])
        );
        assert_eq!(badge(&badges, "/2"), "bytes · 3 · packed? 0, 1, 2");
    }

    #[test]
    fn groups_and_errors() {
        // 1: group { 2: 7 }.
        let (node, badges) = decode(&[0x0b, 0x10, 0x07, 0x0c]).unwrap();
        assert_eq!(node.to_json(), "{\n  \"1\": {\n    \"2\": 7\n  }\n}");
        assert_eq!(badge(&badges, "/1"), "group");
        assert!(decode(&[0x0b, 0x10]).is_err());
        assert!(decode(&[0x12, 0x05, 0x01]).is_err());
        assert!(decode(&[0x07]).is_err());
        assert!(decode(b"\x00").is_err());
    }

    #[test]
    fn deep_nesting_stays_bounded() {
        let mut b = vec![0x08, 0x01];
        for _ in 0..200 {
            let mut outer = vec![0x0a];
            let mut len = b.len();
            while len >= 0x80 {
                outer.push((len as u8) | 0x80);
                len >>= 7;
            }
            outer.push(len as u8);
            outer.extend_from_slice(&b);
            b = outer;
        }
        assert!(decode(&b).is_ok());
    }
}
