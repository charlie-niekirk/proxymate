//! The JSON editor's logic (`JsonEditor.tsx` and `useUndoable.ts`), with no
//! gpui: the visible rows, expansion, the selection, the inline edit, the
//! edits themselves (add, duplicate, remove, rename, change type, reset),
//! undo and redo, the keys and the row menu as data. `pm_widgets`'s
//! JsonEditor draws it.
//!
//! Keys: ↑↓ move, ←→ collapse and expand, ↩ edit the value, F2 rename,
//! ⌘↩ add after, ⌘D duplicate, ⌫ remove, ⌘Z / ⇧⌘Z (⌘Y) undo and redo,
//! Escape cancels an edit.

use std::collections::HashSet;
use std::sync::Arc;

use super::doc::{
    self, At, DocIndex, Id, JType, Node, ROOT_ID, RowStatus, blank, convert, index_doc, is_json_number, node_at,
    remove, removed_count, rename, set_value, status_of,
};

/// Undo steps kept (the plan's 200).
pub const UNDO_LIMIT: usize = 200;
/// Containers with more children than this aren't opened by the first
/// expansion.
const EXPAND_MAX_CHILDREN: usize = 500;
/// How deep "Expand all" goes.
const EXPAND_ALL_DEPTH: usize = 64;

/// A value with an undo history (`useUndoable.ts`).
#[derive(Clone, Debug)]
pub struct History<T> {
    past: Vec<T>,
    present: T,
    future: Vec<T>,
    limit: usize,
}

impl<T: Clone> History<T> {
    pub fn new(value: T) -> Self {
        History {
            past: Vec::new(),
            present: value,
            future: Vec::new(),
            limit: UNDO_LIMIT,
        }
    }

    pub fn value(&self) -> &T {
        &self.present
    }

    /// A new value; it becomes one undo step.
    pub fn set(&mut self, next: T) {
        self.past.push(std::mem::replace(&mut self.present, next));
        if self.past.len() > self.limit {
            self.past.remove(0);
        }
        self.future.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.past.pop() else { return false };
        self.future.insert(0, std::mem::replace(&mut self.present, prev));
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.future.is_empty() {
            return false;
        }
        let next = self.future.remove(0);
        self.past.push(std::mem::replace(&mut self.present, next));
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Starts over from `value`, forgetting the history.
    pub fn reset(&mut self, value: T) {
        *self = History::new(value);
    }
}

/// What a row is called: the root has nothing, a member its key, an item
/// its index.
#[derive(Clone, Debug, PartialEq)]
pub enum Label {
    Root,
    Key(String),
    Index(usize),
}

#[derive(Clone, Debug)]
pub struct Row {
    pub path: Vec<Id>,
    pub id: Id,
    pub depth: usize,
    pub label: Label,
    pub node: Arc<Node>,
    pub status: RowStatus,
    /// Original children no longer there.
    pub removed: usize,
    pub expanded: bool,
}

impl Row {
    /// A container with children (it shows a caret).
    pub fn expandable(&self) -> bool {
        self.node.is_container() && !self.node.children().is_empty()
    }

    /// The value as the row shows it when not editing.
    pub fn value_text(&self) -> String {
        match &*self.node {
            Node::Obj(m) | Node::Arr(m) => {
                let (open, close) = if matches!(*self.node, Node::Arr(_)) {
                    ("[", "]")
                } else {
                    ("{", "}")
                };
                if m.is_empty() {
                    format!("{open}{close}")
                } else if self.expanded {
                    open.to_owned()
                } else {
                    format!("{open}…{close}")
                }
            }
            Node::Str(s) => doc_quote(s),
            Node::Num(n) => n.clone(),
            Node::Bool(b) => b.to_string(),
            Node::Null => "null".into(),
        }
    }

    /// "3 keys", "1 item" after a non-empty container.
    pub fn size_text(&self) -> Option<String> {
        let n = self.node.children().len();
        if !self.node.is_container() || n == 0 {
            return None;
        }
        let unit = if matches!(*self.node, Node::Arr(_)) { "item" } else { "key" };
        Some(format!("{n} {unit}{}", if n == 1 { "" } else { "s" }))
    }

    /// The label as the row shows it (keys quoted, indexes bare).
    pub fn label_text(&self) -> Option<String> {
        match &self.label {
            Label::Root => None,
            Label::Key(k) => Some(doc_quote(k)),
            Label::Index(i) => Some(i.to_string()),
        }
    }

