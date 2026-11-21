//! The JSON tree's rows (`JsonViews.tsx`'s `flatten`, `treeMatches`,
//! `expandedTo`): a parsed document flattened into the rows that are
//! visible for a set of expanded containers (by JSON pointer), the filter's
//! path matches as roots, and the pointers the search terms hit.

use std::collections::HashSet;

use super::json_doc::JNode;
use super::json_path::{self, Seg, to_json_path, to_pointer};
use super::search::{MATCH_LIMIT, Search};

/// A shown value is cut here (the whole value is copied).
const SCALAR_MAX: usize = 2000;
/// Containers with more children aren't walked just to pre-expand them.
const EXPAND_WALK_MAX: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Label {
    /// The root.
    None,
    Key(String),
    Index(usize),
    /// A filter match at depth 0: its JSONPath.
    Match(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowValue {
    Object(usize),
    Array(usize),
    /// The value as shown (strings quoted, cut at 2,000 characters).
    String(String),
    Number(String),
    Literal(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub label: Label,
    pub depth: usize,
    pub path: Vec<Seg>,
    pub ptr: String,
    pub value: RowValue,
    pub expanded: bool,
}

impl TreeRow {
    pub fn is_container(&self) -> bool {
        matches!(self.value, RowValue::Object(_) | RowValue::Array(_))
    }

    /// A container with children: it has a caret.
    pub fn expandable(&self) -> bool {
        matches!(self.value, RowValue::Object(n) | RowValue::Array(n) if n > 0)
    }

    /// The label as shown (`"key"`, `0`, `$.a[0]`), empty for the root.
    pub fn label_text(&self) -> String {
        match &self.label {
            Label::None => String::new(),
            Label::Key(k) => super::json_doc::quote(k),
            Label::Index(i) => i.to_string(),
            Label::Match(p) => p.clone(),
        }
    }

    /// The value as shown: the scalar, `{…}`/`[…]` closed, `{`/`[` open.
    pub fn value_text(&self) -> String {
        match &self.value {
            RowValue::Object(0) => "{}".into(),
            RowValue::Array(0) => "[]".into(),
            RowValue::Object(_) if self.expanded => "{".into(),
            RowValue::Array(_) if self.expanded => "[".into(),
            RowValue::Object(_) => "{…}".into(),
            RowValue::Array(_) => "[…]".into(),
            RowValue::String(s) | RowValue::Number(s) | RowValue::Literal(s) => s.clone(),
        }
    }

    /// "3 keys", "1 item", or nothing for scalars and empty containers.
    pub fn size_text(&self) -> Option<String> {
        let (n, unit) = match self.value {
            RowValue::Object(n) => (n, "key"),
            RowValue::Array(n) => (n, "item"),
            _ => return None,
        };
        (n > 0).then(|| format!("{n} {unit}{}", if n == 1 { "" } else { "s" }))
    }

    pub fn json_path(&self) -> String {
        to_json_path(&self.path)
    }

    /// The pointer to copy: `/` for the root.
    pub fn pointer_text(&self) -> String {
        if self.ptr.is_empty() {
            "/".into()
        } else {
            self.ptr.clone()
        }
    }

    /// The key to copy, for a member of an object or array.
    pub fn key_text(&self) -> Option<String> {
        match &self.label {
            Label::Key(k) => Some(k.clone()),
            Label::Index(i) => Some(i.to_string()),
            Label::None | Label::Match(_) => None,
        }
    }
}

pub struct Root<'a> {
    pub label: Label,
    pub path: Vec<Seg>,
    pub value: &'a JNode,
}

/// The roots to show for a filter: the document, or each match of a
/// JSONPath or pointer. `Err` is the filter's error (the document is shown).
pub fn roots<'a>(doc: &'a JNode, filter: &str) -> (Vec<Root<'a>>, Result<Option<usize>, String>) {
    let whole = || {
        vec![Root {
            label: Label::None,
            path: Vec::new(),
            value: doc,
        }]
    };
    let f = filter.trim();
    if f.is_empty() || f == "$" {
        return (whole(), Ok(None));
    }
    match json_path::evaluate(doc, f) {
        Err(e) => (whole(), Err(e)),
        Ok(matches) => {
            let n = matches.len();
            let roots = matches
                .into_iter()
                .map(|m| Root {
                    label: Label::Match(to_json_path(&m.path)),
                    path: m.path,
                    value: m.value,
                })
                .collect();
            (roots, Ok(Some(n)))
        }
    }
}

fn row_value(node: &JNode) -> RowValue {
    match node {
        JNode::Obj(e) => RowValue::Object(e.len()),
        JNode::Arr(i) => RowValue::Array(i.len()),
        JNode::Str(_) => {
            let mut text = node.scalar_text();
            if text.len() > SCALAR_MAX {
                let mut cut = SCALAR_MAX;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
                text.push('…');
            }
            RowValue::String(text)
        }
        JNode::Num(n) => RowValue::Number(n.clone()),
        JNode::Bool(_) | JNode::Null => RowValue::Literal(node.scalar_text()),
    }
}

/// The visible rows.
pub fn flatten(roots: &[Root<'_>], expanded: &HashSet<String>) -> Vec<TreeRow> {
    fn visit(
        label: Label,
        node: &JNode,
        depth: usize,
        path: Vec<Seg>,
        expanded: &HashSet<String>,
        rows: &mut Vec<TreeRow>,
    ) {
        let ptr = to_pointer(&path);
        let open = node.is_container() && expanded.contains(&ptr);
        rows.push(TreeRow {
            label,
            depth,
            ptr,
            value: row_value(node),
            expanded: open,
            path: path.clone(),
        });
        if !open {
            return;
        }
        match node {
            JNode::Arr(items) => {
                for (i, v) in items.iter().enumerate() {
                    let mut p = path.clone();
                    p.push(Seg::Index(i));
                    visit(Label::Index(i), v, depth + 1, p, expanded, rows);
                }
            }
            JNode::Obj(entries) => {
                for (k, v) in entries {
                    let mut p = path.clone();
                    p.push(Seg::Key(k.clone()));
                    visit(Label::Key(k.clone()), v, depth + 1, p, expanded, rows);
                }
            }
            _ => {}
        }
    }
    let mut rows = Vec::new();
    for r in roots {
        visit(r.label.clone(), r.value, 0, r.path.clone(), expanded, &mut rows);
    }
    rows
}

/// Pointers of every container down to `depth` under `path` (the initial
/// expansion, and "Expand all below").
pub fn expanded_to(node: &JNode, depth: usize, path: &mut Vec<Seg>, out: &mut HashSet<String>) {
    if !node.is_container() {
        return;
    }
    out.insert(to_pointer(path));
    if depth == 0 || node.len() > EXPAND_WALK_MAX {
        return;
    }
    match node {
        JNode::Arr(items) => {
            for (i, v) in items.iter().enumerate() {
                path.push(Seg::Index(i));
                expanded_to(v, depth - 1, path, out);
                path.pop();
            }
        }
        JNode::Obj(entries) => {
            for (k, v) in entries {
                path.push(Seg::Key(k.clone()));
                expanded_to(v, depth - 1, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

/// The expansion a document opens with: the root and its children.
pub fn initial_expansion(doc: &JNode) -> HashSet<String> {
    let mut out = HashSet::new();
    expanded_to(doc, 1, &mut Vec::new(), &mut out);
    out
}

/// Pointers of the keys and scalar values `search` matches, in document
/// order, up to [`MATCH_LIMIT`].
pub fn tree_matches(doc: &JNode, search: &Search) -> Vec<String> {
    fn visit(node: &JNode, path: &mut Vec<Seg>, key: Option<&str>, search: &Search, out: &mut Vec<String>) {
        if out.len() >= MATCH_LIMIT {
            return;
        }
        let scalar_hit = !node.is_container() && search.is_match(&node.value_text());
        if key.is_some_and(|k| search.is_match(k)) || scalar_hit {
            out.push(to_pointer(path));
        }
        match node {
            JNode::Arr(items) => {
                for (i, v) in items.iter().enumerate() {
                    path.push(Seg::Index(i));
                    visit(v, path, None, search, out);
                    path.pop();
                }
            }
            JNode::Obj(entries) => {
                for (k, v) in entries {
                    path.push(Seg::Key(k.clone()));
                    visit(v, path, Some(k), search, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    visit(doc, &mut Vec::new(), None, search, &mut out);
    out
}

/// The containers above a pointer: `/a/0/b` → "", `/a`, `/a/0`.
pub fn ancestors(ptr: &str) -> Vec<String> {
    let parts: Vec<&str> = ptr.split('/').skip(1).collect();
    (0..parts.len())
        .map(|i| {
            if i == 0 {
                String::new()
            } else {
                format!("/{}", parts[..i].join("/"))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::json_doc::parse;
    use super::*;

    #[test]
    fn flattens_what_is_expanded() {
        let doc = parse(r#"{"a":[1,{"b":"x"}],"c":{},"d":null}"#).unwrap();
        let (roots, filter) = roots(&doc, "");
        assert_eq!(filter, Ok(None));
        let expanded = initial_expansion(&doc);
        let rows = flatten(&roots, &expanded);
        let shown: Vec<(String, String, usize)> =
            rows.iter().map(|r| (r.label_text(), r.value_text(), r.depth)).collect();
        assert_eq!(
            shown,
            [
                ("".into(), "{".into(), 0),
                ("\"a\"".into(), "[".into(), 1),
                ("0".into(), "1".into(), 2),
                ("1".into(), "{…}".into(), 2),
                ("\"c\"".into(), "{}".into(), 1),
                ("\"d\"".into(), "null".into(), 1),
            ]
        );
        assert_eq!(rows[1].size_text().as_deref(), Some("2 items"));
        assert_eq!(rows[3].ptr, "/a/1");
        assert_eq!(rows[3].json_path(), "$.a[1]");
        assert_eq!(rows[0].pointer_text(), "/");
        assert!(!rows[4].expandable());
    }

    #[test]
    fn filter_matches_become_roots() {
        let doc = parse(r#"{"items":[{"id":1},{"id":2}]}"#).unwrap();
        let (roots, filter) = roots(&doc, "$..id");
        assert_eq!(filter, Ok(Some(2)));
        let rows = flatten(&roots, &HashSet::new());
        assert_eq!(rows[0].label_text(), "$.items[0].id");
        assert_eq!(rows[1].value_text(), "2");
        let (roots, filter) = roots_err(&doc);
        assert!(filter.is_err());
        assert_eq!(roots, 1);
    }

    fn roots_err(doc: &JNode) -> (usize, Result<Option<usize>, String>) {
        let (r, f) = roots(doc, "items");
        (r.len(), f)
    }

    #[test]
    fn search_hits_keys_and_values_in_document_order() {
        let doc = parse(r#"{"user":{"name":"Ada","id":7},"tags":["admin","USER"]}"#).unwrap();
        let search = Search::new(&["user"]).unwrap();
        assert_eq!(tree_matches(&doc, &search), ["/user", "/tags/1"]);
        assert_eq!(ancestors("/tags/1"), ["", "/tags"]);
    }

    #[test]
    fn expand_all_below_is_bounded() {
        let items = (0..600).map(|i| format!("[{i}]")).collect::<Vec<_>>().join(",");
        let doc = parse(&format!("{{\"big\":[{items}]}}")).unwrap();
        let mut out = HashSet::new();
        expanded_to(&doc, 64, &mut Vec::new(), &mut out);
        assert!(out.contains("") && out.contains("/big"));
        assert!(!out.contains("/big/0"), "600 children aren't walked");
    }
}
