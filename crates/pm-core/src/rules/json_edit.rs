//! Byte-exact JSON edits for `JsonSet` and `JsonRemove`.
//!
//! Parsing a body into a `serde_json::Value` and writing it back sorts the
//! keys, rewrites numbers (`1.50` → `1.5`, 64-bit ids through `f64` if
//! they don't fit `u64`/`i64`) and drops the formatting. A debugging proxy
//! must change only what a rule asks for, so these edits scan the bytes to
//! the addressed value and splice in its replacement:
//! - everything else stays byte-identical;
//! - an inserted member copies the whitespace and `:` style of its last
//!   sibling (or of the document, in an empty container);
//! - a removed member takes its comma with it.
//!
//! The semantics are those of the old parse-and-reserialise code (kept as
//! the reference in the tests): missing object members on the way are
//! created, `-` (or the length) appends to an array, and an object that
//! repeats a key is read as the last one (like serde_json and `JSON.parse`).
//!
//! A run of edits whose targets don't touch each other is applied in one
//! pass, back to front; anything else (two edits in the same array whose
//! length one of them changes, overlapping targets…) runs one by one.

use serde::de::IgnoredAny;

/// One JSON edit.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Edit<'a> {
    /// Sets the value at `pointer` to `raw`, which must be valid JSON text.
    Set {
        pointer: &'a str,
        raw: &'a str,
    },
    Remove {
        pointer: &'a str,
    },
}

impl Edit<'_> {
    fn pointer(&self) -> &str {
        match self {
            Edit::Set { pointer, .. } | Edit::Remove { pointer } => pointer,
        }
    }
}

/// Applies `edits` in order to a JSON document. Each gets its own outcome:
/// whether it changed the document, or why it couldn't run. A failed edit
/// changes nothing; the others still apply.
pub(crate) fn apply(doc: &mut Vec<u8>, edits: &[Edit<'_>]) -> Vec<Result<bool, String>> {
    if let Err(e) = serde_json::from_slice::<IgnoredAny>(doc) {
        return vec![Err(format!("the body isn't JSON ({e})")); edits.len()];
    }
    if edits.len() > 1
        && let Some(results) = apply_disjoint(doc, edits)
    {
        return results;
    }
    edits.iter().map(|edit| apply_one(doc, *edit)).collect()
}

/// A replacement of `start..end` in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Splice {
    start: usize,
    end: usize,
    text: Vec<u8>,
}

/// What an edit comes to on one document.
struct Planned {
    splice: Splice,
    /// The pointer of an array whose length this edit changes: later edits
    /// in that array would address other items once it has run.
    resizes_array: Option<String>,
    /// A removal of a key the object repeats: only the last one is planned.
    repeated: bool,
}

fn apply_one(doc: &mut Vec<u8>, edit: Edit<'_>) -> Result<bool, String> {
    match edit {
        Edit::Set { pointer, raw } => {
            let planned = plan_set(doc, pointer, raw)?;
            let s = planned.splice;
            let changed = doc[s.start..s.end] != s.text[..];
            doc.splice(s.start..s.end, s.text);
            Ok(changed)
        }
        Edit::Remove { pointer } => {
            // A repeated key goes one occurrence at a time, last first.
            let mut next = plan_remove(doc, pointer);
            let removed = next.is_some();
            while let Some(planned) = next {
                let s = planned.splice;
                doc.splice(s.start..s.end, s.text);
                next = if planned.repeated {
                    plan_remove(doc, pointer)
                } else {
                    None
                };
            }
            Ok(removed)
        }
    }
}

