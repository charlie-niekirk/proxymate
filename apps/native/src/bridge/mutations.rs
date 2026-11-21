//! Engine writes and what each makes stale (the port of `lib/mutations.ts`,
//! R4's rule): a write runs on the background executor, then invalidates
//! the caches it changed, or reports its failure as a toast. Views call
//! these, never the engine's write methods directly, so a cache can't be
//! forgotten. Phases add their writes here.
//!
//! The SSL lists are the workspace's state (the sidebar's menus tick them),
//! so their writes go through [`Workspace`], which stores the engine's
//! answer.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _};
use pm_core::Engine;
use pm_model::{FlowSelection, LIVE_STORE, StoreId, WsDirection, WsOpcode};

use super::keys;
use crate::features::clipboard::{self, report_error};
use crate::workspace::Workspace;

fn engine(cx: &App) -> Option<Arc<Engine>> {
    Workspace::global(cx).and_then(|ws| ws.read(cx).engine().cloned())
}

/// Runs `write` off the foreground thread, then `done` with its answer, or
/// reports `what` failed.
fn run<T: Send + 'static>(
    what: &'static str,
    write: impl FnOnce(&Engine) -> Result<T, pm_core::Error> + Send + 'static,
    done: impl FnOnce(T, &mut App) + 'static,
    cx: &mut App,
) {
    let Some(engine) = engine(cx) else { return };
    let task = cx.background_spawn(async move { write(&engine) });
    cx.spawn(async move |cx| {
        let answer = task.await;
        cx.update(|cx| match answer {
            Ok(answer) => done(answer, cx),
            Err(err) => report_error(what, err, cx),
        });
    })
    .detach();
}

/// Pins or unpins flows: pinned live flows are kept when old flows are
/// evicted. A live pin reaches the view as a changed row; a document's
/// doesn't, so its cached flows are refetched.
pub fn pin_flows(store: StoreId, selection: FlowSelection, pinned: bool, cx: &mut App) {
    run(
        if pinned { "Pin failed" } else { "Unpin failed" },
        move |e| e.pin_flows_in(store, &selection, pinned),
        move |_, cx| keys::store_changed(store, cx),
        cx,
    );
}

/// Deletes flows; the view's structural delta drops them from the table
/// and the caches.
pub fn remove_flows(
    store: StoreId,
    selection: FlowSelection,
    done: impl FnOnce(u32, &mut App) + 'static,
    cx: &mut App,
) {
    run(
        "Delete failed",
        move |e| e.remove_flows_in(store, &selection),
        move |n, cx| {
            if store != LIVE_STORE {
                keys::store_changed(store, cx);
            }
            done(n, cx);
        },
        cx,
    );
}

/// Stops recording `host` (its traffic is still proxied), then says so.
pub fn never_record(host: String, cx: &mut App) {
    let shown = host.clone();
    run(
        "Can't change the recording lists",
        move |e| e.never_record(&host),
        move |_, cx| {
            clipboard::success_with_action(
                &format!("Not recording {shown}"),
                "Its traffic is still proxied. Settings › Storage lists the hosts that aren't recorded.",
                "Settings",
                Box::new(crate::features::chrome::OpenSettings { page: "storage".into() }),
                cx,
            );
        },
        cx,
    );
}

/// Turns a rule on or off (a rule badge's "Disable rule"). G7's Rules
/// window adds the rules cache this invalidates.
pub fn set_rule_enabled(id: String, enabled: bool, cx: &mut App) {
    run(
        if enabled {
            "Couldn't enable the rule"
        } else {
            "Couldn't disable the rule"
        },
        move |e| e.set_rule_enabled(&id, enabled),
        |_, _| {},
        cx,
    );
}

/// Stops decrypting `host`; its next CONNECTs are tunnelled.
pub fn add_ssl_bypass(host: String, cx: &mut App) {
    if let Some(ws) = Workspace::global(cx) {
        ws.update(cx, |w, cx| w.add_ssl_bypass(host, cx));
    }
}

/// Accepts (or stops accepting) `host`'s upstream certificate unverified.
pub fn set_insecure_upstream(host: String, allow: bool, cx: &mut App) {
    if let Some(ws) = Workspace::global(cx) {
        ws.update(cx, |w, cx| w.set_insecure_upstream(host, allow, cx));
    }
}

/// Sends a message into an open WebSocket of the live capture (the
/// composer): `done` gets the engine's answer, so the composer can show an
/// error inline. The flow's lists go stale either way.
pub fn ws_send(
    flow: keys::FlowKey,
    message: pm_model::WsSend,
    done: impl FnOnce(Result<(), String>, &mut App) + 'static,
    cx: &mut App,
) {
    let Some(engine) = engine(cx) else {
        // After the caller's update, which `done` may want to update again.
        cx.defer(move |cx| done(Err("No engine".into()), cx));
        return;
    };
    let task = cx.background_spawn(async move { engine.ws_send(flow.id, &message) });
    cx.spawn(async move |cx| {
        let answer = task.await.map_err(|e| e.to_string());
        cx.update(|cx| {
            keys::websocket_changed(flow, cx);
            done(answer, cx);
        });
    })
    .detach();
}

/// Resend: message `index` again, its payload as it was forwarded, in its
/// direction. Failures toast.
pub fn ws_resend(flow: keys::FlowKey, index: u32, direction: WsDirection, opcode: WsOpcode, cx: &mut App) {
    run(
        "Couldn't resend the message",
        move |e| {
            let payload = e.websocket_payload_in(flow.store, flow.id, index)?;
            e.ws_send(flow.id, &pm_view::stream::resend_of(direction, opcode, &payload))
        },
        move |_, cx| keys::websocket_changed(flow, cx),
        cx,
    );
}

/// Closes an open WebSocket of the live capture: a normal close frame to
/// each side.
pub fn ws_close(flow: keys::FlowKey, cx: &mut App) {
    run(
        "Couldn't close the connection",
        move |e| e.ws_close(flow.id),
        move |_, cx| keys::websocket_changed(flow, cx),
        cx,
    );
}
