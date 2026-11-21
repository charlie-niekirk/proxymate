//! The dev remote (feature `dev-harness`): drive the running app from a
//! shell, as `src/dev/remote.ts` does for the Tauri UI.
//!
//! `PROXYMATE_DEV_REMOTE=1 cargo run -p proxymate-native --features dev-harness`
//! serves `http://127.0.0.1:1430/__pm_remote` (`PROXYMATE_DEV_REMOTE_PORT`
//! moves it). Each POST is one JSON command, answered when it has run:
//!
//! - `{"theme": "light" | "dark" | "system"}`
//! - `{"screenshot": "/abs/path.png"}`: the window, activated first, as
//!   composited (vibrancy included, `screencapture -l`); without Screen Recording permission it
//!   falls back to gpui's own frame (`"offscreen": true` asks for that).
//! - `{"click": "<element id>"}`: a real click through hit testing, on an
//!   element rendered with `.id(…)` and `.test_support()`.
//! - The flow table (G1): `{"select": 3}` (a row index, as in the Tauri
//!   remote; `null` clears), `{"selectIds": [4, 9]}`, `{"selectAll": true}`, `{"scrollTo": 500}`,
//!   `{"headerColumns": [{"part": "request", "name": "x-request-id"}]}`,
//!   `{"filter": "api"}` (the search text), `{"autoSelect": true}` and
//!   `{"table": true}`; each answers with the table's state (rows, total,
//!   selection, focused flow, columns, last filter timing).
//! - The shell chrome (G2): `{"filter": "host:x"}` types into the search
//!   box (debounced like typing; `"filterNow": true` skips the wait),
//!   `{"search": "ho"}` types and answers with the suggestions,
//!   `{"protocol": "https"}`, `{"content": ["json"]}`, `{"scope": {"host": "*.x.com"}}`,
//!   `{"keys": "cmd-k"}` (keystrokes, space-separated, as if typed),
//!   `{"palette": true | false}` (answers with the listed commands),
//!   `{"command": "<id>"}` (runs one), `{"store": 1}`, `{"openDocument": "/abs/x.har"}`,
//!   `{"closeDocument": 1}`, `{"saveSession": "/abs/x.proxymate"}`,
//!   `{"exportHar": "/abs/x.har"}`, `{"recording": false}`, `{"clear": true}`,
//!   `{"contextMenu": "listening" | "empty" | "save" | "tab:<store>" | "sidebar:<key>",
//!   "run": "<label>"}` (answers with the entries, and the clipboard after
//!   running one), `{"sidebarFilter": "saf"}` and `{"workspace": true}`;
//!   each answers with the workspace's state (store, documents, filter,
//!   proxy, save status, last toast). `{"tableShare": 0.3}` sets the
//!   table's share of the height over the inspector (G6, for screenshots).
//! - The inspector (G3): `{"requestTab": "Raw"}`, `{"responseTab": "TLS"}`,
//!   `{"focus": "inspector" | "table"}`, `{"curl": 3}` (Copy as cURL's
//!   command for flow 3 of the tab shown) and `{"inspector": true}`; each
//!   answers with the inspector's state (flow, tabs, failure, rule badges,
//!   which pane has the keyboard). `contextMenu` also takes the inspector's
//!   menus (`url`, `kv:<table>[:<row>]`, `rule:<n>`, `cert:<n>`,
//!   `timing[:<row>]`, `failure`, `raw:request|response`) and the flow row
//!   menu, `row:<flow id>[:<column id>]` (`row:3:status`).
//! - The stream panels (G6), on the inspected WebSocket or SSE flow:
//!   `{"streamSelect": 4}` (a row of the list), `{"streamDirection": "sent"
//!   | "received" | "all"}`, `{"streamSearch": "ping"}`, `{"streamViewer":
//!   "hex"}`, `{"wsOriginal": true}`, `{"wsCompose": {"text": "hi",
//!   "direction": "received", "mode": "hex"}}` (fills the composer),
//!   `{"wsComposeSend": true}` (Send, as ⌘↩) and `{"stream": true}`; each
//!   answers with the panel's state (status, counts, the rows on screen,
//!   the selection, its detail, the composer's error). `contextMenu` takes
//!   `stream:<row>` and `payload`. As in the Tauri remote, `{"wsSend":
//!   {"flow": 3, "direction": "received", "payload": "hi", "binary": false}}`
//!   sends into a live WebSocket and `{"wsClose": 3}` closes it.
//!
//! - The body viewers (G4): `{"viewer": {"kind": "json", "viewer": "json-tree"}}`
//!   picks the viewer for a content kind (as the dropdown does),
//!   `{"loadAll": "response"}` and `{"highlightAnyway": "response"}` press
//!   those buttons, and `{"body": true}` answers with both Body tabs' state
//!   (kind, viewer, what shows, matches). `contextMenu` takes
//!   `body:request|response` (the viewer's menu). `PROXYMATE_DEV_ZOO=1`
//!   sends the body-viewer zoo through the app at launch ([`zoo`]).
//! - Rules (G7, `dev/rules.rs`): `{"rules": true}` (opens the Rules window
//!   and answers with its state), `{"openRule": "<id>"}`, `{"newRule": true}`,
//!   `{"preset": {"flow": 3, "kind": "map_local" | "breakpoint"}}`,
//!   `{"ruleEdit": [["name", "Mock"], ["add", "MapLocal"], ["a.0.inline", "{}"], …]}`
//!   (field keys of `pm_view::rules::edit`; also `remove`/`up`/`down` with
//!   an action index, `addRow`/`removeRow`), `{"saveRule": true}`,
//!   `{"testRule": true}`, `{"deleteRule": "<id>"}`, `{"throttle": "Edge" | null}`,
//!   `{"noCache": true}`, `{"noCookies": false}`, `{"breakpointTimeout": 30}`,
//!   `{"rulesMenu": "rule:<id>" | "group:<id>" | "picker:<key>" | "throttle", "run": "<label>"}`,
//!   `{"rulesClick": "<element id>"}`, `{"rulesDrag": ["rule-row-<id>", "rule-row-<id>" | "group-header-<id>"]}`,
//!   `{"rulesSelect": "<id>"}`, `{"rulesRename": "<name>"}` (the group being renamed)
//!   and `{"rulesScreenshot": "/abs/x.png"}`.
//!
//! Every phase adds the verbs its surfaces need (see `docs/PLAN.md`).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, AsyncApp, ElementId, Focusable as _};
use serde_json::{Value, json};

