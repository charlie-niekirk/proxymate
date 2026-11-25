//! The dev remote's editing verbs (G8); see the list in `dev/mod.rs`.
//! Breakpoint verbs answer with the Breakpoint Editor's state (the queue,
//! the shown flow, what its form would send, its error); Compose verbs with
//! Compose's (its draft, error and the flow it sent).

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{AnyWindowHandle, App, AsyncApp, Entity};
use pm_model::{Header, MessageEdits, RequestSpec};
use pm_view::breakpoints::{MessageMode, countdown};
use pm_view::kv::KvRow;
use serde_json::{Value, json};

use crate::features::breakpoints::form::HttpForm;
use crate::features::breakpoints::message::MessageForm;
use crate::features::breakpoints::window::{BreakpointsWindow, Pane, close as close_breakpoints, now_ms};
use crate::features::compose::{self, ComposeWindow};
use crate::features::editing::body::BodyMode;
use crate::features::inspector::actions::FlowRef;
use crate::features::replay;

const VERBS: [&str; 16] = [
    "breakpoints",
    "breakpoint",
    "breakpointSelect",
    "breakpointEdit",
    "breakpointKeys",
    "breakpointType",
    "breakpointAction",
    "breakpointMenu",
    "breakpointScreenshot",
    "replay",
    "editReplay",
    "replaySelection",
    "compose",
    "composeSend",
    "composeEdit",
    "composeScreenshot",
];

async fn until(limit: Duration, cx: &mut AsyncApp, done: impl Fn(&App) -> bool) {
    let step = Duration::from_millis(25);
    let mut waited = Duration::ZERO;
    while waited < limit && !cx.update(|cx| done(cx)) {
        cx.background_executor().timer(step).await;
        waited += step;
    }
}

fn breakpoints_window(cx: &App) -> Option<(AnyWindowHandle, Entity<BreakpointsWindow>)> {
    BreakpointsWindow::global(cx)
}

fn pane(cx: &App) -> Option<Pane> {
    breakpoints_window(cx).and_then(|(_, w)| w.read(cx).pane().cloned())
}

fn rows_of(value: &Value) -> Option<Vec<KvRow>> {
    value.as_array().map(|rows| {
        rows.iter()
            .filter_map(|r| Some(KvRow::new(r.get(0)?.as_str()?, r.get(1)?.as_str()?)))
            .collect()
    })
}

/// Sets each `[name, value]`: the first row of that name gets the value,
/// a name not there is added.
fn set_headers(
    h: &mut pm_widgets::kv_editor::KvEditor,
    rows: &[KvRow],
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<pm_widgets::kv_editor::KvEditor>,
) {
    for row in rows {
        match h
            .rows()
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(&row.name))
            .map(|r| (r.id, r.name.clone()))
        {
            Some((id, name)) => h.set(id, &name, &row.value, window, cx),
            None => h.push(&row.name, &row.value, cx),
        }
    }
}

fn headers_json(headers: &[Header]) -> Value {
    headers.iter().map(|h| json!([h.name, h.value])).collect()
}

fn edits_json(edits: &Result<MessageEdits, String>) -> Value {
    match edits {
        Ok(e) => serde_json::to_value(e).unwrap_or(Value::Null),
        Err(err) => json!({ "error": err }),
    }
}

fn body_mode(form: &HttpForm, cx: &App) -> Option<&'static str> {
    form.body().map(|b| match b.read(cx).mode() {
        BodyMode::Text => "text",
        BodyMode::Tree => "tree",
    })
}