    /// For assistive tools: "name: \"Ana\", changed".
    pub fn accessible_label(&self) -> String {
        let mut out = match self.label_text() {
            Some(l) => format!("{l}: {}", self.value_text()),
            None => format!("root {}", self.value_text()),
        };
        if let Some(size) = self.size_text() {
            out.push_str(&format!(", {size}"));
        }
        if self.status != RowStatus::Same {
            out.push_str(&format!(", {}", self.status.id()));
        }
        if self.removed > 0 {
            out.push_str(&format!(", {} removed", self.removed));
        }
        out
    }
}

fn doc_quote(s: &str) -> String {
    crate::body::json_doc::quote(s)
}

/// The original a document is compared with.
#[derive(Clone, Debug)]
pub struct Reference {
    pub root: Arc<Node>,
    pub index: DocIndex,
}

impl Reference {
    pub fn new(root: Arc<Node>) -> Self {
        let index = index_doc(&root);
        Reference { root, index }
    }
}

/// The rows shown: the root, then the children of every expanded container.
pub fn flatten(root: &Arc<Node>, expanded: &HashSet<Id>, original: Option<&Reference>) -> Vec<Row> {
    #[allow(clippy::too_many_arguments)]
    fn visit(
        node: &Arc<Node>,
        path: &mut Vec<Id>,
        id: Id,
        label: Label,
        status: RowStatus,
        depth: usize,
        expanded: &HashSet<Id>,
        original: Option<&Reference>,
        rows: &mut Vec<Row>,
    ) {
        let open = node.is_container() && expanded.contains(&id);
        rows.push(Row {
            path: path.clone(),
            id,
            depth,
            label,
            node: node.clone(),
            status,
            removed: match original {
                Some(r) if node.is_container() => removed_count(node, id, &r.index, &r.root),
                _ => 0,
            },
            expanded: open,
        });
        if !open {
            return;
        }
        for (i, c) in node.children().iter().enumerate() {
            let s = original.map_or(RowStatus::Same, |r| status_of(c, &r.index));
            let s = if s == RowStatus::Same && status == RowStatus::Added {
                RowStatus::Added
            } else {
                s
            };
            let label = match &c.key {
                Some(k) => Label::Key(k.clone()),
                None => Label::Index(i),
            };
            path.push(c.id);
            visit(&c.v, path, c.id, label, s, depth + 1, expanded, original, rows);
            path.pop();
        }
    }
    let root_status = match original {
        Some(r) if r.root.t() != root.t() => RowStatus::Changed,
        _ => RowStatus::Same,
    };
    let mut rows = Vec::new();
    visit(
        root,
        &mut Vec::new(),
        ROOT_ID,
        Label::Root,
        root_status,
        0,
        expanded,
        original,
        &mut rows,
    );
    rows
}

/// Ids of the containers down to `depth`, for the first expansion.
pub fn expanded_to(node: &Node, depth: usize) -> HashSet<Id> {
    fn walk(node: &Node, depth: isize, id: Id, out: &mut HashSet<Id>) {
        if !node.is_container() || depth < 0 {
            return;
        }
        out.insert(id);
        let kids = node.children();
        if kids.len() > EXPAND_MAX_CHILDREN {
            return;
        }
        for c in kids {
            walk(&c.v, depth - 1, c.id, out);
        }
    }
    let mut out = HashSet::new();
    walk(node, depth as isize, ROOT_ID, &mut out);
    out
}

/// What an inline edit changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Key,
    Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Editing {
    pub path: Vec<Id>,
    pub field: Field,
}

/// The editor's keys (outside an inline edit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Home,
    End,
    Left,
    Right,
    /// ↩: edit the value (toggle a boolean, open or close a container).
    Enter,
    /// F2: rename.
    Rename,
    /// ⌫ / Delete.
    Remove,
    Undo,
    Redo,
    /// ⌘D.
    Duplicate,
    /// ⌘↩.
    AddAfter,
}

