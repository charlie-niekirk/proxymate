//! An editable JSON document (`lib/json/doc.ts`), for the JSON editor (G8)
//! and the visual rewrite's diff (G9). Unlike `serde_json` values it keeps
//! what an edit must not lose:
//! - object members in their order, and repeated keys;
//! - every number's exact text (`1.50`, `12345678901234567890`);
//! - a stable id per member and item, so the expanded rows and the diff
//!   follow a member through renames and an item through array edits.
//!
//! Nodes are immutable behind `Arc`s: every edit returns a new root that
//! shares the untouched subtrees, which makes undo a stack of roots.
//! Reading goes through [`crate::body::json_doc`], the lossless parser.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::body::json_doc::{self, JNode, quote};

/// A member's or item's id. The root has [`ROOT_ID`].
pub type Id = u64;

/// The root's id; members and items get theirs from [`new_id`].
pub const ROOT_ID: Id = 0;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A fresh id, never handed out before in this process.
pub fn new_id() -> Id {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// A node's address: the ids of the members and items from the root down.
pub type Path = [Id];

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Obj(Vec<Member>),
    Arr(Vec<Member>),
    Str(String),
    /// The number as written.
    Num(String),
    Bool(bool),
    Null,
}

/// A member of an object (with its `key`) or an item of an array (none).
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub id: Id,
    pub key: Option<String>,
    pub v: Arc<Node>,
}

/// A node's type, as the editor's "Change type" and "Add" menus name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JType {
    Str,
    Num,
    Bool,
    Null,
    Obj,
    Arr,
}

impl JType {
    /// In the menus' order.
    pub const ALL: [JType; 6] = [JType::Str, JType::Num, JType::Bool, JType::Null, JType::Obj, JType::Arr];

    pub fn label(self) -> &'static str {
        match self {
            JType::Str => "String",
            JType::Num => "Number",
            JType::Bool => "Boolean",
            JType::Null => "Null",
            JType::Obj => "Object",
            JType::Arr => "Array",
        }
    }

    /// `doc.ts`'s short name (`str`, `num`, …), for ids and the dev remote.
    pub fn id(self) -> &'static str {
        match self {
            JType::Str => "str",
            JType::Num => "num",
            JType::Bool => "bool",
            JType::Null => "null",
            JType::Obj => "obj",
            JType::Arr => "arr",
        }
    }

    pub fn from_id(id: &str) -> Option<JType> {
        JType::ALL.into_iter().find(|t| t.id() == id)
    }
}

impl Node {
    pub fn t(&self) -> JType {
        match self {
            Node::Obj(_) => JType::Obj,
            Node::Arr(_) => JType::Arr,
            Node::Str(_) => JType::Str,
            Node::Num(_) => JType::Num,
            Node::Bool(_) => JType::Bool,
            Node::Null => JType::Null,
        }
    }

    pub fn is_container(&self) -> bool {
        matches!(self, Node::Obj(_) | Node::Arr(_))
    }

    /// The members of a container (none for a scalar).
    pub fn children(&self) -> &[Member] {
        match self {
            Node::Obj(m) | Node::Arr(m) => m,
            _ => &[],
        }
    }

    /// JSON text, numbers exactly as held: compact with `indent` 0, else
    /// members on their own lines like `JSON.stringify(v, null, indent)`.
    pub fn to_json(&self, indent: usize) -> String {
        let mut out = String::new();
        write(self, indent, 0, &mut out);
        out
    }

    /// What "Copy value" copies: a string's own text, else its JSON.
    pub fn value_text(&self) -> String {
        match self {
            Node::Str(s) => s.clone(),
            other => other.to_json(2),
        }
    }
}

fn write(node: &Node, indent: usize, depth: usize, out: &mut String) {
    let pad = |out: &mut String, depth: usize| {
        if indent > 0 {
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', indent * depth));
        }
    };
    match node {
        Node::Str(s) => out.push_str(&quote(s)),
        Node::Num(raw) => out.push_str(raw),
        Node::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Node::Null => out.push_str("null"),
        Node::Arr(m) | Node::Obj(m) if m.is_empty() => {
            out.push_str(if matches!(node, Node::Arr(_)) { "[]" } else { "{}" })
        }
        Node::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                pad(out, depth + 1);
                write(&item.v, indent, depth + 1, out);
            }
            pad(out, depth);
            out.push(']');
        }
        Node::Obj(entries) => {
            out.push('{');
            for (i, e) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                pad(out, depth + 1);
                out.push_str(&quote(e.key.as_deref().unwrap_or_default()));
                out.push_str(if indent > 0 { ": " } else { ":" });
                write(&e.v, indent, depth + 1, out);
            }
            pad(out, depth);
            out.push('}');
        }
    }
}