use crate::prefs::Appearance;

mod rules;
pub mod zoo;

pub const DEFAULT_PORT: u16 = 1430;
const PATH: &str = "/__pm_remote";

type Reply = async_channel::Sender<Value>;

pub fn init(cx: &mut App) {
    if std::env::var("PROXYMATE_DEV_REMOTE").as_deref() != Ok("1") {
        return;
    }
    let port = pm_app::env_or("PROXYMATE_DEV_REMOTE_PORT", DEFAULT_PORT);
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => listener,
        Err(err) => return eprintln!("dev remote: can't listen on 127.0.0.1:{port}: {err}"),
    };
    eprintln!("dev remote: POST JSON to http://127.0.0.1:{port}{PATH}");
    let (tx, rx) = async_channel::unbounded::<(Value, Reply)>();
    std::thread::Builder::new()
        .name("pm-dev-remote".into())
        .spawn(move || serve(listener, tx))
        .expect("dev remote thread");
    cx.spawn(async move |cx| {
        while let Ok((command, reply)) = rx.recv().await {
            let answer = run(command, cx).await;
            let _ = reply.send(answer).await;
        }
    })
    .detach();
}

fn serve(listener: TcpListener, commands: async_channel::Sender<(Value, Reply)>) {
    for stream in listener.incoming().flatten() {
        let (status, answer) = match read_request(&stream) {
            Ok(command) => {
                let (tx, rx) = async_channel::bounded(1);
                if commands.send_blocking((command, tx)).is_err() {
                    return;
                }
                match rx.recv_blocking() {
                    Ok(answer) if answer.get("error").is_some() => ("422 Unprocessable Entity", answer),
                    Ok(answer) => ("200 OK", answer),
                    Err(_) => ("500 Internal Server Error", json!({"error": "no answer"})),
                }
            }
            Err(err) => ("400 Bad Request", json!({ "error": err })),
        };
        let body = answer.to_string();
        let mut stream = stream;
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

fn read_request(stream: &TcpStream) -> Result<Value, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method != "POST" || path != PATH {
        return Err(format!("POST a JSON command to {PATH}"));
    }
    let mut length = 0usize;
    loop {
        line.clear();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().map_err(|_| "bad Content-Length")?;
        }
    }
    let mut body = vec![0; length.min(1 << 20)];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| format!("not JSON: {e}"))
}

