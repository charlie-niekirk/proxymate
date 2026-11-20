//! The flow table's columns and the layout the user arranges, ported from
//! `FlowTable/columns.ts` and the column half of `store/prefs.ts`: the 14
//! built-in columns, up to [`MAX_HEADER_COLUMNS`] header columns, and the
//! widths, order and hidden set that persist in `ui-prefs.json`.
//!
//! Ids serialise as the Tauri UI spells them (`"url"`,
//! `"hdr:request:x-request-id"`), so the file reads the same way.

use std::collections::BTreeMap;
use std::fmt;

use pm_model::{BodyPart, HeaderColumn, MAX_HEADER_COLUMNS, SortKey, SortSpec};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const ROW_HEIGHT: f32 = 22.0;
pub const HEADER_HEIGHT: f32 = 22.0;
/// The widest a column can be dragged or keyed to.
pub const MAX_COLUMN_WIDTH: f32 = 1600.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    Id,
    State,
    Url,
    Client,
    Method,
    Status,
    Time,
    Duration,
    Req,
    Resp,
    Proto,
    Ssl,
    Notes,
    Tools,
}

impl Builtin {
    pub const ALL: [Builtin; 14] = [
        Builtin::Id,
        Builtin::State,
        Builtin::Url,
        Builtin::Client,
        Builtin::Method,
        Builtin::Status,
        Builtin::Time,
        Builtin::Duration,
        Builtin::Req,
        Builtin::Resp,
        Builtin::Proto,
        Builtin::Ssl,
        Builtin::Notes,
        Builtin::Tools,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Builtin::Id => "id",
            Builtin::State => "state",
            Builtin::Url => "url",
            Builtin::Client => "client",
            Builtin::Method => "method",
            Builtin::Status => "status",
            Builtin::Time => "time",
            Builtin::Duration => "duration",
            Builtin::Req => "req",
            Builtin::Resp => "resp",
            Builtin::Proto => "proto",
            Builtin::Ssl => "ssl",
            Builtin::Notes => "notes",
            Builtin::Tools => "tools",
        }
    }

    /// The column's definition.
    pub fn column(self) -> Column {
        let (title, label, width, min_width, align, sort_key) = match self {
            Builtin::Id => ("#", "Number", 64., 36., Align::Right, Some(SortKey::Id)),
            Builtin::State => ("", "State", 18., 18., Align::Center, None),
            Builtin::Url => ("URL", "URL", 240., 120., Align::Left, Some(SortKey::Url)),
            Builtin::Client => ("Client", "Client", 120., 48., Align::Left, None),
            Builtin::Method => ("Method", "Method", 72., 48., Align::Center, Some(SortKey::Method)),
            Builtin::Status => ("Status", "Status", 64., 48., Align::Center, Some(SortKey::Status)),
            Builtin::Time => ("Time", "Time", 92., 56., Align::Right, None),
            Builtin::Duration => ("Duration", "Duration", 72., 48., Align::Right, Some(SortKey::Duration)),
            Builtin::Req => (
                "Request",
                "Request size",
                72.,
                48.,
                Align::Right,
                Some(SortKey::ReqSize),
            ),
            Builtin::Resp => (
                "Response",
                "Response size",
                76.,
                48.,
                Align::Right,
                Some(SortKey::RespSize),
            ),
            Builtin::Proto => ("Proto", "Protocol", 42., 32., Align::Center, None),
            Builtin::Ssl => ("SSL", "SSL", 34., 28., Align::Center, None),
            Builtin::Notes => ("Notes", "Notes (label, comment, edited)", 54., 36., Align::Center, None),
            Builtin::Tools => ("", "Tools", 28., 28., Align::Center, None),
        };
        Column {
            id: ColumnId::Builtin(self),
            title: title.into(),
            label: label.into(),
            width,
            min_width,
            align,
            sort_key,
            grow: self == Builtin::Url,
            required: self == Builtin::Url,
            header: None,
        }
    }
}

/// A built-in column, or a header column (M17).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ColumnId {
    Builtin(Builtin),
    Header(HeaderColumn),
}

impl ColumnId {
    pub fn header(part: BodyPart, name: &str) -> ColumnId {
        ColumnId::Header(HeaderColumn {
            part,
            name: name.trim().to_ascii_lowercase(),
        })
    }

    pub fn is_header(&self) -> bool {
        matches!(self, ColumnId::Header(_))
    }

