//! The dev remote's Rules verbs (G7); see the list in `dev/mod.rs`. Each
//! answers with the Rules window's state: the listed rules and groups, what
//! the form shows (its rule, unsaved, invalid, the dry run), the toolbar's
//! tools and the last toast.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{App, AsyncApp, ElementId};
use pm_model::PresetKind;
use pm_view::rules::{self, edit};
use serde_json::{Value, json};

use crate::features::inspector::actions::FlowRef;
use crate::features::rules::form::RuleForm;
use crate::features::rules::tools::ToolState;
use crate::features::rules::{self as feature, RulesWindow, Target};

const VERBS: [&str; 17] = [
    "rules",
    "openRule",
    "newRule",
    "preset",
    "ruleEdit",
    "saveRule",
    "testRule",
    "deleteRule",
    "throttle",
    "noCache",
    "noCookies",
    "breakpointTimeout",
    "rulesMenu",
    "rulesClick",
    "rulesScreenshot",
    "rulesSelect",
    "rulesRename",
];

fn dispatch(action: Box<dyn gpui_kit::Action>, cx: &mut AsyncApp) {
    cx.update(|cx| cx.dispatch_action(action.as_ref()));
}

/// Waits until `done` holds (or `limit` passes).
async fn until(limit: Duration, cx: &mut AsyncApp, done: impl Fn(&App) -> bool) {
    let step = Duration::from_millis(25);
    let mut waited = Duration::ZERO;
    while waited < limit && !cx.update(|cx| done(cx)) {
        cx.background_executor().timer(step).await;
        waited += step;
    }
}

fn form_of(cx: &App) -> Option<gpui_kit::Entity<RuleForm>> {
    RulesWindow::global(cx).and_then(|(_, w)| w.read(cx).form().cloned())
}

