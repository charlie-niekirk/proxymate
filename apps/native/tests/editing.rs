//! Editing (G8) driven like a user would: the Breakpoint Editor from an
//! injected queue and bodies (its forms, menus, the edits Execute sends,
//! the window opening and closing with the queue), a held WebSocket
//! message switching modes, and Compose sending with ⌘↩ through a real
//! (in-memory) engine to a pm-testkit upstream, plus the flow menu's
//! Replay items.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use pm_core::{Engine, EngineConfig};
use pm_model::{
    BreakpointInfo, HeldMessage, MessageEdits, Phase, ProxyStatus, RequestSpec, Scheme, WsDirection, WsOpcode,
};
use pm_view::breakpoints::MessageMode;
use pm_view::json::editor::Key;
use pm_view::sample;
use proxymate_native::bridge::{self, keys};
use proxymate_native::features::breakpoints::window::{BreakpointsWindow, Pane};
use proxymate_native::features::breakpoints::{self, form::HttpForm, message::MessageForm};
use proxymate_native::features::compose::ComposeWindow;
use proxymate_native::features::editing::body::BodyMode;
use proxymate_native::features::flow_menu::RowContext;
use proxymate_native::features::replay;
use proxymate_native::workspace::Workspace;

fn init(cx: &mut TestAppContext, engine: Option<Arc<Engine>>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        proxymate_native::prefs::init(None, cx);
        proxymate_native::features::register(cx);
        let ws = cx.new(|cx| Workspace::new(engine, ProxyStatus::default(), cx));
        Workspace::set_global(ws, cx);
    });
}

fn set_queue(cx: &mut VisualTestContext, queue: Vec<BreakpointInfo>) {
    cx.update(|_, cx| {
        bridge::breakpoints(cx).update(cx, |q, cx| {
            q.value = Some(queue);
            cx.notify();
        })
    });
    cx.run_until_parked();
}

fn set_body(cx: &mut VisualTestContext, id: u32, body: &[u8]) {
    let body = Arc::new(body.to_vec());
    cx.update(|_, cx| {
        keys::editing_caches(cx)
            .breakpoint_body
            .update(cx, |c, cx| c.set(id, body, cx))
    });
    cx.run_until_parked();
}

fn message_bp(id: u32) -> BreakpointInfo {
    let mut bp = sample::breakpoint(id, Phase::Request);
    bp.request.method = "GET".into();
    bp.request.url = "wss://api.example.com/socket".into();
    bp.message = Some(HeldMessage {
        direction: WsDirection::Sent,
        opcode: WsOpcode::Text,
    });
    bp
}

fn open_editor(
    cx: &mut TestAppContext,
    queue: Vec<BreakpointInfo>,
) -> (Entity<BreakpointsWindow>, &mut VisualTestContext) {
    init(cx, None);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let queue = bridge::breakpoints(cx);
        BreakpointsWindow::new(queue, window, cx)
    });
    let v = view.clone();
    cx.update(|window, cx| BreakpointsWindow::register(window.window_handle(), &v, cx));
    for bp in &queue {
        set_body(cx, bp.id, sample::BREAKPOINT_BODY.as_bytes());
    }
    set_queue(cx, queue);
    (view, cx)
}

fn http_form(view: &Entity<BreakpointsWindow>, cx: &mut VisualTestContext) -> Entity<HttpForm> {
    match cx.update(|_, cx| view.read(cx).pane().cloned()) {
        Some(Pane::Http(form)) => form,
        _ => panic!("no HTTP form"),
    }
}

fn message_form(view: &Entity<BreakpointsWindow>, cx: &mut VisualTestContext) -> Entity<MessageForm> {
    match cx.update(|_, cx| view.read(cx).pane().cloned()) {
        Some(Pane::Message(form)) => form,
        _ => panic!("no message form"),
    }
}