/// Parses JSON text into a document (fresh ids), or the parser's error.
pub fn parse(text: &str) -> Result<Arc<Node>, String> {
    json_doc::parse(text).map(|n| Arc::new(from_jnode(&n)))
}

/// A lossless reader's value as a document, with fresh ids.
pub fn from_jnode(node: &JNode) -> Node {
    match node {
        JNode::Null => Node::Null,
        JNode::Bool(b) => Node::Bool(*b),
        JNode::Num(n) => Node::Num(n.clone()),
        JNode::Str(s) => Node::Str(s.clone()),
        JNode::Arr(items) => Node::Arr(
            items
                .iter()
                .map(|v| Member {
                    id: new_id(),
                    key: None,
                    v: Arc::new(from_jnode(v)),
                })
                .collect(),
        ),
        JNode::Obj(entries) => Node::Obj(
            entries
                .iter()
                .map(|(k, v)| Member {
                    id: new_id(),
                    key: Some(k.clone()),
                    v: Arc::new(from_jnode(v)),
                })
                .collect(),
        ),
    }
}

/// True if both nodes hold the same JSON (member order and number text
/// count; ids don't).
pub fn same_json(a: &Node, b: &Node) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }
    match (a, b) {
        (Node::Str(x), Node::Str(y)) | (Node::Num(x), Node::Num(y)) => x == y,
        (Node::Bool(x), Node::Bool(y)) => x == y,
        (Node::Null, Node::Null) => true,
        (Node::Arr(x), Node::Arr(y)) | (Node::Obj(x), Node::Obj(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.key == q.key && same_json(&p.v, &q.v))
        }
        _ => false,
    }
}

/// Escapes one pointer token (RFC 6901).
pub fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// `/a/0/b` from tokens.
pub fn pointer_of<S: AsRef<str>>(tokens: &[S]) -> String {
    tokens
        .iter()
        .map(|t| format!("/{}", escape_token(t.as_ref())))
        .collect()
}

/// The JSON pointer of the node at `path` (`""` for the root).
pub fn pointer_at(root: &Node, path: &Path) -> String {
    let mut tokens = Vec::with_capacity(path.len());
    let mut node = root;
    for id in path {
        let kids = node.children();
        let Some((i, c)) = kids.iter().enumerate().find(|(_, c)| c.id == *id) else {
            break;
        };
        tokens.push(c.key.clone().unwrap_or_else(|| i.to_string()));
        node = &c.v;
    }
    pointer_of(&tokens)
}

// ---- Addressing ----

/// The node at `path`, or `None` if an id is gone.
pub fn node_at<'a>(root: &'a Arc<Node>, path: &Path) -> Option<&'a Arc<Node>> {
    let mut node = root;
    for id in path {
        node = &node.children().iter().find(|c| c.id == *id)?.v;
    }
    Some(node)
}

/// The member (with its key) or item at `path`; `None` for the root.
pub fn member_at<'a>(root: &'a Arc<Node>, path: &Path) -> Option<&'a Member> {
    let (last, parent) = path.split_last()?;
    node_at(root, parent)?.children().iter().find(|c| c.id == *last)
}

/// Applies `change` to the node at `path`, copying the way down; the rest
/// is shared.
pub fn update(root: &Arc<Node>, path: &Path, change: impl FnOnce(&Arc<Node>) -> Arc<Node>) -> Arc<Node> {
    let Some((id, rest)) = path.split_first() else {
        return change(root);
    };
    let (kids, object) = match &**root {
        Node::Obj(m) => (m, true),
        Node::Arr(m) => (m, false),
        _ => return root.clone(),
    };
    let Some(i) = kids.iter().position(|c| c.id == *id) else {
        return root.clone();
    };
    let mut next = kids.clone();
    next[i].v = update(&kids[i].v, rest, change);
    Arc::new(if object { Node::Obj(next) } else { Node::Arr(next) })
}

