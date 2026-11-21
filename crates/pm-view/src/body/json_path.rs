//! Path filters for the JSON tree (`lib/body/jsonPath.ts`): an RFC 6901
//! pointer (`/items/0/name`) or a JSONPath subset (`$.items[*].name`,
//! `$..id`, `$['a b'][0:2]`), over [`JNode`]s.

use super::json_doc::JNode;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Seg {
    Key(String),
    Index(usize),
}

pub type Path = Vec<Seg>;

#[derive(Clone, Debug, PartialEq)]
pub struct PathMatch<'a> {
    pub path: Path,
    pub value: &'a JNode,
}

#[derive(Clone, Debug)]
enum Step {
    Key(String),
    Index(i64),
    Slice(Option<i64>, Option<i64>),
    Wildcard,
    Descend(Box<Step>),
}

/// The matches of `expr` in `root`, or why it can't be evaluated.
pub fn evaluate<'a>(root: &'a JNode, expr: &str) -> Result<Vec<PathMatch<'a>>, String> {
    let e = expr.trim();
    if e.is_empty() || e == "$" {
        return Ok(vec![PathMatch {
            path: Vec::new(),
            value: root,
        }]);
    }
    if e.starts_with('/') {
        return Ok(pointer(root, e).into_iter().collect());
    }
    if !e.starts_with('$') {
        return Err("Start with $ (JSONPath) or / (JSON pointer)".into());
    }
    let steps = parse_json_path(e)?;
    let mut current = vec![PathMatch {
        path: Vec::new(),
        value: root,
    }];
    for step in &steps {
        current = current.iter().flat_map(|m| apply(step, m)).collect();
    }
    Ok(current)
}

/// The node at a JSON pointer.
pub fn pointer<'a>(root: &'a JNode, expr: &str) -> Option<PathMatch<'a>> {
    let mut value = root;
    let mut path = Vec::new();
    if expr.is_empty() {
        return Some(PathMatch { path, value });
    }
    for raw in expr.strip_prefix('/')?.split('/') {
        let t = raw.replace("~1", "/").replace("~0", "~");
        match value {
            JNode::Arr(items) if is_index(&t) && t.parse::<usize>().is_ok_and(|i| i < items.len()) => {
                let i: usize = t.parse().ok()?;
                value = &items[i];
                path.push(Seg::Index(i));
            }
            JNode::Obj(entries) => {
                let (_, v) = entries.iter().rev().find(|(k, _)| *k == t)?;
                value = v;
                path.push(Seg::Key(t));
            }
            _ => return None,
        }
    }
    Some(PathMatch { path, value })
}

fn is_index(t: &str) -> bool {
    t == "0" || (!t.is_empty() && !t.starts_with('0') && t.bytes().all(|b| b.is_ascii_digit()))
}

