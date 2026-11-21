//! Settings (G12): the Settings window (`components/Settings/*`), a second
//! native window with a page list beside the chosen page: Proxy, Network,
//! SSL Proxying, Clients, Storage, Protobuf, Mocking, Appearance, Advanced,
//! MCP and Updates. ⌘, and the toolbar's gear open it; `OpenSettings` (the
//! failure banner's fixes, the footer, the listening pill) opens it at a
//! page. Also the mocking entry points (`lib/actions.ts`): Mock from
//! session… (a tab's menu, the palette), Mock from a file…, Import OpenAPI
//! spec… and Auto-save now.
//!
//! Pages read engine state through `bridge::keys::settings_caches` and the
//! pushes (storage, devices, MCP); every write is a `bridge::mutations`
//! entry. Inputs are bound to keys ([`window::SettingsWindow::type_text`]),
//! switches and pickers to [`SettingsChoice`], buttons to [`SettingsPress`],
//! so menus, the dev remote and tests drive the page as a user would.

pub mod menus;
pub mod pages;
pub mod parts;
pub mod sample;
pub mod window;

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyView, App, Context, PathPromptOptions, Render, SharedString, Window};
use pm_model::StoreId;
use pm_theme::kit::TokensExt as _;

use crate::bridge::mutations::{self, MockFrom};
use crate::features::chrome::OpenSettings;
use crate::features::clipboard::{self, report_error};
use crate::features::commands::{self, always};
use crate::features::rules::{self, Target};
use crate::shell::toolbar;
use crate::workspace::Workspace;

pub use window::SettingsWindow;

/// A switch or picker of the open Settings window: `key` set to `value`
/// (`proxy.lan` = `true`, `net.up.0.kind` = `socks5`, `appearance` = `dark`).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SettingsChoice {
    pub key: SharedString,
    pub value: SharedString,
}

/// A button of the open Settings window, by id (`net.save`,
/// `list.ssl.include.remove:example.com`, `mock.remove:<id>`).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SettingsPress {
    pub id: SharedString,
}

/// "Mock from session…" on a store (a tab's menu; 0 is the live capture).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct MockFromStore {
    pub store: StoreId,
}

gpui_kit::actions!(proxymate, [MockFromFile, ImportOpenApi, AutosaveNow, CloseSettings]);

pub fn register(cx: &mut App) {
    cx.on_action(|a: &OpenSettings, cx| open(&a.page, cx));
    cx.on_action(|a: &SettingsChoice, cx| {
        let (key, value) = (a.key.to_string(), a.value.to_string());
        SettingsWindow::update_global(cx, move |w, window, cx| w.choose(&key, &value, window, cx));
    });
    cx.on_action(|a: &SettingsPress, cx| {
        let id = a.id.to_string();
        SettingsWindow::update_global(cx, move |w, window, cx| w.press(&id, window, cx));
    });
    cx.on_action(|a: &MockFromStore, cx| mock_from(MockFrom::Store(a.store), cx));
    cx.on_action(|_: &MockFromFile, cx| mock_from_file(cx));
    cx.on_action(|_: &ImportOpenApi, cx| import_openapi(cx));
    cx.on_action(|_: &AutosaveNow, cx| mutations::autosave_now(cx));
    cx.on_action(|_: &CloseSettings, cx| close(cx));
    cx.bind_keys([gpui_kit::KeyBinding::new(
        "escape",
        CloseSettings,
        Some("SettingsWindow && !Input"),
    )]);

    commands::add("settings", always, |_, cx| open("", cx), cx);
    for (page, _) in pm_view::commands::SETTINGS_PAGES {
        commands::add(&format!("settings-{page}"), always, move |_, cx| open(page, cx), cx);
    }
    commands::add("autosave-now", always, |_, cx| mutations::autosave_now(cx), cx);
    commands::add(
        "mock-from-session",
        always,
        |_, cx| {
            let store = Workspace::global(cx).map_or(pm_model::LIVE_STORE, |w| w.read(cx).store);
            mock_from(MockFrom::Store(store), cx);
        },
        cx,
    );
    commands::add("mock-from-file", always, |_, cx| mock_from_file(cx), cx);
    commands::add("import-openapi", always, |_, cx| import_openapi(cx), cx);

    toolbar::add_tool(toolbar::order::SETTINGS, settings_button, cx);
}