    pub fn parse(s: &str) -> Option<ColumnId> {
        if let Some(rest) = s.strip_prefix("hdr:") {
            let (part, name) = rest.split_once(':')?;
            let part = match part {
                "request" => BodyPart::Request,
                "response" => BodyPart::Response,
                _ => return None,
            };
            return (!name.is_empty()).then(|| ColumnId::header(part, name));
        }
        Builtin::ALL
            .into_iter()
            .find(|b| b.as_str() == s)
            .map(ColumnId::Builtin)
    }
}

impl fmt::Display for ColumnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColumnId::Builtin(b) => f.write_str(b.as_str()),
            ColumnId::Header(h) => write!(f, "hdr:{}:{}", part_str(h.part), h.name.to_ascii_lowercase()),
        }
    }
}

fn part_str(part: BodyPart) -> &'static str {
    match part {
        BodyPart::Request => "request",
        BodyPart::Response => "response",
    }
}

impl Serialize for ColumnId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ColumnId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        ColumnId::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("unknown column {s:?}")))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub id: ColumnId,
    /// Header text; empty for icon-only columns.
    pub title: String,
    /// What the column menu calls it.
    pub label: String,
    /// Default width in px; the URL column grows past it.
    pub width: f32,
    pub min_width: f32,
    pub align: Align,
    pub sort_key: Option<SortKey>,
    /// Takes the leftover width.
    pub grow: bool,
    /// Can't be hidden.
    pub required: bool,
    /// A header column: its index in the rows' `extra`.
    pub header: Option<usize>,
}

impl Column {
    /// Whether `sort` sorts by this column.
    pub fn sorted_by(&self, sort: &SortSpec) -> bool {
        self.sort_key == Some(sort.key) && (sort.key != SortKey::Extra || self.header == Some(sort.extra as usize))
    }

    /// The sort a click on this column's title asks for: ascending, or the
    /// other way round when it already sorts the table ascending.
    pub fn next_sort(&self, sort: &SortSpec) -> Option<SortSpec> {
        let key = self.sort_key?;
        let dir = if self.sorted_by(sort) && sort.dir == pm_model::SortDir::Asc {
            pm_model::SortDir::Desc
        } else {
            pm_model::SortDir::Asc
        };
        Some(SortSpec {
            key,
            dir,
            extra: self.header.unwrap_or(0) as u8,
        })
    }
}

/// The column for header column number `index`.
pub fn header_column_def(c: &HeaderColumn, index: usize) -> Column {
    Column {
        id: ColumnId::Header(c.clone()),
        title: c.name.clone(),
        label: format!(
            "{} header {}",
            match c.part {
                BodyPart::Request => "Request",
                BodyPart::Response => "Response",
            },
            c.name
        ),
        width: 140.,
        min_width: 48.,
        align: Align::Left,
        sort_key: Some(SortKey::Extra),
        grow: false,
        required: false,
        header: Some(index),
    }
}

/// A visible column with its current width.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedColumn {
    pub column: Column,
    pub px: f32,
}

impl std::ops::Deref for PlacedColumn {
    type Target = Column;
    fn deref(&self) -> &Column {
        &self.column
    }
}

/// The flow table's columns as the user arranged them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColumnLayout {
    /// Every column, in display order.
    #[serde(deserialize_with = "lenient_ids")]
    pub order: Vec<ColumnId>,
    /// Widths in px the user dragged to, by column id; missing ones use
    /// the default.
    pub widths: BTreeMap<String, f32>,
    #[serde(deserialize_with = "lenient_ids")]
    pub hidden: Vec<ColumnId>,
}

/// A saved list of ids, dropping ones this version doesn't know.
fn lenient_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ColumnId>, D::Error> {
    let ids = Vec::<String>::deserialize(d)?;
    Ok(ids.iter().filter_map(|s| ColumnId::parse(s)).collect())
}

impl Default for ColumnLayout {
    fn default() -> Self {
        Self {
            order: Builtin::ALL.into_iter().map(ColumnId::Builtin).collect(),
            widths: BTreeMap::new(),
            hidden: Vec::new(),
        }
    }
}

pub const DEFAULT_SORT: SortSpec = SortSpec {
    key: SortKey::Id,
    dir: pm_model::SortDir::Asc,
    extra: 0,
};

/// The table's prefs: the column layout, the sort and the header columns,
/// in the order the engine fills them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TablePrefs {
    pub columns: ColumnLayout,
    pub sort: SortSpec,
    pub header_columns: Vec<HeaderColumn>,
}