pub async fn command(command: &Value, cx: &mut AsyncApp) -> Option<Value> {
    if !VERBS.iter().any(|v| command.get(v).is_some()) {
        return None;
    }
    let mut extra = serde_json::Map::new();
    if command.get("rules").and_then(Value::as_bool) == Some(true) {
        cx.update(|cx| feature::open(Target::Keep, cx));
    }
    if let Some(id) = command.get("openRule").and_then(Value::as_str) {
        dispatch(
            Box::new(crate::features::chrome::OpenRule {
                id: id.to_owned().into(),
            }),
            cx,
        );
    }
    if command.get("newRule").and_then(Value::as_bool) == Some(true) {
        dispatch(Box::new(feature::NewRule), cx);
    }
    if let Some(p) = command.get("preset") {
        let Some(flow) = p.get("flow").and_then(Value::as_u64) else {
            return Some(json!({"error": "preset takes {flow, kind}"}));
        };
        let preset = match p.get("kind").and_then(Value::as_str) {
            Some("map_local") => PresetKind::MapLocal,
            Some("breakpoint") => PresetKind::Breakpoint,
            other => return Some(json!({ "error": format!("no preset {other:?} here") })),
        };
        let store =
            cx.update(|cx| crate::workspace::Workspace::global(cx).map_or(pm_model::LIVE_STORE, |w| w.read(cx).store));
        dispatch(
            Box::new(feature::OpenPreset {
                flow: FlowRef {
                    store,
                    id: flow as pm_model::FlowId,
                },
                preset,
            }),
            cx,
        );
        until(Duration::from_secs(2), cx, |cx| {
            RulesWindow::global(cx).is_some_and(|(_, w)| w.read(cx).is_draft())
        })
        .await;
    }
    // Let a window just asked for open (deferred) before acting in it.
    until(Duration::from_secs(2), cx, |cx| RulesWindow::global(cx).is_some()).await;
    if let Some(id) = command.get("rulesSelect").and_then(Value::as_str) {
        let id = id.to_owned();
        cx.update(|cx| RulesWindow::update_global(cx, move |w, window, cx| w.select(Some(id), window, cx)));
    }
    if let Some(edits) = command.get("ruleEdit").and_then(Value::as_array) {
        for e in edits {
            let Some(step) = e.as_array() else {
                return Some(json!({"error": "ruleEdit takes [[key, value], …]"}));
            };
            let s = |i: usize| {
                step.get(i)
                    .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
            };
            let (Some(key), value, extra_arg) = (s(0), s(1).unwrap_or_default(), s(2)) else {
                return Some(json!({"error": "an empty edit"}));
            };
            let result: Result<(), String> = cx.update(|cx| {
                let Some((handle, _)) = RulesWindow::global(cx) else {
                    return Err("the Rules window isn't open".into());
                };
                let form = form_of(cx).ok_or("no rule open")?;
                handle
                    .update(cx, |_, window, cx| {
                        form.update(cx, |f, cx| -> Result<(), String> {
                            let index = || value.parse::<usize>().map_err(|_| format!("{key} takes an index"));
                            match key.as_str() {
                                "add" => f.add_action(&value, window, cx),
                                "remove" | "up" | "down" => {
                                    let i = index()?;
                                    f.edit_structure(window, cx, |r| match key.as_str() {
                                        "remove" => edit::remove_action(r, i),
                                        k => edit::move_action(r, i, k == "down"),
                                    });
                                }
                                "addRow" => f.edit_structure(window, cx, |r| edit::add_row(r, &value)),
                                "removeRow" => {
                                    let i: usize = extra_arg
                                        .as_deref()
                                        .and_then(|x| x.parse().ok())
                                        .ok_or("removeRow takes [which, index]")?;
                                    f.edit_structure(window, cx, |r| edit::remove_row(r, &value, i));
                                }
                                _ if key == "group" || edit::chosen(f.rule(), &key).is_some() => {
                                    f.choose(&key, &value, window, cx);
                                }
                                _ if edit::text(f.rule(), &key).is_some() => f.type_text(&key, &value, window, cx),
                                _ => return Err(format!("no field {key} in this rule")),
                            }
                            Ok(())
                        })
                    })
                    .map_err(|e| e.to_string())?
            });
            if let Err(err) = result {
                return Some(json!({ "error": err }));
            }
        }
        // Live validation runs 200 ms after the last change.
        let seen = cx.update(|cx| form_of(cx).map(|f| f.read(cx).validations()));
        cx.background_executor().timer(Duration::from_millis(250)).await;
        until(Duration::from_secs(2), cx, |cx| {
            form_of(cx).map(|f| f.read(cx).validations()) != seen
        })
        .await;
    }
    if command.get("saveRule").and_then(Value::as_bool) == Some(true) {
        let ok = cx.update(|cx| {
            let Some(form) = form_of(cx) else { return false };
            let can = form.read(cx).can_save();
            form.update(cx, |f, cx| f.save(cx));
            can
        });
        if !ok {
            extra.insert("saved".into(), json!(false));
        }
        until(Duration::from_secs(3), cx, |cx| {
            form_of(cx).is_some_and(|f| !f.read(cx).is_unsaved() || f.read(cx).error().is_some())
        })
        .await;
        cx.background_executor().timer(Duration::from_millis(150)).await;
    }
    if command.get("testRule").and_then(Value::as_bool) == Some(true) {
        cx.update(|cx| {
            if let Some(form) = form_of(cx) {
                form.update(cx, |f, cx| f.run_test(cx));
            }
        });
        until(Duration::from_secs(5), cx, |cx| {
            form_of(cx).is_some_and(|f| !f.read(cx).test.busy)
        })
        .await;
    }
    if let Some(id) = command.get("deleteRule").and_then(Value::as_str) {
        dispatch(
            Box::new(feature::DeleteRule {
                id: id.to_owned().into(),
            }),
            cx,
        );
        cx.background_executor().timer(Duration::from_millis(150)).await;
    }
    if let Some(t) = command.get("throttle") {
        dispatch(
            Box::new(feature::SetThrottle {
                preset: t.as_str().map(|s| s.to_owned().into()),
            }),
            cx,
        );
    }
    for (verb, id) in [
        ("noCache", rules::NO_CACHE_RULE_ID),
        ("noCookies", rules::NO_COOKIES_RULE_ID),
    ] {
        if let Some(on) = command.get(verb).and_then(Value::as_bool) {
            cx.update(|cx| crate::bridge::mutations::set_rule_enabled(id.into(), on, cx));
        }
    }
    if let Some(secs) = command.get("breakpointTimeout").and_then(Value::as_u64) {
        cx.update(|cx| {
            crate::bridge::mutations::set_rule_settings(
                pm_model::RuleSettings {
                    breakpoint_timeout_secs: secs as u32,
                },
                cx,
            );
        });
    }
    if let Some(name) = command.get("rulesRename") {
        let name = name.as_str().unwrap_or_default().to_owned();
        cx.update(|cx| {
            RulesWindow::update_global(cx, move |w, window, cx| {
                if let Some(id) = w.renaming().map(str::to_owned) {
                    w.type_group_name(&name, window, cx);
                    w.finish_rename(&id, true, window, cx);
                }
            });
        });
    }
    if let Some(name) = command.get("rulesMenu").and_then(Value::as_str) {
        let run = command.get("run").and_then(Value::as_str).map(str::to_owned);
        let answer: Result<Value, String> = cx.update(|cx| {
            let menu = match name {
                "throttle" => {
                    let book = feature::book(cx);
                    let state = ToolState::new(book.as_ref(), None);
                    Some(crate::features::rules::menus::throttle_menu(state.throttle.as_ref()))
                }
                _ => {
                    let (_, view) = RulesWindow::global(cx).ok_or("the Rules window isn't open")?;
                    match name.strip_prefix("picker:") {
                        Some(key) => form_of(cx).and_then(|f| f.read(cx).picker_menu(key, cx)),
                        None => view.read(cx).menu_for(name, cx),
                    }
                }
            }
            .ok_or_else(|| format!("no menu {name:?} now"))?;
            let json = menu.to_json();
            if let Some(label) = run {
                let handle = RulesWindow::global(cx)
                    .map(|(h, _)| h)
                    .or_else(|| crate::shell::parts(cx).map(|p| p.window))
                    .ok_or("no window")?;
                let ran = handle
                    .update(cx, |_, window, cx| menu.run(&label, window, cx))
                    .map_err(|e| e.to_string())?;
                if !ran {
                    return Err(format!("no entry {label:?}"));
                }
            }
            Ok(json)
        });
        match answer {
            Ok(menu) => extra.insert("menu".into(), menu),
            Err(err) => return Some(json!({ "error": err })),
        };
        cx.background_executor().timer(Duration::from_millis(150)).await;
    }
    if let Some(id) = command.get("rulesClick").and_then(Value::as_str) {
        let Some(handle) = cx.update(|cx| RulesWindow::global(cx).map(|(h, _)| h)) else {
            return Some(json!({"error": "the Rules window isn't open"}));
        };
        let element = ElementId::Name(id.to_owned().into());
        let clicked = handle.update(cx, |_, window, cx| match window.try_find(element.clone()) {
            Some(_) => {
                window.click(element, cx);
                Ok(())
            }
            None => Err("no element with that id (does it have .test_support()?)"),
        });
        match clicked {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Some(json!({ "error": err })),
            Err(err) => return Some(json!({ "error": err.to_string() })),
        }
        cx.background_executor().timer(Duration::from_millis(100)).await;
    }
    // Let pushes (the rule book, a toast) land.
    cx.background_executor().timer(Duration::from_millis(120)).await;
    if let Some(path) = command.get("rulesScreenshot").and_then(Value::as_str) {
        let Some(handle) = cx.update(|cx| RulesWindow::global(cx).map(|(h, _)| h)) else {
            return Some(json!({"error": "the Rules window isn't open"}));
        };
        let shot = super::screenshot(handle, PathBuf::from(path), false, cx).await;
        extra.insert("screenshot".into(), shot);
    }
    let mut answer = cx.update(state);
    if let Value::Object(map) = &mut answer {
        map.extend(extra);
    }
    Some(answer)
}

