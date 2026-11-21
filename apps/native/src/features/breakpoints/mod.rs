//! The Breakpoint Editor (G8, `Breakpoints/BreakpointEditor.tsx`): a
//! window with the queue of paused flows, each with its countdown and
//! menu, beside the selected one's form. A paused request or response is
//! edited (head, headers, body as code or a tree) and then executed,
//! aborted, or answered directly (Respond…); a held WebSocket message is
//! edited as text, a tree or hex, then sent or dropped. The first
//! resolution wins, from here or an agent.
//!
//! The queue is the engine's push (`bridge::breakpoints`); the window
//! opens itself when something pauses and closes when nothing is left, as
//! the Tauri dialog did. The toolbar's badge (G7) and ⌘K "Breakpoints…"
//! open it too.

pub mod form;
pub mod message;
pub mod window;

use gpui_kit::App;
use pm_model::{BreakpointId, BreakpointResolution, FlowId};
use pm_view::breakpoints::{QueueChange, queue_change};

use crate::bridge::{self, mutations};
use crate::features::clipboard::report_error;
use crate::features::rules::OpenBreakpoints;

pub use window::BreakpointsWindow;

/// The queue menu's "Continue unmodified".
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ContinueBreakpoint {
    pub id: BreakpointId,
}

/// The queue menu's "Abort" / "Drop the message".
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct AbortBreakpoint {
    pub id: BreakpointId,
}

/// The queue menu's "Show flow #n": the live tab, with it selected.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ShowPausedFlow {
    pub id: FlowId,
}

pub fn register(cx: &mut App) {
    cx.on_action(|_: &OpenBreakpoints, cx| open(cx));
    cx.on_action(|a: &ContinueBreakpoint, cx| {
        resolve_from_menu(
            a.id,
            BreakpointResolution::Continue { edits: None },
            "Couldn't continue the flow",
            cx,
        )
    });
    cx.on_action(|a: &AbortBreakpoint, cx| {
        resolve_from_menu(a.id, BreakpointResolution::Abort, "Couldn't abort the flow", cx)
    });
    cx.on_action(|a: &ShowPausedFlow, cx| crate::features::replay::show_live_flow(a.id, cx));
    // Only with an engine: tests drive the queue by hand (`watch_queue`).
    if cx.try_global::<bridge::Bridge>().is_some() {
        watch_queue(cx);
    }
}

fn resolve_from_menu(id: BreakpointId, resolution: BreakpointResolution, what: &'static str, cx: &mut App) {
    mutations::resolve_breakpoint(
        id,
        resolution,
        move |answer, cx| {
            if let Err(err) = answer {
                report_error(what, err, cx);
            }
        },
        cx,
    );
}

/// Opens the window (or brings it forward).
pub fn open(cx: &mut App) {
    // Deferred: the action that asked is still being dispatched.
    cx.defer(window::open_or_focus);
}

/// Opens the window when something pauses and closes it once the queue is
/// empty again.
pub fn watch_queue(cx: &mut App) {
    let queue = bridge::breakpoints(cx);
    let mut before = queue.read(cx).value.as_ref().map_or(0, Vec::len);
    cx.observe(&queue, move |queue, cx| {
        let after = queue.read(cx).value.as_ref().map_or(0, Vec::len);
        match queue_change(before, after) {
            QueueChange::Open => open(cx),
            QueueChange::Close => cx.defer(window::close),
            QueueChange::Stay => {}
        }
        before = after;
    })
    .detach();
}
