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
use pm_model::{
    FlowSelection, LIVE_STORE, Rule, RuleGroupPatch, RuleOrigin, RulePatch, RuleSettings, StoreId, WsDirection,
    WsOpcode,
};
use pm_view::rules::{self, ThrottlePreset};

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

/// Turns a rule on or off (its checkbox, a rule badge's "Disable rule").
/// The rule book reaches the Rules window as a push (`bridge::rule_book`),
/// so rule writes have no cache to invalidate.
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

// ---- Rules (G7) ----

/// Saves a rule: new without an id (it gets one), else replacing the rule
/// with its id. `done` gets the rule as saved; a failure goes to `failed`
/// (the form shows it) rather than a toast.
pub fn save_rule(
    rule: Rule,
    done: impl FnOnce(Rule, &mut App) + 'static,
    failed: impl FnOnce(String, &mut App) + 'static,
    cx: &mut App,
) {
    let Some(engine) = engine(cx) else { return };
    let task = cx.background_spawn(async move {
        if rule.id.is_empty() {
            engine.create_rule(rule)
        } else {
            engine.update_rule(rule)
        }
    });
    cx.spawn(async move |cx| {
        let answer = task.await;
        cx.update(|cx| match answer {
            Ok(rule) => done(rule, cx),
            Err(err) => failed(err.to_string(), cx),
        });
    })
    .detach();
}

/// Changes some of a rule's fields in one step (the throttle presets).
pub fn patch_rule(id: String, patch: RulePatch, what: &'static str, cx: &mut App) {
    run(what, move |e| e.patch_rule(&id, patch), |_, _| {}, cx);
}

/// The toolbar's network conditions: the built-in throttle rule set to a
/// preset, or off.
pub fn set_throttle(preset: Option<ThrottlePreset>, cx: &mut App) {
    patch_rule(
        rules::THROTTLE_RULE_ID.into(),
        rules::throttle_patch(preset.as_ref()),
        "Couldn't change the network conditions",
        cx,
    );
}

pub fn delete_rule(id: String, done: impl FnOnce(&mut App) + 'static, cx: &mut App) {
    run(
        "Couldn't delete the rule",
        move |e| e.delete_rule(&id),
        move |(), cx| done(cx),
        cx,
    );
}

/// Deletes every rule an agent made, then says how many.
pub fn delete_agent_rules(cx: &mut App) {
    run(
        "Couldn't remove the agent rules",
        |e| e.delete_agent_rules(),
        |n, cx| {
            clipboard::info(
                &format!("Removed {n} agent rule{}", if n == 1 { "" } else { "s" }),
                None,
                cx,
            );
        },
        cx,
    );
}

/// Puts the listed rules first, in that order (each stays in its section).
pub fn reorder_rules(ids: Vec<String>, cx: &mut App) {
    run("Couldn't move the rule", move |e| e.reorder_rules(&ids), |_, _| {}, cx);
}

/// Moves rules to the end of a group, or out of any (`None`).
pub fn move_rules_to_group(ids: Vec<String>, group: Option<String>, cx: &mut App) {
    run(
        "Couldn't move the rule",
        move |e| e.move_rules_to_group(&ids, group.as_deref()),
        |_, _| {},
        cx,
    );
}

/// Drops `moving` on another rule: into that rule's group first, then into
/// its place (`dropOnRule`).
pub fn drop_rule(moving: Rule, target_group: Option<String>, order: Vec<String>, cx: &mut App) {
    run(
        "Couldn't move the rule",
        move |e| {
            if moving.group != target_group {
                e.move_rules_to_group(std::slice::from_ref(&moving.id), target_group.as_deref())?;
            }
            e.reorder_rules(&order)
        },
        |_, _| {},
        cx,
    );
}

/// A new group, then `rules` moved into it; `done` gets its id.
pub fn create_rule_group(rules: Vec<String>, done: impl FnOnce(String, &mut App) + 'static, cx: &mut App) {
    run(
        "Couldn't create the group",
        move |e| {
            let group = e.create_rule_group("New group", true, RuleOrigin::User)?;
            if !rules.is_empty() {
                e.move_rules_to_group(&rules, Some(&group.id))?;
            }
            Ok(group.id)
        },
        done,
        cx,
    );
}

/// Renames a group or turns it on or off.
pub fn patch_rule_group(id: String, patch: RuleGroupPatch, cx: &mut App) {
    run(
        "Couldn't change the group",
        move |e| e.patch_rule_group(&id, patch),
        |_, _| {},
        cx,
    );
}

/// Deletes a group, and its rules with `delete_rules` (else they stay,
/// ungrouped).
pub fn delete_rule_group(id: String, delete_rules: bool, cx: &mut App) {
    run(
        "Couldn't delete the group",
        move |e| e.delete_rule_group(&id, delete_rules),
        |_, _| {},
        cx,
    );
}

pub fn reorder_rule_groups(ids: Vec<String>, cx: &mut App) {
    run(
        "Couldn't move the group",
        move |e| e.reorder_rule_groups(&ids),
        |_, _| {},
        cx,
    );
}

/// The breakpoint timeout (0 waits forever); the settings cache takes the
/// engine's answer.
pub fn set_rule_settings(settings: RuleSettings, cx: &mut App) {
    run(
        "Couldn't set the timeout",
        move |e| e.set_rule_settings(settings),
        |saved, cx| {
            let caches = keys::rules_caches(cx);
            caches.settings.update(cx, |c, cx| c.set((), saved, cx));
        },
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