/// The row menu, as data (the app builds its `Menu` from it).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuSpec {
    /// "Edit value ↩" (strings and numbers).
    pub edit_value: bool,
    /// "Rename F2" (members of an object).
    pub rename: bool,
    /// "Change type ▸", with this one disabled.
    pub current: JType,
    /// "Add key ▸" / "Add item ▸" (containers).
    pub add_child: Option<&'static str>,
    /// "Add after ▸", "Duplicate" and "Remove" (anything but the root).
    pub not_root: bool,
    /// "Reset to original".
    pub reset: bool,
    /// What "Copy value" copies.
    pub value: String,
    /// What "Copy JSON pointer" copies.
    pub pointer: String,
}

/// The editable document with its view state.
#[derive(Clone, Debug)]
pub struct JsonEditorModel {
    doc: History<Arc<Node>>,
    reference: Option<Reference>,
    expanded: HashSet<Id>,
    selected: Option<Vec<Id>>,
    editing: Option<Editing>,
    rows: Vec<Row>,
}

impl JsonEditorModel {
    /// `root` edited, compared with `original` (none shows no changes).
    pub fn new(root: Arc<Node>, original: Option<Arc<Node>>) -> Self {
        let mut model = JsonEditorModel {
            expanded: expanded_to(&root, 1),
            doc: History::new(root),
            reference: original.map(Reference::new),
            selected: Some(Vec::new()),
            editing: None,
            rows: Vec::new(),
        };
        model.refresh();
        model
    }

    /// A text body as a tree: its ids lined up with the original's where
    /// they match, so only real edits are tinted (`TreeBody`).
    pub fn for_text(text: &str, original: Option<&str>) -> Result<Self, String> {
        let root = doc::parse(text)?;
        let reference = original.and_then(|o| doc::parse(o).ok());
        let root = match &reference {
            Some(r) => doc::align(r, &root),
            None => root,
        };
        Ok(JsonEditorModel::new(root, reference))
    }

    pub fn root(&self) -> &Arc<Node> {
        self.doc.value()
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn reference(&self) -> Option<&Reference> {
        self.reference.as_ref()
    }

    pub fn can_undo(&self) -> bool {
        self.doc.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.doc.can_redo()
    }

    pub fn editing(&self) -> Option<&Editing> {
        self.editing.as_ref()
    }

    pub fn selected(&self) -> Option<&[Id]> {
        self.selected.as_deref()
    }

    /// The selected row's index.
    pub fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.rows.iter().position(|r| &r.path == selected)
    }

    /// The row being edited, and which field.
    pub fn editing_row(&self) -> Option<(usize, Field)> {
        let e = self.editing.as_ref()?;
        Some((self.rows.iter().position(|r| r.path == e.path)?, e.field))
    }

    /// The document as text: pretty if `pretty` (as the original was).
    pub fn text(&self, pretty: bool) -> String {
        self.root().to_json(if pretty { 2 } else { 0 })
    }

    fn refresh(&mut self) {
        self.rows = flatten(self.doc.value(), &self.expanded, self.reference.as_ref());
        // A selection that an undo removed falls back to the root.
        if let Some(sel) = &self.selected
            && !sel.is_empty()
            && node_at(self.doc.value(), sel).is_none()
        {
            self.selected = Some(Vec::new());
        }
        if let Some(e) = &self.editing
            && node_at(self.doc.value(), &e.path).is_none()
        {
            self.editing = None;
        }
    }

    fn change(&mut self, next: Arc<Node>) {
        if Arc::ptr_eq(&next, self.doc.value()) {
            return;
        }
        self.doc.set(next);
        self.refresh();
    }

    pub fn select(&mut self, path: Option<Vec<Id>>) {
        self.selected = path;
    }

    pub fn select_row(&mut self, index: usize) {
        if let Some(row) = self.rows.get(index) {
            self.selected = Some(row.path.clone());
        }
    }

    /// Opens (`Some(true)`), closes or toggles the container `id`.
    pub fn toggle(&mut self, id: Id, open: Option<bool>) {
        let open = open.unwrap_or(!self.expanded.contains(&id));
        if open {
            self.expanded.insert(id);
        } else {
            self.expanded.remove(&id);
        }
        self.refresh();
    }

    pub fn expand_all(&mut self) {
        self.expanded = expanded_to(self.root(), EXPAND_ALL_DEPTH);
        self.refresh();
    }

    pub fn collapse_all(&mut self) {
        self.expanded = HashSet::from([ROOT_ID]);
        self.refresh();
    }

    pub fn undo(&mut self) -> bool {
        self.editing = None;
        let done = self.doc.undo();
        self.refresh();
        done
    }