async fn run(command: Value, cx: &mut AsyncApp) -> Value {
    // The main window: the Rules window (G7) is another one.
    let Some(window) = cx.update(|cx| {
        crate::shell::parts(cx)
            .map(|p| p.window)
            .or_else(|| cx.windows().first().copied())
    }) else {
        return json!({"error": "no window"});
    };
    if let Some(theme) = command.get("theme").and_then(Value::as_str) {
        let appearance = match theme {
            "light" => Appearance::Light,
            "dark" => Appearance::Dark,
            "system" => Appearance::System,
            other => return json!({ "error": format!("unknown theme {other:?}") }),
        };
        cx.update(|cx| crate::appearance::set(appearance, cx));
        return json!({ "theme": theme });
    }
    if let Some(id) = command.get("click").and_then(Value::as_str) {
        let id = ElementId::Name(id.to_owned().into());
        let clicked = window.update(cx, |_, window, cx| match window.try_find(id.clone()) {
            Some(_) => {
                window.click(id, cx);
                Ok(())
            }
            None => Err("no element with that id (does it have .test_support()?)"),
        });
        return match clicked {
            Ok(Ok(())) => json!({ "clicked": command["click"] }),
            Ok(Err(err)) => json!({ "error": err }),
            Err(err) => json!({ "error": err.to_string() }),
        };
    }
    if let Some(answer) = rules::command(&command, cx).await {
        return answer;
    }
    if let Some(answer) = shell_command(&command, window, cx).await {
        return answer;
    }
    // Table verbs first, so `{"select": 3, "responseTab": "TLS"}` selects
    // then switches the tab, answering with the inspector.
    let table = table_command(&command, cx);
    if let Some(Value::Object(err)) = &table
        && err.contains_key("error")
    {
        return Value::Object(err.clone());
    }
    if let Some(answer) = inspector_command(&command, window, cx).await {
        return answer;
    }
    if let Some(answer) = body_command(&command, window, cx).await {
        return answer;
    }
    if let Some(answer) = stream_command(&command, window, cx).await {
        return answer;
    }
    if let Some(answer) = table {
        return answer;
    }
    if let Some(path) = command.get("screenshot").and_then(Value::as_str) {
        let offscreen = command.get("offscreen").and_then(Value::as_bool).unwrap_or(false);
        return screenshot(window, PathBuf::from(path), offscreen, cx).await;
    }
    json!({"error": "unknown command: try theme, screenshot, click, select, selectIds, selectAll, scrollTo, headerColumns, filter, search, protocol, content, scope, keys, palette, command, store, openDocument, closeDocument, saveSession, exportHar, recording, clear, contextMenu, sidebarFilter, workspace, autoSelect, table, requestTab, responseTab, focus, curl, inspector, streamSelect, streamDirection, streamSearch, streamViewer, wsOriginal, wsCompose, wsComposeSend, stream, wsSend, wsClose, rules, openRule, newRule, preset, ruleEdit, saveRule, testRule, deleteRule, throttle, noCache, noCookies, breakpointTimeout, rulesMenu, rulesClick, rulesDrag, rulesSelect, rulesRename, rulesScreenshot, viewer, loadAll, highlightAnyway or body"})
}

pub(crate) async fn screenshot(window: AnyWindowHandle, path: PathBuf, offscreen: bool, cx: &mut AsyncApp) -> Value {
    // Front and active, so the vibrancy shows its active material; then let
    // the frames of earlier commands (a theme switch, a click) land.
    let _ = window.update(cx, |_, window, cx| {
        cx.activate(true);
        window.activate_window();
    });
    cx.background_executor().timer(Duration::from_millis(400)).await;
    if !offscreen {
        let number = window.update(cx, |_, window, _| pm_platform::window::window_number(window));
        if let Ok(Ok(number)) = number {
            let target = path.clone();
            let captured = cx
                .background_spawn(async move { pm_platform::window::capture(number, &target) })
                .await;
            match captured {
                Ok(()) => return json!({ "screenshot": path, "source": "window server" }),
                Err(err) => eprintln!("dev remote: window capture failed ({err}); using gpui's frame"),
            }
        }
    }
    let image = window.update(cx, |_, window, _| window.render_to_image());
    match image {
        Ok(Ok(image)) => match image.save(&path) {
            Ok(()) => json!({ "screenshot": path, "source": "offscreen frame (no vibrancy)" }),
            Err(err) => json!({ "error": err.to_string() }),
        },
        Ok(Err(err)) | Err(err) => json!({ "error": err.to_string() }),
    }
}

