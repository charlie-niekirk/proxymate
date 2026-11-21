//! Context-menu helpers (`components/menus/`): a [`Menu`] is a list of
//! entries built from state by a plain function, shown as a native menu on
//! right-click. Copy entries copy the whole value (not what's displayed
//! when it's truncated) through [`CopyText`], so every copy toasts the same
//! way; other entries carry their own action.
//!
//! A list keeps one builder keyed by its row (the Tauri UI's
//! `data-menu-key`): the sidebar asks `app_menu(name)` for the row clicked,
//! rather than holding a menu per row. Because menus are data, tests (and
//! the dev remote's `contextMenu`) read their entries and run them without
//! an OS menu.

use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::{Action, App, Pixels, Point, SharedString, Window};

use crate::features::clipboard::CopyText;

pub enum Entry {
    /// Copies `text`; the toast names it `what`.
    Copy {
        label: SharedString,
        text: SharedString,
        what: SharedString,
    },
    Item {
        label: SharedString,
        action: Box<dyn Action>,
        disabled: bool,
    },
    Check {
        label: SharedString,
        checked: bool,
        action: Box<dyn Action>,
    },
    /// A submenu ("Copy ▸", "Filter by ▸"); [`Menu::run`] reaches its
    /// entries as `"Copy › Host"`.
    Submenu {
        label: SharedString,
        menu: Menu,
    },
    Separator,
}

impl Entry {
    pub fn label(&self) -> Option<&str> {
        match self {
            Entry::Copy { label, .. }
            | Entry::Item { label, .. }
            | Entry::Check { label, .. }
            | Entry::Submenu { label, .. } => Some(label),
            Entry::Separator => None,
        }
    }

    /// What choosing it dispatches.
    pub fn action(&self) -> Option<Box<dyn Action>> {
        match self {
            Entry::Copy { text, what, .. } => Some(Box::new(CopyText::new(text.clone(), what.clone()))),
            Entry::Item { action, disabled, .. } => (!disabled).then(|| action.boxed_clone()),
            Entry::Check { action, .. } => Some(action.boxed_clone()),
            Entry::Submenu { .. } | Entry::Separator => None,
        }
    }
}

/// What a submenu's path separates: `"Copy › Host"`.
pub const PATH: &str = " › ";

/// The label of [`Menu::selection`]'s entry.
pub const SELECTION_LABEL: &str = "Copy";

/// The text selected in `window` (selectable text in the inspector), for
/// [`Menu::selection`].
pub fn selected_text(window: &mut Window, cx: &mut App) -> Option<String> {
    let text = gpui_kit::base::TextSelection::selected_text(window, cx);
    (!text.is_empty()).then_some(text)
}

#[derive(Default)]
pub struct Menu {
    entries: Vec<Entry>,
}

impl Menu {
    pub fn new() -> Menu {
        Menu::default()
    }

    pub fn copy(
        mut self,
        label: impl Into<SharedString>,
        text: impl Into<SharedString>,
        what: impl Into<SharedString>,
    ) -> Menu {
        self.entries.push(Entry::Copy {
            label: label.into(),
            text: text.into(),
            what: what.into(),
        });
        self
    }

    pub fn item(mut self, label: impl Into<SharedString>, action: impl Action) -> Menu {
        self.entries.push(Entry::Item {
            label: label.into(),
            action: Box::new(action),
            disabled: false,
        });
        self
    }

    pub fn disabled(mut self, label: impl Into<SharedString>, action: impl Action) -> Menu {
        self.entries.push(Entry::Item {
            label: label.into(),
            action: Box::new(action),
            disabled: true,
        });
        self
    }

    pub fn check(mut self, label: impl Into<SharedString>, checked: bool, action: impl Action) -> Menu {
        self.entries.push(Entry::Check {
            label: label.into(),
            checked,
            action: Box::new(action),
        });
        self
    }

    /// A submenu, unless it is empty.
    pub fn submenu(mut self, label: impl Into<SharedString>, menu: Menu) -> Menu {
        if !menu.is_empty() {
            self.entries.push(Entry::Submenu {
                label: label.into(),
                menu,
            });
        }
        self
    }

    /// "Copy ⌘C" for the text selected when the menu opened, then a
    /// separator: our menus replace the native one, which would offer it.
    pub fn selection(self, selected: Option<String>) -> Menu {
        match selected.filter(|s| !s.is_empty()) {
            Some(text) => self.copy(SELECTION_LABEL, text, "selection").separator(),
            None => self,
        }
    }

    /// A separator, unless the menu is empty or already ends with one.
    pub fn separator(mut self) -> Menu {
        if self.entries.last().is_some_and(|e| !matches!(e, Entry::Separator)) {
            self.entries.push(Entry::Separator);
        }
        self
    }

