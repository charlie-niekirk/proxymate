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
//!
//! Every phase adds the verbs its surfaces need (see `docs/PLAN.md`).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, AsyncApp, ElementId};
use serde_json::{Value, json};

use crate::prefs::Appearance;

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
    let Some(window) = cx.update(|cx| cx.windows().first().copied()) else {
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
    if let Some(answer) = table_command(&command, cx) {
        return answer;
    }
    if let Some(path) = command.get("screenshot").and_then(Value::as_str) {
        let offscreen = command.get("offscreen").and_then(Value::as_bool).unwrap_or(false);
        return screenshot(window, PathBuf::from(path), offscreen, cx).await;
    }
    json!({"error": "unknown command: try theme, screenshot, click, select, selectIds, selectAll, scrollTo, headerColumns, filter, autoSelect or table"})
}

async fn screenshot(window: AnyWindowHandle, path: PathBuf, offscreen: bool, cx: &mut AsyncApp) -> Value {
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
    const VERBS: [&str; 7] = [
        "select",
        "selectIds",
        "selectAll",
        "headerColumns",
        "filter",
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
        if let Some(text) = command.get("filter").and_then(Value::as_str) {
            let filter = pm_model::FilterSpec {
                text: text.into(),
                ..table.read(cx).filter().clone()
            };
            table.update(cx, |t, cx| t.set_filter(filter, cx));
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