fn parse_json_path(e: &str) -> Result<Vec<Step>, String> {
    let chars: Vec<char> = e.chars().collect();
    let mut i = 1;
    let mut steps = Vec::new();
    let fail = |msg: &str, i: usize| Err(format!("{msg} at position {i}"));
    let name = |i: &mut usize| -> Result<String, String> {
        let start = *i;
        let first = |c: char| c.is_ascii_alphabetic() || c == '_' || c == '$' || c as u32 >= 0xa0;
        if !chars.get(*i).is_some_and(|c| first(*c)) {
            return Err(format!("Expected a name at position {i}"));
        }
        while chars
            .get(*i)
            .is_some_and(|c| first(*c) || c.is_ascii_digit() || *c == '-')
        {
            *i += 1;
        }
        Ok(chars[start..*i].iter().collect())
    };
    let bracket = |i: &mut usize| -> Result<Step, String> {
        *i += 1; // [
        let close = chars[*i..]
            .iter()
            .position(|c| *c == ']')
            .map(|p| *i + p)
            .ok_or_else(|| format!("Unsupported bracket expression at position {i}"))?;
        let inner: String = chars[*i..close].iter().collect();
        let t = inner.trim();
        let quoted = |q: char| {
            t.len() >= 2 && t.starts_with(q) && t.ends_with(q) && {
                // The closing quote isn't escaped.
                let body = &t[1..t.len() - 1];
                !body.ends_with('\\') || body.ends_with("\\\\")
            }
        };
        let step = if t == "*" {
            Step::Wildcard
        } else if quoted('\'') || quoted('"') {
            let mut key = String::new();
            let mut escaped = false;
            for c in t[1..t.len() - 1].chars() {
                if escaped {
                    key.push(c);
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else {
                    key.push(c);
                }
            }
            Step::Key(key)
        } else if let Some((a, b)) = t.split_once(':') {
            let num = |s: &str| -> Result<Option<i64>, ()> {
                let s = s.trim();
                if s.is_empty() {
                    Ok(None)
                } else {
                    s.parse().map(Some).map_err(|_| ())
                }
            };
            match (num(a), num(b)) {
                (Ok(a), Ok(b)) => Step::Slice(a, b),
                _ => return Err(format!("Unsupported bracket expression at position {i}")),
            }
        } else if let Ok(n) = t.parse::<i64>() {
            Step::Index(n)
        } else {
            return Err(format!("Unsupported bracket expression at position {i}"));
        };
        *i = close + 1;
        Ok(step)
    };
    while i < chars.len() {
        if chars[i] == '.' && chars.get(i + 1) == Some(&'.') {
            i += 2;
            let then = match chars.get(i) {
                Some('[') => bracket(&mut i)?,
                Some('*') => {
                    i += 1;
                    Step::Wildcard
                }
                _ => Step::Key(name(&mut i)?),
            };
            steps.push(Step::Descend(Box::new(then)));
        } else if chars[i] == '.' {
            i += 1;
            if chars.get(i) == Some(&'*') {
                i += 1;
                steps.push(Step::Wildcard);
            } else {
                steps.push(Step::Key(name(&mut i)?));
            }
        } else if chars[i] == '[' {
            steps.push(bracket(&mut i)?);
        } else {
            return fail("Unexpected character", i);
        }
    }
    Ok(steps)
}

fn with(path: &Path, seg: Seg) -> Path {
    let mut p = path.clone();
    p.push(seg);
    p
}

fn children<'a>(m: &PathMatch<'a>) -> Vec<PathMatch<'a>> {
    match m.value {
        JNode::Arr(items) => items
            .iter()
            .enumerate()
            .map(|(i, value)| PathMatch {
                path: with(&m.path, Seg::Index(i)),
                value,
            })
            .collect(),
        JNode::Obj(entries) => entries
            .iter()
            .map(|(k, value)| PathMatch {
                path: with(&m.path, Seg::Key(k.clone())),
                value,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn apply<'a>(step: &Step, m: &PathMatch<'a>) -> Vec<PathMatch<'a>> {
    match step {
        Step::Key(key) => match m.value {
            JNode::Obj(entries) => entries
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, value)| PathMatch {
                    path: with(&m.path, Seg::Key(key.clone())),
                    value,
                })
                .into_iter()
                .collect(),
            _ => Vec::new(),
        },
        Step::Index(index) => match m.value {
            JNode::Arr(items) => {
                let len = items.len() as i64;
                let i = if *index < 0 { len + index } else { *index };
                if (0..len).contains(&i) {
                    vec![PathMatch {
                        path: with(&m.path, Seg::Index(i as usize)),
                        value: &items[i as usize],
                    }]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        },
        Step::Slice(start, end) => match m.value {
            JNode::Arr(items) => {
                let len = items.len() as i64;
                let norm = |n: Option<i64>, d: i64| match n {
                    None => d,
                    Some(n) if n < 0 => (len + n).max(0),
                    Some(n) => n.min(len),
                };
                (norm(*start, 0)..norm(*end, len))
                    .map(|i| PathMatch {
                        path: with(&m.path, Seg::Index(i as usize)),
                        value: &items[i as usize],
                    })
                    .collect()
            }
            _ => Vec::new(),
        },
        Step::Wildcard => children(m),
        Step::Descend(then) => {
            let mut out = Vec::new();
            fn visit<'a>(then: &Step, node: &PathMatch<'a>, out: &mut Vec<PathMatch<'a>>) {
                out.extend(apply(then, node));
                for child in children(node) {
                    visit(then, &child, out);
                }
            }
            visit(then, m, &mut out);
            out
        }
    }
}