    pub fn redo(&mut self) -> bool {
        self.editing = None;
        let done = self.doc.redo();
        self.refresh();
        done
    }

    /// Replaces the document (and forgets the history), as a new body.
    pub fn reset_doc(&mut self, root: Arc<Node>) {
        self.expanded = expanded_to(&root, 1);
        self.doc.reset(root);
        self.selected = Some(Vec::new());
        self.editing = None;
        self.refresh();
    }

    // ---- Edits ----

    fn parent_is_object(&self, path: &[Id]) -> bool {
        path.split_last()
            .and_then(|(_, parent)| node_at(self.root(), parent))
            .is_some_and(|n| matches!(**n, Node::Obj(_)))
    }

    /// "Add after" (⌘↩): a blank value after the row, its key (in an
    /// object) or value opened for editing. On the root, adds a child.
    pub fn add_after(&mut self, index: usize, t: JType) {
        let Some(row) = self.rows.get(index).cloned() else { return };
        if row.path.is_empty() {
            return self.add_child(index, t);
        }
        let parent = &row.path[..row.path.len() - 1];
        let in_object = self.parent_is_object(&row.path);
        let (next, path) = doc::insert(self.root(), parent, At::After(row.id), blank(t), "key");
        self.change(next);
        self.selected = Some(path.clone());
        self.editing = Some(Editing {
            path,
            field: if in_object { Field::Key } else { Field::Value },
        });
    }

    /// "Add key / item": a blank value at the end of the container.
    pub fn add_child(&mut self, index: usize, t: JType) {
        let Some(row) = self.rows.get(index).cloned() else { return };
        if !row.node.is_container() {
            return;
        }
        let object = matches!(*row.node, Node::Obj(_));
        let (next, path) = doc::insert(self.root(), &row.path, At::End, blank(t), "key");
        self.change(next);
        self.toggle(row.id, Some(true));
        self.selected = Some(path.clone());
        self.editing = Some(Editing {
            path,
            field: if object { Field::Key } else { Field::Value },
        });
    }

    /// Removes the row (not the root); the selection moves to the next row
    /// at its level or above, else the one before, else the parent.
    pub fn remove_row(&mut self, index: usize) {
        let Some(row) = self.rows.get(index).cloned() else { return };
        if row.path.is_empty() {
            return;
        }
        let next = self.rows[index + 1..]
            .iter()
            .find(|r| r.depth <= row.depth)
            .or_else(|| index.checked_sub(1).and_then(|i| self.rows.get(i)))
            .map(|r| r.path.clone());
        let root = remove(self.root(), &row.path);
        self.change(root);
        self.selected = Some(match next {
            Some(p) if p != row.path => p,
            _ => row.path[..row.path.len() - 1].to_vec(),
        });
    }

    pub fn duplicate_row(&mut self, index: usize) {
        let Some(row) = self.rows.get(index).cloned() else { return };
        if row.path.is_empty() {
            return;
        }
        let (next, path) = doc::duplicate(self.root(), &row.path);
        self.change(next);
        self.selected = Some(path);
    }

    pub fn change_type(&mut self, index: usize, t: JType) {
        let Some(row) = self.rows.get(index).cloned() else { return };
        let converted = convert(&row.node, t);
        let container = converted.is_container();
        let root = set_value(self.root(), &row.path, converted);
        self.change(root);
        if container {
            self.toggle(row.id, Some(true));
        }
    }

    /// Whether "Reset to original" applies to the row.
    pub fn is_changed(&self, index: usize) -> bool {
        let (Some(row), Some(reference)) = (self.rows.get(index), &self.reference) else {
            return false;
        };
        row.status != RowStatus::Same
            || row.removed > 0
            || (row.path.is_empty() && !doc::same_json(self.root(), &reference.root))
    }

    /// "Reset to original": the original value and key; a row the original
    /// didn't have is removed.
    pub fn reset(&mut self, index: usize) {
        let Some(row) = self.rows.get(index).cloned() else { return };
        let Some(reference) = self.reference.clone() else { return };
        if row.path.is_empty() {
            return self.change(reference.root.clone());
        }
        let Some((_, was)) = reference.index.by_id.get(&row.id) else {
            return self.remove_row(index);
        };
        let mut next = set_value(self.root(), &row.path, was.v.clone());
        if let Some(key) = &was.key {
            next = rename(&next, &row.path, key);
        }
        self.change(next);
    }

