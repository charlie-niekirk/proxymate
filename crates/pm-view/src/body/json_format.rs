//! The JSON viewer's pretty print (`lib/body/jsonFormat.ts`): re-indents
//! JSON text without parsing it into values, so numbers keep their exact
//! digits (`1.50`, 64-bit ids) and broken or cut-off JSON still comes out
//! readable. One pass over the bytes, so 20 MB takes a few tens of ms.

/// The text re-indented by `indent`, or `None` if it doesn't start like
/// JSON (an object or an array).
pub fn format_json(src: &str, indent: &str) -> Option<String> {
    let text = src.trim_start_matches('\u{feff}').trim();
    let b = text.as_bytes();
    if !matches!(b.first(), Some(b'{' | b'[')) {
        return None;
    }
    let mut out = String::with_capacity(text.len() + text.len() / 2);
    let mut depth = 0usize;
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        for _ in 0..depth {
            out.push_str(indent);
        }
    };
    let is_ws = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'\x0b');
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' => {
                // Copy the string verbatim, escapes included.
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                let end = (j + 1).min(b.len());
                out.push_str(&text[i..end]);
                i = end;
                continue;
            }
            b'{' | b'[' => {
                // Keep empty containers on one line.
                let close = if c == b'{' { b'}' } else { b']' };
                let mut j = i + 1;
                while j < b.len() && is_ws(b[j]) {
                    j += 1;
                }
                if b.get(j) == Some(&close) {
                    out.push(c as char);
                    out.push(close as char);
                    i = j + 1;
                    continue;
                }
                out.push(c as char);
                depth += 1;
                newline(&mut out, depth);
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c as char);
            }
            b',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            b':' => out.push_str(": "),
            c if is_ws(c) => {}
            _ => {
                // A literal: copy up to the next structural character.
                let mut j = i;
                while j < b.len() && !is_ws(b[j]) && !matches!(b[j], b'{' | b'}' | b'[' | b']' | b',' | b':' | b'"') {
                    j += 1;
                }
                out.push_str(&text[i..j]);
                i = j;
                continue;
            }
        }
        i += 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indents_like_json_stringify_for_ordinary_documents() {
        let doc = r#"{"a":[1,true,null],"b":{"c":"x:y,{z}"},"d":[],"e":{}}"#;
        assert_eq!(
            format_json(doc, "  ").unwrap(),
            "{\n  \"a\": [\n    1,\n    true,\n    null\n  ],\n  \"b\": {\n    \"c\": \"x:y,{z}\"\n  },\n  \"d\": [],\n  \"e\": {}\n}"
        );
    }

    #[test]
    fn keeps_number_literals_and_escapes_exactly() {
        assert_eq!(
            format_json(r#"{"price":1.50,"id":12345678901234567890,"s":"a\"b"}"#, "  ").unwrap(),
            "{\n  \"price\": 1.50,\n  \"id\": 12345678901234567890,\n  \"s\": \"a\\\"b\"\n}"
        );
    }

    #[test]
    fn formats_loosely_when_the_json_is_broken_or_cut_short() {
        assert_eq!(format_json(r#"{"a": [1, 2"#, "  ").unwrap(), "{\n  \"a\": [\n    1,\n    2");
        assert_eq!(format_json("hello", "  "), None);
        assert_eq!(format_json("{\"a\":\"unterminated", "  ").unwrap(), "{\n  \"a\": \"unterminated");
        assert_eq!(format_json("\u{feff} [ ] ", "  ").unwrap(), "[]");
        assert_eq!(format_json("{\"é\":\"ü\"}", "  ").unwrap(), "{\n  \"é\": \"ü\"\n}");
    }
}