/// Opens the Settings window (or brings it forward) at `page` (empty: the
/// page it showed, or Proxy).
pub fn open(page: &str, cx: &mut App) {
    let page = (!page.is_empty()).then(|| pm_view::settings::page_or_default(page));
    // Deferred: the action that asked is still being dispatched by its window.
    cx.defer(move |cx| window::open_or_focus(page, cx));
}

/// Closes the Settings window (Escape, the dev remote).
pub fn close(cx: &mut App) {
    cx.defer(|cx| {
        if let Some((window, _)) = SettingsWindow::global(cx) {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    });
}

/// `PROXYMATE_DEV_OPEN=settings[:<page>]` opens the window at launch (dev only).
pub fn open_from_env(cx: &mut App) {
    if let Some(open_at) = crate::dev_env::value("PROXYMATE_DEV_OPEN")
        && let Some(page) = open_at.strip_prefix("settings")
    {
        open(page.trim_start_matches(':'), cx);
    }
}

/// Copies a store's flows into the mock library, then opens the unsaved
/// Serve from rule in the Rules window.
pub fn mock_from(from: MockFrom, cx: &mut App) {
    let what = match from {
        MockFrom::Store(_) => "Couldn't mock from this session",
        MockFrom::File(_) => "Couldn't mock from that file",
    };
    mutations::create_mock_source(
        from,
        |rule, cx| rules::open(Target::Draft(Box::new(rule)), cx),
        move |err, cx| report_error(what, err, cx),
        cx,
    );
}

/// Asks for a file, then [`mock_from`] it.
pub fn mock_from_file(cx: &mut App) {
    pick_file("Mock from a session, HAR or Charles file", false, cx, |paths, cx| {
        if let Some(path) = paths.into_iter().next() {
            mock_from(MockFrom::File(path), cx);
        }
    });
}

/// Asks for a spec, imports it, then shows Settings › Mocking.
pub fn import_openapi(cx: &mut App) {
    pick_file("Import an OpenAPI 3 spec (JSON or YAML)", false, cx, |paths, cx| {
        if let Some(path) = paths.into_iter().next() {
            import_openapi_path(path, cx);
        }
    });
}

/// Imports the spec at `path` (the dev remote's `importOpenApi`).
pub fn import_openapi_path(path: PathBuf, cx: &mut App) {
    mutations::import_openapi(
        path,
        |spec, cx| {
            clipboard::info(&format!("Imported {}", spec.name), None, cx);
            open("mocking", cx);
        },
        |err, cx| report_error("Couldn't import the spec", err, cx),
        cx,
    );
}

/// gpui's open panel (it takes no type filter: the engine refuses what it
/// can't read), then `then` with the paths picked.
pub fn pick_file(
    prompt: &'static str,
    multiple: bool,
    cx: &mut App,
    then: impl FnOnce(Vec<PathBuf>, &mut App) + 'static,
) {
    pick(prompt, multiple, false, cx, then);
}

/// A folder (auto-save's Choose…).
pub fn pick_folder(prompt: &'static str, cx: &mut App, then: impl FnOnce(Vec<PathBuf>, &mut App) + 'static) {
    pick(prompt, false, true, cx, then);
}

fn pick(
    prompt: &'static str,
    multiple: bool,
    directories: bool,
    cx: &mut App,
    then: impl FnOnce(Vec<PathBuf>, &mut App) + 'static,
) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: !directories,
        directories,
        multiple,
        prompt: Some(prompt.into()),
    });
    cx.spawn(async move |cx| match picked.await {
        Ok(Ok(Some(paths))) if !paths.is_empty() => cx.update(|cx| then(paths, cx)),
        Ok(Err(err)) => cx.update(|cx| report_error("Couldn't show the open panel", err, cx)),
        _ => {}
    })
    .detach();
}

pub const SETTINGS_TIP: &str = "Settings · ⌘,";

/// The toolbar's gear: opens Settings.
pub fn settings_button(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| SettingsButton).into()
}

pub struct SettingsButton;

impl Render for SettingsButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        Button::new("settings-button")
            .ghost()
            .small()
            .accessibility_label("Settings")
            .tooltip(SETTINGS_TIP)
            .child(pm_widgets::icon(IconName::Settings, t.foreground.hsla(), 15.))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings { page: "".into() }), cx))
    }
}