/// Applies `edits` in one pass if they can't affect each other; `None`
/// means they must run one by one.
fn apply_disjoint(doc: &mut Vec<u8>, edits: &[Edit<'_>]) -> Option<Vec<Result<bool, String>>> {
    let mut planned = Vec::with_capacity(edits.len());
    for edit in edits {
        // A failure or a no-op might not be one after an earlier edit.
        let p = match *edit {
            Edit::Set { pointer, raw } => plan_set(doc, pointer, raw).ok()?,
            Edit::Remove { pointer } => plan_remove(doc, pointer)?,
        };
        if p.repeated {
            return None;
        }
        planned.push(p);
    }
    for (i, p) in planned.iter().enumerate() {
        if let Some(array) = &p.resizes_array {
            let inside =
                |ptr: &str| ptr == array || ptr.strip_prefix(array.as_str()).is_some_and(|r| r.starts_with('/'));
            if edits.iter().enumerate().any(|(j, e)| j != i && inside(e.pointer())) {
                return None;
            }
        }
    }
    let mut order: Vec<usize> = (0..planned.len()).collect();
    order.sort_by_key(|&i| (planned[i].splice.start, planned[i].splice.end));
    for pair in order.windows(2) {
        let (a, b) = (&planned[pair[0]].splice, &planned[pair[1]].splice);
        // Overlapping, two edits at one spot (two inserts into one object),
        // or an insert next to another edit (after a member being removed).
        let insert = a.start == a.end || b.start == b.end;
        if a.end > b.start || a.start == b.start || (insert && a.end == b.start) {
            return None;
        }
    }
    let results = planned
        .iter()
        .map(|p| Ok(doc[p.splice.start..p.splice.end] != p.splice.text[..]))
        .collect();
    let grow: usize = planned.iter().map(|p| p.splice.text.len()).sum();
    let mut out = Vec::with_capacity(doc.len() + grow);
    let mut at = 0;
    for &i in &order {
        let s = &planned[i].splice;
        out.extend_from_slice(&doc[at..s.start]);
        out.extend_from_slice(&s.text);
        at = s.end;
    }
    out.extend_from_slice(&doc[at..]);
    *doc = out;
    Some(results)
}

pub(crate) fn pointer_tokens(pointer: &str) -> Result<Vec<String>, String> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    let Some(rest) = pointer.strip_prefix('/') else {
        return Err(format!("{pointer:?} isn't a JSON pointer"));
    };
    Ok(rest
        .split('/')
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect())
}

/// The pointer of the value holding the last token.
fn parent_pointer(pointer: &str) -> String {
    pointer.rsplit_once('/').map_or("", |(parent, _)| parent).to_owned()
}

fn plan_set(doc: &[u8], pointer: &str, raw: &str) -> Result<Planned, String> {
    let tokens = pointer_tokens(pointer)?;
    let root = ws(doc, 0);
    let plain = |splice| Planned {
        splice,
        resizes_array: None,
        repeated: false,
    };
    let Some((last, parents)) = tokens.split_last() else {
        return Ok(plain(Splice {
            start: root,
            end: value_end(doc, root),
            text: raw.as_bytes().to_vec(),
        }));
    };
    let at = match walk(doc, root, parents) {
        Walk::Found(at) => at,
        Walk::Missing { object, depth } => {
            // Create the rest of the path as nested objects.
            let colon = Style::colon(doc);
            let value = tokens[depth + 1..]
                .iter()
                .rev()
                .fold(raw.as_bytes().to_vec(), |inner, key| {
                    let mut v = b"{".to_vec();
                    v.extend_from_slice(&key_text(key));
                    v.extend_from_slice(colon);
                    v.extend_from_slice(&inner);
                    v.push(b'}');
                    v
                });
            return Ok(plain(insert(doc, object, Some(&tokens[depth]), &value)));
        }
        Walk::NotContainer(token) => {
            return Err(format!(
                "{pointer}: {token:?} is inside a value that isn't an object or array"
            ));
        }
        Walk::OutOfRange { token, len } => {
            return Err(format!("{token:?} isn't an index of an array of {len}"));
        }
    };
    match doc[at] {
        b'{' => {
            let (members, _) = collect(doc, at);
            Ok(plain(match members.iter().rev().find(|m| key_is(doc, m, last)) {
                Some(m) => Splice {
                    start: m.value.0,
                    end: m.value.1,
                    text: raw.as_bytes().to_vec(),
                },
                None => insert(doc, at, Some(last), raw.as_bytes()),
            }))
        }
        b'[' => {
            let (members, _) = collect(doc, at);
            let append = Planned {
                splice: insert(doc, at, None, raw.as_bytes()),
                resizes_array: Some(parent_pointer(pointer)),
                repeated: false,
            };
            if last == "-" {
                return Ok(append);
            }
            let i = last
                .parse::<usize>()
                .map_err(|_| format!("{pointer}: {last:?} isn't an array index"))?;
            match i.cmp(&members.len()) {
                std::cmp::Ordering::Less => Ok(plain(Splice {
                    start: members[i].value.0,
                    end: members[i].value.1,
                    text: raw.as_bytes().to_vec(),
                })),
                std::cmp::Ordering::Equal => Ok(append),
                std::cmp::Ordering::Greater => Err(format!("{pointer}: index {i} is out of range")),
            }
        }
        _ => Err(format!("{pointer}: the parent isn't an object or array")),
    }
}