/// `/items/0/na~1me`.
pub fn to_pointer(path: &[Seg]) -> String {
    path.iter()
        .map(|s| match s {
            Seg::Index(i) => format!("/{i}"),
            Seg::Key(k) => format!("/{}", k.replace('~', "~0").replace('/', "~1")),
        })
        .collect()
}

/// `$.items[0]['na/me']`.
pub fn to_json_path(path: &[Seg]) -> String {
    let mut out = String::from("$");
    for s in path {
        match s {
            Seg::Index(i) => out.push_str(&format!("[{i}]")),
            Seg::Key(k) => {
                let plain = k
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
                    && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
                if plain {
                    out.push('.');
                    out.push_str(k);
                } else {
                    out.push_str(&format!("['{}']", k.replace('\\', "\\\\").replace('\'', "\\'")));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::json_doc::parse;
    use super::*;

    fn doc() -> JNode {
        parse(
            r#"{"items":[{"id":1,"name":"a","tags":["x"]},{"id":2,"name":"b","owner":{"id":9}}],
               "a/b":{"~":true},"with space":3}"#,
        )
        .unwrap()
    }

    fn values(expr: &str) -> Vec<String> {
        let d = doc();
        evaluate(&d, expr).unwrap().iter().map(|m| m.value.to_json(0)).collect()
    }

    #[test]
    fn follows_json_pointers() {
        assert_eq!(values("/items/1/name"), ["\"b\""]);
        assert_eq!(values("/a~1b/~0"), ["true"]);
        assert!(values("/items/5").is_empty());
    }

    #[test]
    fn evaluates_a_json_path_subset() {
        assert_eq!(values("$.items[*].name"), ["\"a\"", "\"b\""]);
        assert_eq!(values("$..id"), ["1", "2", "9"]);
        assert_eq!(values("$.items[-1].id"), ["2"]);
        assert_eq!(values("$.items[0:1].id"), ["1"]);
        assert_eq!(values("$['with space']"), ["3"]);
        assert_eq!(values("$.items[0].*"), ["1", "\"a\"", "[\"x\"]"]);
        assert_eq!(values("$").len(), 1);
    }

    #[test]
    fn reports_errors() {
        let d = doc();
        assert!(evaluate(&d, "items").is_err());
        assert!(evaluate(&d, "$.items[?(@.id)]").is_err());
    }

    #[test]
    fn treats_numbers_as_leaves_and_keeps_their_text() {
        let big = parse(r#"{"items":[{"id":12345678901234567890},{"id":1.50}]}"#).unwrap();
        let found: Vec<(String, String)> = evaluate(&big, "$..id")
            .unwrap()
            .iter()
            .map(|m| (to_json_path(&m.path), m.value.to_json(0)))
            .collect();
        assert_eq!(
            found,
            [
                ("$.items[0].id".to_owned(), "12345678901234567890".to_owned()),
                ("$.items[1].id".to_owned(), "1.50".to_owned())
            ]
        );
        assert!(evaluate(&big, "$.items[0].id.length").unwrap().is_empty());
        assert!(evaluate(&big, "/items/0/id/0").unwrap().is_empty());
        assert!(evaluate(&big, "$.items[0].id.*").unwrap().is_empty());
    }

    #[test]
    fn builds_pointers_and_paths() {
        let path = [Seg::Key("items".into()), Seg::Index(0), Seg::Key("a/b".into())];
        assert_eq!(to_pointer(&path), "/items/0/a~1b");
        let path = [Seg::Key("items".into()), Seg::Index(0), Seg::Key("with space".into())];
        assert_eq!(to_json_path(&path), "$.items[0]['with space']");
    }
}