impl Default for TablePrefs {
    fn default() -> Self {
        Self {
            columns: ColumnLayout::default(),
            sort: DEFAULT_SORT,
            header_columns: Vec::new(),
        }
    }
}

impl TablePrefs {
    /// After loading: columns added in a later version join at their
    /// default place, ones that no longer exist are dropped, and every
    /// header column has a place (`store/prefs.ts`'s `merge`).
    pub fn normalize(&mut self) {
        self.header_columns.truncate(MAX_HEADER_COLUMNS);
        let known = |id: &ColumnId| match id {
            ColumnId::Builtin(_) => true,
            ColumnId::Header(h) => self.header_columns.contains(h),
        };
        let mut order: Vec<ColumnId> = Vec::new();
        for id in &self.columns.order {
            if known(id) && !order.contains(id) {
                order.push(id.clone());
            }
        }
        for (i, c) in ColumnLayout::default().order.into_iter().enumerate() {
            if !order.contains(&c) {
                order.insert(i.min(order.len()), c);
            }
        }
        for h in &self.header_columns {
            let id = ColumnId::Header(h.clone());
            if !order.contains(&id) {
                order.push(id);
            }
        }
        self.columns.hidden.retain(|c| known(c));
        self.columns.order = order;
        if self.sort.key == SortKey::Extra && self.sort.extra as usize >= self.header_columns.len() {
            self.sort = DEFAULT_SORT;
        }
    }

    /// The columns to draw, in order.
    pub fn place(&self) -> Vec<PlacedColumn> {
        let lookup = |id: &ColumnId| -> Option<Column> {
            match id {
                ColumnId::Builtin(b) => Some(b.column()),
                ColumnId::Header(h) => {
                    let index = self.header_columns.iter().position(|c| c == h)?;
                    Some(header_column_def(h, index))
                }
            }
        };
        self.columns
            .order
            .iter()
            .filter_map(|id| {
                let column = lookup(id)?;
                if !column.required && self.columns.hidden.contains(id) {
                    return None;
                }
                let px = self
                    .columns
                    .widths
                    .get(&id.to_string())
                    .copied()
                    .unwrap_or(column.width);
                Some(PlacedColumn {
                    px: px.max(column.min_width),
                    column,
                })
            })
            .collect()
    }

    pub fn set_width(&mut self, id: &ColumnId, width: f32) {
        self.columns.widths.insert(id.to_string(), width.round());
    }

    /// Moves `id` to just before `before` (or to the end).
    pub fn move_column(&mut self, id: &ColumnId, before: Option<&ColumnId>) {
        if Some(id) == before {
            return;
        }
        let mut order: Vec<ColumnId> = self.columns.order.iter().filter(|c| *c != id).cloned().collect();
        let at = before
            .and_then(|b| order.iter().position(|c| c == b))
            .unwrap_or(order.len());
        order.insert(at, id.clone());
        self.columns.order = order;
    }

    pub fn set_hidden(&mut self, id: &ColumnId, hidden: bool) {
        self.columns.hidden.retain(|c| c != id);
        if hidden {
            self.columns.hidden.push(id.clone());
        }
    }

    pub fn is_hidden(&self, id: &ColumnId) -> bool {
        self.columns.hidden.contains(id)
    }

    pub fn reset_columns(&mut self) {
        let headers: Vec<ColumnId> = self.header_columns.iter().cloned().map(ColumnId::Header).collect();
        self.columns = ColumnLayout::default();
        // Header columns stay, back before Tools (the Tauri UI drops them
        // from the order until its next launch's merge).
        let at = self.columns.order.len() - 1;
        self.columns.order.splice(at..at, headers);
    }

    /// Adds a header column before the Tools column; false if it is there
    /// already, the name is empty, or there are [`MAX_HEADER_COLUMNS`].
    pub fn add_header_column(&mut self, column: HeaderColumn) -> bool {
        let name = column.name.trim().to_ascii_lowercase();
        let column = HeaderColumn {
            part: column.part,
            name,
        };
        let id = ColumnId::Header(column.clone());
        if column.name.is_empty() || self.header_columns.len() >= MAX_HEADER_COLUMNS || self.columns.order.contains(&id)
        {
            return false;
        }
        let tools = ColumnId::Builtin(Builtin::Tools);
        let had_tools = self.columns.order.contains(&tools);
        self.columns.order.retain(|c| *c != tools);
        self.columns.order.push(id);
        if had_tools {
            self.columns.order.push(tools);
        }
        self.header_columns.push(column);
        true
    }