/// The removal of the value at `pointer` (its last occurrence, if an
/// object repeats the key), or `None` if there is nothing there.
fn plan_remove(doc: &[u8], pointer: &str) -> Option<Planned> {
    let tokens = pointer_tokens(pointer).ok()?;
    let (last, parents) = tokens.split_last()?;
    let Walk::Found(at) = walk(doc, ws(doc, 0), parents) else {
        return None;
    };
    let (members, close) = match doc[at] {
        b'{' | b'[' => collect(doc, at),
        _ => return None,
    };
    let (index, repeated, resizes_array) = if doc[at] == b'{' {
        let first = members.iter().position(|m| key_is(doc, m, last))?;
        let index = members.iter().rposition(|m| key_is(doc, m, last)).unwrap_or(first);
        (index, index != first, None)
    } else {
        let i = last.parse::<usize>().ok().filter(|&i| i < members.len())?;
        (i, false, Some(parent_pointer(pointer)))
    };
    let (start, end) = if members.len() == 1 {
        (at + 1, close)
    } else if index + 1 < members.len() {
        // A member and its comma, up to the next one.
        (members[index].start, members[index + 1].start)
    } else {
        // The last member, with the comma after the one before.
        (members[index - 1].value.1, members[index].value.1)
    };
    Some(Planned {
        splice: Splice {
            start,
            end,
            text: Vec::new(),
        },
        resizes_array,
        repeated,
    })
}

enum Walk<'t> {
    /// The start of the value the path leads to.
    Found(usize),
    /// `tokens[depth]` isn't a member of the object at `object`.
    Missing {
        object: usize,
        depth: usize,
    },
    /// The path goes into a value that is neither an object nor an array.
    NotContainer(&'t str),
    OutOfRange {
        token: &'t str,
        len: usize,
    },
}

fn walk<'t>(doc: &[u8], mut at: usize, tokens: &'t [String]) -> Walk<'t> {
    for (depth, token) in tokens.iter().enumerate() {
        match doc[at] {
            b'{' => match Members::new(doc, at).filter(|m| key_is(doc, m, token)).last() {
                Some(m) => at = m.value.0,
                None => return Walk::Missing { object: at, depth },
            },
            b'[' => {
                let index = token.parse::<usize>().ok();
                let mut len = 0;
                let mut found = None;
                for m in Members::new(doc, at) {
                    if Some(len) == index {
                        found = Some(m.value.0);
                        break;
                    }
                    len += 1;
                }
                match found {
                    Some(start) => at = start,
                    None => return Walk::OutOfRange { token, len },
                }
            }
            _ => return Walk::NotContainer(token),
        }
    }
    Walk::Found(at)
}

/// A member of an object (`"key": value`) or an item of an array.
#[derive(Debug, Clone, Copy)]
struct Member {
    /// Where the key (or the item) starts.
    start: usize,
    /// The key string, quotes included.
    key: Option<(usize, usize)>,
    value: (usize, usize),
}

/// The members of the container opening at `open`, in order. Once it is
/// exhausted, `close` is the closing bracket's offset.
struct Members<'a> {
    doc: &'a [u8],
    at: usize,
    object: bool,
    close: Option<usize>,
}