/// The flow table's verbs; `None` when `command` has none of them.
fn table_command(command: &Value, cx: &mut AsyncApp) -> Option<Value> {
    const VERBS: [&str; 6] = [
        "select",
        "selectIds",
        "selectAll",
        "headerColumns",
        "autoSelect",
        "table",
    ];
    if !VERBS.iter().any(|v| command.get(v).is_some()) {
        return None;
    }
    let Some(table) = cx.update(|cx| crate::features::flow_table::first(cx)) else {
        return Some(json!({"error": "no flow table"}));
    };
    let result: Result<(), String> = cx.update(|cx| {
        let grid = table.read(cx).grid().clone();
        if let Some(v) = command.get("select") {
            match v.as_u64() {
                Some(index) => grid.update(cx, |g, cx| g.select_index(index as usize, cx)),
                None if v.is_null() => grid.update(cx, |g, cx| g.select(None, cx)),
                None => return Err("select takes a row index or null".into()),
            }
        }
        if let Some(v) = command.get("selectIds") {
            let ids: Vec<u32> = serde_json::from_value(v.clone()).map_err(|e| format!("selectIds: {e}"))?;
            grid.update(cx, |g, cx| g.select_ids(&ids, cx));
        }
        if let Some(index) = command.get("scrollTo").and_then(Value::as_u64) {
            grid.update(cx, |g, cx| g.scroll_to(index as usize, cx));
        }
        if command.get("selectAll").and_then(Value::as_bool) == Some(true) {
            grid.update(cx, |g, cx| g.select_all(cx));
        }
        if let Some(v) = command.get("headerColumns") {
            let columns: Vec<pm_model::HeaderColumn> =
                serde_json::from_value(v.clone()).map_err(|e| format!("headerColumns: {e}"))?;
            table.update(cx, |t, cx| t.set_header_columns(columns, cx));
        }
        if let Some(on) = command.get("autoSelect").and_then(Value::as_bool) {
            grid.update(cx, |g, cx| g.set_auto_select(on, cx));
        }
        Ok(())
    });
    if let Err(err) = result {
        return Some(json!({ "error": err }));
    }
    Some(cx.update(|cx| {
        let t = table.read(cx);
        let g = t.grid().read(cx);
        json!({
            "rows": g.len(),
            "total": t.total(),
            "selection": g.flow_selection(),
            "selected": g.selection_count(),
            "focused": g.selected().focused,
            "columns": g.columns().iter().map(|c| c.id.to_string()).collect::<Vec<_>>(),
            "sort": g.prefs().sort,
            "filterTiming": t.filter_timing().map(|f| json!({
                "engineMs": f.engine_ms, "endToEndMs": f.end_to_end_ms, "rows": f.rows
            })),
        })
    }))
}

/// The flow row menu: `<flow id>[:<column id>]`.
fn row_menu(spec: &str, cx: &App) -> Option<crate::features::context_menu::Menu> {
    let (id, column) = spec.split_once(':').map_or((spec, None), |(i, c)| (i, Some(c)));
    let id: pm_model::FlowId = id.parse().ok()?;
    let column = column.and_then(pm_view::columns::ColumnId::parse);
    let table = crate::features::flow_table::first(cx)?;
    table.read(cx).row_menu_for(id, column.as_ref(), cx)
}

/// A Body tab's viewer menu, with the text selected now.
fn body_menu(part: &str, window: &mut gpui_kit::Window, cx: &mut App) -> Option<crate::features::context_menu::Menu> {
    let part = match part {
        "request" => pm_model::BodyPart::Request,
        "response" => pm_model::BodyPart::Response,
        _ => return None,
    };
    let inspector = crate::shell::parts(cx)?.inspector.upgrade()?;
    let selected = crate::features::context_menu::selected_text(window, cx);
    let pane = inspector.read(cx).body_pane(part).clone();
    Some(pane.read(cx).menu(selected, cx))
}

/// One of the inspector's menus, with the text selected now.
fn inspector_menu(
    name: &str,
    window: &mut gpui_kit::Window,
    cx: &mut App,
) -> Option<crate::features::context_menu::Menu> {
    let inspector = crate::shell::parts(cx)?.inspector.upgrade()?;
    let selected = crate::features::context_menu::selected_text(window, cx);
    inspector.read(cx).menu_for(name, selected, cx)
}