/// Replaces the children of the container at `path`.
fn with_children(root: &Arc<Node>, path: &Path, change: impl FnOnce(Vec<Member>, bool) -> Vec<Member>) -> Arc<Node> {
    update(root, path, |node| match &**node {
        Node::Obj(m) => Arc::new(Node::Obj(change(m.clone(), true))),
        Node::Arr(m) => Arc::new(Node::Arr(change(m.clone(), false))),
        _ => node.clone(),
    })
}

// ---- Edits ----

/// Sets the value at `path` (the root with `[]`).
pub fn set_value(root: &Arc<Node>, path: &Path, value: Arc<Node>) -> Arc<Node> {
    update(root, path, |_| value)
}

/// Renames the member at `path`; a no-op for array items.
pub fn rename(root: &Arc<Node>, path: &Path, key: &str) -> Arc<Node> {
    let Some((id, parent)) = path.split_last() else {
        return root.clone();
    };
    with_children(root, parent, |kids, object| {
        if !object {
            return kids;
        }
        kids.into_iter()
            .map(|c| {
                if c.id == *id {
                    Member {
                        key: Some(key.to_owned()),
                        ..c
                    }
                } else {
                    c
                }
            })
            .collect()
    })
}

pub fn remove(root: &Arc<Node>, path: &Path) -> Arc<Node> {
    let Some((id, parent)) = path.split_last() else {
        return root.clone();
    };
    with_children(root, parent, |kids, _| {
        kids.into_iter().filter(|c| c.id != *id).collect()
    })
}

/// A key not yet used in `obj`, based on `base`.
pub fn free_key(obj: &Node, base: &str) -> String {
    let used: HashSet<&str> = match obj {
        Node::Obj(m) => m.iter().filter_map(|c| c.key.as_deref()).collect(),
        _ => HashSet::new(),
    };
    if !used.contains(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base}{i}"))
        .find(|k| !used.contains(k.as_str()))
        .unwrap_or_default()
}

/// Where [`insert`] puts a new member or item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum At {
    Start,
    End,
    After(Id),
}

/// Inserts a new member or item into the container at `parent`. Returns
/// the new root and the new child's path.
pub fn insert(root: &Arc<Node>, parent: &Path, at: At, value: Arc<Node>, key: &str) -> (Arc<Node>, Vec<Id>) {
    let id = new_id();
    let key = match node_at(root, parent).map(|n| &**n) {
        Some(obj @ Node::Obj(_)) => Some(free_key(obj, key)),
        _ => None,
    };
    let child = Member { id, key, v: value };
    let next = with_children(root, parent, |mut kids, _| {
        let index = match at {
            At::End => kids.len(),
            At::Start => 0,
            At::After(after) => kids.iter().position(|c| c.id == after).map_or(0, |i| i + 1),
        };
        kids.insert(index, child);
        kids
    });
    let mut path = parent.to_vec();
    path.push(id);
    (next, path)
}

/// A deep copy with fresh ids: a new value, not the original's.
pub fn clone_fresh(node: &Arc<Node>) -> Arc<Node> {
    let fresh = |m: &[Member]| -> Vec<Member> {
        m.iter()
            .map(|c| Member {
                id: new_id(),
                key: c.key.clone(),
                v: clone_fresh(&c.v),
            })
            .collect()
    };
    match &**node {
        Node::Obj(m) => Arc::new(Node::Obj(fresh(m))),
        Node::Arr(m) => Arc::new(Node::Arr(fresh(m))),
        _ => node.clone(),
    }
}

/// Duplicates the member or item at `path` just after it; returns the
/// copy's path.
pub fn duplicate(root: &Arc<Node>, path: &Path) -> (Arc<Node>, Vec<Id>) {
    let Some(member) = member_at(root, path) else {
        return (root.clone(), path.to_vec());
    };
    let key = member.key.as_ref().map(|k| format!("{k}_copy")).unwrap_or_default();
    let copy = clone_fresh(&member.v);
    insert(root, &path[..path.len() - 1], At::After(member.id), copy, &key)
}