fn breakpoint_state(cx: &App) -> Value {
    let Some((_, view)) = breakpoints_window(cx) else {
        return json!({ "open": false });
    };
    let w = view.read(cx);
    let now = now_ms();
    let queue: Vec<Value> = w
        .queue(cx)
        .iter()
        .map(|b| {
            json!({
                "id": b.id,
                "flow": b.flow_id,
                "phase": pm_view::breakpoints::phase_label(b.phase),
                "method": b.request.method,
                "url": b.request.url,
                "rule": b.rule_name,
                "message": b.message.is_some(),
                "countdown": countdown(b.deadline_ms, now),
            })
        })
        .collect();
    let form = match w.pane() {
        Some(Pane::Http(form)) => {
            let f = form.read(cx);
            json!({
                "kind": "http",
                "id": f.bp.id,
                "edits": edits_json(&f.edits(cx)),
                "bodyMode": body_mode(f, cx),
                "body": f.body().map(|b| b.read(cx).text().to_owned()),
                "responding": f.is_responding(),
                "error": f.error(),
            })
        }
        Some(Pane::Message(form)) => {
            let f = form.read(cx);
            json!({
                "kind": "message",
                "id": f.bp.id,
                "mode": f.draft().map(|d| d.mode.id()),
                "modes": f.draft().map(|d| d.modes().iter().map(|m| m.id()).collect::<Vec<_>>()),
                "text": f.draft().map(|d| d.current_text()),
                "edits": edits_json(&f.edits()),
                "error": f.error(),
            })
        }
        None => Value::Null,
    };
    json!({ "open": true, "queue": queue, "shown": w.shown(cx).map(|b| b.id), "form": form })
}

fn compose_state(cx: &App) -> Value {
    let Some((_, view)) = ComposeWindow::global(cx) else {
        return json!({ "open": false });
    };
    let w = view.read(cx);
    let draft = w.draft(cx);
    let spec = draft.spec();
    json!({
        "open": true,
        "title": if w.is_edit_replay() { "Edit & Replay" } else { "Compose" },
        "method": spec.method,
        "url": spec.url,
        "headers": headers_json(&spec.headers),
        "body": spec.body,
        "binary": spec.body_base64,
        "contentType": format!("{:?}", draft.content_type_choice()),
        "applyRules": w.apply_rules(),
        "error": w.error(),
        "sent": w.sent,
    })
}

fn http_edit(form: &Entity<HttpForm>, edit: &Value, window: AnyWindowHandle, cx: &mut AsyncApp) -> Result<(), String> {
    let edit = edit.clone();
    window
        .update(cx, |_, window, cx| {
            form.update(cx, |f, cx| {
                let s = |k: &str| edit.get(k).and_then(Value::as_str);
                f.set_head(s("method"), s("url"), s("status"), window, cx);
                if let Some(rows) = edit.get("headers").and_then(rows_of) {
                    f.headers().update(cx, |h, cx| set_headers(h, &rows, window, cx));
                }
                if let Some(names) = edit.get("removeHeaders").and_then(Value::as_array) {
                    f.headers().update(cx, |h, cx| {
                        for name in names.iter().filter_map(Value::as_str) {
                            let ids: Vec<u64> = h
                                .rows()
                                .iter()
                                .filter(|r| r.name.eq_ignore_ascii_case(name))
                                .map(|r| r.id)
                                .collect();
                            for id in ids {
                                h.remove(id, cx);
                            }
                        }
                    });
                }
                if let Some(text) = s("body")
                    && let Some(body) = f.body().cloned()
                {
                    body.update(cx, |b, cx| b.set_text(text.to_owned(), window, cx));
                }
                if let Some(mode) = s("bodyMode")
                    && let Some(body) = f.body().cloned()
                {
                    let mode = if mode == "tree" { BodyMode::Tree } else { BodyMode::Text };
                    body.update(cx, |b, cx| b.set_mode(mode, window, cx));
                }
                if edit.get("respond").and_then(Value::as_bool) == Some(true) {
                    f.start_responding(window, cx);
                }
                if let Some(answer) = edit.get("answer") {
                    let a = |k: &str| answer.get(k).and_then(Value::as_str);
                    if let Some(rows) = answer.get("headers").and_then(rows_of) {
                        f.answer_headers().update(cx, |h, cx| set_headers(h, &rows, window, cx));
                    }
                    if let Some(body) = a("body")
                        && let Some(editor) = f.answer_body().cloned()
                    {
                        editor.update(cx, |b, cx| b.set_text(body.to_owned(), window, cx));
                    }
                    if let Some(status) = a("status") {
                        f.set_answer_status(status, window, cx);
                    }
                }
            })
        })
        .map_err(|e| e.to_string())
}