#[gpui_kit::test]
fn the_request_form_sends_only_its_edits(cx: &mut TestAppContext) {
    let (view, cx) = open_editor(cx, vec![sample::breakpoint(1, Phase::Request)]);
    let form = http_form(&view, cx);
    let edits = cx.update(|_, cx| form.read(cx).edits(cx)).unwrap();
    assert_eq!(edits, MessageEdits::default(), "nothing edited, nothing sent");

    // The head, a header and the body as a tree.
    form.update_in(cx, |f, window, cx| {
        f.set_head(
            Some("put"),
            Some("https://api.example.com/v1/orders?limit=3"),
            None,
            window,
            cx,
        );
        f.headers().update(cx, |h, cx| h.push("x-debug", "1", cx));
    });
    let body = cx
        .update(|_, cx| form.read(cx).body().cloned())
        .expect("the body loaded");
    body.update_in(cx, |b, window, cx| b.set_mode(BodyMode::Tree, window, cx));
    cx.run_until_parked();
    let tree = cx
        .update(|_, cx| body.read(cx).tree().cloned())
        .expect("JSON opens as a tree");
    // Rows: $, id, total, items: edit total.
    tree.update_in(cx, |t, window, cx| {
        t.press(Key::Down, window, cx);
        t.press(Key::Down, window, cx);
        t.press(Key::Enter, window, cx);
        t.type_text("13.00", window, cx);
        t.commit_input(window, cx);
    });
    cx.run_until_parked();
    let edits = cx.update(|_, cx| form.read(cx).edits(cx)).unwrap();
    assert_eq!(edits.method.as_deref(), Some("PUT"));
    assert_eq!(edits.url.as_deref(), Some("https://api.example.com/v1/orders?limit=3"));
    assert_eq!(edits.headers.unwrap().last().unwrap().name, "x-debug");
    // Numbers stay as written; only the edited one changed.
    assert_eq!(
        edits.body.as_deref(),
        Some(r#"{"id":"o1","total":13.00,"items":[1,2]}"#)
    );
    assert_eq!(edits.status, None);
}

#[gpui_kit::test]
fn execute_shows_the_engines_answer_inline(cx: &mut TestAppContext) {
    // An engine with nothing paused: the resolution fails, and says so.
    init(cx, Some(Engine::with_config(EngineConfig::default())));
    let (view, cx) = cx.add_window_view(|window, cx| {
        let queue = bridge::breakpoints(cx);
        BreakpointsWindow::new(queue, window, cx)
    });
    set_body(cx, 1, b"{}");
    set_queue(cx, vec![sample::breakpoint(1, Phase::Request)]);
    let form = http_form(&view, cx);
    form.update(cx, |f, cx| f.execute(None, cx));
    for _ in 0..50 {
        cx.run_until_parked();
        if cx.update(|_, cx| form.read(cx).error().is_some()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let error = cx.update(|_, cx| form.read(cx).error().map(str::to_owned));
    assert!(
        error.is_some_and(|e| e.contains('1')),
        "the unknown breakpoint is named"
    );
}

#[gpui_kit::test]
fn a_response_form_edits_the_status_and_responds(cx: &mut TestAppContext) {
    let (view, cx) = open_editor(
        cx,
        vec![
            sample::breakpoint(1, Phase::Request),
            sample::breakpoint(2, Phase::Response),
        ],
    );
    assert_eq!(
        cx.update(|_, cx| view.read(cx).shown(cx).map(|b| b.id)),
        Some(1),
        "the oldest first"
    );
    view.update_in(cx, |w, window, cx| w.select(2, window, cx));
    let form = http_form(&view, cx);
    form.update_in(cx, |f, window, cx| f.set_head(None, None, Some("503"), window, cx));
    let edits = cx.update(|_, cx| form.read(cx).edits(cx)).unwrap();
    assert_eq!(edits.status, Some(503));
    form.update_in(cx, |f, window, cx| f.set_head(None, None, Some("abc"), window, cx));
    assert!(cx.update(|_, cx| form.read(cx).edits(cx)).is_err());

    // Respond… opens the answer form; its status is checked too.
    form.update_in(cx, |f, window, cx| {
        f.start_responding(window, cx);
        f.set_answer_status("42", window, cx);
        f.respond(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| form.read(cx).error().map(str::to_owned)).as_deref(),
        Some("The status must be a number from 100 to 999")
    );

    // Resolved elsewhere (an agent): the next one is shown.
    set_queue(cx, vec![sample::breakpoint(1, Phase::Request)]);
    assert_eq!(cx.update(|_, cx| view.read(cx).shown(cx).map(|b| b.id)), Some(1));
    set_queue(cx, Vec::new());
    assert!(cx.update(|_, cx| view.read(cx).pane().is_none()));
}

#[gpui_kit::test]
fn the_queue_menu_names_the_flow_and_rule(cx: &mut TestAppContext) {
    let (view, cx) = open_editor(cx, vec![sample::breakpoint(1, Phase::Request), message_bp(2)]);
    let labels = cx.update(|_, cx| view.read(cx).menu_for(1, cx).unwrap().labels());
    assert_eq!(
        labels,
        [
            "Copy URL",
            "Copy method",
            "—",
            "Show flow #4",
            "Open rule “Pause orders”",
            "—",
            "Continue unmodified",
            "Abort"
        ]
    );
    let menu = cx.update(|_, cx| view.read(cx).menu_for(2, cx).unwrap());
    assert_eq!(menu.copy_text("Copy URL"), Some("wss://api.example.com/socket"));
    assert!(menu.labels().contains(&"Drop the message".to_owned()));
}

#[gpui_kit::test]
fn a_held_message_is_edited_as_a_tree_text_or_hex(cx: &mut TestAppContext) {
    let (view, cx) = open_editor(cx, vec![message_bp(5)]);
    let form = message_form(&view, cx);
    let mode = cx.update(|_, cx| form.read(cx).draft().map(|d| d.mode));
    assert_eq!(mode, Some(MessageMode::Tree), "JSON opens as a tree");
    let tree = cx.update(|_, cx| form.read(cx).tree().cloned()).unwrap();
    tree.update_in(cx, |t, window, cx| {
        t.press(Key::Down, window, cx);
        t.press(Key::Enter, window, cx);
        t.type_text("o9", window, cx);
        t.commit_input(window, cx);
    });
    cx.run_until_parked();
    let edits = cx.update(|_, cx| form.read(cx).edits()).unwrap();
    assert_eq!(
        edits.body.as_deref(),
        Some(r#"{"id":"o9","total":12.50,"items":[1,2]}"#)
    );

    // Hex carries the edit over; edited bytes go as base64.
    form.update_in(cx, |f, window, cx| f.switch_to(MessageMode::Hex, window, cx));
    form.update_in(cx, |f, window, cx| f.set_payload("7b 7d", window, cx));
    let edits = cx.update(|_, cx| form.read(cx).edits()).unwrap();
    assert_eq!(edits.body_base64.as_deref(), Some("e30="));
    form.update_in(cx, |f, window, cx| f.set_payload("zz", window, cx));
    assert!(cx.update(|_, cx| form.read(cx).edits()).is_err());
    form.update_in(cx, |f, window, cx| f.switch_to(MessageMode::Text, window, cx));
    assert!(
        cx.update(|_, cx| form.read(cx).error().map(str::to_owned)).is_some(),
        "bytes that aren't text stay hex"
    );
}

#[gpui_kit::test]
fn the_window_opens_when_something_pauses_and_closes_when_empty(cx: &mut TestAppContext) {
    init(cx, None);
    cx.update(breakpoints::watch_queue);
    let cx = cx.add_empty_window();
    assert!(cx.update(|_, cx| BreakpointsWindow::global(cx).is_none()));
    set_body(cx, 1, b"{}");
    set_queue(cx, vec![sample::breakpoint(1, Phase::Request)]);
    assert!(
        cx.update(|_, cx| BreakpointsWindow::global(cx).is_some()),
        "opened by the pause"
    );
    set_queue(cx, Vec::new());
    assert!(
        cx.update(|_, cx| BreakpointsWindow::global(cx).is_none()),
        "closed once empty"
    );
}

#[gpui_kit::test]
fn compose_sends_with_cmd_enter(cx: &mut TestAppContext) {
    // The send runs on tokio, whose threads wake the test's tasks.
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    let upstream = runtime.block_on(pm_testkit::Upstream::start());
    let engine = Engine::with_config(EngineConfig::default());
    init(cx, Some(engine.clone()));
    let spec = RequestSpec {
        method: "POST".into(),
        url: upstream.url("/echo"),
        headers: vec![sample::header("content-type", "application/json")],
        body: Some(r#"{"n":12345678901234567890}"#.into()),
        body_base64: false,
    };
    let (view, cx) = cx.add_window_view(|window, cx| ComposeWindow::new(Some(spec), window, cx));
    // Not registered as the open Compose window: a send would close it.
    let v = view.clone();
    cx.update(|window, cx| {
        let focus = v.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    cx.run_until_parked();
    assert!(cx.update(|_, cx| view.read(cx).is_edit_replay()));
    // ⌘↩ on macOS, Ctrl+↩ elsewhere.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    });
    let mut sent = None;
    for _ in 0..500 {
        cx.run_until_parked();
        sent = cx.update(|_, cx| view.read(cx).sent);
        let error = cx.update(|_, cx| view.read(cx).error().map(str::to_owned));
        assert!(error.is_none(), "the send failed: {error:?}");
        if sent.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let id = sent.expect("⌘↩ sent the request");
    engine.flush();
    let flow = engine.get_flow_in(pm_model::LIVE_STORE, id).expect("recorded live");
    assert_eq!(&*flow.method, "POST");
    assert!(flow.sent_by.is_some(), "marked as sent by the app");
}

#[gpui_kit::test]
fn the_flow_menu_offers_replay_for_plain_requests(cx: &mut TestAppContext) {
    init(cx, None);
    let row = sample::row();
    let mut ws = sample::row();
    ws.2 = Scheme::Wss;
    let menu = |row: pm_model::FlowRow, cx: &mut TestAppContext| {
        cx.update(|cx| {
            replay::send_items(
                &RowContext {
                    store: 0,
                    row,
                    column: None,
                    live: true,
                },
                cx,
            )
        })
    };
    let plain = menu(row, cx);
    assert_eq!(plain.labels(), ["Replay", "Edit & Replay…"]);
    assert!(plain.is_enabled("Replay"));
    let socket = menu(ws, cx);
    assert!(!socket.is_enabled("Replay") && !socket.is_enabled("Edit & Replay…"));
}
