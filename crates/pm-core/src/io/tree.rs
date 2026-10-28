//! An ordered value tree for decoded binary bodies (schema-less protobuf,
//! MessagePack, CBOR), written out as pretty JSON text with badges by JSON
//! pointer.
//!
//! serde_json's `Map` sorts its keys (and `preserve_order` stays off, see
//! `docs/PLAN.md`), but field `10` must not come before field `2`, so the
//! tree writes its own JSON.

use pm_model::Badge;

/// One decoded value.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Null,
    Bool(bool),
    U64(u64),
    I64(i64),
    F64(f64),
    Str(String),
    Arr(Vec<Node>),
    Obj(Vec<(String, Node)>),
}

/// A tree being built, with notes on its values.
#[derive(Debug, Default)]
pub struct Badges(Vec<Badge>);

impl Badges {
    pub fn add(&mut self, pointer: &str, label: impl Into<String>) {
        self.0.push(Badge {
            pointer: pointer.to_owned(),
            label: label.into(),
        });
    }

    pub fn into_vec(self) -> Vec<Badge> {
        self.0
    }
}

/// `pointer` extended by one key or index, escaped per RFC 6901.
pub fn child(pointer: &str, key: &str) -> String {
    let mut out = String::with_capacity(pointer.len() + key.len() + 1);
    out.push_str(pointer);
    out.push('/');
    for c in key.chars() {
        match c {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            c => out.push(c),
        }
    }
    out
}

impl Node {
    /// Pretty JSON text, indented by two spaces like `serde_json`.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        match self {
            Node::Null => out.push_str("null"),
            Node::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Node::U64(n) => out.push_str(&n.to_string()),
            Node::I64(n) => out.push_str(&n.to_string()),
            Node::F64(f) => match serde_json::Number::from_f64(*f) {
                Some(n) => out.push_str(&n.to_string()),
                // JSON has no NaN or infinity.
                None => push_str(out, &f.to_string()),
            },
            Node::Str(s) => push_str(out, s),
            Node::Arr(items) if items.is_empty() => out.push_str("[]"),
            Node::Obj(fields) if fields.is_empty() => out.push_str("{}"),
            Node::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    item.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push(']');
            }
            Node::Obj(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    push_str(out, key);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push('}');
            }
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn push_str(out: &mut String, s: &str) {
    // serde_json escapes exactly as JSON needs; a string never fails.
    out.push_str(&serde_json::to_string(s).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_ordered_pretty_json_with_exact_numbers() {
        let node = Node::Obj(vec![
            ("2".into(), Node::U64(u64::MAX)),
            ("10".into(), Node::Arr(vec![Node::I64(-1), Node::F64(1.5)])),
            ("a\"b".into(), Node::Obj(vec![])),
            ("nan".into(), Node::F64(f64::NAN)),
        ]);
        let json = node.to_json();
        assert_eq!(
            json,
            "{\n  \"2\": 18446744073709551615,\n  \"10\": [\n    -1,\n    1.5\n  ],\n  \"a\\\"b\": {},\n  \"nan\": \"NaN\"\n}"
        );
        assert!(serde_json::from_str::<serde_json::Value>(&json).is_ok());
    }

    #[test]
    fn pointers_escape_keys() {
        assert_eq!(child("", "a/b~c"), "/a~1b~0c");
        assert_eq!(child("/1", "0"), "/1/0");
    }
}
