//! Capture commands (`lib/commands.ts`, Capture group): Clear all flows
//! (⌘⌫, live capture only), Start/Stop recording (⌘E, whichever applies)
//! and Toggle Auto Select. The toolbar and footer buttons run the same
//! actions.

use gpui_kit::App;

use crate::features::commands::{self, always};
use crate::features::documents::with_workspace;
use crate::features::flow_table;
use crate::workspace::Workspace;

gpui_kit::actions!(proxymate, [ClearFlows, ToggleRecording, ToggleAutoSelect]);

pub fn register(cx: &mut App) {
    cx.on_action(|_: &ClearFlows, cx| clear(cx));
    cx.on_action(|_: &ToggleRecording, cx| toggle_recording(cx));
    cx.on_action(|_: &ToggleAutoSelect, cx| toggle_auto_select(cx));
    commands::add("clear", is_live, |_, cx| clear(cx), cx);
    commands::add("record-start", |cx| !recording(cx), |_, cx| toggle_recording(cx), cx);
    commands::add("record-stop", recording, |_, cx| toggle_recording(cx), cx);
    commands::add("auto-select", always, |_, cx| toggle_auto_select(cx), cx);
}

fn is_live(cx: &App) -> bool {
    Workspace::global(cx).is_some_and(|w| w.read(cx).is_live())
}

/// Whether the proxy records; recording is the default.
pub fn recording(cx: &App) -> bool {
    Workspace::global(cx).is_none_or(|w| w.read(cx).proxy.recording)
}

/// Clears every live flow (nothing on a document tab).
pub fn clear(cx: &mut App) {
    if !is_live(cx) {
        return;
    }
    if let Some(table) = flow_table::first(cx) {
        let grid = table.read(cx).grid().clone();
        grid.update(cx, |g, cx| g.select(None, cx));
    }
    with_workspace(cx, |w, cx| w.clear_flows(cx));
}

pub fn toggle_recording(cx: &mut App) {
    let on = !recording(cx);
    with_workspace(cx, |w, cx| w.set_recording(on, cx));
}

pub fn toggle_auto_select(cx: &mut App) {
    if let Some(table) = flow_table::first(cx) {
        let grid = table.read(cx).grid().clone();
        grid.update(cx, |g, cx| {
            let on = !g.auto_select();
            g.set_auto_select(on, cx);
        });
    }
}