/// The Rules window's state, the tools' and the last toast.
fn state(cx: &mut App) -> Value {
    let book = feature::book(cx);
    let queue = crate::bridge::breakpoints(cx).read(cx).value.clone();
    let tools = ToolState::new(book.as_ref(), queue.as_deref());
    let settings = crate::bridge::keys::rules_caches(cx)
        .settings
        .read(cx)
        .peek(&())
        .map(|s| s.breakpoint_timeout_secs);
    let none = rules::Names::new();
    let window = RulesWindow::global(cx).map(|(_, w)| {
        let w = w.read(cx);
        let (listed, groups) = w.listed(cx);
        let form = w.form().map(|f| {
            let f = f.read(cx);
            json!({
                "rule": f.rule(),
                "unsaved": f.is_unsaved(),
                "canSave": f.can_save(),
                "invalid": f.invalid(),
                "error": f.error(),
                "matcher": rules::matcher_summary(&f.rule().matcher),
                "actions": rules::actions_summary(f.rule(), &none, &none),
                "test": f.test.result.as_ref().map(|r| json!({
                    "matched": r.matched,
                    "mismatch": r.mismatch,
                    "responseMatched": r.response_matched,
                    "notes": r.notes,
                    "request": format!("{} {}", r.request.method, r.request.url),
                    "response": r.response.as_ref().map(|x| x.status),
                    "responseBody": r.response_body.as_ref().map(|b| b.chars().take(200).collect::<String>()),
                })),
                "testError": f.test.error,
            })
        });
        json!({
            "rules": listed.iter().map(|r| json!({
                "id": r.id,
                "name": r.name,
                "enabled": r.enabled,
                "group": r.group,
                "origin": rules::origin_label(&r.origin),
                "matcher": rules::matcher_summary(&r.matcher),
                "actions": rules::actions_summary(r, &none, &none),
            })).collect::<Vec<_>>(),
            "groups": groups.iter().map(|g| json!({"id": g.id, "name": g.name, "enabled": g.enabled})).collect::<Vec<_>>(),
            "selected": w.selected(),
            "draft": w.is_draft(),
            "renaming": w.renaming(),
            "form": form,
        })
    });
    json!({
        "window": window,
        "tools": {
            "noCache": tools.no_cache,
            "noCookies": tools.no_cookies,
            "throttle": tools.throttle.as_ref().map(|p| p.name.clone()),
            "paused": tools.paused,
        },
        "breakpointTimeout": settings,
        "toast": crate::features::clipboard::Toasts::last(cx).map(|t| json!({"title": t.title, "description": t.description})),
    })
}