/// The inspector's verbs (G3); `None` when `command` has none of them.
async fn inspector_command(command: &Value, window: AnyWindowHandle, cx: &mut AsyncApp) -> Option<Value> {
    use crate::features::inspector::{RequestTab, ResponseTab};

    const VERBS: [&str; 5] = ["requestTab", "responseTab", "focus", "curl", "inspector"];
    if !VERBS.iter().any(|v| command.get(v).is_some()) {
        return None;
    }
    let Some(inspector) = cx.update(|cx| crate::shell::parts(cx).and_then(|p| p.inspector.upgrade())) else {
        return Some(json!({"error": "no inspector"}));
    };
    let mut extra = serde_json::Map::new();
    if let Some(id) = command.get("curl").and_then(Value::as_u64) {
        let engine = cx.update(|cx| crate::bridge::Bridge::engine(cx));
        let store =
            cx.update(|cx| crate::workspace::Workspace::global(cx).map_or(pm_model::LIVE_STORE, |w| w.read(cx).store));
        let curl = cx
            .background_spawn(async move { engine.curl_command(store, id as pm_model::FlowId) })
            .await;
        match curl {
            Ok(curl) => extra.insert("curl".into(), json!(curl)),
            Err(err) => return Some(json!({ "error": err.to_string() })),
        };
    }
    let result: Result<(), String> = window
        .update(cx, |_, window, cx| -> Result<(), String> {
            if let Some(tab) = command.get("requestTab").and_then(Value::as_str) {
                let tab = RequestTab::parse(tab).ok_or_else(|| format!("no request tab {tab:?}"))?;
                inspector.update(cx, |i, cx| i.set_request_tab(tab, cx));
            }
            if let Some(tab) = command.get("responseTab").and_then(Value::as_str) {
                let tab = ResponseTab::parse(tab).ok_or_else(|| format!("no response tab {tab:?}"))?;
                inspector.update(cx, |i, cx| i.set_response_tab(tab, cx));
            }
            match command.get("focus").and_then(Value::as_str) {
                Some("inspector") => crate::shell::focus_inspector(window, cx),
                Some("table") => crate::shell::focus_table(window, cx),
                Some(other) => return Err(format!("focus takes inspector or table, not {other:?}")),
                None => {}
            }
            Ok(())
        })
        .unwrap_or_else(|e| Err(e.to_string()));
    if let Err(err) = result {
        return Some(json!({ "error": err }));
    }
    // Let a refetch of the inspected flow land.
    cx.background_executor().timer(Duration::from_millis(150)).await;
    let state = window.update(cx, |_, window, cx| {
        let i = inspector.read(cx);
        let insp = i.inspected(cx);
        let focus = i.focus_handle(cx);
        let table_focus = crate::shell::parts(cx)
            .and_then(|p| p.table.upgrade())
            .map(|t| t.read(cx).grid().focus_handle(cx));
        let focused = if focus.contains_focused(window, cx) {
            "inspector"
        } else if table_focus.is_some_and(|f| f.is_focused(window)) {
            "table"
        } else {
            "elsewhere"
        };
        json!({
            "flow": i.key(cx).map(|k| json!({"store": k.store, "id": k.id})),
            "url": insp.as_ref().and_then(|x| x.detail.as_ref()).map(|d| d.request.url.clone()),
            "status": insp.as_ref().and_then(|x| x.flow.status),
            "tls": insp.as_ref().map(|x| x.flow.tls),
            "failure": insp.as_ref().and_then(|x| pm_view::errors::explain_failure(&x.flow, x.detail.as_ref()))
                .map(|h| json!({"title": h.title, "detail": h.detail, "actions": format!("{:?}", h.actions)})),
            "rules": insp.as_ref().and_then(|x| x.detail.as_ref())
                .map(|d| d.rules.applied.iter().map(|r| r.name.clone()).collect::<Vec<_>>()),
            "requestTab": i.request_tab().label(),
            "responseTab": i.response_tab().label(),
            "focused": focused,
            "toast": crate::features::clipboard::Toasts::last(cx).map(|t| json!({"title": t.title, "description": t.description})),
        })
    });
    let mut answer = state.unwrap_or_else(|e| json!({ "error": e.to_string() }));
    if let Value::Object(map) = &mut answer {
        map.extend(extra);
    }
    Some(answer)
}

