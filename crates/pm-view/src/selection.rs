//! The flow table's multi-selection (M17), ported from `lib/selection.ts`:
//! by flow id, never by position, since rows move as traffic arrives and
//! filters change. ⌘A selects the whole view without listing it: `all`,
//! with `ids` as the exceptions.

use std::collections::BTreeSet;

use pm_model::{FlowId, FlowSelection, ViewId};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableSelection {
    pub all: bool,
    /// Selected ids, or with `all`, the ids left out.
    pub ids: BTreeSet<FlowId>,
}

impl TableSelection {
    pub fn empty() -> Self {
        Self::default()
    }

    /// The whole view (⌘A).
    pub fn whole_view() -> Self {
        Self {
            all: true,
            ids: BTreeSet::new(),
        }
    }

    pub fn single(id: Option<FlowId>) -> Self {
        Self {
            all: false,
            ids: id.into_iter().collect(),
        }
    }

    /// ⇧-click: exactly the rows between the anchor and the click.
    pub fn range(ids: impl IntoIterator<Item = FlowId>) -> Self {
        Self {
            all: false,
            ids: ids.into_iter().collect(),
        }
    }

    pub fn is_selected(&self, id: FlowId) -> bool {
        self.all != self.ids.contains(&id)
    }

    /// How many flows are selected, in a view of `len` rows.
    pub fn count(&self, len: usize) -> usize {
        if self.all {
            len.saturating_sub(self.ids.len())
        } else {
            self.ids.len()
        }
    }

    /// ⌘-click: in or out of the selection.
    pub fn toggle(&self, id: FlowId) -> Self {
        let mut ids = self.ids.clone();
        if !ids.remove(&id) {
            ids.insert(id);
        }
        Self { all: self.all, ids }
    }

    /// True for more than one flow (the bulk menu's case).
    pub fn is_multi(&self, len: usize) -> bool {
        self.count(len) > 1
    }

    /// The selected ids when the selection lists them (not a whole view),
    /// in arrival order.
    pub fn listed_ids(&self) -> Option<Vec<FlowId>> {
        (!self.all).then(|| self.ids.iter().copied().collect())
    }

    /// The selection as the engine takes it; ids in arrival order.
    pub fn to_flow_selection(&self, view: Option<ViewId>) -> FlowSelection {
        match (self.all, view) {
            (true, Some(view)) => FlowSelection::View {
                view,
                except: self.ids.iter().copied().collect(),
            },
            (true, None) => FlowSelection::All,
            (false, _) => FlowSelection::Ids {
                ids: self.ids.iter().copied().collect(),
            },
        }
    }
}

/// The selection with the focused flow (the one the inspector shows).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selected {
    pub selection: TableSelection,
    pub focused: Option<FlowId>,
}

impl Selected {
    /// A plain click: this flow alone.
    pub fn select(&mut self, id: Option<FlowId>) {
        self.selection = TableSelection::single(id);
        self.focused = id;
    }

    /// ⌘-click, as `store/ui.ts`'s `toggleSelected`: unselecting the focused
    /// flow moves the focus to another selected one.
    pub fn toggle(&mut self, id: FlowId) {
        let selection = self.selection.toggle(id);
        let added = selection.is_selected(id);
        let rest = if selection.all {
            None
        } else {
            selection.ids.iter().next_back().copied()
        };
        self.focused = if added {
            Some(id)
        } else if self.focused == Some(id) {
            rest
        } else {
            self.focused
        };
        self.selection = selection;
    }

    /// A range or ⌘A, keeping or moving the focus.
    pub fn set(&mut self, selection: TableSelection, focus: Option<Option<FlowId>>) {
        self.selection = selection;
        if let Some(focus) = focus {
            self.focused = focus;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_ids_in_and_out() {
        let mut sel = TableSelection::single(Some(3));
        sel = sel.toggle(7);
        assert_eq!(
            [sel.is_selected(3), sel.is_selected(7), sel.is_selected(5)],
            [true, true, false]
        );
        sel = sel.toggle(3);
        assert!(!sel.is_selected(3));
        assert_eq!(sel.count(100), 1);
        assert!(!sel.is_multi(100));
    }

    #[test]
    fn keeps_a_whole_view_as_exceptions_never_as_a_list() {
        let mut sel = TableSelection::whole_view();
        sel = sel.toggle(4);
        assert_eq!([sel.is_selected(4), sel.is_selected(99_999)], [false, true]);
        assert_eq!(sel.count(100_000), 99_999);
        assert_eq!(sel.listed_ids(), None);
        assert_eq!(
            sel.to_flow_selection(Some(12)),
            FlowSelection::View {
                view: 12,
                except: vec![4]
            }
        );
    }

    #[test]
    fn lists_ids_in_arrival_order_for_the_engine() {
        assert_eq!(
            TableSelection::range([9, 2, 5]).to_flow_selection(Some(1)),
            FlowSelection::Ids { ids: vec![2, 5, 9] }
        );
        assert_eq!(
            TableSelection::empty().to_flow_selection(Some(1)),
            FlowSelection::Ids { ids: vec![] }
        );
        assert!(TableSelection::single(None).ids.is_empty());
        assert_eq!(TableSelection::whole_view().to_flow_selection(None), FlowSelection::All);
    }

    #[test]
    fn toggling_the_focused_flow_out_moves_the_focus() {
        let mut s = Selected::default();
        s.select(Some(3));
        s.toggle(7);
        assert_eq!(s.focused, Some(7));
        s.toggle(9);
        s.toggle(9);
        assert_eq!(s.focused, Some(7), "toggling another flow out keeps the focus");
        s.toggle(7);
        assert_eq!(s.focused, Some(3), "the focus moves to a flow still selected");
        s.toggle(3);
        assert_eq!((s.focused, s.selection.count(10)), (None, 0));
    }

    #[test]
    fn set_keeps_the_focus_unless_told() {
        let mut s = Selected::default();
        s.select(Some(2));
        s.set(TableSelection::whole_view(), None);
        assert_eq!(s.focused, Some(2));
        s.set(TableSelection::range([1, 2, 3]), Some(Some(3)));
        assert_eq!(s.focused, Some(3));
    }
}