    /// Another menu's entries after these (a slot's items).
    pub fn append(mut self, other: Menu) -> Menu {
        for entry in other.entries {
            if matches!(entry, Entry::Separator) {
                self = self.separator();
            } else {
                self.entries.push(entry);
            }
        }
        self
    }

    /// Entries added only when `cond` holds.
    pub fn when(self, cond: bool, f: impl FnOnce(Menu) -> Menu) -> Menu {
        if cond { f(self) } else { self }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(|e| matches!(e, Entry::Separator))
    }

    /// Every entry's label, `—` for a separator.
    pub fn labels(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|e| e.label().unwrap_or("—").to_owned())
            .collect()
    }

    /// The copy entry labelled `label`: its text.
    pub fn copy_text(&self, label: &str) -> Option<&str> {
        match self.find(label)? {
            Entry::Copy { text, .. } => Some(text.as_ref()),
            _ => None,
        }
    }

    /// The entry labelled `label`; `"Copy › Host"` finds one in a submenu.
    pub fn find(&self, label: &str) -> Option<&Entry> {
        if let Some(entry) = self.entries.iter().find(|e| e.label() == Some(label)) {
            return Some(entry);
        }
        let (parent, rest) = label.split_once(PATH)?;
        self.entries.iter().find_map(|e| match e {
            Entry::Submenu { label, menu } if label == parent => menu.find(rest),
            _ => None,
        })
    }

    /// Whether the entry labelled `label` can be chosen.
    pub fn is_enabled(&self, label: &str) -> bool {
        self.find(label).is_some_and(|e| e.action().is_some())
    }

    /// Runs the entry labelled `label`, as choosing it would. False if
    /// there is none (or it is disabled).
    pub fn run(&self, label: &str, window: &mut Window, cx: &mut App) -> bool {
        let Some(action) = self.find(label).and_then(Entry::action) else {
            return false;
        };
        window.dispatch_action(action, cx);
        true
    }

    /// The entries as JSON (the dev remote).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.entries
                .iter()
                .map(|e| match e {
                    Entry::Copy { label, text, .. } => serde_json::json!({ "label": label, "copies": text }),
                    Entry::Item { label, disabled, .. } => serde_json::json!({ "label": label, "disabled": disabled }),
                    Entry::Check { label, checked, .. } => serde_json::json!({ "label": label, "checked": checked }),
                    Entry::Submenu { label, menu } => serde_json::json!({ "label": label, "submenu": menu.to_json() }),
                    Entry::Separator => serde_json::json!("—"),
                })
                .collect(),
        )
    }

    pub fn native(self) -> NativeMenu {
        let mut menu = NativeMenu::new();
        for entry in self.entries {
            menu = match entry {
                Entry::Copy { label, text, what } => menu.menu(label, Box::new(CopyText::new(text, what))),
                Entry::Item {
                    label,
                    action,
                    disabled,
                } => menu.menu_with_disabled(label, disabled, action),
                Entry::Check { label, checked, action } => menu.menu_with_check(label, checked, action),
                Entry::Submenu { label, menu: sub } => menu.submenu(label, sub.native()),
                Entry::Separator => menu.separator(),
            };
        }
        menu
    }

    /// Shows the menu at `position` (window coordinates).
    pub fn show(self, position: Point<Pixels>, window: &mut Window, cx: &mut App) {
        if !self.is_empty() {
            self.native().show(position, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    gpui_kit::actions!(test_menu, [Noop]);

    #[test]
    fn separators_never_lead_or_repeat() {
        let menu = Menu::new()
            .separator()
            .copy("Copy name", "a", "name")
            .separator()
            .separator()
            .when(false, |m| m.item("Hidden", Noop))
            .disabled("Later", Noop)
            .check("On", true, Noop);
        assert_eq!(menu.labels(), ["Copy name", "—", "Later", "On"]);
        assert_eq!(menu.copy_text("Copy name"), Some("a"));
        assert!(menu.find("Later").unwrap().action().is_none(), "disabled");
        assert!(Menu::new().separator().is_empty());
    }

    #[test]
    fn submenus_are_reached_by_their_path() {
        let menu = Menu::new()
            .selection(Some("sel".into()))
            .submenu("Copy", Menu::new().copy("Host", "a.example", "host"))
            .submenu("Empty", Menu::new());
        assert_eq!(menu.labels(), ["Copy", "—", "Copy"]);
        assert_eq!(menu.copy_text("Copy › Host"), Some("a.example"));
        assert_eq!(menu.copy_text("Copy"), Some("sel"), "the selection item comes first");
        assert!(menu.is_enabled("Copy › Host"));
        assert!(!menu.is_enabled("Copy › Path"));
    }
}