/// The stream panels' verbs (G6); `None` when `command` has none of them.
async fn stream_command(command: &Value, window: AnyWindowHandle, cx: &mut AsyncApp) -> Option<Value> {
    use crate::features::inspector::streams::{Row, Viewer};
    use pm_model::WsDirection;
    use pm_view::stream::ComposeMode;

    const VERBS: [&str; 10] = [
        "streamSelect",
        "streamDirection",
        "streamSearch",
        "streamViewer",
        "wsOriginal",
        "wsCompose",
        "wsComposeSend",
        "stream",
        "wsSend",
        "wsClose",
    ];
    if !VERBS.iter().any(|v| command.get(v).is_some()) {
        return None;
    }
    let engine = cx.update(|cx| crate::bridge::Bridge::engine(cx));
    if let Some(v) = command.get("wsSend") {
        let flow = v.get("flow").and_then(Value::as_u64).unwrap_or(u64::MAX) as pm_model::FlowId;
        let message: pm_model::WsSend = match serde_json::from_value(v.clone()) {
            Ok(m) => m,
            Err(err) => return Some(json!({ "error": format!("wsSend: {err}") })),
        };
        let e = engine.clone();
        if let Err(err) = cx.background_spawn(async move { e.ws_send(flow, &message) }).await {
            return Some(json!({ "error": err.to_string() }));
        }
    }
    if let Some(flow) = command.get("wsClose").and_then(Value::as_u64) {
        let e = engine.clone();
        if let Err(err) = cx
            .background_spawn(async move { e.ws_close(flow as pm_model::FlowId) })
            .await
        {
            return Some(json!({ "error": err.to_string() }));
        }
    }
    // The panel is made when the inspector draws the flow: give a
    // selection just made a few frames.
    let mut panel = None;
    for _ in 0..10 {
        panel = cx.update(|cx| {
            crate::shell::parts(cx)
                .and_then(|p| p.inspector.upgrade())
                .and_then(|i| i.read(cx).stream_panel(cx))
        });
        if panel.is_some() {
            break;
        }
        let _ = window.update(cx, |_, window, _| window.refresh());
        cx.background_executor().timer(Duration::from_millis(50)).await;
    }
    let Some(panel) = panel else {
        if command.get("wsSend").is_some() || command.get("wsClose").is_some() {
            return Some(json!({ "ok": true }));
        }
        return Some(json!({"error": "the inspected flow has no stream panel"}));
    };
    let direction = |v: &Value| match v.as_str() {
        Some("sent") => Ok(Some(WsDirection::Sent)),
        Some("received") => Ok(Some(WsDirection::Received)),
        Some("all") | None => Ok(None),
        Some(other) => Err(format!("direction takes sent, received or all, not {other:?}")),
    };
    let result: Result<(), String> = window
        .update(cx, |_, window, cx| -> Result<(), String> {
            if let Some(v) = command.get("streamDirection") {
                let d = direction(v)?;
                panel.update(cx, |p, cx| p.set_direction(d, cx));
            }
            if let Some(text) = command.get("streamSearch").and_then(Value::as_str) {
                panel.update(cx, |p, cx| p.set_search(text, window, cx));
            }
            if let Some(index) = command.get("streamSelect").and_then(Value::as_u64) {
                panel.update(cx, |p, cx| p.select_index(index as usize, cx));
            }
            if let Some(v) = command.get("streamViewer").and_then(Value::as_str) {
                let viewer = Viewer::parse(v).ok_or_else(|| format!("no viewer {v:?}: text or hex"))?;
                panel.update(cx, |p, cx| p.set_viewer(viewer, cx));
            }
            if let Some(on) = command.get("wsOriginal").and_then(Value::as_bool) {
                panel.update(cx, |p, cx| p.set_original(on, cx));
            }
            if let Some(c) = command.get("wsCompose") {
                let text = c.get("text").and_then(Value::as_str).unwrap_or_default().to_owned();
                let d = match c.get("direction") {
                    Some(v) => direction(v)?,
                    None => None,
                };
                let mode = match c.get("mode").and_then(Value::as_str) {
                    Some("hex") => Some(ComposeMode::Hex),
                    Some("text") => Some(ComposeMode::Text),
                    Some(other) => return Err(format!("mode takes text or hex, not {other:?}")),
                    None => None,
                };
                panel.update(cx, |p, cx| p.compose(&text, d, mode, window, cx));
            }
            if command.get("wsComposeSend").and_then(Value::as_bool) == Some(true) {
                panel.update(cx, |p, cx| p.send(cx));
            }
            Ok(())
        })
        .unwrap_or_else(|e| Err(e.to_string()));
    if let Err(err) = result {
        return Some(json!({ "error": err }));
    }
    // Let the sends, the heads and the pages they make stale land.
    for _ in 0..4 {
        cx.background_executor().timer(Duration::from_millis(100)).await;
        let _ = window.update(cx, |_, window, _| window.refresh());
    }
    let state = window.update(cx, |_, _, cx| {
        let p = panel.read(cx);
        let len = p.len();
        let rows: Vec<Value> = (len.saturating_sub(20)..len)
            .filter_map(|i| p.peek_row(i, cx).map(|r| (i, r)))
            .map(|(i, r)| match r {
                Row::Ws(m) => json!({
                    "row": i, "index": m.index, "direction": m.direction, "opcode": m.opcode, "len": m.len,
                    "preview": pm_view::stream::row_preview(&m),
                    "flags": pm_view::stream::message_flags(&m).iter().map(|f| f.label()).collect::<Vec<_>>(),
                }),
                Row::Sse(e) => json!({"row": i, "index": e.index, "event": e.event, "id": e.id, "data": e.data}),
            })
            .collect();
        let shown = p.shown(cx).map(|r| match r {
            Row::Ws(m) => json!({"index": m.index, "lines": pm_view::stream::detail_lines(&m, None)}),
            Row::Sse(e) => json!({"index": e.index, "fields": pm_view::stream::sse_fields(&e)}),
        });
        json!({
            "kind": format!("{:?}", p.kind()),
            "flow": {"store": p.flow().store, "id": p.flow().id},
            "open": p.is_open(),
            "sendable": p.sendable(),
            "len": len,
            "total": p.total(),
            "direction": p.direction(),
            "search": p.search_text(),
            "rows": rows,
            "selectedRow": p.selected_index(),
            "shown": shown,
            "original": p.original(),
            "viewer": p.viewer().map(|v| v.label()),
            "composeError": p.composer().and_then(|c| c.error.as_ref().map(|e| e.to_string())),
            "toast": crate::features::clipboard::Toasts::last(cx).map(|t| json!({"title": t.title, "description": t.description})),
        })
    });
    Some(state.unwrap_or_else(|e| json!({ "error": e.to_string() })))
}

