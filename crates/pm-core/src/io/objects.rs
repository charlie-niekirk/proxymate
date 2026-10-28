//! MessagePack and CBOR bodies as JSON trees. Values JSON can't hold are
//! badged: binary (as Base64), MessagePack ext types, CBOR tags, and map
//! keys that aren't strings. A body holding several values back to back
//! (a MessagePack stream, a CBOR sequence) gives one message each.

use base64::Engine as _;
use pm_model::Badge;

use super::tree::{Badges, Node, child};

/// Nesting deeper than this fails rather than overflowing the stack.
const MAX_DEPTH: usize = 256;
/// More top-level values than this are left undecoded.
const MAX_VALUES: usize = 10_000;

pub type Decoded = (Node, Vec<Badge>);

/// Every MessagePack value in `bytes`; the error says where decoding stopped.
pub fn msgpack(bytes: &[u8]) -> (Vec<Decoded>, Option<String>) {
    let mut rest = bytes;
    let mut out = Vec::new();
    while !rest.is_empty() && out.len() < MAX_VALUES {
        let offset = bytes.len() - rest.len();
        match rmpv::decode::read_value_with_max_depth(&mut rest, MAX_DEPTH) {
            Ok(value) => {
                let mut badges = Badges::default();
                let node = msgpack_node(value, "", &mut badges);
                out.push((node, badges.into_vec()));
            }
            Err(e) => return (out, Some(format!("not MessagePack at byte {offset}: {e}"))),
        }
    }
    (out, None)
}

fn msgpack_node(value: rmpv::Value, pointer: &str, badges: &mut Badges) -> Node {
    use rmpv::Value as V;
    match value {
        V::Nil => Node::Null,
        V::Boolean(b) => Node::Bool(b),
        V::Integer(i) => match (i.as_u64(), i.as_i64()) {
            (Some(u), _) => Node::U64(u),
            (_, Some(n)) => Node::I64(n),
            _ => Node::Null,
        },
        V::F32(f) => {
            badges.add(pointer, "float32");
            Node::F64(f.to_string().parse().unwrap_or(f64::from(f)))
        }
        V::F64(f) => Node::F64(f),
        V::String(s) => match s.into_str() {
            Some(s) => Node::Str(s),
            None => {
                badges.add(pointer, "invalid UTF-8");
                Node::Str(String::new())
            }
        },
        V::Binary(b) => {
            badges.add(pointer, format!("bin · {}", b.len()));
            Node::Str(base64::engine::general_purpose::STANDARD.encode(b))
        }
        V::Ext(kind, data) => {
            badges.add(pointer, ext_label(kind, &data));
            Node::Str(base64::engine::general_purpose::STANDARD.encode(data))
        }
        V::Array(items) => Node::Arr(
            items
                .into_iter()
                .enumerate()
                .map(|(i, v)| msgpack_node(v, &child(pointer, &i.to_string()), badges))
                .collect(),
        ),
        V::Map(entries) => {
            let mut fields = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                let (name, note) = match key {
                    V::String(s) if s.is_str() => (s.into_str().unwrap_or_default(), None),
                    other => (other.to_string(), Some(key_kind_msgpack(&other))),
                };
                let ptr = child(pointer, &name);
                if let Some(note) = note {
                    badges.add(&ptr, format!("key: {note}"));
                }
                fields.push((name, msgpack_node(value, &ptr, badges)));
            }
            Node::Obj(fields)
        }
    }
}

/// Ext type -1 is the timestamp extension; others are application types.
fn ext_label(kind: i8, data: &[u8]) -> String {
    if kind == -1
        && let Some(time) = msgpack_timestamp(data)
    {
        return format!("ext -1 · timestamp {time}");
    }
    format!("ext {kind} · {} bytes", data.len())
}

/// Seconds (and nanoseconds) since the epoch, per the timestamp spec.
fn msgpack_timestamp(data: &[u8]) -> Option<String> {
    let (secs, nanos) = match data.len() {
        4 => (i64::from(u32::from_be_bytes(data.try_into().ok()?)), 0),
        8 => {
            let v = u64::from_be_bytes(data.try_into().ok()?);
            ((v & 0x3_FFFF_FFFF) as i64, (v >> 34) as u32)
        }
        12 => (
            i64::from_be_bytes(data[4..].try_into().ok()?),
            u32::from_be_bytes(data[..4].try_into().ok()?),
        ),
        _ => return None,
    };
    let time = time::OffsetDateTime::from_unix_timestamp(secs).ok()? + time::Duration::nanoseconds(i64::from(nanos));
    time.format(&time::format_description::well_known::Rfc3339).ok()
}

fn key_kind_msgpack(key: &rmpv::Value) -> &'static str {
    use rmpv::Value as V;
    match key {
        V::Integer(_) => "int",
        V::Boolean(_) => "bool",
        V::Nil => "nil",
        V::F32(_) | V::F64(_) => "float",
        V::Binary(_) => "bin",
        V::Array(_) => "array",
        V::Map(_) => "map",
        V::Ext(..) => "ext",
        V::String(_) => "string",
    }
}

/// Every CBOR item in `bytes` (an RFC 8742 sequence when there are several).
pub fn cbor(bytes: &[u8]) -> (Vec<Decoded>, Option<String>) {
    let mut rest = bytes;
    let mut out = Vec::new();
    while !rest.is_empty() && out.len() < MAX_VALUES {
        let offset = bytes.len() - rest.len();
        match ciborium::de::from_reader_with_recursion_limit::<ciborium::Value, _>(&mut rest, MAX_DEPTH) {
            Ok(value) => {
                let mut badges = Badges::default();
                let node = cbor_node(value, "", &mut badges);
                out.push((node, badges.into_vec()));
            }
            Err(e) => return (out, Some(format!("not CBOR at byte {offset}: {e}"))),
        }
    }
    (out, None)
}

