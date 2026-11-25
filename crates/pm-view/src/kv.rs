//! The name/value editor's logic (`Rewrite/KvEditor.tsx`, with
//! `READ_ONLY_HEADERS` of `lib/rewrite/diff.ts`): rows compared with the
//! original (added, changed), removed rows listed so they can be put back
//! in their place, the framing headers the proxy owns kept read-only, and
//! the names a row can't change. Headers, query parameters and form fields
//! all use it; G8's Breakpoint Editor and Compose edit headers with it.

use std::collections::HashMap;

use pm_model::Header;

use crate::json::doc::new_id;

/// A row of a name/value table (headers, query, form).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KvRow {
    pub id: u64,
    pub name: String,
    pub value: String,
}

impl KvRow {
    /// A new row with a fresh id.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        KvRow {
            id: new_id(),
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Headers the proxy owns: re-framing sets them.
pub const READ_ONLY_HEADERS: [&str; 3] = ["content-length", "content-encoding", "transfer-encoding"];

pub const FRAMING_NOTE: &str = "Set by the proxy when the body changes";
pub const RENAME_NOTE: &str = "Remove the row and add a new one to rename it";
pub const REPEATED_NOTE: &str = "Repeated: it can be removed, not edited";
pub const DUPLICATE_NOTE: &str = "This name is already used";

/// How a table treats its rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KvOptions {
    /// Names compare case-insensitively (headers).
    pub fold: bool,
    /// The framing headers can't be edited or removed (a rewrite).
    pub framing_locked: bool,
    /// A name the original repeats can be removed but not edited (query,
    /// form in a rewrite).
    pub repeated_read_only: bool,
    /// An original row keeps its name (a rewrite: renaming is a removal and
    /// an addition). Off, every name is editable (breakpoints, Compose).
    pub names_locked: bool,
}

/// How a row differs from the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KvStatus {
    Same,
    Added,
    Changed,
}

impl KvStatus {
    pub fn id(self) -> &'static str {
        match self {
            KvStatus::Same => "same",
            KvStatus::Added => "added",
            KvStatus::Changed => "changed",
        }
    }
}

/// One row as the table shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowView {
    pub id: u64,
    pub status: KvStatus,
    pub name_locked: bool,
    pub value_locked: bool,
    pub removable: bool,
    /// Why the row can't be edited, or what's wrong with it (its tooltip).
    pub note: Option<&'static str>,
    /// Its name is used by another row where names must be unique.
    pub duplicate: bool,
}

/// The rows and the original rows that are gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KvView {
    pub rows: Vec<RowView>,
    pub removed: Vec<KvRow>,
}

/// Whether `name` is a header the proxy sets itself.
pub fn is_framing(name: &str) -> bool {
    READ_ONLY_HEADERS.contains(&name.trim().to_ascii_lowercase().as_str())
}

pub fn view(rows: &[KvRow], original: &[KvRow], opts: KvOptions) -> KvView {
    let norm = |n: &str| if opts.fold { n.to_lowercase() } else { n.to_owned() };
    let before: HashMap<u64, &KvRow> = original.iter().map(|r| (r.id, r)).collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for r in original {
        *counts.entry(norm(&r.name)).or_default() += 1;
    }
    let mut names: HashMap<String, usize> = HashMap::new();
    for r in rows {
        *names.entry(norm(&r.name)).or_default() += 1;
    }
    let views = rows
        .iter()
        .map(|r| {
            let was = before.get(&r.id);
            let locked = opts.framing_locked && is_framing(&r.name);
            let repeated =
                opts.repeated_read_only && was.is_some_and(|w| counts.get(&norm(&w.name)).copied().unwrap_or(0) > 1);
            let status = match was {
                None => KvStatus::Added,
                Some(w) if w.name != r.name || w.value != r.value => KvStatus::Changed,
                Some(_) => KvStatus::Same,
            };
            let duplicate = opts.repeated_read_only
                && !repeated
                && !r.name.is_empty()
                && names.get(&norm(&r.name)).copied().unwrap_or(0) > 1;
            let note = if locked {
                Some(FRAMING_NOTE)
            } else if repeated {
                Some(REPEATED_NOTE)
            } else if duplicate {
                Some(DUPLICATE_NOTE)
            } else if opts.names_locked && was.is_some() {
                Some(RENAME_NOTE)
            } else {
                None
            };
            RowView {
                id: r.id,
                status,
                name_locked: locked || repeated || (opts.names_locked && was.is_some()),
                value_locked: locked || repeated,
                removable: !locked,
                note,
                duplicate,
            }
        })
        .collect();
    let ids: std::collections::HashSet<u64> = rows.iter().map(|r| r.id).collect();
    KvView {
        rows: views,
        removed: original.iter().filter(|r| !ids.contains(&r.id)).cloned().collect(),
    }
}