/// The body viewers' verbs (G4); `None` when `command` has none of them.
async fn body_command(command: &Value, window: AnyWindowHandle, cx: &mut AsyncApp) -> Option<Value> {
    use crate::features::body;
    use pm_model::BodyPart;

    const VERBS: [&str; 4] = ["viewer", "loadAll", "highlightAnyway", "body"];
    if !VERBS.iter().any(|v| command.get(v).is_some()) {
        return None;
    }
    let Some(inspector) = cx.update(|cx| crate::shell::parts(cx).and_then(|p| p.inspector.upgrade())) else {
        return Some(json!({"error": "no inspector"}));
    };
    let part_of = |v: &Value| match v.as_str() {
        Some("request") => Ok(BodyPart::Request),
        Some("response") => Ok(BodyPart::Response),
        _ => Err(format!("takes request or response, not {v}")),
    };
    let result: Result<(), String> = window
        .update(cx, |_, _, cx| -> Result<(), String> {
            if let Some(v) = command.get("viewer") {
                let kind: pm_model::ContentKind =
                    serde_json::from_value(v["kind"].clone()).map_err(|_| format!("no content kind {}", v["kind"]))?;
                let viewer = v["viewer"]
                    .as_str()
                    .and_then(pm_view::body::registry::ViewerId::parse)
                    .ok_or_else(|| format!("no viewer {}", v["viewer"]))?;
                body::choose(kind, viewer, cx);
            }
            if let Some(v) = command.get("loadAll") {
                let pane = inspector.read(cx).body_pane(part_of(v)?).clone();
                pane.update(cx, |p, cx| p.load_all(cx));
            }
            if let Some(v) = command.get("highlightAnyway") {
                let pane = inspector.read(cx).body_pane(part_of(v)?).clone();
                pane.update(cx, |p, cx| p.highlight_anyway(cx));
            }
            Ok(())
        })
        .unwrap_or_else(|e| Err(e.to_string()));
    if let Err(err) = result {
        return Some(json!({ "error": err }));
    }
    // Let the body load and its viewer be prepared.
    for _ in 0..40 {
        cx.background_executor().timer(Duration::from_millis(50)).await;
        let busy = cx.update(|cx| {
            let i = inspector.read(cx);
            [BodyPart::Request, BodyPart::Response]
                .iter()
                .any(|p| i.body_pane(*p).read(cx).is_preparing())
        });
        if !busy {
            break;
        }
    }
    Some(cx.update(|cx| {
        let i = inspector.read(cx);
        json!({
            "request": i.body_pane(BodyPart::Request).read(cx).describe(cx),
            "response": i.body_pane(BodyPart::Response).read(cx).describe(cx),
        })
    }))
}

