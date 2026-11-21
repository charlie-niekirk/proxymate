//! Replay (G8, `lib/actions.ts`'s replay half): ⌘R sends the focused
//! request again as it was sent, Edit & Replay… opens it in Compose
//! (body decoded), and Replay in Order sends the selected requests one
//! after the other. The flow menu's Send slot gets Replay and Edit &
//! Replay…. A flow the app sends is recorded live and selected.

use gpui_kit::{App, AppContext as _};
use pm_model::{FlowId, LIVE_STORE};
use pm_view::compose::{NOT_SENDABLE, NOTHING_TO_REPLAY, is_sendable};

use crate::bridge::mutations;
use crate::features::clipboard::{info, report_error};
use crate::features::commands;
use crate::features::context_menu::Menu;
use crate::features::flow_menu::{self, RowContext, Slot};
use crate::features::inspector::actions::FlowRef;
use crate::workspace::Workspace;

/// Replay: sends the request again, as recorded.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ReplayFlow {
    pub flow: FlowRef,
}

/// Edit & Replay…: the request in Compose.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct EditReplayFlow {
    pub flow: FlowRef,
}

gpui_kit::actions!(proxymate, [ReplaySelection]);

pub fn register(cx: &mut App) {
    cx.on_action(|a: &ReplayFlow, cx| replay(a.flow, cx));
    cx.on_action(|a: &EditReplayFlow, cx| edit_and_replay(a.flow, cx));
    cx.on_action(|_: &ReplaySelection, cx| replay_selection(cx));
    let focused = |cx: &App| crate::features::inspector::focused(cx).is_some();
    commands::add(
        "replay",
        focused,
        |_, cx| {
            if let Some(flow) = sendable_focus(cx) {
                replay(flow, cx);
            }
        },
        cx,
    );
    commands::add(
        "edit-replay",
        focused,
        |_, cx| {
            if let Some(flow) = sendable_focus(cx) {
                edit_and_replay(flow, cx);
            }
        },
        cx,
    );
    commands::add(
        "replay-selection",
        |cx| crate::features::flow_table::first(cx).is_some_and(|t| t.read(cx).grid().read(cx).selection_count() > 0),
        |_, cx| replay_selection(cx),
        cx,
    );
    flow_menu::extend(Slot::Send, send_items, cx);
}

/// The flow menu's Replay and Edit & Replay… (off for tunnels and
/// WebSockets).
pub fn send_items(ctx: &RowContext, _: &App) -> Menu {
    let flow = ctx.flow();
    let row = &ctx.row;
    if is_sendable(&row.3, row.2) {
        Menu::new()
            .item("Replay", ReplayFlow { flow })
            .item("Edit & Replay…", EditReplayFlow { flow })
    } else {
        Menu::new()
            .disabled("Replay", ReplayFlow { flow })
            .disabled("Edit & Replay…", EditReplayFlow { flow })
    }
}

/// The focused flow, if it can be sent again (else a toast says why).
fn sendable_focus(cx: &mut App) -> Option<FlowRef> {
    let Some((store, id)) = crate::features::inspector::focused(cx) else {
        info(NOTHING_TO_REPLAY.0, Some(NOTHING_TO_REPLAY.1.into()), cx);
        return None;
    };
    if let Some(row) = crate::features::inspector::focused_row(cx)
        && !is_sendable(&row.3, row.2)
    {
        report_error(NOT_SENDABLE.0, NOT_SENDABLE.1, cx);
        return None;
    }
    Some(FlowRef { store, id })
}

/// Sends a recorded request again; the new flow is selected.
pub fn replay(flow: FlowRef, cx: &mut App) {
    mutations::replay(
        flow.store,
        flow.id,
        |answer, cx| match answer {
            Ok(id) => show_live_flow(id, cx),
            Err(err) => report_error("Replay failed", err, cx),
        },
        cx,
    );
}

/// Opens Compose with a recorded request (its body decoded).
pub fn edit_and_replay(flow: FlowRef, cx: &mut App) {
    let Some(engine) = Workspace::global(cx).and_then(|ws| ws.read(cx).engine().cloned()) else {
        return;
    };
    let task = cx.background_spawn(async move { engine.request_spec(flow.store, flow.id, true) });
    cx.spawn(async move |cx| {
        let spec = task.await;
        cx.update(|cx| match spec {
            Ok(spec) => crate::features::compose::open(Some(spec), cx),
            Err(err) => report_error("Can't edit this request", err, cx),
        });
    })
    .detach();
}

/// Replay in Order over the table's selection.
pub fn replay_selection(cx: &mut App) {
    let Some(table) = crate::features::flow_table::first(cx) else {
        return;
    };
    let (store, selection) = {
        let t = table.read(cx);
        (t.store(), t.grid().read(cx).flow_selection())
    };
    mutations::replay_in_order(
        store,
        selection,
        |answer, cx| match answer {
            Ok((last, failed, total)) => {
                if let Some(id) = last {
                    show_live_flow(id, cx);
                }
                if failed > 0 {
                    report_error(
                        "Some flows weren't replayed",
                        format!("{failed} of {total} couldn't be sent"),
                        cx,
                    );
                }
            }
            Err(err) => report_error("Replay failed", err, cx),
        },
        cx,
    );
}

/// Shows a flow the app just sent (or a paused one): the live tab, with it
/// selected. The row may land a delta later; the selection is by id.
pub fn show_live_flow(id: FlowId, cx: &mut App) {
    if let Some(ws) = Workspace::global(cx) {
        ws.update(cx, |w, cx| w.set_store(LIVE_STORE, cx));
    }
    // After the tab switch has reached the table.
    cx.defer(move |cx| {
        if let Some(table) = crate::shell::parts(cx).and_then(|p| p.table.upgrade()) {
            let grid = table.read(cx).grid().clone();
            grid.update(cx, |g, cx| g.select(Some(id), cx));
        }
    });
}
