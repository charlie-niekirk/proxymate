//! The stream panels (G6) driven from injected snapshots (no engine): a
//! WebSocket flow's rows, flags, filters, menus, detail and composer, and
//! an SSE flow's fields and copies. The list heads and pages come from
//! the bridge's `ws_*` and `sse_*` caches, as the engine would fill them.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use pm_model::{FlowSummary, ProxyStatus, SidebarSnapshot, StorageStatus, WsDirection, WsMessage};
use pm_view::sample;
use pm_view::stream::{ComposeMode, NOT_HEX};
use proxymate_native::bridge::keys::{self, FlowKey, Inspected, PageKey, SseListKey, WsListKey, WsPayloadKey};
use proxymate_native::bridge::stream::Latest;
use proxymate_native::features::clipboard::Toasts;
use proxymate_native::features::context_menu::Menu;
use proxymate_native::features::flow_table::FlowTable;
use proxymate_native::features::inspector::Inspector;
use proxymate_native::features::inspector::streams::{Row, StreamPanel, Viewer};
use proxymate_native::shell::{self, MainWindow, Parts};
use proxymate_native::workspace::Workspace;

const WS: u32 = 6;
const SSE: u32 = 7;

/// Flows 0–7: 6 is the WebSocket, 7 the event stream.
fn flows() -> Vec<FlowSummary> {
    (0..8u32)
        .map(|id| match id {
            WS => sample::ws_summary(),
            SSE => sample::sse_summary(),
            _ => {
                let mut f = sample::summary();
                f.id = id;
                f
            }
        })
        .collect()
}

struct Window {
    table: Entity<FlowTable>,
    inspector: Entity<Inspector>,
}

fn open(cx: &mut TestAppContext) -> (Window, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        proxymate_native::prefs::init(None, cx);
        proxymate_native::features::register(cx);
    });
    let workspace = cx.new(|cx| {
        Workspace::new(
            None,
            ProxyStatus {
                listening: Some("127.0.0.1:9090".into()),
                recording: true,
                ..Default::default()
            },
            cx,
        )
    });
    cx.update(|cx| Workspace::set_global(workspace.clone(), cx));
    let storage = cx.new(|_| Latest {
        value: Some(StorageStatus::default()),
    });
    let sidebar = cx.new(|_| Latest {
        value: Some(SidebarSnapshot::default()),
    });
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let rows = flows().iter().map(sample::row_of).collect();
        let table = cx.new(|cx| FlowTable::snapshot(rows, 8, cx));
        let parts = Parts {
            workspace,
            storage,
            sidebar,
            engine: None,
        };
        let main = cx.new(|cx| MainWindow::new(parts, table, window, cx));
        shell::root(main, window, cx)
    });
    cx.run_until_parked();
    let main = cx.update(|_, cx| {
        shell::parts(cx)
            .and_then(|p| p.main.upgrade())
            .expect("the main window")
    });
    let (table, inspector) = cx.update(|_, cx| {
        let m = main.read(cx);
        (m.table().clone(), m.inspector().clone())
    });
    (Window { table, inspector }, cx)
}

fn ws_list(direction: Option<WsDirection>, text: &str) -> WsListKey {
    WsListKey {
        flow: FlowKey::new(0, WS),
        direction,
        text: text.into(),
    }
}

/// Puts `messages` in the caches as the list for `direction` and `text`,
/// open or not.
fn put_messages(
    cx: &mut VisualTestContext,
    direction: Option<WsDirection>,
    text: &str,
    messages: Vec<WsMessage>,
    open: bool,
) {
    cx.update(|_, cx| {
        let caches = keys::caches(cx);
        let list = ws_list(direction, text);
        let mut head = sample::ws_page(&messages, open);
        head.total = sample::ws_messages().len() as u32;
        caches
            .ws_head
            .update(cx, |c, cx| c.set(list.clone(), Arc::new(head), cx));
        let page = PageKey {
            list,
            page: 0,
            dropped: 0,
        };
        caches.ws_rows.update(cx, |c, cx| c.set(page, Arc::new(messages), cx));
    });
}

