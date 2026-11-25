//! The dev remote's Settings verbs (G12); see the list in `dev/mod.rs`.
//! The window's verbs answer with its state (the page, errors, what is
//! busy, the fields typed and Network's draft) plus what the engine holds
//! for that page. The Tauri remote's M16, M20 and M21 verbs work without
//! the window (as its `remote.ts` does) and answer with the lists they
//! change; `neverRecord` is G13's (`dev/integration.rs`).

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{App, AsyncApp, ElementId};
use serde_json::{Value, json};

use crate::bridge::Bridge;
use crate::bridge::mutations::{self, MockFrom};
use crate::features::settings::{self as feature, SettingsWindow};
use crate::prefs::Appearance;

const WINDOW_VERBS: [&str; 8] = [
    "settings",
    "settingsEdit",
    "settingsChoose",
    "settingsPress",
    "settingsClick",
    "settingsMenu",
    "settingsScreenshot",
    "settingsState",
];

const ENGINE_VERBS: [&str; 13] = [
    "appearance",
    "mockFrom",
    "mockFromFile",
    "importOpenApi",
    "generateMocks",
    "removeMockSource",
    "removeOpenApi",
    "importProto",
    "removeProto",
    "recordingLists",
    "autosave",
    "autosaveNow",
    "mocking",
];

/// Waits until `done` holds (or `limit` passes).
async fn until(limit: Duration, cx: &mut AsyncApp, done: impl Fn(&App) -> bool) {
    let step = Duration::from_millis(25);
    let mut waited = Duration::ZERO;
    while waited < limit && !cx.update(|cx| done(cx)) {
        cx.background_executor().timer(step).await;
        waited += step;
    }
}

/// Runs `f` on the open Settings window, in it.
fn in_window<R>(
    cx: &mut AsyncApp,
    f: impl FnOnce(&mut SettingsWindow, &mut gpui_kit::Window, &mut gpui_kit::Context<SettingsWindow>) -> R,
) -> Result<R, String> {
    cx.update(|cx| {
        let (handle, view) = SettingsWindow::global(cx).ok_or("the Settings window isn't open")?;
        handle
            .update(cx, |_, window, cx| view.update(cx, |w, cx| f(w, window, cx)))
            .map_err(|e| e.to_string())
    })
}

/// `[[key, value], …]` as pairs.
fn pairs(v: &Value) -> Result<Vec<(String, String)>, String> {
    let steps = v.as_array().ok_or("takes [[key, value], …]")?;
    steps
        .iter()
        .map(|step| {
            let s = |i: usize| {
                step.get(i)
                    .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
            };
            Ok((s(0).ok_or("an empty step")?, s(1).unwrap_or_default()))
        })
        .collect()
}