fn message_edit(
    form: &Entity<MessageForm>,
    edit: &Value,
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let edit = edit.clone();
    window
        .update(cx, |_, window, cx| {
            form.update(cx, |f, cx| {
                if let Some(mode) = edit
                    .get("messageMode")
                    .and_then(Value::as_str)
                    .and_then(MessageMode::from_id)
                {
                    f.switch_to(mode, window, cx);
                }
                if let Some(payload) = edit.get("payload").and_then(Value::as_str) {
                    f.set_payload(payload, window, cx);
                }
            })
        })
        .map_err(|e| e.to_string())
}

/// The JsonEditor that has the keyboard in the Breakpoint Editor, if any.
fn focused_tree(cx: &App) -> Option<Entity<pm_widgets::json_editor::JsonEditor>> {
    match pane(cx)? {
        Pane::Http(f) => f.read(cx).body()?.read(cx).tree().cloned(),
        Pane::Message(f) => f.read(cx).tree().cloned(),
    }
}

pub async fn command(command: &Value, cx: &mut AsyncApp) -> Option<Value> {
    if !VERBS.iter().any(|v| command.get(v).is_some()) {
        return None;
    }
    let mut extra = serde_json::Map::new();
    let mut compose_verb = false;

    // ---- Breakpoints ----
    if let Some(open) = command.get("breakpoints").and_then(Value::as_bool) {
        if open {
            cx.update(crate::features::breakpoints::open);
            until(Duration::from_secs(2), cx, |cx| breakpoints_window(cx).is_some()).await;
        } else {
            cx.update(close_breakpoints);
        }
    }
    if command.get("breakpoint").is_some() || command.get("breakpointSelect").is_some() {
        // Wait for the queue's push and the form's body.
        until(Duration::from_secs(2), cx, |cx| pane(cx).is_some()).await;
    }
    if let Some(id) = command.get("breakpointSelect").and_then(Value::as_u64) {
        let Some((handle, view)) = cx.update(|cx| breakpoints_window(cx)) else {
            return Some(json!({"error": "the Breakpoint Editor isn't open"}));
        };
        let _ = handle.update(cx, |_, window, cx| {
            view.update(cx, |w, cx| w.select(id as u32, window, cx))
        });
    }
    if let Some(edit) = command.get("breakpointEdit") {
        let Some((handle, _)) = cx.update(|cx| breakpoints_window(cx)) else {
            return Some(json!({"error": "the Breakpoint Editor isn't open"}));
        };
        // The body arrives from the engine a moment after the form opens.
        until(Duration::from_secs(2), cx, |cx| match pane(cx) {
            Some(Pane::Http(f)) => f.read(cx).body().is_some(),
            Some(Pane::Message(f)) => f.read(cx).draft().is_some(),
            None => false,
        })
        .await;
        let result = match cx.update(|cx| pane(cx)) {
            Some(Pane::Http(form)) => http_edit(&form, edit, handle, cx),
            Some(Pane::Message(form)) => message_edit(&form, edit, handle, cx),
            None => Err("nothing is paused".into()),
        };
        if let Err(err) = result {
            return Some(json!({ "error": err }));
        }
    }
    if let Some(keys) = command.get("breakpointKeys").and_then(Value::as_str) {
        let Some((handle, _)) = cx.update(|cx| breakpoints_window(cx)) else {
            return Some(json!({"error": "the Breakpoint Editor isn't open"}));
        };
        let keys = keys.to_owned();
        let tree = cx.update(|cx| focused_tree(cx));
        let done = handle.update(cx, |_, window, cx| -> Result<(), String> {
            // The keys go to the tree, focused first.
            if let Some(tree) = &tree {
                let focus = gpui_kit::Focusable::focus_handle(tree.read(cx), cx);
                if !focus.contains_focused(window, cx) {
                    window.focus(&focus, cx);
                }
            }
            for key in keys.split_whitespace() {
                let key: &'static str = Box::leak(key.to_owned().into_boxed_str());
                let stroke = gpui_kit::Keystroke::parse(key).map_err(|e| format!("{key}: {e:?}"))?;
                window.dispatch_keystroke(stroke, cx);
            }
            Ok(())
        });
        if let Ok(Err(err)) = done {
            return Some(json!({ "error": err }));
        }
    }
    if let Some(text) = command.get("breakpointType").and_then(Value::as_str) {
        let (Some((handle, _)), Some(tree)) =
            (cx.update(|cx| breakpoints_window(cx)), cx.update(|cx| focused_tree(cx)))
        else {
            return Some(json!({"error": "no JSON tree is shown"}));
        };
        let text = text.to_owned();
        let _ = handle.update(cx, |_, window, cx| {
            tree.update(cx, |t, cx| {
                t.type_text(&text, window, cx);
                t.commit_input(window, cx);
            })
        });
    }
    if let Some(action) = command.get("breakpointAction") {
        until(Duration::from_secs(2), cx, |cx| pane(cx).is_some()).await;
        let Some((handle, _)) = cx.update(|cx| breakpoints_window(cx)) else {
            return Some(json!({"error": "the Breakpoint Editor isn't open"}));
        };
        let execute = match action.get("execute") {
            Some(Value::Object(_)) => match serde_json::from_value::<MessageEdits>(action["execute"].clone()) {
                Ok(edits) => Some(Some(edits)),
                Err(err) => return Some(json!({ "error": format!("execute: {err}") })),
            },
            Some(Value::Bool(true)) => Some(None),
            _ => None,
        };
        let abort = action.get("abort").and_then(Value::as_bool) == Some(true);
        let respond = action.get("respond").and_then(Value::as_bool) == Some(true);
        let resolved =
            cx.update(|cx| pane(cx).map(|p| (p, breakpoints_window(cx).and_then(|(_, w)| w.read(cx).shown(cx)))));
        let Some((pane, shown)) = resolved else {
            return Some(json!({"error": "nothing is paused"}));
        };
        let _ = handle.update(cx, |_, _, cx| match pane {
            Pane::Http(form) => form.update(cx, |f, cx| {
                if let Some(extra) = execute {
                    f.execute(extra, cx);
                } else if abort {
                    f.abort(cx);
                } else if respond {
                    f.respond(cx);
                }
            }),
            Pane::Message(form) => form.update(cx, |f, cx| {
                if let Some(extra) = execute {
                    f.send(extra, cx);
                } else if abort {
                    f.drop_message(cx);
                }
            }),
        });
        // Until the queue drops it (or the form shows an error).
        let id = shown.map(|b| b.id);
        until(Duration::from_secs(3), cx, |cx| {
            let gone =
                breakpoints_window(cx).is_none_or(|(_, w)| !w.read(cx).queue(cx).iter().any(|b| Some(b.id) == id));
            let failed = match self::pane(cx) {
                Some(Pane::Http(f)) => f.read(cx).error().is_some(),
                Some(Pane::Message(f)) => f.read(cx).error().is_some(),
                None => false,
            };
            gone || failed
        })
        .await;
        extra.insert("resolved".into(), json!(id));
    }
    if let Some(id) = command.get("breakpointMenu").and_then(Value::as_u64) {
        let Some((handle, view)) = cx.update(|cx| breakpoints_window(cx)) else {
            return Some(json!({"error": "the Breakpoint Editor isn't open"}));
        };
        let run = command.get("run").and_then(Value::as_str).map(str::to_owned);
        let answer = handle.update(cx, |_, window, cx| {
            let Some(menu) = view.read(cx).menu_for(id as u32, cx) else {
                return json!({"error": "no such paused flow"});
            };
            let ran = run.as_deref().map(|label| menu.run(label, window, cx));
            json!({ "menu": menu.to_json(), "ran": ran })
        });
        if let Ok(answer) = answer {
            extra.insert("menu".into(), answer);
        }
    }
    if let Some(path) = command.get("breakpointScreenshot").and_then(Value::as_str) {
        let Some((handle, _)) = cx.update(|cx| breakpoints_window(cx)) else {
            return Some(json!({"error": "the Breakpoint Editor isn't open"}));
        };
        let shot = super::screenshot(handle, PathBuf::from(path), false, cx).await;
        extra.insert("screenshot".into(), shot);
    }

    // ---- Replay ----
    let store =
        cx.update(|cx| crate::workspace::Workspace::global(cx).map_or(pm_model::LIVE_STORE, |w| w.read(cx).store));
    if let Some(id) = command.get("replay").and_then(Value::as_u64) {
        let flow = FlowRef { store, id: id as u32 };
        cx.update(|cx| replay::replay(flow, cx));
        cx.background_executor().timer(Duration::from_millis(300)).await;
        extra.insert(
            "selected".into(),
            json!(cx.update(|cx| crate::features::inspector::focused(cx).map(|(_, id)| id))),
        );
    }
    if command.get("replaySelection").and_then(Value::as_bool) == Some(true) {
        cx.update(replay::replay_selection);
        cx.background_executor().timer(Duration::from_millis(500)).await;
        extra.insert(
            "selected".into(),
            json!(cx.update(|cx| crate::features::inspector::focused(cx).map(|(_, id)| id))),
        );
    }
    if let Some(id) = command.get("editReplay").and_then(Value::as_u64) {
        let flow = FlowRef { store, id: id as u32 };
        cx.update(|cx| replay::edit_and_replay(flow, cx));
        until(Duration::from_secs(2), cx, |cx| ComposeWindow::global(cx).is_some()).await;
        compose_verb = true;
    }

    // ---- Compose ----
    if let Some(spec) = command.get("compose") {
        compose_verb = true;
        let spec = match spec {
            Value::Null => None,
            other => {
                let mut base = json!({"method": "GET", "url": "", "headers": [], "body": null, "body_base64": false});
                if let (Value::Object(b), Value::Object(o)) = (&mut base, other) {
                    b.extend(o.clone());
                }
                match serde_json::from_value::<RequestSpec>(base) {
                    Ok(spec) => Some(spec),
                    Err(err) => return Some(json!({ "error": format!("compose: {err}") })),
                }
            }
        };
        cx.update(|cx| compose::open(spec, cx));
        until(Duration::from_secs(2), cx, |cx| ComposeWindow::global(cx).is_some()).await;
    }
    if let Some(edit) = command.get("composeEdit") {
        compose_verb = true;
        let Some((handle, view)) = cx.update(|cx| ComposeWindow::global(cx)) else {
            return Some(json!({"error": "Compose isn't open"}));
        };
        let edit = edit.clone();
        let _ = handle.update(cx, |_, window, cx| {
            view.update(cx, |w, cx| {
                if let Some(ct) = edit.get("contentType") {
                    w.set_content_type(ct.as_str(), window, cx);
                }
                if let Some(on) = edit.get("applyRules").and_then(Value::as_bool) {
                    w.set_apply_rules(on, cx);
                }
                if let Some(m) = edit.get("method").and_then(Value::as_str) {
                    w.set_method(m, window, cx);
                }
                if edit.get("replaceBinary").and_then(Value::as_bool) == Some(true) {
                    w.replace_binary(cx);
                }
            })
        });
    }
    if command.get("composeSend").and_then(Value::as_bool) == Some(true) {
        compose_verb = true;
        let Some((handle, view)) = cx.update(|cx| ComposeWindow::global(cx)) else {
            return Some(json!({"error": "Compose isn't open"}));
        };
        let _ = handle.update(cx, |_, _, cx| view.update(cx, |w, cx| w.send(cx)));
        until(Duration::from_secs(3), cx, |cx| {
            ComposeWindow::global(cx).is_none_or(|(_, w)| w.read(cx).error().is_some() || w.read(cx).sent.is_some())
        })
        .await;
        cx.background_executor().timer(Duration::from_millis(200)).await;
        let failed = cx.update(|cx| ComposeWindow::global(cx).and_then(|(_, w)| w.read(cx).error().map(str::to_owned)));
        extra.insert("sendError".into(), json!(failed));
        extra.insert(
            "selected".into(),
            json!(cx.update(|cx| crate::features::inspector::focused(cx).map(|(_, id)| id))),
        );
    }
    if let Some(path) = command.get("composeScreenshot").and_then(Value::as_str) {
        compose_verb = true;
        let Some((handle, _)) = cx.update(|cx| ComposeWindow::global(cx)) else {
            return Some(json!({"error": "Compose isn't open"}));
        };
        let shot = super::screenshot(handle, PathBuf::from(path), false, cx).await;
        extra.insert("screenshot".into(), shot);
    }

    let mut answer = if compose_verb {
        cx.update(|cx| compose_state(cx))
    } else {
        cx.update(|cx| breakpoint_state(cx))
    };
    if let Value::Object(map) = &mut answer {
        map.extend(extra);
    }
    Some(answer)
}
