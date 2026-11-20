//! The menu bar and the app-level actions. Its items mirror the Tauri app's
//! `menu.rs`; File's items are actions that later phases handle (G2: Open,
//! Save Session, Export HAR; G8: New Request; G10: Scripts), so until then
//! gpui shows them disabled. Feature modules add their own items through
//! their `register`.

use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{App, AppContext as _, KeyBinding, Menu, MenuItem, OsAction, PromptLevel, SystemMenuType};

use crate::prefs::{self, Appearance};

gpui_kit::actions!(
    proxymate,
    [
        Quit,
        About,
        CheckForUpdates,
        Hide,
        HideOthers,
        ShowAll,
        NewRequest,
        Open,
        ImportHar,
        SaveSession,
        ExportHar,
        Scripts,
        CloseWindow,
        Minimize,
        Zoom,
        ToggleFullScreen,
        AppearanceSystem,
        AppearanceLight,
        AppearanceDark,
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-n", NewRequest, None),
        KeyBinding::new("cmd-o", Open, None),
        KeyBinding::new("shift-cmd-o", ImportHar, None),
        KeyBinding::new("cmd-s", SaveSession, None),
        KeyBinding::new("shift-cmd-e", ExportHar, None),
        KeyBinding::new("shift-cmd-j", Scripts, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &About, cx| about(cx));
    cx.on_action(|_: &CheckForUpdates, cx| crate::updater::check_now(cx));
    cx.on_action(|_: &AppearanceSystem, cx| crate::appearance::set(Appearance::System, cx));
    cx.on_action(|_: &AppearanceLight, cx| crate::appearance::set(Appearance::Light, cx));
    cx.on_action(|_: &AppearanceDark, cx| crate::appearance::set(Appearance::Dark, cx));
    on_window(cx, |window, _| window.remove_window(), CloseWindow);
    on_window(cx, |window, _| window.minimize_window(), Minimize);
    on_window(cx, |window, _| window.zoom_window(), Zoom);
    on_window(cx, |window, _| window.toggle_fullscreen(), ToggleFullScreen);
    refresh(cx);
}

/// A window action on the active window.
fn on_window<A: gpui_kit::Action>(cx: &mut App, f: fn(&mut gpui_kit::Window, &mut App), _: A) {
    cx.on_action(move |_: &A, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, cx| f(window, cx));
        }
    });
}

/// Rebuilds the menus (the Appearance items' checks follow the prefs).
pub fn refresh(cx: &mut App) {
    let appearance = cx
        .try_global::<prefs::Stored>()
        .map(|s| s.prefs.appearance)
        .unwrap_or_default();
    let file = Menu::new("File").items([
        MenuItem::action("New Request…", NewRequest),
        MenuItem::separator(),
        MenuItem::action("Open…", Open),
        MenuItem::action("Import HAR…", ImportHar),
        MenuItem::separator(),
        MenuItem::action("Save Session…", SaveSession),
        MenuItem::action("Export HAR…", ExportHar),
        MenuItem::separator(),
        MenuItem::action("Scripts…", Scripts),
        MenuItem::separator(),
        MenuItem::action("Close Window", CloseWindow),
    ]);
    let edit = Menu::new("Edit").items([
        MenuItem::os_action("Undo", Undo, OsAction::Undo),
        MenuItem::os_action("Redo", Redo, OsAction::Redo),
        MenuItem::separator(),
        MenuItem::os_action("Cut", Cut, OsAction::Cut),
        MenuItem::os_action("Copy", Copy, OsAction::Copy),
        MenuItem::os_action("Paste", Paste, OsAction::Paste),
        MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
    ]);
    let view = Menu::new("View").items([
        MenuItem::submenu(Menu::new("Appearance").items([
            MenuItem::action("System", AppearanceSystem).checked(appearance == Appearance::System),
            MenuItem::action("Light", AppearanceLight).checked(appearance == Appearance::Light),
            MenuItem::action("Dark", AppearanceDark).checked(appearance == Appearance::Dark),
        ])),
        MenuItem::separator(),
        MenuItem::action("Enter Full Screen", ToggleFullScreen),
    ]);
    let window = Menu::new("Window").items([MenuItem::action("Minimize", Minimize), MenuItem::action("Zoom", Zoom)]);
    let check_for_updates = MenuItem::action("Check for Updates…", CheckForUpdates);
    let menus = if cfg!(target_os = "macos") {
        vec![
            Menu::new("Proxymate").items([
                MenuItem::action("About Proxymate", About),
                check_for_updates,
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Proxymate", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Proxymate", Quit),
            ]),
            file,
            edit,
            view,
            window,
        ]
    } else {
        vec![file, edit, view, window, Menu::new("Help").items([check_for_updates])]
    };
    cx.set_menus(menus);
}

fn about(cx: &mut App) {
    let detail = format!("Version {} (native preview)", env!("CARGO_PKG_VERSION"));
    prompt(cx, PromptLevel::Info, "Proxymate", detail);
}

fn prompt(cx: &mut App, level: PromptLevel, message: &'static str, detail: String) {
    let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        return;
    };
    let _ = window.update(cx, |_, window, cx| {
        let answer = window.prompt(level, message, Some(&detail), &["OK"], cx);
        cx.background_spawn(async move {
            let _ = answer.await;
        })
        .detach();
    });
}
