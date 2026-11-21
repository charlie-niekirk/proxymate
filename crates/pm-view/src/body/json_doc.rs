//! Lossless, ordered JSON (the reading half of `lib/json/doc.ts` and
//! `losslessJson.ts`): members keep their order and duplicates, numbers
//! keep their exact text (`1.50`, 64-bit ids). serde_json would sort keys
//! and round numbers unless `preserve_order` and `arbitrary_precision` were
//! on, which the workspace keeps off (CLAUDE.md, M13).

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum JNode {
    Null,
    Bool(bool),
    /// The number as written.
    Num(String),
    Str(String),
    Arr(Vec<JNode>),
    Obj(Vec<(String, JNode)>),
}

/// Nesting deeper than this is refused rather than risking the stack.
const MAX_DEPTH: usize = 1000;

impl JNode {
    pub fn is_container(&self) -> bool {
        matches!(self, JNode::Arr(_) | JNode::Obj(_))
    }

    /// Children count of a container, 0 for a scalar.
    pub fn len(&self) -> usize {
        match self {
            JNode::Arr(items) => items.len(),
            JNode::Obj(entries) => entries.len(),
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The last member named `key` (as JSON readers take it).
    pub fn member(&self, key: &str) -> Option<&JNode> {
        match self {
            JNode::Obj(entries) => entries.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// JSON text: compact with `indent` 0, else pretty with that many spaces.
    pub fn to_json(&self, indent: usize) -> String {
        let mut out = String::new();
        write_node(self, indent, 0, &mut out);
        out
    }

    /// What "Copy value" copies: a string's own text, else its JSON.
    pub fn value_text(&self) -> String {
        match self {
            JNode::Str(s) => s.clone(),
            other => other.to_json(2),
        }
    }

    /// A scalar as the tree shows it (strings quoted).
    pub fn scalar_text(&self) -> String {
        match self {
            JNode::Null => "null".into(),
            JNode::Bool(b) => b.to_string(),
            JNode::Num(n) => n.clone(),
            JNode::Str(s) => quote(s),
            JNode::Arr(_) | JNode::Obj(_) => String::new(),
        }
    }
}

/// `s` as a JSON string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_node(node: &JNode, indent: usize, depth: usize, out: &mut String) {
    let pad = |out: &mut String, depth: usize| {
        if indent > 0 {
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', indent * depth));
        }
    };
    match node {
        JNode::Arr(items) if items.is_empty() => out.push_str("[]"),
        JNode::Obj(entries) if entries.is_empty() => out.push_str("{}"),
        JNode::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                pad(out, depth + 1);
                write_node(item, indent, depth + 1, out);
            }
            pad(out, depth);
            out.push(']');
        }
        JNode::Obj(entries) => {
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                pad(out, depth + 1);
                out.push_str(&quote(k));
                out.push_str(if indent > 0 { ": " } else { ":" });
                write_node(v, indent, depth + 1, out);
            }
            pad(out, depth);
            out.push('}');
        }
        scalar => out.push_str(&scalar.scalar_text()),
    }
}