    /// Starts an inline edit of the row's key or value. A boolean's value
    /// flips instead, a container opens or closes, and null has nothing to
    /// edit. Returns whether an inline edit started.
    pub fn start_edit(&mut self, index: usize, field: Field) -> bool {
        let Some(row) = self.rows.get(index).cloned() else { return false };
        if field == Field::Key && !matches!(row.label, Label::Key(_)) {
            return false;
        }
        if field == Field::Value {
            match &*row.node {
                Node::Bool(b) => {
                    let root = set_value(self.root(), &row.path, Arc::new(Node::Bool(!b)));
                    self.change(root);
                    return false;
                }
                n if n.is_container() => {
                    self.toggle(row.id, None);
                    return false;
                }
                Node::Null => return false,
                _ => {}
            }
        }
        self.selected = Some(row.path.clone());
        self.editing = Some(Editing { path: row.path, field });
        true
    }

    /// The text an inline edit starts from.
    pub fn edit_text(&self) -> Option<String> {
        let (i, field) = self.editing_row()?;
        let row = &self.rows[i];
        Some(match field {
            Field::Key => match &row.label {
                Label::Key(k) => k.clone(),
                _ => String::new(),
            },
            Field::Value => match &*row.node {
                Node::Str(s) => editable_string(s),
                Node::Num(n) => n.clone(),
                _ => String::new(),
            },
        })
    }

    /// Checks an inline edit's text without committing it.
    pub fn validate(&self, text: &str) -> Option<String> {
        let (i, field) = self.editing_row()?;
        (field == Field::Value && matches!(*self.rows[i].node, Node::Num(_)) && !is_json_number(text.trim()))
            .then(|| "Not a JSON number".to_owned())
    }

    /// Commits the inline edit; an error keeps it open.
    pub fn commit(&mut self, text: &str) -> Result<(), String> {
        let Some((i, field)) = self.editing_row() else {
            return Ok(());
        };
        let row = self.rows[i].clone();
        match field {
            Field::Key => {
                let parent = &row.path[..row.path.len() - 1];
                if let Some(Node::Obj(entries)) = node_at(self.root(), parent).map(|n| &**n)
                    && entries.iter().any(|e| e.key.as_deref() == Some(text) && e.id != row.id)
                {
                    return Err(format!("{} is already a key here", doc_quote(text)));
                }
                let root = rename(self.root(), &row.path, text);
                self.change(root);
            }
            Field::Value => match &*row.node {
                Node::Num(_) => {
                    if !is_json_number(text.trim()) {
                        return Err("Not a JSON number".into());
                    }
                    let root = set_value(self.root(), &row.path, Arc::new(Node::Num(text.trim().to_owned())));
                    self.change(root);
                }
                Node::Str(was) => {
                    let value = from_editable(was, text);
                    let root = set_value(self.root(), &row.path, Arc::new(Node::Str(value)));
                    self.change(root);
                }
                _ => {}
            },
        }
        self.editing = None;
        Ok(())
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }

    /// A key outside an inline edit; returns whether it was handled.
    pub fn key(&mut self, key: Key) -> bool {
        if self.editing.is_some() {
            return false;
        }
        let len = self.rows.len();
        if len == 0 {
            return false;
        }
        let at = self.selected_index();
        let index = at.unwrap_or(0);
        let row = self.rows[index].clone();
        let select = |m: &mut Self, i: usize| {
            m.selected = m.rows.get(i).map(|r| r.path.clone());
        };
        match key {
            Key::Undo => {
                self.undo();
            }
            Key::Redo => {
                self.redo();
            }
            Key::Duplicate => self.duplicate_row(index),
            Key::AddAfter => self.add_after(index, JType::Str),
            Key::Down => select(self, at.map_or(0, |i| (i + 1).min(len - 1))),
            Key::Up => select(self, at.map_or(0, |i| i.saturating_sub(1))),
            Key::Home => select(self, 0),
            Key::End => select(self, len - 1),
            Key::Right => {
                if row.expandable() && !row.expanded {
                    self.toggle(row.id, Some(true));
                } else if row.expanded {
                    select(self, (index + 1).min(len - 1));
                }
            }
            Key::Left => {
                if row.expanded {
                    self.toggle(row.id, Some(false));
                } else if !row.path.is_empty() {
                    self.selected = Some(row.path[..row.path.len() - 1].to_vec());
                }
            }
            Key::Enter => {
                self.start_edit(index, Field::Value);
            }
            Key::Rename => {
                self.start_edit(index, Field::Key);
            }
            Key::Remove => self.remove_row(index),
        }
        true
    }