/// The shell's verbs (G2); `None` when `command` has none of them.
async fn shell_command(command: &Value, window: AnyWindowHandle, cx: &mut AsyncApp) -> Option<Value> {
    use crate::features::{capture, commands, documents};
    use crate::shell;
    use crate::workspace::{SaveKind, Workspace};

    const VERBS: [&str; 21] = [
        "tableShare",
        "filter",
        "search",
        "protocol",
        "content",
        "scope",
        "keys",
        "palette",
        "command",
        "store",
        "openDocument",
        "closeDocument",
        "saveSession",
        "exportHar",
        "recording",
        "clear",
        "contextMenu",
        "sidebarFilter",
        "workspace",
        "filterNow",
        "run",
    ];
    if !VERBS.iter().any(|v| command.get(v).is_some()) {
        return None;
    }
    let Some(ws) = cx.update(|cx| Workspace::global(cx)) else {
        return Some(json!({"error": "no workspace"}));
    };
    let mut extra = serde_json::Map::new();
    let result: Result<(), String> = window
        .update(cx, |_, window, cx| -> Result<(), String> {
            let bar = shell::parts(cx).and_then(|p| p.filter_bar.upgrade());
            if let Some(share) = command.get("tableShare").and_then(Value::as_f64)
                && let Some(main) = shell::parts(cx).and_then(|p| p.main.upgrade())
            {
                main.update(cx, |m, cx| m.set_table_share(share as f32, cx));
            }
            if let Some(text) = command.get("filter").and_then(Value::as_str) {
                let bar = bar.clone().ok_or("no filter bar")?;
                bar.update(cx, |b, cx| b.type_text(text, window, cx));
                if command.get("filterNow").and_then(Value::as_bool) == Some(true)
                    && let Some(table) = shell::parts(cx).and_then(|p| p.table.upgrade())
                {
                    let filter = ws.read(cx).filter.clone();
                    table.update(cx, |t, cx| t.set_filter(filter, cx));
                }
            }
            if let Some(text) = command.get("search").and_then(Value::as_str) {
                let bar = bar.clone().ok_or("no filter bar")?;
                // Typing puts the caret at the end, which focuses the box.
                bar.update(cx, |b, cx| b.type_text(text, window, cx));
                let labels: Vec<String> = bar.read(cx).suggestions(cx).into_iter().map(|s| s.label).collect();
                extra.insert("suggestions".into(), json!(labels));
            }
            if let Some(p) = command.get("protocol") {
                let p: pm_model::ProtocolFilter = serde_json::from_value(p.clone()).map_err(|e| e.to_string())?;
                ws.update(cx, |w, cx| w.set_protocol(p, cx));
            }
            if let Some(c) = command.get("content") {
                let groups: Vec<pm_model::ContentGroup> =
                    serde_json::from_value(c.clone()).map_err(|e| e.to_string())?;
                ws.update(cx, |w, cx| {
                    w.clear_content(cx);
                    for g in groups {
                        w.toggle_content(g, cx);
                    }
                });
            }
            if let Some(scope) = command.get("scope") {
                let get = |k: &str| scope.get(k).and_then(Value::as_str).map(str::to_owned);
                ws.update(cx, |w, cx| w.set_scope(get("host"), get("client"), cx));
            }
            if let Some(keys) = command.get("keys").and_then(Value::as_str) {
                for key in keys.split_whitespace() {
                    // Dev only: a keystroke parses from a `'static` string.
                    let key: &'static str = Box::leak(key.to_owned().into_boxed_str());
                    let stroke = gpui_kit::Keystroke::parse(key).map_err(|e| format!("{key}: {e:?}"))?;
                    window.dispatch_keystroke(stroke, cx);
                }
            }
            if let Some(open) = command.get("palette").and_then(Value::as_bool)
                && open != commands::is_palette_open(cx)
            {
                commands::toggle_palette(window, cx);
            }
            if let Some(id) = command.get("command").and_then(Value::as_str)
                && !commands::run(id, window, cx)
            {
                return Err(format!("{id} has no handler, or doesn't apply now"));
            }
            if let Some(store) = command.get("store").and_then(Value::as_u64) {
                ws.update(cx, |w, cx| w.set_store(store as pm_model::StoreId, cx));
            }
            if let Some(path) = command.get("openDocument").and_then(Value::as_str) {
                documents::open_path(path.into(), cx);
            }
            if let Some(store) = command.get("closeDocument").and_then(Value::as_u64) {
                ws.update(cx, |w, cx| w.close_document(store as pm_model::StoreId, cx));
            }
            if let Some(path) = command.get("saveSession").and_then(Value::as_str) {
                documents::save_to(SaveKind::Session, pm_model::FlowSelection::All, path.into(), cx);
            }
            if let Some(path) = command.get("exportHar").and_then(Value::as_str) {
                documents::export_shown(path.into(), cx);
            }
            if let Some(on) = command.get("recording").and_then(Value::as_bool) {
                ws.update(cx, |w, cx| w.set_recording(on, cx));
            }
            if command.get("clear").and_then(Value::as_bool) == Some(true) {
                capture::clear(cx);
            }
            if let Some(text) = command.get("sidebarFilter").and_then(Value::as_str)
                && let Some(sidebar) = shell::parts(cx).and_then(|p| p.sidebar.upgrade())
            {
                sidebar.update(cx, |s, cx| s.set_filter_text(text, window, cx));
            }
            if let Some(name) = command.get("contextMenu").and_then(Value::as_str) {
                let menu = match name.split_once(':') {
                    None if name == "listening" => shell::toolbar::current_listening_menu(cx),
                    None if name == "empty" => shell::toolbar::empty_state_menu(cx),
                    None if name == "save" => shell::footer::current_save_menu(cx),
                    Some(("tab", store)) => store
                        .parse()
                        .ok()
                        .and_then(|store| shell::tabs::current_tab_menu(store, cx)),
                    Some(("sidebar", key)) => shell::sidebar::menu_for(key, cx),
                    Some(("row", rest)) => row_menu(rest, cx),
                    Some(("body", part)) => body_menu(part, window, cx),
                    _ => inspector_menu(name, window, cx),
                }
                .ok_or_else(|| format!("no menu {name:?} now"))?;
                extra.insert("menu".into(), menu.to_json());
                if let Some(label) = command.get("run").and_then(Value::as_str)
                    && !menu.run(label, window, cx)
                {
                    return Err(format!("no entry {label:?}"));
                }
            }
            Ok(())
        })
        .unwrap_or_else(|e| Err(e.to_string()));
    if let Err(err) = result {
        return Some(json!({ "error": err }));
    }
    // Let deferred work (a copy's toast, a palette command) land.
    cx.background_executor().timer(Duration::from_millis(50)).await;
    Some(cx.update(|cx| {
        let w = ws.read(cx);
        let mut answer = json!({
            "store": w.store,
            "documents": w.documents,
            "filter": w.filter,
            "proxy": w.proxy,
            "save": w.save.as_ref().map(|j| json!({"label": j.label, "progress": j.progress})),
            "paletteOpen": crate::features::commands::is_palette_open(cx),
            "palette": crate::features::commands::palette_groups(cx)
                .into_iter()
                .map(|(g, c)| json!({"group": g.label(), "commands": c.into_iter().map(|c| c.title.into_owned()).collect::<Vec<_>>()}))
                .collect::<Vec<_>>(),
            "toast": crate::features::clipboard::Toasts::last(cx).map(|t| json!({"title": t.title, "description": t.description})),
            "clipboard": cx.read_from_clipboard().and_then(|c| c.text()),
        });
        if let Value::Object(map) = &mut answer {
            map.extend(extra);
        }
        answer
    }))
}