/// Parses `text` (a leading BOM and surrounding whitespace allowed).
pub fn parse(text: &str) -> Result<JNode, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
    };
    p.ws();
    let node = p.value(0)?;
    p.ws();
    if p.i < p.s.len() {
        return Err(p.error("Unexpected data after the JSON value"));
    }
    Ok(node)
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        if self.i >= self.s.len() {
            format!("{what}: unexpected end of input")
        } else {
            format!("{what} at position {}", self.i)
        }
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<JNode, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("Nested too deeply"));
        }
        match self.s.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut entries = Vec::new();
                self.ws();
                if self.eat(b'}') {
                    return Ok(JNode::Obj(entries));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return Err(self.error("Expected a member name"));
                    }
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(b':') {
                        return Err(self.error("Expected ':'"));
                    }
                    self.ws();
                    let value = self.value(depth + 1)?;
                    entries.push((key, value));
                    self.ws();
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b'}') {
                        return Ok(JNode::Obj(entries));
                    }
                    return Err(self.error("Expected ',' or '}'"));
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat(b']') {
                    return Ok(JNode::Arr(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b']') {
                        return Ok(JNode::Arr(items));
                    }
                    return Err(self.error("Expected ',' or ']'"));
                }
            }
            Some(b'"') => Ok(JNode::Str(self.string()?)),
            Some(b't') if self.s[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(JNode::Bool(true))
            }
            Some(b'f') if self.s[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(JNode::Bool(false))
            }
            Some(b'n') if self.s[self.i..].starts_with(b"null") => {
                self.i += 4;
                Ok(JNode::Null)
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("Unexpected token")),
        }
    }

    fn number(&mut self) -> Result<JNode, String> {
        let start = self.i;
        self.eat(b'-');
        let digits = |p: &mut Parser| {
            let from = p.i;
            while p.s.get(p.i).is_some_and(u8::is_ascii_digit) {
                p.i += 1;
            }
            p.i > from
        };
        if self.eat(b'0') {
        } else if !digits(self) {
            return Err(self.error("Invalid number"));
        }
        if self.eat(b'.') && !digits(self) {
            return Err(self.error("Invalid number"));
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !digits(self) {
                return Err(self.error("Invalid number"));
            }
        }
        Ok(JNode::Num(self.text[start..self.i].to_owned()))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .text
            .get(self.i..self.i + 4)
            .ok_or_else(|| self.error("Bad \\u escape"))?;
        let n = u32::from_str_radix(digits, 16).map_err(|_| self.error("Bad \\u escape"))?;
        self.i += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // the opening quote
        let mut out = String::new();
        loop {
            // Copy the run up to the next quote or escape at once.
            let start = self.i;
            while self.i < self.s.len() && !matches!(self.s[self.i], b'"' | b'\\') {
                if self.s[self.i] < 0x20 {
                    return Err(self.error("Control character in a string"));
                }
                self.i += 1;
            }
            out.push_str(&self.text[start..self.i]);
            match self.s.get(self.i) {
                None => return Err(self.error("Unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => {
                    self.i += 1;
                    let escape = *self.s.get(self.i).ok_or_else(|| self.error("Unterminated string"))?;
                    self.i += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let c = if (0xd800..0xdc00).contains(&hi) && self.s[self.i..].starts_with(b"\\u") {
                                let save = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xdc00..0xe000).contains(&lo) {
                                    char::from_u32(0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00))
                                } else {
                                    self.i = save;
                                    None
                                }
                            } else {
                                char::from_u32(hi)
                            };
                            out.push(c.unwrap_or('\u{fffd}'));
                        }
                        _ => {
                            self.i -= 1;
                            return Err(self.error("Bad escape"));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_duplicates_and_exact_numbers() {
        let doc = parse(r#"{"z":1,"a":1.50,"id":12345678901234567890,"a":true}"#).unwrap();
        assert_eq!(doc.to_json(0), r#"{"z":1,"a":1.50,"id":12345678901234567890,"a":true}"#);
        assert_eq!(doc.member("a"), Some(&JNode::Bool(true)), "the last duplicate wins");
        assert_eq!(
            doc.to_json(2),
            "{\n  \"z\": 1,\n  \"a\": 1.50,\n  \"id\": 12345678901234567890,\n  \"a\": true\n}"
        );
    }

    #[test]
    fn reads_escapes_and_surrogates() {
        let doc = parse(r#"["a\"b\\cé😀\n", [], {}]"#).unwrap();
        assert_eq!(
            doc,
            JNode::Arr(vec![
                JNode::Str("a\"b\\cé😀\n".into()),
                JNode::Arr(vec![]),
                JNode::Obj(vec![])
            ])
        );
        assert_eq!(doc.to_json(0), r#"["a\"b\\cé😀\n",[],{}]"#);
    }

    #[test]
    fn reports_errors_with_positions() {
        assert_eq!(parse("{\"a\" 1}").unwrap_err(), "Expected ':' at position 5");
        assert_eq!(parse("[1,").unwrap_err(), "Unexpected token: unexpected end of input");
        assert!(parse("[1] x").is_err());
        assert!(parse("01").is_err());
        assert!(parse(&"[".repeat(2000)).is_err());
    }
}