    /// The row menu for `index` (the selection moves there first).
    pub fn menu(&self, index: usize) -> Option<MenuSpec> {
        let row = self.rows.get(index)?;
        Some(MenuSpec {
            edit_value: matches!(*row.node, Node::Str(_) | Node::Num(_)),
            rename: self.parent_is_object(&row.path),
            current: row.node.t(),
            add_child: match *row.node {
                Node::Obj(_) => Some("Add key"),
                Node::Arr(_) => Some("Add item"),
                _ => None,
            },
            not_root: !row.path.is_empty(),
            reset: self.is_changed(index),
            value: row.node.value_text(),
            pointer: {
                let p = doc::pointer_at(self.root(), &row.path);
                if p.is_empty() { "/".into() } else { p }
            },
        })
    }
}

/// Strings with line breaks or tabs are edited escaped, so they fit one line.
pub fn editable_string(v: &str) -> String {
    if v.contains(['\n', '\r', '\t']) {
        let quoted = doc_quote(v);
        quoted[1..quoted.len() - 1].to_owned()
    } else {
        v.to_owned()
    }
}

/// The edited text back as a string: unescaped if the original was edited
/// escaped (left as typed if it doesn't unescape).
pub fn from_editable(original: &str, text: &str) -> String {
    if !original.contains(['\n', '\r', '\t']) {
        return text.to_owned();
    }
    match crate::body::json_doc::parse(&format!("\"{text}\"")) {
        Ok(crate::body::json_doc::JNode::Str(s)) => s,
        _ => text.to_owned(),
    }
}

