//! The editing widgets' menus, built from their menu data: the JsonEditor's
//! row menu (`JsonEditor.tsx`'s context menu) and a KvEditor's removed-row
//! menu. Their entries dispatch the widgets' actions, which the focused
//! widget handles; copies go through `CopyText`.

use pm_view::json::doc::JType;
use pm_view::json::editor::MenuSpec;
use pm_view::kv::KvRow;
use pm_widgets::json_editor::{AddAfter, AddChild, ChangeType, Duplicate, EditValue, RemoveRow, Rename, ResetRow};
use pm_widgets::kv_editor::PutBack;

use crate::features::context_menu::Menu;

/// A type submenu whose entries make `action(t)`; `current` is disabled.
fn types<A: gpui_kit::Action>(current: Option<JType>, action: impl Fn(JType) -> A) -> Menu {
    JType::ALL.into_iter().fold(Menu::new(), |menu, t| {
        if Some(t) == current {
            menu.disabled(t.label(), action(t))
        } else {
            menu.item(t.label(), action(t))
        }
    })
}

/// The JSON editor's row menu (the row is selected first).
pub fn json_row(spec: &MenuSpec) -> Menu {
    let mut menu = Menu::new();
    if spec.edit_value {
        menu = menu.item("Edit value  ↩", EditValue);
    }
    if spec.rename {
        menu = menu.item("Rename  F2", Rename);
    }
    menu = menu
        .submenu(
            "Change type",
            types(Some(spec.current), |t| ChangeType { t: t.id().into() }),
        )
        .separator();
    if let Some(label) = spec.add_child {
        menu = menu.submenu(label, types(None, |t| AddChild { t: t.id().into() }));
    }
    if spec.not_root {
        menu = menu
            .submenu("Add after  ⌘↩", types(None, |t| AddAfter { t: t.id().into() }))
            .item("Duplicate  ⌘D", Duplicate);
    }
    if spec.reset {
        menu = menu.item("Reset to original", ResetRow);
    }
    if spec.not_root {
        menu = menu.item("Remove  ⌫", RemoveRow);
    }
    menu.separator().copy("Copy value", spec.value.clone(), "value").copy(
        "Copy JSON pointer",
        spec.pointer.clone(),
        "JSON pointer",
    )
}

/// A removed row's menu in a KvEditor.
pub fn removed_row(row: &KvRow) -> Menu {
    Menu::new()
        .copy("Copy name", row.name.clone(), "name")
        .copy("Copy value", row.value.clone(), "value")
        .separator()
        .item("Put back", PutBack { id: row.id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_json_menu_offers_what_the_row_allows() {
        let m = pm_view::json::editor::JsonEditorModel::for_text(r#"{"a":[1],"s":"x"}"#, None).unwrap();
        let root = json_row(&m.menu(0).unwrap());
        assert_eq!(
            root.labels(),
            ["Change type", "—", "Add key", "—", "Copy value", "Copy JSON pointer"]
        );
        assert_eq!(root.copy_text("Copy JSON pointer"), Some("/"));
        let s = json_row(&m.menu(m.rows().len() - 1).unwrap());
        assert!(s.labels().contains(&"Edit value  ↩".to_owned()));
        assert!(s.labels().contains(&"Remove  ⌫".to_owned()));
        assert!(!s.labels().contains(&"Reset to original".to_owned()));
    }
}