/// Inspects the WebSocket flow with its messages, open.
fn inspect_ws(w: &Window, cx: &mut VisualTestContext, open: bool) -> Entity<StreamPanel> {
    put_messages(cx, None, "", sample::ws_messages(), open);
    cx.update(|_, cx| {
        let caches = keys::caches(cx);
        let inspected = Inspected {
            flow: sample::ws_summary(),
            detail: Some(sample::ws_detail()),
        };
        caches
            .inspected
            .update(cx, |c, cx| c.set(FlowKey::new(0, WS), Some(Arc::new(inspected)), cx));
        for m in sample::ws_messages() {
            let key = WsPayloadKey {
                flow: FlowKey::new(0, WS),
                index: m.index,
                original: false,
            };
            let bytes = match m.opcode {
                pm_model::WsOpcode::Binary => vec![0xde, 0xad, 0xbe, 0xef, 0, 1],
                _ => m.preview.clone().into_bytes(),
            };
            caches
                .ws_payload
                .update(cx, |c, cx| c.set(key, Some(Arc::new(bytes)), cx));
        }
        let original = WsPayloadKey {
            flow: FlowKey::new(0, WS),
            index: 4,
            original: true,
        };
        caches.ws_payload.update(cx, |c, cx| {
            c.set(original, Some(Arc::new(br#"{"price":102}"#.to_vec())), cx)
        });
        let grid = w.table.read(cx).grid().clone();
        grid.update(cx, |g, cx| g.select(Some(WS), cx));
    });
    cx.run_until_parked();
    panel(w, cx)
}

fn panel(w: &Window, cx: &mut VisualTestContext) -> Entity<StreamPanel> {
    cx.update(|_, cx| w.inspector.read(cx).stream_panel(cx))
        .expect("a WebSocket or SSE flow has a stream panel")
}

fn menu(w: &Window, cx: &mut VisualTestContext, name: &str) -> Menu {
    cx.update(|_, cx| w.inspector.read(cx).menu_for(name, None, cx))
        .unwrap_or_else(|| panic!("no menu {name:?}"))
}

fn run(w: &Window, cx: &mut VisualTestContext, name: &str, label: &str) -> Option<String> {
    let menu = menu(w, cx, name);
    assert!(
        cx.update(|window, cx| menu.run(label, window, cx)),
        "{name}: no entry {label:?}"
    );
    cx.run_until_parked();
    cx.read_from_clipboard().and_then(|c| c.text())
}

#[gpui_kit::test]
fn a_websocket_flow_lists_its_messages_with_their_flags(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    let panel = inspect_ws(&w, cx, true);
    cx.update(|window, cx| {
        let p = panel.read(cx);
        assert_eq!((p.len(), p.total()), (9, 9));
        assert!(p.is_open() && p.sendable());
        assert!(window.try_find("stream-toolbar").is_some());
        assert!(
            window.try_find("ws-composer").is_some(),
            "an open live connection takes messages"
        );
        assert!(window.try_find("ws-close").is_some());
        assert!(window.try_find("stream-list").is_some());
        // Rows 3, 4 and 5 are injected, edited and dropped.
        let flags = |i| match p.peek_row(i, cx) {
            Some(Row::Ws(m)) => pm_view::stream::message_flags(&m),
            _ => panic!("row {i} isn't loaded"),
        };
        use pm_view::stream::Flag;
        assert_eq!(
            [flags(3), flags(4), flags(5)],
            [vec![Flag::Injected], vec![Flag::Edited], vec![Flag::Dropped]]
        );
    });
    // Nothing is selected: the detail asks for one.
    assert!(cx.update(|_, cx| panel.read(cx).shown(cx)).is_none());
    // The rows on screen draw their badges.
    cx.update(|_, cx| panel.update(cx, |p, cx| p.select_index(4, cx)));
    cx.run_until_parked();
    assert!(
        cx.update(|window, _| window.try_find("ws-flag-12").is_some()),
        "Edited on message 4"
    );

    // A direction filter is its own list.
    let received: Vec<_> = sample::ws_messages()
        .into_iter()
        .filter(|m| m.direction == WsDirection::Received)
        .collect();
    put_messages(cx, Some(WsDirection::Received), "", received, true);
    cx.update(|_, cx| panel.update(cx, |p, cx| p.set_direction(Some(WsDirection::Received), cx)));
    cx.run_until_parked();
    cx.update(|_, cx| {
        let p = panel.read(cx);
        assert_eq!((p.len(), p.total()), (5, 9));
        assert!(matches!(p.peek_row(0, cx), Some(Row::Ws(m)) if m.index == 1));
    });
    // So is a search; the clear button empties it.
    put_messages(cx, None, "price", sample::ws_messages()[2..=4].to_vec(), true);
    cx.update(|window, cx| {
        panel.update(cx, |p, cx| {
            p.set_direction(None, cx);
            p.set_search("price", window, cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| panel.read(cx).len()), 3);
    cx.update(|window, cx| window.click("stream-search-clear", cx));
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!((panel.read(cx).search_text(), panel.read(cx).len()), ("", 9)));
}

#[gpui_kit::test]
fn a_selected_message_shows_its_payload_and_the_original(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    let panel = inspect_ws(&w, cx, true);
    cx.update(|_, cx| panel.update(cx, |p, cx| p.select_index(0, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(window.try_find("ws-message-detail").is_some());
        assert!(window.try_find("ws-detail-resend").is_some());
        assert_eq!(panel.read(cx).viewer(), Some(Viewer::Text));
    });
    assert_eq!(
        run(&w, cx, "payload", "Copy payload").as_deref(),
        Some(r#"{"type":"subscribe","channel":"prices"}"#)
    );
    // Binary opens as hex.
    cx.update(|_, cx| panel.update(cx, |p, cx| p.select_index(6, cx)));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| panel.read(cx).viewer()), Some(Viewer::Hex));
    assert_eq!(
        run(&w, cx, "payload", "Copy payload as hex").as_deref(),
        Some("de ad be ef 00 01")
    );
    // An edited message: Forwarded, then Original.
    cx.update(|_, cx| panel.update(cx, |p, cx| p.select_index(4, cx)));
    cx.run_until_parked();
    assert!(cx.update(|window, _| window.try_find("ws-edited-bar").is_some()));
    assert_eq!(
        run(&w, cx, "payload", "Copy payload").as_deref(),
        Some(r#"{"price":999}"#)
    );
    cx.update(|window, cx| window.click("ws-show-original", cx));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| panel.read(cx).original()));
    assert_eq!(
        run(&w, cx, "payload", "Copy payload").as_deref(),
        Some(r#"{"price":102}"#)
    );
    // ↑ moves the selection.
    cx.update(|window, cx| {
        let focus = gpui_kit::Focusable::focus_handle(panel.read(cx), cx);
        window.focus(&focus, cx);
    });
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| panel.read(cx).selected_index()), Some(3));
}

#[gpui_kit::test]
fn row_menus_copy_and_offer_resend_only_while_sendable(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    inspect_ws(&w, cx, true);
    assert_eq!(
        menu(&w, cx, "stream:0").labels(),
        [
            "Copy payload",
            "Copy payload as Base64",
            "Copy payload as hex",
            "—",
            "Resend to the server"
        ]
    );
    let binary = menu(&w, cx, "stream:6");
    assert!(!binary.is_enabled("Copy payload"), "binary isn't text");
    assert!(binary.is_enabled("Copy payload as Base64"));
    assert_eq!(
        menu(&w, cx, "stream:7").labels(),
        ["Copy payload", "Copy payload as Base64", "Copy payload as hex"],
        "a ping can't be resent"
    );
    assert_eq!(
        run(&w, cx, "stream:8", "Copy close code and reason").as_deref(),
        Some("1000 Normal Closure · bye")
    );
    let toast = cx.update(|_, cx| Toasts::last(cx).map(|t| t.title.to_string()));
    assert_eq!(toast.as_deref(), Some("Copied close reason"));

    // Closed: no Resend, no composer, no Close.
    put_messages(cx, None, "", sample::ws_messages(), false);
    cx.run_until_parked();
    assert_eq!(
        menu(&w, cx, "stream:0").labels().last().map(String::as_str),
        Some("Copy payload as hex")
    );
    cx.update(|window, _| {
        assert!(window.try_find("ws-composer").is_none());
        assert!(window.try_find("ws-close").is_none());
    });
}

#[gpui_kit::test]
fn the_composer_refuses_what_isnt_hex(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    let panel = inspect_ws(&w, cx, true);
    cx.update(|window, cx| {
        panel.update(cx, |p, cx| {
            p.compose("zz", Some(WsDirection::Received), Some(ComposeMode::Hex), window, cx)
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.click("ws-compose-send", cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let c = panel.read(cx).composer().expect("a composer");
        assert_eq!(c.error.as_deref(), Some(NOT_HEX));
        assert_eq!((c.direction, c.mode), (WsDirection::Received, ComposeMode::Hex));
        assert!(window.try_find("ws-compose-error").is_some());
    });
    // Text mode reads anything; with no engine nothing goes out, but the
    // error clears.
    cx.update(|window, cx| window.click("ws-compose-text", cx));
    cx.run_until_parked();
    cx.update(|_, cx| panel.update(cx, |p, cx| p.send(cx)));
    cx.run_until_parked();
    let error = cx.update(|_, cx| panel.read(cx).composer().and_then(|c| c.error.clone()));
    assert_eq!(error.as_deref(), Some("No engine"));
}

#[gpui_kit::test]
fn an_event_stream_lists_its_events_fields_and_copies(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    cx.update(|_, cx| {
        let caches = keys::caches(cx);
        let events = sample::sse_events();
        let list = SseListKey {
            flow: FlowKey::new(0, SSE),
            text: String::new(),
        };
        caches.sse_head.update(cx, |c, cx| {
            c.set(list.clone(), Arc::new(sample::sse_page(&events, true)), cx)
        });
        let page = PageKey {
            list,
            page: 0,
            dropped: 0,
        };
        caches.sse_rows.update(cx, |c, cx| c.set(page, Arc::new(events), cx));
        let inspected = Inspected {
            flow: sample::sse_summary(),
            detail: Some(sample::sse_detail()),
        };
        caches
            .inspected
            .update(cx, |c, cx| c.set(FlowKey::new(0, SSE), Some(Arc::new(inspected)), cx));
        let grid = w.table.read(cx).grid().clone();
        grid.update(cx, |g, cx| g.select(Some(SSE), cx));
    });
    cx.run_until_parked();
    let panel = panel(&w, cx);
    cx.update(|window, cx| {
        let p = panel.read(cx);
        assert_eq!((p.len(), p.total(), p.is_open(), p.sendable()), (6, 6, true, false));
        assert!(window.try_find("ws-composer").is_none());
    });
    assert_eq!(
        menu(&w, cx, "stream:0").labels(),
        ["Copy data", "Copy as event-stream text", "Copy event name", "Copy id"]
    );
    assert_eq!(
        run(&w, cx, "stream:0", "Copy as event-stream text").as_deref(),
        Some("id: evt-0\nretry: 3000\ndata: {\"n\":0,\"msg\":\"update 0\"}")
    );
    assert_eq!(run(&w, cx, "stream:1", "Copy event name").as_deref(), Some("tick"));
    assert_eq!(menu(&w, cx, "stream:1").labels().len(), 3, "no id, no Copy id");

    cx.update(|_, cx| panel.update(cx, |p, cx| p.select_index(0, cx)));
    cx.run_until_parked();
    cx.update(|window, _| {
        assert!(window.try_find("sse-event-detail").is_some());
        assert!(window.try_find("sse-fields").is_some());
    });
    assert_eq!(
        run(&w, cx, "payload", "Copy data").as_deref(),
        Some("{\"n\":0,\"msg\":\"update 0\"}")
    );
}