/// Whether a body's tree edits are written back pretty (it was multi-line).
pub fn is_pretty(original: &str) -> bool {
    original.trim().contains('\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(text: &str) -> JsonEditorModel {
        let root = doc::parse(text).unwrap();
        JsonEditorModel::new(root.clone(), Some(root))
    }

    fn labels(m: &JsonEditorModel) -> Vec<String> {
        m.rows()
            .iter()
            .map(|r| r.label_text().unwrap_or_else(|| "$".into()))
            .collect()
    }

    fn find(m: &JsonEditorModel, label: &str) -> usize {
        m.rows()
            .iter()
            .position(|r| r.label_text().as_deref() == Some(label))
            .unwrap()
    }

    #[test]
    fn opens_the_first_level_and_moves_with_the_arrows() {
        let mut m = model(r#"{"a":{"b":1},"c":[true]}"#);
        assert_eq!(labels(&m), ["$", "\"a\"", "\"b\"", "\"c\"", "0"]);
        assert_eq!(m.selected_index(), Some(0));
        m.key(Key::Down);
        assert_eq!(m.selected_index(), Some(1));
        m.key(Key::Right);
        assert_eq!(m.selected_index(), Some(2), "→ on an open container moves in");
        m.key(Key::Left);
        assert_eq!(m.selected_index(), Some(1), "← on a leaf goes to the parent");
        m.key(Key::Left);
        assert_eq!(labels(&m), ["$", "\"a\"", "\"c\"", "0"]);
        m.key(Key::Right);
        assert_eq!(labels(&m).len(), 5);
        assert_eq!(m.selected_index(), Some(1));
        m.key(Key::End);
        assert_eq!(m.selected_index(), Some(4));
        m.key(Key::Home);
        assert_eq!(m.selected_index(), Some(0));
        m.key(Key::Up);
        assert_eq!(m.selected_index(), Some(0));
    }

    #[test]
    fn edits_values_and_keys_inline_with_validation() {
        let mut m = model(r#"{"name":"Ana","n":1,"ok":false,"none":null}"#);
        let n = find(&m, "\"n\"");
        assert!(m.start_edit(n, Field::Value));
        assert_eq!(m.edit_text().as_deref(), Some("1"));
        assert_eq!(m.validate("1."), Some("Not a JSON number".into()));
        assert_eq!(m.commit("x"), Err("Not a JSON number".into()));
        assert!(m.editing().is_some(), "an error keeps the edit open");
        m.commit(" 1.50 ").unwrap();
        assert_eq!(m.text(false), r#"{"name":"Ana","n":1.50,"ok":false,"none":null}"#);

        let name = find(&m, "\"name\"");
        m.select_row(name);
        m.key(Key::Rename);
        assert_eq!(m.edit_text().as_deref(), Some("name"));
        assert_eq!(m.commit("n"), Err("\"n\" is already a key here".into()));
        m.commit("first").unwrap();
        assert_eq!(m.rows()[name].status, RowStatus::Renamed);

        let ok = find(&m, "\"ok\"");
        assert!(!m.start_edit(ok, Field::Value), "↩ flips a boolean");
        assert_eq!(m.rows()[ok].value_text(), "true");
        let none = find(&m, "\"none\"");
        assert!(!m.start_edit(none, Field::Value));
        assert!(!m.start_edit(0, Field::Key), "the root has no key");
    }

    #[test]
    fn strings_with_line_breaks_are_edited_escaped() {
        let mut m = model(r#"{"s":"a\nb"}"#);
        m.start_edit(1, Field::Value);
        assert_eq!(m.edit_text().as_deref(), Some("a\\nb"));
        m.commit("a\\nb\\tc").unwrap();
        assert_eq!(m.text(false), r#"{"s":"a\nb\tc"}"#);
        assert_eq!(from_editable("plain", "x\\ny"), "x\\ny");
        assert_eq!(from_editable("a\nb", "bad \\q"), "bad \\q");
    }

    #[test]
    fn adds_duplicates_and_removes_with_their_keys() {
        let mut m = model(r#"{"a":1,"b":[1]}"#);
        m.select_row(1);
        m.key(Key::AddAfter);
        assert_eq!(m.text(false), r#"{"a":1,"key":"","b":[1]}"#);
        assert_eq!(m.editing_row(), Some((2, Field::Key)), "a new member starts with its key");
        m.commit("x").unwrap();
        m.key(Key::Duplicate);
        assert_eq!(m.text(false), r#"{"a":1,"x":"","x_copy":"","b":[1]}"#);
        assert_eq!(m.selected_index(), Some(3));
        m.key(Key::Remove);
        assert_eq!(m.text(false), r#"{"a":1,"x":"","b":[1]}"#);
        assert_eq!(m.selected_index(), Some(3), "the next row at the same level");

        let b = find(&m, "\"b\"");
        m.add_child(b, JType::Num);
        assert_eq!(m.text(false), r#"{"a":1,"x":"","b":[1,0]}"#);
        assert_eq!(m.editing().map(|e| e.field), Some(Field::Value), "a new item starts with its value");
        m.cancel_edit();
        let last = m.rows().len() - 1;
        m.select_row(last);
        m.key(Key::Remove);
        assert_eq!(m.selected_index(), Some(last - 1), "the last row falls back to the one before");
        m.select_row(0);
        m.key(Key::Remove);
        assert_eq!(m.text(false), r#"{"a":1,"x":"","b":[1]}"#, "the root stays");
    }

    #[test]
    fn changes_type_and_opens_new_containers() {
        let mut m = model(r#"{"a":"[1]"}"#);
        m.change_type(1, JType::Arr);
        assert_eq!(m.text(false), r#"{"a":[]}"#);
        assert!(m.rows()[1].expanded);
        m.change_type(1, JType::Str);
        assert_eq!(m.text(false), r#"{"a":"[]"}"#);
        assert_eq!(m.rows()[1].status, RowStatus::Changed);
    }

    #[test]
    fn undo_and_redo_step_through_edits() {
        let mut m = model(r#"{"a":1}"#);
        assert!(!m.can_undo());
        m.select_row(1);
        m.key(Key::Duplicate);
        m.key(Key::Remove);
        assert_eq!(m.text(false), r#"{"a":1}"#);
        m.key(Key::Undo);
        assert_eq!(m.text(false), r#"{"a":1,"a_copy":1}"#);
        m.key(Key::Undo);
        assert_eq!(m.text(false), r#"{"a":1}"#);
        assert!(!m.can_undo());
        assert!(m.can_redo());
        m.key(Key::Redo);
        assert_eq!(m.text(false), r#"{"a":1,"a_copy":1}"#);
        // A selection the undo took away falls back to the root.
        m.select_row(2);
        m.key(Key::Undo);
        assert_eq!(m.selected_index(), Some(0));
    }

    #[test]
    fn history_keeps_its_limit() {
        let mut h = History::new(0);
        for i in 1..=(UNDO_LIMIT + 10) {
            h.set(i);
        }
        let mut undone = 0;
        while h.undo() {
            undone += 1;
        }
        assert_eq!(undone, UNDO_LIMIT);
        assert_eq!(*h.value(), 10);
    }

    #[test]
    fn tints_rows_and_resets_them() {
        let mut m = model(r#"{"a":1,"b":{"c":2,"d":3}}"#);
        m.expand_all();
        let c = find(&m, "\"c\"");
        m.start_edit(c, Field::Value);
        m.commit("5").unwrap();
        let d = find(&m, "\"d\"");
        m.select_row(d);
        m.key(Key::Remove);
        let b = find(&m, "\"b\"");
        assert_eq!(m.rows()[b].removed, 1);
        assert!(m.is_changed(b));
        assert!(m.menu(b).unwrap().reset);
        let c = find(&m, "\"c\"");
        assert_eq!(m.rows()[c].status, RowStatus::Changed);
        m.reset(c);
        assert_eq!(m.rows()[find(&m, "\"c\"")].status, RowStatus::Same);
        // An added row resets by going away; the root resets everything.
        m.add_after(0, JType::Null);
        m.cancel_edit();
        let added = find(&m, "\"key\"");
        assert_eq!(m.rows()[added].status, RowStatus::Added);
        m.reset(added);
        assert!(m.rows().iter().all(|r| r.label != Label::Key("key".into())));
        m.reset(0);
        assert_eq!(m.text(false), r#"{"a":1,"b":{"c":2,"d":3}}"#);
        assert!(!m.is_changed(0));
    }

    #[test]
    fn children_of_an_added_container_are_added() {
        let mut m = model(r#"{}"#);
        m.add_child(0, JType::Obj);
        m.commit("o").unwrap();
        m.add_child(1, JType::Num);
        let statuses: Vec<RowStatus> = m.rows().iter().map(|r| r.status).collect();
        assert_eq!(statuses, [RowStatus::Same, RowStatus::Added, RowStatus::Added]);
    }

    #[test]
    fn the_menu_offers_what_applies() {
        let m = model(r#"{"a":[1],"s":"x"}"#);
        let root = m.menu(0).unwrap();
        assert!(!root.not_root && !root.rename && !root.edit_value && !root.reset);
        assert_eq!(root.add_child, Some("Add key"));
        assert_eq!(root.pointer, "/");
        let a = m.menu(1).unwrap();
        assert_eq!(a.add_child, Some("Add item"));
        assert!(a.rename && a.not_root);
        assert_eq!(a.value, "[\n  1\n]");
        assert_eq!(a.current, JType::Arr);
        let s = m.menu(find(&m, "\"s\"")).unwrap();
        assert!(s.edit_value);
        assert_eq!(s.value, "x");
        assert_eq!(s.pointer, "/s");
    }

    #[test]
    fn a_text_body_lines_up_with_its_original() {
        let m = JsonEditorModel::for_text(r#"{"a":2,"b":1}"#, Some(r#"{"a":1,"b":1}"#)).unwrap();
        let statuses: Vec<RowStatus> = m.rows().iter().map(|r| r.status).collect();
        assert_eq!(statuses, [RowStatus::Same, RowStatus::Changed, RowStatus::Same]);
        assert!(JsonEditorModel::for_text("{", None).is_err());
        assert!(is_pretty("{\n}"));
        assert!(!is_pretty(" {} \n"));
    }

    #[test]
    fn rows_describe_themselves() {
        let mut m = model(r#"{"a":[1,2],"e":{}}"#);
        assert_eq!(m.rows()[1].value_text(), "[");
        assert_eq!(m.rows()[1].size_text().as_deref(), Some("2 items"));
        assert_eq!(m.rows()[2].label, Label::Index(0));
        m.toggle(m.rows()[1].id, Some(false));
        assert_eq!(m.rows()[1].value_text(), "[…]");
        assert_eq!(m.rows()[2].value_text(), "{}");
        assert_eq!(m.rows()[2].size_text(), None);
        assert_eq!(m.rows()[0].accessible_label(), "root {, 2 keys");
        m.toggle(m.rows()[1].id, Some(true));
        m.change_type(2, JType::Str);
        assert_eq!(m.rows()[2].accessible_label(), "0: \"1\", changed");
    }
}