/// A value of type `t` made from `node`, keeping what carries over.
pub fn convert(node: &Arc<Node>, t: JType) -> Arc<Node> {
    if node.t() == t {
        return node.clone();
    }
    let text = match &**node {
        Node::Str(s) => s.clone(),
        Node::Num(n) => n.clone(),
        Node::Bool(b) => b.to_string(),
        _ => String::new(),
    };
    Arc::new(match t {
        JType::Str => Node::Str(match &**node {
            n if n.is_container() => n.to_json(0),
            Node::Null => String::new(),
            _ => text,
        }),
        JType::Num => Node::Num(if is_json_number(text.trim()) {
            text.trim().to_owned()
        } else if let Node::Bool(b) = &**node {
            if *b { "1" } else { "0" }.to_owned()
        } else {
            "0".to_owned()
        }),
        JType::Bool => Node::Bool(match &**node {
            Node::Str(s) => s == "true",
            Node::Num(n) => n.parse::<f64>().is_ok_and(|v| v != 0.),
            _ => false,
        }),
        JType::Null => Node::Null,
        JType::Obj => Node::Obj(match &**node {
            Node::Arr(items) => items
                .iter()
                .enumerate()
                .map(|(i, c)| Member {
                    id: c.id,
                    key: Some(i.to_string()),
                    v: c.v.clone(),
                })
                .collect(),
            _ => Vec::new(),
        }),
        JType::Arr => Node::Arr(match &**node {
            Node::Obj(entries) => entries
                .iter()
                .map(|c| Member {
                    id: c.id,
                    key: None,
                    v: c.v.clone(),
                })
                .collect(),
            _ => Vec::new(),
        }),
    })
}

/// A blank value of a type, for "Add".
pub fn blank(t: JType) -> Arc<Node> {
    Arc::new(match t {
        JType::Str => Node::Str(String::new()),
        JType::Num => Node::Num("0".into()),
        JType::Bool => Node::Bool(false),
        JType::Null => Node::Null,
        JType::Obj => Node::Obj(Vec::new()),
        JType::Arr => Node::Arr(Vec::new()),
    })
}

/// True if `text` is a JSON number literal.
pub fn is_json_number(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - start
    };
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(c) if c.is_ascii_digit() => {
            digits(&mut i);
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if digits(&mut i) == 0 {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == b.len()
}

// ---- Comparing with the original ----

/// Where each original member or item is: its parent's path and itself.
#[derive(Clone, Debug, Default)]
pub struct DocIndex {
    pub by_id: HashMap<Id, (Vec<Id>, Member)>,
}

pub fn index_doc(root: &Node) -> DocIndex {
    fn walk(node: &Node, path: &mut Vec<Id>, out: &mut HashMap<Id, (Vec<Id>, Member)>) {
        for c in node.children() {
            out.insert(c.id, (path.clone(), c.clone()));
            if c.v.is_container() {
                path.push(c.id);
                walk(&c.v, path, out);
                path.pop();
            }
        }
    }
    let mut by_id = HashMap::new();
    walk(root, &mut Vec::new(), &mut by_id);
    DocIndex { by_id }
}

/// How a row differs from the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowStatus {
    Same,
    Added,
    Changed,
    Renamed,
}

impl RowStatus {
    pub fn id(self) -> &'static str {
        match self {
            RowStatus::Same => "same",
            RowStatus::Added => "added",
            RowStatus::Changed => "changed",
            RowStatus::Renamed => "renamed",
        }
    }
}

/// How the member or item `member` differs from the original.
pub fn status_of(member: &Member, original: &DocIndex) -> RowStatus {
    let Some((_, was)) = original.by_id.get(&member.id) else {
        return RowStatus::Added;
    };
    let (node, old) = (&*member.v, &*was.v);
    if node.t() != old.t() || (!node.is_container() && !same_json(node, old)) {
        return RowStatus::Changed;
    }
    if member.key.is_some() && was.key.is_some() && member.key != was.key {
        return RowStatus::Renamed;
    }
    RowStatus::Same
}

/// How many of the original's children of `id` ([`ROOT_ID`] for the root)
/// are gone from `node`.
pub fn removed_count(node: &Node, id: Id, original: &DocIndex, original_root: &Node) -> usize {
    let before = if id == ROOT_ID {
        Some(original_root)
    } else {
        original.by_id.get(&id).map(|(_, m)| &*m.v)
    };
    let Some(before) = before.filter(|b| b.t() == node.t()) else {
        return 0;
    };
    let now: HashSet<Id> = node.children().iter().map(|c| c.id).collect();
    before.children().iter().filter(|c| !now.contains(&c.id)).count()
}