impl<'a> Members<'a> {
    fn new(doc: &'a [u8], open: usize) -> Members<'a> {
        Members {
            doc,
            at: ws(doc, open + 1),
            object: doc[open] == b'{',
            close: None,
        }
    }
}

impl Iterator for Members<'_> {
    type Item = Member;

    fn next(&mut self) -> Option<Member> {
        if self.close.is_some() {
            return None;
        }
        let doc = self.doc;
        let start = self.at;
        if matches!(doc[start], b'}' | b']') {
            self.close = Some(start);
            return None;
        }
        let (key, value_start) = if self.object {
            let key_end = string_end(doc, start);
            // Past the `:`.
            (Some((start, key_end)), ws(doc, ws(doc, key_end) + 1))
        } else {
            (None, start)
        };
        let value_end = value_end(doc, value_start);
        let after = ws(doc, value_end);
        self.at = if doc[after] == b',' { ws(doc, after + 1) } else { after };
        Some(Member {
            start,
            key,
            value: (value_start, value_end),
        })
    }
}

/// Every member of the container at `open`, and its closing offset.
fn collect(doc: &[u8], open: usize) -> (Vec<Member>, usize) {
    let mut members = Members::new(doc, open);
    let list: Vec<Member> = members.by_ref().collect();
    (list, members.close.unwrap_or(doc.len()))
}

/// Inserts a member (`key` in an object, an item in an array) at the end
/// of the container at `open`, formatted like its siblings.
fn insert(doc: &[u8], open: usize, key: Option<&str>, value: &[u8]) -> Splice {
    let (members, close) = collect(doc, open);
    let key = key.map(key_text);
    let mut text = Vec::new();
    if let Some(last) = members.last() {
        // `,` then the whitespace and `:` the last member has.
        let mut gap = last.start;
        while gap > open + 1 && is_ws(doc[gap - 1]) {
            gap -= 1;
        }
        text.push(b',');
        text.extend_from_slice(&doc[gap..last.start]);
        if let (Some(key), Some((_, key_end))) = (&key, last.key) {
            text.extend_from_slice(key);
            text.extend_from_slice(&doc[key_end..last.value.0]);
        }
        text.extend_from_slice(value);
        return Splice {
            start: last.value.1,
            end: last.value.1,
            text,
        };
    }
    // An empty container: follow the document's style.
    let pretty = doc.contains(&b'\n');
    let indent = line_indent(doc, open);
    if pretty {
        text.push(b'\n');
        text.extend_from_slice(indent);
        text.extend_from_slice(Style::unit(doc));
    }
    if let Some(key) = &key {
        text.extend_from_slice(key);
        text.extend_from_slice(Style::colon(doc));
    }
    text.extend_from_slice(value);
    if pretty {
        text.push(b'\n');
        text.extend_from_slice(indent);
    }
    Splice {
        start: open + 1,
        end: close,
        text,
    }
}

/// A key as JSON text.
fn key_text(key: &str) -> Vec<u8> {
    serde_json::to_vec(key).expect("a string serialises")
}

fn key_is(doc: &[u8], member: &Member, token: &str) -> bool {
    let Some((start, end)) = member.key else {
        return false;
    };
    let inner = &doc[start + 1..end - 1];
    if !inner.contains(&b'\\') {
        return inner == token.as_bytes();
    }
    serde_json::from_slice::<String>(&doc[start..end]).is_ok_and(|k| k == token)
}

/// How the document is formatted, for inserts into empty containers.
struct Style;

impl Style {
    /// The text between the first key and its value (`:` or `: `).
    fn colon(doc: &[u8]) -> &[u8] {
        let mut i = 0;
        let mut string_end_at = None;
        while i < doc.len() {
            match doc[i] {
                b'"' => {
                    i = string_end(doc, i);
                    string_end_at = Some(i);
                    continue;
                }
                b':' => {
                    if let Some(from) = string_end_at {
                        return &doc[from..ws(doc, i + 1)];
                    }
                }
                _ => {}
            }
            i += 1;
        }
        b":"
    }

