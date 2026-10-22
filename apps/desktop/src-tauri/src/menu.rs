//! The app menu bar. File's items are handled by the UI: each click is
//! emitted to the webview as a [`MENU_EVENT`] carrying the item's id, so
//! the UI decides what is saved or exported (the active document, the
//! selection) exactly as its own buttons do. "Check for Updates…" is
//! handled here (`updater.rs`).

use tauri::menu::{Menu, MenuEvent, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Runtime};

/// The webview event a menu click is emitted as; its payload is the id.
pub const MENU_EVENT: &str = "pm-menu";

const CHECK_FOR_UPDATES: &str = "check-for-updates";

/// File items: id, label, accelerator.
const FILE_ITEMS: &[&[(&str, &str, &str)]] = &[
    &[("compose", "New Request…", "CmdOrCtrl+N")],
    &[
        ("open", "Open…", "CmdOrCtrl+O"),
        ("import-har", "Import HAR…", "CmdOrCtrl+Shift+O"),
    ],
    &[
        ("save-session", "Save Session…", "CmdOrCtrl+S"),
        ("export-har", "Export HAR…", "CmdOrCtrl+Shift+E"),
    ],
];

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let mut file = SubmenuBuilder::new(app, "File");
    for (i, group) in FILE_ITEMS.iter().enumerate() {
        if i > 0 {
            file = file.separator();
        }
        for (id, label, accelerator) in group.iter() {
            file = file.item(
                &MenuItemBuilder::with_id(*id, *label)
                    .accelerator(*accelerator)
                    .build(app)?,
            );
        }
    }
    let file = file.separator().close_window().build()?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let view = SubmenuBuilder::new(app, "View").fullscreen().build()?;
    let window = SubmenuBuilder::new(app, "Window").minimize().maximize().build()?;
    let check_for_updates = MenuItemBuilder::with_id(CHECK_FOR_UPDATES, "Check for Updates…").build(app)?;
    #[cfg(target_os = "macos")]
    {
        let app_menu = SubmenuBuilder::new(app, "Proxymate")
            .about(None)
            .item(&check_for_updates)
            .separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?;
        Menu::with_items(app, &[&app_menu, &file, &edit, &view, &window])
    }
    #[cfg(not(target_os = "macos"))]
    {
        let help = SubmenuBuilder::new(app, "Help").item(&check_for_updates).build()?;
        Menu::with_items(app, &[&file, &edit, &view, &window, &help])
    }
}

/// Forwards File's own items to the webview.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().0.as_str();
    if id == CHECK_FOR_UPDATES {
        return crate::updater::check_now(app);
    }
    if FILE_ITEMS.iter().flat_map(|g| g.iter()).any(|(item, _, _)| *item == id) {
        let _ = app.emit(MENU_EVENT, id);
    }
}