/// Gives `result` the ids of `original` where they line up: members by key
/// (the last of a repeated key, as JSON readers do), items by index. Used
/// to rebuild edits from a dry run, where everything left alone must look
/// untouched.
pub fn align(original: &Arc<Node>, result: &Arc<Node>) -> Arc<Node> {
    match (&**original, &**result) {
        (Node::Obj(before), Node::Obj(after)) => {
            let by_key: HashMap<&str, &Member> = before.iter().filter_map(|m| Some((m.key.as_deref()?, m))).collect();
            let mut used = HashSet::new();
            Arc::new(Node::Obj(
                after
                    .iter()
                    .map(|e| match e.key.as_deref().and_then(|k| by_key.get(k)) {
                        Some(m) if used.insert(m.id) => Member {
                            id: m.id,
                            key: e.key.clone(),
                            v: align(&m.v, &e.v),
                        },
                        _ => e.clone(),
                    })
                    .collect(),
            ))
        }
        (Node::Arr(before), Node::Arr(after)) => Arc::new(Node::Arr(
            after
                .iter()
                .enumerate()
                .map(|(i, item)| match before.get(i) {
                    Some(m) => Member {
                        id: m.id,
                        key: None,
                        v: align(&m.v, &item.v),
                    },
                    None => item.clone(),
                })
                .collect(),
        )),
        _ => result.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(n: &Node) -> &[Member] {
        match n {
            Node::Obj(m) => m,
            other => panic!("not an object: {other:?}"),
        }
    }

    fn key_id(n: &Node, key: &str) -> Id {
        entries(n).iter().find(|e| e.key.as_deref() == Some(key)).unwrap().id
    }

    fn keys(n: &Node) -> Vec<&str> {
        entries(n).iter().map(|e| e.key.as_deref().unwrap()).collect()
    }

    #[test]
    fn keeps_member_order_integer_like_keys_and_repeated_keys() {
        let doc = parse(r#"{"b":1,"2":2,"a":3,"1":4,"a":5}"#).unwrap();
        assert_eq!(keys(&doc), ["b", "2", "a", "1", "a"]);
        assert_eq!(doc.to_json(0), r#"{"b":1,"2":2,"a":3,"1":4,"a":5}"#);
    }

    #[test]
    fn keeps_number_text_exactly() {
        let text = "[12345678901234567890,1.50,-0,1e400,0.1]";
        assert_eq!(parse(text).unwrap().to_json(0), text);
    }

    #[test]
    fn gives_every_member_and_item_its_own_id() {
        let doc = parse(r#"{"a":[1,2],"b":{"c":null}}"#).unwrap();
        let ids: Vec<Id> = index_doc(&doc).by_id.keys().copied().collect();
        assert_eq!(ids.len(), 5);
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
    }

    #[test]
    fn pretty_prints_like_json_stringify() {
        let text = r#"{"a":[1,{"b":"x"}],"c":{},"d":[]}"#;
        assert_eq!(
            parse(text).unwrap().to_json(2),
            "{\n  \"a\": [\n    1,\n    {\n      \"b\": \"x\"\n    }\n  ],\n  \"c\": {},\n  \"d\": []\n}"
        );
    }

    fn base() -> Arc<Node> {
        parse(r#"{"name":"Ana","tags":["x","y"],"n":1}"#).unwrap()
    }

    #[test]
    fn edits_are_immutable_and_share_untouched_subtrees() {
        let base = base();
        let id = key_id(&base, "name");
        let next = set_value(&base, &[id], Arc::new(Node::Str("Bea".into())));
        assert_eq!(base.to_json(0), r#"{"name":"Ana","tags":["x","y"],"n":1}"#);
        assert_eq!(next.to_json(0), r#"{"name":"Bea","tags":["x","y"],"n":1}"#);
        assert!(Arc::ptr_eq(&entries(&next)[1].v, &entries(&base)[1].v));
    }

    #[test]
    fn rename_remove_insert_and_duplicate_keep_ids_stable() {
        let base = base();
        let name_id = key_id(&base, "name");
        let doc = rename(&base, &[name_id], "first");
        assert_eq!(key_id(&doc, "first"), name_id);
        let doc = remove(&doc, &[key_id(&doc, "n")]);
        let (added, _) = insert(&doc, &[], At::After(name_id), Arc::new(Node::Bool(true)), "flag");
        assert_eq!(added.to_json(0), r#"{"first":"Ana","flag":true,"tags":["x","y"]}"#);
        let tags_id = key_id(&added, "tags");
        let (copy, path) = duplicate(&added, &[tags_id]);
        assert_eq!(
            copy.to_json(0),
            r#"{"first":"Ana","flag":true,"tags":["x","y"],"tags_copy":["x","y"]}"#
        );
        // The copy's items are new values, not the original's.
        let original = index_doc(&base);
        let copied = node_at(&copy, &path).unwrap();
        assert!(copied.children().iter().all(|i| !original.by_id.contains_key(&i.id)));
    }

    #[test]
    fn inserts_pick_a_free_key() {
        let (one, _) = insert(&base(), &[], At::End, Arc::new(Node::Null), "name");
        assert_eq!(entries(&one).last().unwrap().key.as_deref(), Some("name2"));
        let (two, _) = insert(&one, &[], At::Start, Arc::new(Node::Null), "name");
        assert_eq!(keys(&two)[0], "name3");
    }

    #[test]
    fn convert_keeps_what_carries_over() {
        let s = |v: &str| Arc::new(Node::Str(v.into()));
        assert_eq!(*convert(&s("42"), JType::Num), Node::Num("42".into()));
        assert_eq!(*convert(&s("x"), JType::Num), Node::Num("0".into()));
        assert_eq!(
            *convert(&Arc::new(Node::Num("1.50".into())), JType::Str),
            Node::Str("1.50".into())
        );
        assert_eq!(*convert(&Arc::new(Node::Bool(true)), JType::Num), Node::Num("1".into()));
        assert_eq!(
            *convert(&Arc::new(Node::Num("0.0".into())), JType::Bool),
            Node::Bool(false)
        );
        assert_eq!(*convert(&s("true"), JType::Bool), Node::Bool(true));
        assert_eq!(
            convert(&parse("[1,2]").unwrap(), JType::Obj).to_json(0),
            r#"{"0":1,"1":2}"#
        );
        assert_eq!(
            convert(&parse(r#"{"a":1}"#).unwrap(), JType::Str).to_json(0),
            r#""{\"a\":1}""#
        );
        assert_eq!(convert(&parse(r#"{"a":1}"#).unwrap(), JType::Arr).to_json(0), "[1]");
    }

    #[test]
    fn validates_number_text_against_the_json_grammar() {
        for ok in ["0", "-1", "1.50", "1e400", "12345678901234567890", "-0.0E+3"] {
            assert!(is_json_number(ok), "{ok}");
        }
        for bad in ["01", "1.", ".5", "+1", "NaN", "1e", "0x10", ""] {
            assert!(!is_json_number(bad), "{bad}");
        }
    }

    #[test]
    fn marks_added_changed_and_renamed_rows_and_counts_removals() {
        let original = parse(r#"{"a":1,"b":2,"c":3}"#).unwrap();
        let index = index_doc(&original);
        let doc = set_value(&original, &[key_id(&original, "a")], Arc::new(Node::Num("1.0".into())));
        let doc = rename(&doc, &[key_id(&doc, "b")], "bb");
        let doc = remove(&doc, &[key_id(&doc, "c")]);
        let (doc, _) = insert(&doc, &[], At::End, Arc::new(Node::Null), "d");
        let statuses: Vec<RowStatus> = entries(&doc).iter().map(|e| status_of(e, &index)).collect();
        assert_eq!(statuses, [RowStatus::Changed, RowStatus::Renamed, RowStatus::Added]);
        assert_eq!(removed_count(&doc, ROOT_ID, &index, &original), 1);
    }

    #[test]
    fn align_gives_a_dry_runs_result_the_originals_ids() {
        let original = parse(r#"{"a":{"x":1},"items":[1,2],"gone":true}"#).unwrap();
        let result = align(&original, &parse(r#"{"a":{"x":2},"items":[1,2,3],"new":0}"#).unwrap());
        let index = index_doc(&original);
        assert_eq!(key_id(&result, "a"), key_id(&original, "a"));
        let statuses: Vec<RowStatus> = entries(&result).iter().map(|e| status_of(e, &index)).collect();
        assert_eq!(statuses, [RowStatus::Same, RowStatus::Same, RowStatus::Added]);
        assert!(same_json(
            &result,
            &parse(r#"{"a":{"x":2},"items":[1,2,3],"new":0}"#).unwrap()
        ));
    }

    #[test]
    fn pointers_escape_and_index_arrays() {
        let doc = parse(r#"{"a/b":[{"~":1}]}"#).unwrap();
        let a = &entries(&doc)[0];
        let item = &a.v.children()[0];
        let tilde = &item.v.children()[0];
        assert_eq!(pointer_at(&doc, &[a.id, item.id, tilde.id]), "/a~1b/0/~0");
        assert_eq!(pointer_at(&doc, &[]), "");
    }
}