    /// One level of indentation: that of the first indented line.
    fn unit(doc: &[u8]) -> &[u8] {
        let mut lines = doc.split(|&b| b == b'\n').skip(1);
        lines
            .find_map(|line| {
                let n = line.iter().take_while(|&&b| b == b' ' || b == b'\t').count();
                (n > 0 && n < line.len()).then(|| &line[..n])
            })
            .unwrap_or(b"  ")
    }
}

/// The leading whitespace of the line holding `at`.
fn line_indent(doc: &[u8], at: usize) -> &[u8] {
    let line = doc[..at].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let n = doc[line..].iter().take_while(|&&b| b == b' ' || b == b'\t').count();
    &doc[line..line + n]
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

fn ws(doc: &[u8], mut i: usize) -> usize {
    while i < doc.len() && is_ws(doc[i]) {
        i += 1;
    }
    i
}

/// Past the closing quote of the string opening at `i`.
fn string_end(doc: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < doc.len() {
        match doc[i] {
            b'"' => return i + 1,
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    doc.len()
}

/// Past the end of the (valid) value starting at `i`.
fn value_end(doc: &[u8], i: usize) -> usize {
    match doc.get(i) {
        Some(b'"') => string_end(doc, i),
        Some(b'{' | b'[') => {
            let mut depth = 0usize;
            let mut j = i;
            while j < doc.len() {
                match doc[j] {
                    b'"' => {
                        j = string_end(doc, j);
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return j + 1;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            doc.len()
        }
        Some(_) => {
            let mut j = i;
            while j < doc.len() && !matches!(doc[j], b',' | b'}' | b']') && !is_ws(doc[j]) {
                j += 1;
            }
            j
        }
        None => i,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn set(doc: &str, pointer: &str, raw: &str) -> Result<String, String> {
        let mut bytes = doc.as_bytes().to_vec();
        apply(&mut bytes, &[Edit::Set { pointer, raw }]).remove(0)?;
        Ok(String::from_utf8(bytes).unwrap())
    }

    fn remove(doc: &str, pointer: &str) -> (bool, String) {
        let mut bytes = doc.as_bytes().to_vec();
        let changed = apply(&mut bytes, &[Edit::Remove { pointer }]).remove(0).unwrap();
        (changed, String::from_utf8(bytes).unwrap())
    }

    const PRETTY: &str =
        "{\n  \"z\": 1,\n  \"id\": 12345678901234567890,\n  \"price\": 1.50,\n  \"a\": {\n    \"b\": [1, 2]\n  }\n}\n";

    #[test]
    fn a_set_changes_only_its_value() {
        assert_eq!(
            set(PRETTY, "/price", "2.00").unwrap(),
            PRETTY.replace("1.50", "2.00"),
            "key order, big ids and formatting are kept"
        );
        assert_eq!(
            set(PRETTY, "/a/b/1", "\"x\"").unwrap(),
            PRETTY.replace("[1, 2]", "[1, \"x\"]")
        );
        assert_eq!(set("[1,2]", "", "{}").unwrap(), "{}");
        assert_eq!(set(" 1 ", "", "2").unwrap(), " 2 ");
    }

    #[test]
    fn inserts_copy_the_siblings_style() {
        assert_eq!(
            set(PRETTY, "/new", "true").unwrap(),
            PRETTY.replace("\n  }\n}", "\n  },\n  \"new\": true\n}")
        );
        assert_eq!(set(r#"{"a":1}"#, "/b", "2").unwrap(), r#"{"a":1,"b":2}"#);
        assert_eq!(set(r#"{"a" : 1}"#, "/b", "2").unwrap(), r#"{"a" : 1,"b" : 2}"#);
        assert_eq!(set("[1, 2]", "/-", "3").unwrap(), "[1, 2, 3]");
        assert_eq!(set("[1, 2]", "/2", "3").unwrap(), "[1, 2, 3]");
        assert_eq!(set("{\"a\":[]}", "/a/-", "1").unwrap(), "{\"a\":[1]}");
        assert_eq!(set("{}", "/k", "1").unwrap(), "{\"k\":1}");
        assert_eq!(
            set("{\n  \"a\": {}\n}", "/a/k", "1").unwrap(),
            "{\n  \"a\": {\n    \"k\": 1\n  }\n}"
        );
        assert_eq!(
            set(r#"{"a":1}"#, "/x/y/z", "[]").unwrap(),
            r#"{"a":1,"x":{"y":{"z":[]}}}"#,
            "missing parents are created"
        );
        assert_eq!(set("{}", "/a~1b/c~0d", "1").unwrap(), r#"{"a/b":{"c~d":1}}"#);
        assert_eq!(set("{}", "/q\"", "1").unwrap(), r#"{"q\"":1}"#);
    }

    #[test]
    fn removals_take_their_comma() {
        let (changed, out) = remove(PRETTY, "/id");
        assert!(changed);
        assert_eq!(out, PRETTY.replace("  \"id\": 12345678901234567890,\n", ""));
        assert_eq!(
            remove(PRETTY, "/a").1,
            PRETTY.replace(",\n  \"a\": {\n    \"b\": [1, 2]\n  }", "")
        );
        assert_eq!(remove("[1, 2, 3]", "/1").1, "[1, 3]");
        assert_eq!(remove("[1, 2, 3]", "/2").1, "[1, 2]");
        assert_eq!(remove("[1]", "/0").1, "[]");
        assert_eq!(remove("{\n  \"a\": 1\n}", "/a").1, "{}");
        assert_eq!(remove("{\"a\":1}", "/b"), (false, "{\"a\":1}".into()));
        assert_eq!(remove("{\"a\":1}", "/a/b"), (false, "{\"a\":1}".into()));
        assert_eq!(remove("{\"a\":1}", ""), (false, "{\"a\":1}".into()));
    }

    #[test]
    fn escapes_and_repeated_keys() {
        assert_eq!(
            set(r#"{"ab":1,"ab":2}"#, "/ab", "3").unwrap(),
            r#"{"ab":1,"ab":3}"#,
            "the last of a repeated key is the one read"
        );
        assert_eq!(
            set(r#"{"s":"a\"}b","t":1}"#, "/t", "2").unwrap(),
            r#"{"s":"a\"}b","t":2}"#
        );
        assert_eq!(remove(r#"{"k":1,"x":0,"k":2}"#, "/k").1, r#"{"x":0}"#);
    }

    #[test]
    fn errors_match_the_old_code() {
        assert!(set("not json", "/a", "1").is_err());
        assert!(set("{\"a\":1}", "/a/b", "1").is_err());
        assert!(set("[1]", "/5", "1").is_err());
        assert!(set("[1]", "/x", "1").is_err());
        assert!(set("[1]", "/3/a", "1").is_err());
        assert!(set("1", "/a", "1").is_err());
    }

    #[test]
    fn disjoint_edits_run_in_one_pass_and_others_in_order() {
        let mut doc = PRETTY.as_bytes().to_vec();
        let edits = [
            Edit::Set {
                pointer: "/z",
                raw: "2",
            },
            Edit::Remove { pointer: "/price" },
            Edit::Set {
                pointer: "/a/b/0",
                raw: "9",
            },
        ];
        assert!(apply_disjoint(&mut doc.clone(), &edits).is_some());
        let results = apply(&mut doc, &edits);
        assert_eq!(results, vec![Ok(true), Ok(true), Ok(true)]);
        assert_eq!(
            String::from_utf8(doc).unwrap(),
            "{\n  \"z\": 2,\n  \"id\": 12345678901234567890,\n  \"a\": {\n    \"b\": [9, 2]\n  }\n}\n"
        );
        // Removing an item shifts the next one: sequential.
        let edits = [
            Edit::Remove { pointer: "/0" },
            Edit::Set {
                pointer: "/1",
                raw: "0",
            },
        ];
        assert!(apply_disjoint(&mut b"[1,2,3]".to_vec(), &edits).is_none());
        let mut doc = b"[1,2,3]".to_vec();
        apply(&mut doc, &edits);
        assert_eq!(doc, b"[2,0]");
        // Two new keys in one object: sequential, so both get their comma.
        let mut doc = b"{}".to_vec();
        apply(
            &mut doc,
            &[
                Edit::Set {
                    pointer: "/a",
                    raw: "1",
                },
                Edit::Set {
                    pointer: "/b",
                    raw: "2",
                },
            ],
        );
        assert_eq!(doc, br#"{"a":1,"b":2}"#);
    }

    // ---- Differential test against the parse-and-reserialise code ----

    mod reference {
        use super::super::pointer_tokens;
        use serde_json::Value;

        pub fn json_set(doc: &mut Value, pointer: &str, value: Value) -> Result<(), String> {
            let tokens = pointer_tokens(pointer)?;
            let Some((last, parents)) = tokens.split_last() else {
                *doc = value;
                return Ok(());
            };
            let mut node = doc;
            for token in parents {
                node = match node {
                    Value::Object(map) => map
                        .entry(token.clone())
                        .or_insert_with(|| Value::Object(Default::default())),
                    Value::Array(items) => {
                        let i = token
                            .parse::<usize>()
                            .ok()
                            .filter(|&i| i < items.len())
                            .ok_or("index")?;
                        &mut items[i]
                    }
                    _ => return Err("not a container".into()),
                };
            }
            match node {
                Value::Object(map) => {
                    map.insert(last.clone(), value);
                }
                Value::Array(items) if last == "-" => items.push(value),
                Value::Array(items) => {
                    let i = last.parse::<usize>().map_err(|_| "index")?;
                    match i.cmp(&items.len()) {
                        std::cmp::Ordering::Less => items[i] = value,
                        std::cmp::Ordering::Equal => items.push(value),
                        std::cmp::Ordering::Greater => return Err("range".into()),
                    }
                }
                _ => return Err("parent".into()),
            }
            Ok(())
        }

        pub fn json_remove(doc: &mut Value, pointer: &str) -> bool {
            let Ok(tokens) = pointer_tokens(pointer) else {
                return false;
            };
            let Some((last, parents)) = tokens.split_last() else {
                return false;
            };
            let mut node = doc;
            for token in parents {
                node = match node {
                    Value::Object(map) => match map.get_mut(token) {
                        Some(next) => next,
                        None => return false,
                    },
                    Value::Array(items) => match token.parse::<usize>().ok().and_then(|i| items.get_mut(i)) {
                        Some(next) => next,
                        None => return false,
                    },
                    _ => return false,
                };
            }
            match node {
                Value::Object(map) => map.remove(last).is_some(),
                Value::Array(items) => match last.parse::<usize>() {
                    Ok(i) if i < items.len() => {
                        items.remove(i);
                        true
                    }
                    _ => false,
                },
                _ => false,
            }
        }
    }

    /// A tiny deterministic generator (xorshift), so failures reproduce.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    const KEYS: &[&str] = &["a", "b", "id", "x/y", "t~", "é", "q\"", "0", "-"];

    fn gen_value(rng: &mut Rng, depth: usize) -> Value {
        match rng.below(if depth > 3 { 4 } else { 7 }) {
            0 => Value::Null,
            1 => json!(rng.below(2) == 0),
            2 => json!(rng.next() as i64 % 100_000),
            3 => json!(KEYS[rng.below(KEYS.len())]),
            4 => json!(rng.below(1000) as f64 / 8.0),
            5 => Value::Array((0..rng.below(4)).map(|_| gen_value(rng, depth + 1)).collect()),
            _ => Value::Object(
                (0..rng.below(4))
                    .map(|_| (KEYS[rng.below(KEYS.len())].to_owned(), gen_value(rng, depth + 1)))
                    .collect(),
            ),
        }
    }

    fn escape(token: &str) -> String {
        token.replace('~', "~0").replace('/', "~1")
    }

    /// A pointer into `v` (sometimes past it, or nonsense).
    fn gen_pointer(rng: &mut Rng, v: &Value) -> String {
        let mut pointer = String::new();
        let mut node = Some(v);
        loop {
            if rng.below(3) == 0 {
                return pointer;
            }
            let token = match node {
                Some(Value::Object(map)) if !map.is_empty() && rng.below(4) != 0 => {
                    map.keys().nth(rng.below(map.len())).unwrap().clone()
                }
                Some(Value::Array(items)) if rng.below(5) != 0 => {
                    let i = rng.below(items.len() + 2);
                    if rng.below(6) == 0 { "-".into() } else { i.to_string() }
                }
                _ => KEYS[rng.below(KEYS.len())].to_owned(),
            };
            node = match node {
                Some(Value::Object(map)) => map.get(&token),
                Some(Value::Array(items)) => token.parse::<usize>().ok().and_then(|i| items.get(i)),
                _ => None,
            };
            pointer.push('/');
            pointer.push_str(&escape(&token));
        }
    }

    /// Serialises `v` in one of several styles, keeping its key order.
    fn render(v: &Value, rng: &mut Rng) -> String {
        match rng.below(3) {
            0 => serde_json::to_string(v).unwrap(),
            1 => serde_json::to_string_pretty(v).unwrap(),
            _ => {
                let mut buf = Vec::new();
                let fmt = serde_json::ser::PrettyFormatter::with_indent(b"\t");
                let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
                serde::Serialize::serialize(v, &mut ser).unwrap();
                format!("  {}\n", String::from_utf8(buf).unwrap())
            }
        }
    }

    #[test]
    fn differential_against_reparsing() {
        let mut rng = Rng(0x5eed_1234_abcd_ef01);
        for case in 0..4000 {
            let value = gen_value(&mut rng, 0);
            let text = render(&value, &mut rng);
            let n = 1 + rng.below(3);
            let mut edits = Vec::new();
            for _ in 0..n {
                let pointer = gen_pointer(&mut rng, &value);
                let raw = if rng.below(3) == 0 {
                    None
                } else {
                    Some(serde_json::to_string(&gen_value(&mut rng, 2)).unwrap())
                };
                edits.push((pointer, raw));
            }
            let mut expected = value.clone();
            let mut expected_results = Vec::new();
            for (pointer, raw) in &edits {
                expected_results.push(match raw {
                    Some(raw) => reference::json_set(&mut expected, pointer, serde_json::from_str(raw).unwrap())
                        .map(|_| ())
                        .is_ok(),
                    None => reference::json_remove(&mut expected, pointer),
                });
            }
            let mut doc = text.as_bytes().to_vec();
            let list: Vec<Edit<'_>> = edits
                .iter()
                .map(|(pointer, raw)| match raw {
                    Some(raw) => Edit::Set { pointer, raw },
                    None => Edit::Remove { pointer },
                })
                .collect();
            let results = apply(&mut doc, &list);
            let got: Value = serde_json::from_slice(&doc).unwrap_or_else(|e| {
                panic!(
                    "case {case}: invalid output {e}\n{text}\n{edits:?}\n{}",
                    String::from_utf8_lossy(&doc)
                )
            });
            assert_eq!(
                got,
                expected,
                "case {case}: {text}\n{edits:?}\n{}",
                String::from_utf8_lossy(&doc)
            );
            let ok: Vec<bool> = results
                .iter()
                .zip(&edits)
                .map(|(r, (_, raw))| match raw {
                    Some(_) => r.is_ok(),
                    None => *r.as_ref().unwrap(),
                })
                .collect();
            assert_eq!(ok, expected_results, "case {case}: {text}\n{edits:?}");
            // A lone removal of a missing value leaves every byte alone.
            if results.iter().all(|r| r == &Ok(false)) || results.iter().all(|r| r.is_err()) {
                assert_eq!(doc, text.as_bytes(), "case {case}");
            }
        }
    }

    #[test]
    fn untouched_bytes_stay_identical() {
        // A big id and a trailing-zero float sit next to the edit.
        let doc = r#"{"user":{"id":18446744073709551616123,"name":"Ana","score":1.50e+2},"zeta":0}"#;
        let out = set(doc, "/user/name", r#""Bea""#).unwrap();
        assert_eq!(out, doc.replace("Ana", "Bea"));
        let out = set(doc, "/user/tags", "[]").unwrap();
        assert_eq!(
            out,
            r#"{"user":{"id":18446744073709551616123,"name":"Ana","score":1.50e+2,"tags":[]},"zeta":0}"#
        );
    }
}