    pub fn remove_header_column(&mut self, id: &ColumnId) {
        let ColumnId::Header(h) = id else { return };
        let Some(index) = self.header_columns.iter().position(|c| c == h) else {
            return;
        };
        self.header_columns.remove(index);
        self.columns.order.retain(|c| c != id);
        self.columns.hidden.retain(|c| c != id);
        self.columns.widths.remove(&id.to_string());
        // Sorting by a header column that moves or goes falls back to arrival order.
        if self.sort.key == SortKey::Extra {
            self.sort = DEFAULT_SORT;
        }
    }

    /// The header columns the column menu lists, with their ids.
    pub fn header_ids(&self) -> impl Iterator<Item = (usize, ColumnId)> + '_ {
        self.header_columns
            .iter()
            .enumerate()
            .map(|(i, h)| (i, ColumnId::Header(h.clone())))
    }
}

/// Sum of the columns' widths: the table scrolls sideways below it.
pub fn min_table_width(columns: &[PlacedColumn]) -> f32 {
    columns.iter().map(|c| c.px).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(name: &str) -> HeaderColumn {
        HeaderColumn {
            part: BodyPart::Request,
            name: name.into(),
        }
    }

    #[test]
    fn fourteen_builtin_columns_in_the_tauri_order() {
        let placed = TablePrefs::default().place();
        let ids: Vec<String> = placed.iter().map(|c| c.id.to_string()).collect();
        assert_eq!(
            ids,
            [
                "id", "state", "url", "client", "method", "status", "time", "duration", "req", "resp", "proto", "ssl",
                "notes", "tools"
            ]
        );
        assert_eq!(min_table_width(&placed), 1048.0);
    }

    #[test]
    fn ids_round_trip_as_the_tauri_ui_spells_them() {
        let id = ColumnId::header(BodyPart::Response, " X-Request-Id ");
        assert_eq!(id.to_string(), "hdr:response:x-request-id");
        assert_eq!(ColumnId::parse("hdr:response:x-request-id"), Some(id.clone()));
        assert_eq!(ColumnId::parse("url"), Some(ColumnId::Builtin(Builtin::Url)));
        assert_eq!(ColumnId::parse("hdr:body:x"), None);
        assert_eq!(ColumnId::parse("nope"), None);
        assert_eq!(serde_json::to_string(&id).unwrap(), r#""hdr:response:x-request-id""#);
    }

    #[test]
    fn hidden_widths_and_minimums_apply_but_url_cannot_hide() {
        let mut p = TablePrefs::default();
        p.set_hidden(&ColumnId::Builtin(Builtin::Client), true);
        p.set_hidden(&ColumnId::Builtin(Builtin::Url), true);
        p.set_width(&ColumnId::Builtin(Builtin::Id), 10.4);
        p.set_width(&ColumnId::Builtin(Builtin::Time), 120.6);
        let placed = p.place();
        assert!(placed.iter().all(|c| c.id != ColumnId::Builtin(Builtin::Client)));
        assert!(placed.iter().any(|c| c.id == ColumnId::Builtin(Builtin::Url)));
        let px = |b| placed.iter().find(|c| c.id == ColumnId::Builtin(b)).unwrap().px;
        assert_eq!(px(Builtin::Id), 36.0, "never under the minimum");
        assert_eq!(px(Builtin::Time), 121.0, "rounded");
    }

    #[test]
    fn moves_columns_before_another_or_to_the_end() {
        let mut p = TablePrefs::default();
        let (url, id, tools) = (
            ColumnId::Builtin(Builtin::Url),
            ColumnId::Builtin(Builtin::Id),
            ColumnId::Builtin(Builtin::Tools),
        );
        p.move_column(&url, Some(&id));
        assert_eq!(p.columns.order[0], url);
        p.move_column(&url, None);
        assert_eq!(p.columns.order.last(), Some(&url));
        p.move_column(&tools, Some(&tools));
        assert_eq!(p.columns.order.len(), 14);
    }

    #[test]
    fn header_columns_go_before_tools_up_to_eight_once_each() {
        let mut p = TablePrefs::default();
        assert!(p.add_header_column(hdr(" X-Request-Id ")));
        assert!(!p.add_header_column(hdr("x-request-id")), "already there");
        assert!(!p.add_header_column(hdr("  ")), "empty");
        let n = p.columns.order.len();
        assert_eq!(p.columns.order[n - 2].to_string(), "hdr:request:x-request-id");
        assert_eq!(p.columns.order[n - 1], ColumnId::Builtin(Builtin::Tools));
        for i in 1..MAX_HEADER_COLUMNS {
            assert!(p.add_header_column(hdr(&format!("x-{i}"))));
        }
        assert!(!p.add_header_column(hdr("x-one-too-many")));
        let placed = p.place();
        let col = placed.iter().find(|c| c.header == Some(1)).unwrap();
        assert_eq!((col.title.as_str(), col.label.as_str()), ("x-1", "Request header x-1"));
    }

    #[test]
    fn removing_a_header_column_forgets_it_and_its_sort() {
        let mut p = TablePrefs::default();
        p.add_header_column(hdr("a"));
        p.add_header_column(hdr("b"));
        let a = ColumnId::Header(hdr("a"));
        p.set_width(&a, 200.);
        p.set_hidden(&a, true);
        p.sort = SortSpec {
            key: SortKey::Extra,
            dir: pm_model::SortDir::Desc,
            extra: 1,
        };
        p.remove_header_column(&a);
        assert_eq!(p.header_columns, vec![hdr("b")]);
        assert!(!p.columns.order.contains(&a) && p.columns.hidden.is_empty() && p.columns.widths.is_empty());
        assert_eq!(p.sort, DEFAULT_SORT);
        assert_eq!(p.place().iter().find(|c| c.header.is_some()).unwrap().header, Some(0));
    }

    #[test]
    fn sorting_by_a_column_toggles_its_direction() {
        let url = Builtin::Url.column();
        let asc = url.next_sort(&DEFAULT_SORT).unwrap();
        assert_eq!((asc.key, asc.dir), (SortKey::Url, pm_model::SortDir::Asc));
        assert!(url.sorted_by(&asc));
        assert_eq!(url.next_sort(&asc).unwrap().dir, pm_model::SortDir::Desc);
        assert_eq!(
            url.next_sort(&url.next_sort(&asc).unwrap()).unwrap().dir,
            pm_model::SortDir::Asc
        );
        assert_eq!(Builtin::Client.column().next_sort(&DEFAULT_SORT), None);
        let h1 = header_column_def(&hdr("x"), 1);
        let s = h1.next_sort(&DEFAULT_SORT).unwrap();
        assert_eq!((s.key, s.extra), (SortKey::Extra, 1));
        assert!(!header_column_def(&hdr("y"), 0).sorted_by(&s));
    }

    #[test]
    fn saved_prefs_merge_with_new_and_removed_columns() {
        let json = r#"{
            "columns": {"order": ["url", "gone", "id", "hdr:request:x-a", "hdr:request:x-stale"],
                        "widths": {"url": 300}, "hidden": ["client", "hdr:request:x-stale"]},
            "sort": {"key": "extra", "dir": "asc", "extra": 3},
            "headerColumns": [{"part": "request", "name": "x-a"}, {"part": "response", "name": "x-b"}]
        }"#;
        let mut p: TablePrefs = serde_json::from_str(json).unwrap();
        p.normalize();
        let order: Vec<String> = p.columns.order.iter().map(ToString::to_string).collect();
        // Missing built-ins go back at their default index, so saved header
        // columns drift after them; unknown and stale ids are gone.
        assert_eq!(
            order,
            [
                "url",
                "state",
                "id",
                "client",
                "method",
                "status",
                "time",
                "duration",
                "req",
                "resp",
                "proto",
                "ssl",
                "notes",
                "tools",
                "hdr:request:x-a",
                "hdr:response:x-b"
            ]
        );
        assert_eq!(p.columns.hidden, vec![ColumnId::Builtin(Builtin::Client)]);
        assert_eq!(p.sort, DEFAULT_SORT, "a sort by a missing header column falls back");
        assert_eq!(p.place()[0].px, 300.0);
    }

    #[test]
    fn reset_keeps_header_columns_before_tools() {
        let mut p = TablePrefs::default();
        p.add_header_column(hdr("x"));
        p.set_hidden(&ColumnId::Builtin(Builtin::Id), true);
        p.move_column(&ColumnId::Builtin(Builtin::Url), None);
        p.reset_columns();
        let order: Vec<String> = p.columns.order.iter().map(ToString::to_string).collect();
        assert_eq!(order[2], "url");
        assert_eq!(&order[13..], ["hdr:request:x", "tools"]);
        assert!(p.columns.hidden.is_empty());
    }
}