fn cbor_node(value: ciborium::Value, pointer: &str, badges: &mut Badges) -> Node {
    use ciborium::Value as V;
    match value {
        V::Null => Node::Null,
        V::Bool(b) => Node::Bool(b),
        V::Integer(i) => {
            let n = i128::from(i);
            match (u64::try_from(n), i64::try_from(n)) {
                (Ok(u), _) => Node::U64(u),
                (_, Ok(s)) => Node::I64(s),
                // CBOR's negative integers reach -2^64.
                _ => {
                    badges.add(pointer, "int · beyond 64 bits");
                    Node::Str(n.to_string())
                }
            }
        }
        V::Float(f) => Node::F64(f),
        V::Text(s) => Node::Str(s),
        V::Bytes(b) => {
            badges.add(pointer, format!("bytes · {}", b.len()));
            Node::Str(base64::engine::general_purpose::STANDARD.encode(b))
        }
        V::Tag(tag, inner) => {
            badges.add(pointer, tag_label(tag));
            cbor_node(*inner, pointer, badges)
        }
        V::Array(items) => Node::Arr(
            items
                .into_iter()
                .enumerate()
                .map(|(i, v)| cbor_node(v, &child(pointer, &i.to_string()), badges))
                .collect(),
        ),
        V::Map(entries) => {
            let mut fields = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                let (name, note) = match key {
                    V::Text(s) => (s, None),
                    V::Integer(i) => (i128::from(i).to_string(), Some("int")),
                    other => (format!("{other:?}"), Some("other")),
                };
                let ptr = child(pointer, &name);
                if let Some(note) = note {
                    badges.add(&ptr, format!("key: {note}"));
                }
                fields.push((name, cbor_node(value, &ptr, badges)));
            }
            Node::Obj(fields)
        }
        // `Value` is non-exhaustive.
        _ => Node::Null,
    }
}

/// The registered tags people meet most, by name.
fn tag_label(tag: u64) -> String {
    let name = match tag {
        0 => "date/time",
        1 => "epoch time",
        2 => "bignum",
        3 => "negative bignum",
        4 => "decimal fraction",
        21..=23 => "expected encoding",
        24 => "embedded CBOR",
        32 => "URI",
        37 => "UUID",
        55799 => "self-described CBOR",
        _ => return format!("tag {tag}"),
    };
    format!("tag {tag} · {name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn badge(badges: &[Badge], pointer: &str) -> String {
        badges
            .iter()
            .find(|b| b.pointer == pointer)
            .map(|b| b.label.clone())
            .unwrap_or_default()
    }

    #[test]
    fn msgpack_values_bins_exts_and_streams() {
        let value = rmpv::Value::Map(vec![
            ("id".into(), rmpv::Value::from(u64::MAX)),
            ("raw".into(), rmpv::Value::Binary(vec![1, 2, 3])),
            (rmpv::Value::from(7), rmpv::Value::Ext(5, vec![9])),
            ("at".into(), rmpv::Value::Ext(-1, 0u32.to_be_bytes().to_vec())),
        ]);
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, &value).unwrap();
        rmpv::encode::write_value(&mut bytes, &rmpv::Value::from(-5)).unwrap();
        let (values, error) = msgpack(&bytes);
        assert_eq!(error, None);
        assert_eq!(values.len(), 2);
        let (node, badges) = &values[0];
        assert_eq!(
            node.to_json(),
            "{\n  \"id\": 18446744073709551615,\n  \"raw\": \"AQID\",\n  \"7\": \"CQ==\",\n  \"at\": \"AAAAAA==\"\n}"
        );
        assert_eq!(badge(badges, "/raw"), "bin · 3");
        let labels: Vec<&str> = badges
            .iter()
            .filter(|b| b.pointer == "/7")
            .map(|b| b.label.as_str())
            .collect();
        assert_eq!(labels, ["key: int", "ext 5 · 1 bytes"]);
        assert_eq!(badge(badges, "/at"), "ext -1 · timestamp 1970-01-01T00:00:00Z");
        assert_eq!(values[1].0, Node::I64(-5));
        let (_, error) = msgpack(&[0x92, 0x01]);
        assert!(error.unwrap().contains("byte 0"));
    }

    #[test]
    fn cbor_tags_bytes_and_sequences() {
        let value = ciborium::Value::Map(vec![
            ("when".into(), ciborium::Value::Tag(1, Box::new(1_700_000_000.into()))),
            (ciborium::Value::Integer(3.into()), ciborium::Value::Bytes(vec![0xff])),
        ]);
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&value, &mut bytes).unwrap();
        ciborium::ser::into_writer(&ciborium::Value::Text("next".into()), &mut bytes).unwrap();
        let (values, error) = cbor(&bytes);
        assert_eq!(error, None);
        assert_eq!(values.len(), 2);
        let (node, badges) = &values[0];
        assert_eq!(node.to_json(), "{\n  \"when\": 1700000000,\n  \"3\": \"/w==\"\n}");
        assert_eq!(badge(badges, "/when"), "tag 1 · epoch time");
        assert!(badges.iter().any(|b| b.pointer == "/3" && b.label == "bytes · 1"));
        assert_eq!(values[1].0, Node::Str("next".into()));
        assert!(cbor(&[0xa1]).1.is_some());
    }
}