pub async fn command(command: &Value, cx: &mut AsyncApp) -> Option<Value> {
    if ENGINE_VERBS.iter().any(|v| command.get(v).is_some()) {
        return Some(engine_command(command, cx).await);
    }
    if !WINDOW_VERBS.iter().any(|v| command.get(v).is_some()) {
        return None;
    }
    let mut extra = serde_json::Map::new();
    match command.get("settings") {
        Some(Value::String(page)) => cx.update(|cx| feature::open(page, cx)),
        Some(Value::Bool(true)) => cx.update(|cx| feature::open("", cx)),
        Some(Value::Bool(false) | Value::Null) => {
            cx.update(feature::close);
            cx.background_executor().timer(Duration::from_millis(150)).await;
            let open = cx.update(|cx| SettingsWindow::global(cx).is_some());
            return Some(json!({ "open": open }));
        }
        _ => {}
    }
    until(Duration::from_secs(2), cx, |cx| SettingsWindow::global(cx).is_some()).await;
    let _ = in_window(cx, |w, _, cx| w.prefetch(cx));
    until(Duration::from_secs(3), cx, |cx| {
        SettingsWindow::global(cx).is_none_or(|(_, w)| w.read(cx).loaded(cx))
    })
    .await;
    if let Some(page) = command.get("settings").and_then(Value::as_str) {
        let page = pm_view::settings::page_or_default(page);
        until(Duration::from_secs(1), cx, |cx| {
            SettingsWindow::global(cx).is_some_and(|(_, w)| w.read(cx).page() == page)
        })
        .await;
    }
    if let Some(edits) = command.get("settingsEdit") {
        let edits = match pairs(edits) {
            Ok(edits) => edits,
            Err(err) => return Some(json!({ "error": format!("settingsEdit {err}") })),
        };
        for (key, value) in edits {
            if let Err(err) = in_window(cx, |w, window, cx| w.type_text(&key, &value, window, cx)) {
                return Some(json!({ "error": err }));
            }
        }
    }
    if let Some(choices) = command.get("settingsChoose") {
        let choices = match pairs(choices) {
            Ok(choices) => choices,
            Err(err) => return Some(json!({ "error": format!("settingsChoose {err}") })),
        };
        for (key, value) in choices {
            if let Err(err) = in_window(cx, |w, window, cx| w.choose(&key, &value, window, cx)) {
                return Some(json!({ "error": err }));
            }
        }
        cx.background_executor().timer(Duration::from_millis(300)).await;
    }
    if let Some(press) = command.get("settingsPress") {
        let ids: Vec<String> = match press {
            Value::String(id) => vec![id.clone()],
            Value::Array(ids) => ids.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            _ => return Some(json!({"error": "settingsPress takes an id or a list of ids"})),
        };
        for id in ids {
            if let Err(err) = in_window(cx, |w, window, cx| w.press(&id, window, cx)) {
                return Some(json!({ "error": err }));
            }
        }
        // Writes land on the background executor (a port restart takes a
        // moment): wait until nothing is busy, then for the caches.
        cx.background_executor().timer(Duration::from_millis(150)).await;
        until(Duration::from_secs(10), cx, |cx| {
            SettingsWindow::global(cx).is_none_or(|(_, w)| !w.read(cx).any_busy())
        })
        .await;
        cx.background_executor().timer(Duration::from_millis(250)).await;
    }
    if let Some(id) = command.get("settingsClick").and_then(Value::as_str) {
        let Some(handle) = cx.update(|cx| SettingsWindow::global(cx).map(|(h, _)| h)) else {
            return Some(json!({"error": "the Settings window isn't open"}));
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
        cx.background_executor().timer(Duration::from_millis(300)).await;
    }
    if let Some(name) = command.get("settingsMenu").and_then(Value::as_str) {
        let run = command.get("run").and_then(Value::as_str).map(str::to_owned);
        let answer: Result<Value, String> = cx.update(|cx| {
            let (handle, view) = SettingsWindow::global(cx).ok_or("the Settings window isn't open")?;
            let menu = view
                .read(cx)
                .menu_for(name, cx)
                .ok_or_else(|| format!("no menu {name:?} now"))?;
            let json = menu.to_json();
            if let Some(label) = run {
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
        cx.background_executor().timer(Duration::from_millis(300)).await;
        if let Some(text) = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text())) {
            extra.insert("clipboard".into(), json!(text));
        }
    }
    if let Some(path) = command.get("settingsScreenshot").and_then(Value::as_str) {
        let Some(handle) = cx.update(|cx| SettingsWindow::global(cx).map(|(h, _)| h)) else {
            return Some(json!({"error": "the Settings window isn't open"}));
        };
        let shot = super::screenshot(handle, PathBuf::from(path), false, cx).await;
        extra.insert("screenshot".into(), shot);
    }
    let mut state = cx.update(state);
    if let Value::Object(map) = &mut state {
        map.extend(extra);
    }
    Some(state)
}

/// The window's state, and what the engine holds for its page.
fn state(cx: &mut App) -> Value {
    let Some((_, view)) = SettingsWindow::global(cx) else {
        return json!({ "open": false });
    };
    let w = view.read(cx);
    let page = w.page();
    let fields: serde_json::Map<String, Value> = w
        .field_keys()
        .into_iter()
        .map(|k| {
            let text = w.text(&k, cx);
            (k, json!(text))
        })
        .collect();
    let network = w.network_draft().map(|n| {
        json!({
            "dirty": n.dirty(),
            "draft": n.draft,
            "fieldErrors": n.field_errors,
        })
    });
    let mut answer = json!({
        "open": true,
        "page": page,
        "errors": w.errors_json(),
        "notes": w.notes_json(),
        "busy": w.busy_json(),
        "fields": fields,
        "network": network,
        "confirmRegenerate": w.confirming_regenerate(),
        "auditLimit": w.audit_limit(),
        "toast": crate::features::clipboard::Toasts::last(cx).map(|t| t.title.to_string()),
    });
    if let Some(engine) = cx.try_global::<Bridge>().map(|_| Bridge::engine(cx)) {
        let ws = crate::workspace::Workspace::global(cx).map(|w| w.read(cx).proxy.clone());
        let engine_side = match page {
            "proxy" | "advanced" => json!({
                "proxy": engine.proxy_settings(),
                "status": ws,
                "access": engine.device_access(),
                "rules": engine.rule_settings(),
                "scripts": engine.script_settings(),
            }),
            "network" => json!({ "network": engine.network_settings(), "status": ws }),
            "ssl" => json!({ "ssl": engine.ssl_settings() }),
            "clients" => json!({ "devices": engine.devices() }),
            "storage" => json!({
                "storage": engine.storage_settings(),
                "status": engine.storage_status(),
                "recording": engine.recording_settings(),
                "autosave": engine.autosave_settings(),
            }),
            "protobuf" => json!({ "schemas": engine.proto_schemas(), "mappings": engine.proto_mappings() }),
            "mocking" => json!({ "sources": engine.mock_sources(), "specs": engine.openapi_specs() }),
            "appearance" => json!({ "appearance": crate::prefs::get(cx).appearance }),
            "mcp" => json!({ "mcp": engine.mcp_status() }),
            _ => json!({ "updates": engine.update_settings(), "version": crate::updater::VERSION }),
        };
        answer["engine"] = engine_side;
    }
    answer
}

/// The Tauri remote's M16/M20/M21 verbs, against the engine as the app's
/// actions run them (no file dialogs).
async fn engine_command(command: &Value, cx: &mut AsyncApp) -> Value {
    let Some(engine) = cx.update(|cx| cx.try_global::<Bridge>().map(|_| Bridge::engine(cx))) else {
        return json!({"error": "no engine"});
    };
    let toast_before = cx.update(|cx| crate::features::clipboard::Toasts::last(cx).map(|t| t.title.to_string()));
    if let Some(theme) = command.get("appearance").and_then(Value::as_str) {
        let appearance = match theme {
            "light" => Appearance::Light,
            "dark" => Appearance::Dark,
            _ => Appearance::System,
        };
        cx.update(|cx| crate::appearance::set(appearance, cx));
    }
    if let Some(store) = command.get("mockFrom").and_then(Value::as_u64) {
        cx.update(|cx| feature::mock_from(MockFrom::Store(store as pm_model::StoreId), cx));
        wait_for_draft(cx).await;
    }
    if let Some(path) = command.get("mockFromFile").and_then(Value::as_str) {
        cx.update(|cx| feature::mock_from(MockFrom::File(PathBuf::from(path)), cx));
        wait_for_draft(cx).await;
    }
    if let Some(path) = command.get("importOpenApi").and_then(Value::as_str) {
        cx.update(|cx| feature::import_openapi_path(PathBuf::from(path), cx));
    }
    if let Some(id) = command.get("generateMocks").and_then(Value::as_u64) {
        cx.update(|cx| {
            feature::window::generate_mocks(
                id as u32,
                |err, cx| crate::features::clipboard::report_error("Couldn't generate mocks", err, cx),
                cx,
            )
        });
    }
    if let Some(id) = command.get("removeMockSource").and_then(Value::as_str) {
        let id = id.to_owned();
        cx.update(|cx| {
            mutations::remove_mock_source(
                id,
                |err, cx| crate::features::clipboard::report_error("Couldn't remove it", err, cx),
                cx,
            )
        });
    }
    if let Some(id) = command.get("removeOpenApi").and_then(Value::as_u64) {
        cx.update(|cx| {
            mutations::remove_openapi(
                id as u32,
                |err, cx| crate::features::clipboard::report_error("Couldn't remove it", err, cx),
                cx,
            )
        });
    }
    if let Some(paths) = command.get("importProto").and_then(Value::as_array) {
        let paths: Vec<PathBuf> = paths.iter().filter_map(Value::as_str).map(PathBuf::from).collect();
        cx.update(|cx| feature::window::import_proto(paths, cx));
    }
    if let Some(id) = command.get("removeProto").and_then(Value::as_u64) {
        cx.update(|cx| {
            mutations::remove_proto(
                id as u32,
                |err, cx| crate::features::clipboard::report_error("Couldn't remove the schema", err, cx),
                cx,
            )
        });
    }
    if let Some(lists) = command.get("recordingLists") {
        match serde_json::from_value::<pm_model::RecordingSettings>(lists.clone()) {
            Ok(lists) => cx.update(|cx| {
                mutations::set_recording_settings(
                    lists,
                    |err, cx| crate::features::clipboard::report_error("Can't change the recording lists", err, cx),
                    cx,
                )
            }),
            Err(err) => return json!({ "error": format!("recordingLists: {err}") }),
        }
    }
    if let Some(settings) = command.get("autosave") {
        match serde_json::from_value::<pm_model::AutoSaveSettings>(settings.clone()) {
            Ok(settings) => cx.update(|cx| {
                mutations::set_autosave_settings(
                    settings,
                    |err, cx| crate::features::clipboard::report_error("Can't change auto-save", err, cx),
                    cx,
                )
            }),
            Err(err) => return json!({ "error": format!("autosave: {err}") }),
        }
    }
    if command.get("autosaveNow").and_then(Value::as_bool) == Some(true) {
        cx.update(mutations::autosave_now);
    }
    // The writes run on the background executor.
    cx.background_executor().timer(Duration::from_millis(400)).await;
    let toast = cx.update(|cx| crate::features::clipboard::Toasts::last(cx).map(|t| t.title.to_string()));
    let rules = engine.rule_book().rules;
    json!({
        "sources": engine.mock_sources(),
        "specs": engine.openapi_specs(),
        "schemas": engine.proto_schemas(),
        "recording": engine.recording_settings(),
        "autosave": engine.autosave_settings(),
        "storage": engine.storage_status(),
        "rules": rules.iter().map(|r| json!({"id": r.id, "name": r.name, "enabled": r.enabled})).collect::<Vec<_>>(),
        "appearance": cx.update(|cx| crate::prefs::get(cx).appearance),
        "toast": (toast != toast_before).then_some(toast).flatten(),
    })
}

/// "Mock from…" opens the Rules window on an unsaved rule.
async fn wait_for_draft(cx: &mut AsyncApp) {
    until(Duration::from_secs(5), cx, |cx| {
        crate::features::rules::RulesWindow::global(cx).is_some_and(|(_, w)| w.read(cx).is_draft())
    })
    .await;
}
