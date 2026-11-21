//! Documents (`lib/actions.ts` documents section): File › Open… (sessions,
//! HAR and Charles files), Import HAR…, Save Session…, Export HAR…, the
//! tab and sidebar items that switch, save and close documents, and their
//! palette commands. File dialogs run here; the chosen path goes to the
//! engine through the [`Workspace`].
//!
//! gpui's open panel takes no file-type filter, so Open… shows every file
//! and the engine refuses what it can't read (Import HAR… says so in its
//! prompt).

use std::path::PathBuf;

use gpui_kit::{App, PathPromptOptions, SharedString};
use pm_model::{FlowSelection, LIVE_STORE, StoreId};

use crate::features::clipboard::report_error;
use crate::features::commands::{self, always};
use crate::features::flow_table;
use crate::menus;
use crate::workspace::{SaveKind, Workspace};

/// Shows a store's tab (the live capture is store 0).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ShowStore {
    pub store: StoreId,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct CloseDocument {
    pub store: StoreId,
}

/// A tab menu's "Save session…": switches to the tab first.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SaveStore {
    pub store: StoreId,
}

/// A tab menu's "Export HAR…": switches to the tab first, exports all of it.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ExportStore {
    pub store: StoreId,
}

gpui_kit::actions!(proxymate, [DismissSave]);

pub fn register(cx: &mut App) {
    cx.on_action(|_: &menus::Open, cx| open(false, cx));
    cx.on_action(|_: &menus::ImportHar, cx| open(true, cx));
    cx.on_action(|_: &menus::SaveSession, cx| save(SaveKind::Session, None, cx));
    cx.on_action(|_: &menus::ExportHar, cx| save(SaveKind::Har, None, cx));
    cx.on_action(|a: &ShowStore, cx| {
        with_workspace(cx, |w, cx| w.set_store(a.store, cx));
    });
    cx.on_action(|a: &CloseDocument, cx| {
        with_workspace(cx, |w, cx| w.close_document(a.store, cx));
    });
    cx.on_action(|a: &SaveStore, cx| {
        with_workspace(cx, |w, cx| w.set_store(a.store, cx));
        save(SaveKind::Session, None, cx);
    });
    cx.on_action(|a: &ExportStore, cx| {
        with_workspace(cx, |w, cx| w.set_store(a.store, cx));
        save(SaveKind::Har, Some(FlowSelection::All), cx);
    });
    cx.on_action(|_: &DismissSave, cx| {
        with_workspace(cx, |w, cx| w.dismiss_save(cx));
    });
    commands::add("open", always, |_, cx| open(false, cx), cx);
    commands::add("save-session", always, |_, cx| save(SaveKind::Session, None, cx), cx);
    commands::add("export-har", always, |_, cx| save(SaveKind::Har, None, cx), cx);
    commands::add(
        "live",
        |cx| Workspace::global(cx).is_some_and(|w| !w.read(cx).is_live()),
        |_, cx| {
            with_workspace(cx, |w, cx| w.set_store(LIVE_STORE, cx));
        },
        cx,
    );
}

pub fn with_workspace<R>(
    cx: &mut App,
    f: impl FnOnce(&mut Workspace, &mut gpui_kit::Context<Workspace>) -> R,
) -> Option<R> {
    let ws = Workspace::global(cx)?;
    Some(ws.update(cx, f))
}

/// Asks for a session, HAR or Charles file and opens it in a new tab.
pub fn open(har: bool, cx: &mut App) {
    let prompt: SharedString = if har {
        "Open a HAR file".into()
    } else {
        "Open a session, a HAR file or a Charles export (JSON or XML)".into()
    };
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(prompt),
    });
    cx.spawn(async move |cx| {
        let path = match picked.await {
            Ok(Ok(Some(paths))) => paths.into_iter().next(),
            Ok(Err(err)) => {
                cx.update(|cx| report_error("Couldn't show the open panel", err, cx));
                None
            }
            _ => None,
        };
        if let Some(path) = path {
            cx.update(|cx| open_path(path, cx));
        }
    })
    .detach();
}

/// Opens `path` (the dev remote's `openDocument`, a dropped file).
pub fn open_path(path: PathBuf, cx: &mut App) {
    with_workspace(cx, |w, cx| w.open_document(path, cx));
}

/// What Export HAR exports by default: the rows the table shows.
fn shown_rows(cx: &App) -> FlowSelection {
    flow_table::first(cx)
        .and_then(|t| t.read(cx).view())
        .map_or(FlowSelection::All, |view| FlowSelection::View { view, except: vec![] })
}

/// Asks where, then saves the active tab (all of it for a session, what
/// the table shows for HAR unless `selection` says otherwise).
pub fn save(kind: SaveKind, selection: Option<FlowSelection>, cx: &mut App) {
    let Some(ws) = Workspace::global(cx) else { return };
    let name = ws.read(cx).suggested_name(kind);
    let selection = selection.unwrap_or_else(|| match kind {
        SaveKind::Session => FlowSelection::All,
        SaveKind::Har => shown_rows(cx),
    });
    let directory = std::env::var_os("HOME").map_or_else(std::env::temp_dir, PathBuf::from);
    let picked = cx.prompt_for_new_path(&directory, Some(&name));
    cx.spawn(async move |cx| match picked.await {
        Ok(Ok(Some(path))) => cx.update(|cx| save_to(kind, selection, path, cx)),
        Ok(Err(err)) => cx.update(|cx| report_error("Couldn't show the save panel", err, cx)),
        _ => {}
    })
    .detach();
}

/// Saves to `path` without asking (the dev remote's `saveSession`).
pub fn save_to(kind: SaveKind, selection: FlowSelection, path: PathBuf, cx: &mut App) {
    with_workspace(cx, |w, cx| w.save_to(kind, selection, path, cx));
}

/// Export HAR to `path` of what the table shows.
pub fn export_shown(path: PathBuf, cx: &mut App) {
    let selection = shown_rows(cx);
    save_to(SaveKind::Har, selection, path, cx);
}