/// Puts the removed row `id` back in its original place among the rows
/// still there; new rows stay after the original ones.
pub fn put_back(rows: &[KvRow], original: &[KvRow], id: u64) -> Vec<KvRow> {
    let Some(row) = original.iter().find(|r| r.id == id) else {
        return rows.to_vec();
    };
    if rows.iter().any(|r| r.id == id) {
        return rows.to_vec();
    }
    let rank = |r: &KvRow| -> (usize, u64) {
        match original.iter().position(|o| o.id == r.id) {
            Some(i) => (i, 0),
            None => (original.len(), r.id),
        }
    };
    let mut out: Vec<KvRow> = rows.iter().cloned().chain(std::iter::once(row.clone())).collect();
    out.sort_by_key(rank);
    out
}

pub fn set_name(rows: &mut [KvRow], id: u64, name: &str) {
    if let Some(r) = rows.iter_mut().find(|r| r.id == id) {
        r.name = name.to_owned();
    }
}

pub fn set_value(rows: &mut [KvRow], id: u64, value: &str) {
    if let Some(r) = rows.iter_mut().find(|r| r.id == id) {
        r.value = value.to_owned();
    }
}

pub fn remove(rows: &mut Vec<KvRow>, id: u64) {
    rows.retain(|r| r.id != id);
}

/// Headers as rows with fresh ids.
pub fn from_headers(headers: &[Header]) -> Vec<KvRow> {
    headers
        .iter()
        .map(|h| KvRow::new(h.name.clone(), h.value.clone()))
        .collect()
}

/// Rows as headers (`cleanHeaders`): names trimmed, nameless rows dropped.
pub fn to_headers(rows: &[KvRow]) -> Vec<Header> {
    rows.iter()
        .filter(|r| !r.name.trim().is_empty())
        .map(|r| Header {
            name: r.name.trim().to_owned(),
            value: r.value.clone(),
        })
        .collect()
}

/// `cleanHeaders` over headers.
pub fn clean_headers(headers: &[Header]) -> Vec<Header> {
    headers
        .iter()
        .filter(|h| !h.name.trim().is_empty())
        .map(|h| Header {
            name: h.name.trim().to_owned(),
            value: h.value.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(pairs: &[(&str, &str)]) -> Vec<KvRow> {
        pairs.iter().map(|(n, v)| KvRow::new(*n, *v)).collect()
    }

    #[test]
    fn tints_added_and_changed_rows_and_lists_removed_ones() {
        let original = rows(&[("a", "1"), ("b", "2"), ("c", "3")]);
        let mut now = original.clone();
        set_value(&mut now, original[0].id, "9");
        remove(&mut now, original[1].id);
        now.push(KvRow::new("d", "4"));
        let v = view(&now, &original, KvOptions::default());
        let statuses: Vec<KvStatus> = v.rows.iter().map(|r| r.status).collect();
        assert_eq!(statuses, [KvStatus::Changed, KvStatus::Same, KvStatus::Added]);
        assert_eq!(v.removed, vec![original[1].clone()]);
    }

    #[test]
    fn puts_a_removed_row_back_in_its_place() {
        let original = rows(&[("a", "1"), ("b", "2"), ("c", "3")]);
        let mut now = original.clone();
        remove(&mut now, original[1].id);
        let added = KvRow::new("d", "4");
        now.insert(0, added.clone());
        let back = put_back(&now, &original, original[1].id);
        let names: Vec<&str> = back.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c", "d"]);
        assert_eq!(put_back(&back, &original, original[1].id), back, "already back");
    }

    #[test]
    fn keeps_the_framing_headers_read_only() {
        let original = rows(&[("Content-Length", "10"), ("x-a", "1")]);
        let opts = KvOptions {
            fold: true,
            framing_locked: true,
            names_locked: true,
            ..Default::default()
        };
        let v = view(&original, &original, opts);
        assert!(v.rows[0].name_locked && v.rows[0].value_locked && !v.rows[0].removable);
        assert_eq!(v.rows[0].note, Some(FRAMING_NOTE));
        assert!(v.rows[1].name_locked && !v.rows[1].value_locked && v.rows[1].removable);
        assert_eq!(v.rows[1].note, Some(RENAME_NOTE));
        // Without the locks (breakpoints, Compose) everything is editable.
        let free = view(&original, &original, KvOptions::default());
        assert!(
            free.rows
                .iter()
                .all(|r| !r.name_locked && !r.value_locked && r.removable)
        );
    }

    #[test]
    fn repeated_names_can_be_removed_not_edited_and_new_ones_must_be_unique() {
        let original = rows(&[("tag", "a"), ("tag", "b"), ("q", "1")]);
        let mut now = original.clone();
        now.push(KvRow::new("q", "2"));
        let opts = KvOptions {
            repeated_read_only: true,
            ..Default::default()
        };
        let v = view(&now, &original, opts);
        assert_eq!(v.rows[0].note, Some(REPEATED_NOTE));
        assert!(v.rows[0].value_locked && v.rows[0].removable);
        assert!(v.rows[2].duplicate && v.rows[3].duplicate);
        assert_eq!(v.rows[3].note, Some(DUPLICATE_NOTE));
    }

    #[test]
    fn cleans_headers() {
        let r = rows(&[(" ", "x"), (" x-a ", "1")]);
        assert_eq!(
            to_headers(&r),
            vec![Header {
                name: "x-a".into(),
                value: "1".into()
            }]
        );
        assert!(is_framing(" Transfer-Encoding"));
    }
}
