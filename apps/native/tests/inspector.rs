//! The inspector (G3) driven like a user would, from injected snapshots
//! (no engine): the flow comes from the `inspected` cache, its menus copy
//! the right text into the test clipboard, ↩ and Escape move the keyboard
//! between the table and the inspector, the flow row menu copies and
//! filters, and the failure banner's fixes dispatch what they should.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use pm_model::{
    AppliedRule, BodyPart, ErrorKind, FlowState, FlowSummary, Phase, ProxyStatus, RuleOrigin, SidebarSnapshot,
    StorageStatus,
};
use pm_view::columns::ColumnId;
use pm_view::sample;
use proxymate_native::bridge::keys::{self, BodyKey, FlowKey, Inspected};
use proxymate_native::bridge::stream::Latest;
use proxymate_native::features::clipboard::Toasts;
use proxymate_native::features::context_menu::Menu;
use proxymate_native::features::flow_table::FlowTable;
use proxymate_native::features::inspector::{Inspector, RequestTab, ResponseTab};
use proxymate_native::shell::{self, MainWindow, Parts};
use proxymate_native::workspace::Workspace;

/// Flows 0–4 like the sample: flow 3 is the sample itself; flow 4 failed.
fn flows() -> Vec<FlowSummary> {
    (0..5u32)
        .map(|id| {
            let mut f = sample::summary();
            f.id = id;
            if id == 4 {
                f.state = FlowState::Failed;
                f.status = None;
            }
            f
        })
        .collect()
}

struct Window {
    workspace: Entity<Workspace>,
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
    let ws = workspace.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let rows = flows().iter().map(sample::row_of).collect();
        let table = cx.new(|cx| FlowTable::snapshot(rows, 5, cx));
        let parts = Parts {
            workspace: ws,
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
    (
        Window {
            workspace,
            table,
            inspector,
        },
        cx,
    )
}

/// Puts flow `id` with `detail` in the cache and selects its row.
fn inspect(w: &Window, cx: &mut VisualTestContext, id: u32, detail: Option<pm_model::FlowDetail>) {
    let flow = flows().remove(id as usize);
    cx.update(|_, cx| {
        let caches = keys::caches(cx);
        caches.inspected.update(cx, |c, cx| {
            c.set(FlowKey::new(0, id), Some(Arc::new(Inspected { flow, detail })), cx)
        });
        let grid = w.table.read(cx).grid().clone();
        grid.update(cx, |g, cx| g.select(Some(id), cx));
    });
    cx.run_until_parked();
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}

fn last_toast(cx: &mut VisualTestContext) -> (String, Option<String>) {
    cx.update(|_, cx| {
        Toasts::last(cx)
            .map(|t| (t.title.to_string(), t.description.map(|d| d.to_string())))
            .unwrap_or_default()
    })
}

/// The inspector's menu `name`.
fn menu(w: &Window, cx: &mut VisualTestContext, name: &str) -> Menu {
    cx.update(|_, cx| w.inspector.read(cx).menu_for(name, None, cx))
        .unwrap_or_else(|| panic!("no menu {name:?}"))
}

/// Runs `label` of the menu `name` and reads the clipboard.
fn copy(w: &Window, cx: &mut VisualTestContext, name: &str, label: &str) -> Option<String> {
    let menu = menu(w, cx, name);
    let ran = cx.update(|window, cx| menu.run(label, window, cx));
    assert!(ran, "{name}: no entry {label:?}");
    cx.run_until_parked();
    clipboard(cx)
}

fn sample_detail(id: u32) -> pm_model::FlowDetail {
    let mut d = sample::detail();
    d.id = id;
    d
}

#[gpui_kit::test]
fn the_inspector_follows_the_focused_flow_and_says_when_it_cant(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    assert!(cx.update(|_, cx| w.inspector.read(cx).key(cx)).is_none());
    assert!(
        cx.update(|window, _| window.try_find("inspector-message").is_some()),
        "Select a flow"
    );
    inspect(&w, cx, 3, Some(sample_detail(3)));
    assert!(cx.update(|window, _| window.try_find("url-bar").is_some()));
    assert!(cx.update(|window, _| window.try_find("request-headers").is_some()));
    // A flow the store no longer has.
    cx.update(|_, cx| {
        let caches = keys::caches(cx);
        caches.inspected.update(cx, |c, cx| c.set(FlowKey::new(0, 3), None, cx));
    });
    cx.run_until_parked();
    assert!(
        cx.update(|window, _| window.try_find("inspector-message").is_some()),
        "no longer available"
    );
    // A synthetic flow has no detail: both panes say so.
    inspect(&w, cx, 2, None);
    assert!(cx.update(|window, _| window.try_find("url-bar").is_some()));
    assert!(cx.update(|window, _| window.try_find("request-headers").is_none()));
}

#[gpui_kit::test]
fn the_url_bar_and_tables_copy_the_right_text(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    inspect(&w, cx, 3, Some(sample_detail(3)));
    assert_eq!(
        copy(&w, cx, "url", "Copy URL").as_deref(),
        Some("https://api.example.com/v1/users?page=2&q=a+b")
    );
    assert_eq!(last_toast(cx).0, "Copied URL");
    assert_eq!(copy(&w, cx, "url", "Copy status").as_deref(), Some("200 OK"));
    assert_eq!(
        copy(&w, cx, "url", "Copy path and query").as_deref(),
        Some("/v1/users?page=2&q=a+b")
    );

    // Request headers: row 2 is Authorization, row 3 Cookie.
    assert_eq!(
        copy(&w, cx, "kv:request-headers:2", "Copy name").as_deref(),
        Some("Authorization")
    );
    assert_eq!(last_toast(cx).0, "Copied header name");
    assert_eq!(
        copy(&w, cx, "kv:request-headers:2", "Copy as “Name: value”").as_deref(),
        Some("Authorization: Bearer eyJhbGciOi.payload.sig")
    );
    assert_eq!(
        copy(&w, cx, "kv:request-headers:2", "Copy token").as_deref(),
        Some("eyJhbGciOi.payload.sig")
    );
    assert_eq!(
        copy(&w, cx, "kv:request-headers:3", "Copy cookie › theme").as_deref(),
        Some("dark")
    );
    assert_eq!(last_toast(cx).0, "Copied cookie theme");
    assert_eq!(
        copy(&w, cx, "kv:response-headers:1", "Copy cookie value").as_deref(),
        Some("xyz789")
    );
    let all = copy(&w, cx, "kv:response-headers", "Copy all headers").unwrap();
    assert!(all.starts_with("Content-Type: application/json; charset=utf-8\nSet-Cookie: "));
    let json = copy(&w, cx, "kv:query-params", "Copy all as JSON").unwrap();
    assert_eq!(json, "{\n  \"page\": \"2\",\n  \"q\": \"a b\"\n}");
    assert_eq!(
        copy(&w, cx, "kv:query-params:1", "Copy as name=value").as_deref(),
        Some("q=a+b")
    );

    // "Filter flows with this header" narrows the search box.
    let ran = {
        let menu = menu(&w, cx, "kv:response-headers:2");
        cx.update(|window, cx| menu.run("Filter flows with this header", window, cx))
    };
    assert!(ran);
    cx.run_until_parked();
    let text = cx.update(|_, cx| w.workspace.read(cx).filter.text.clone());
    assert_eq!(text, "resheader:\"X-Request-Id: req-42\"");

    // TLS: the connection and the certificates.
    assert_eq!(
        copy(&w, cx, "kv:tls-connection:3", "Copy row").as_deref(),
        Some("Upstream TLS version: TLSv1.3")
    );
    assert_eq!(
        copy(&w, cx, "cert:0", "Copy SHA-256 without colons").as_deref(),
        Some("ABCDEF01")
    );
    assert_eq!(
        copy(&w, cx, "kv:cert-1:0", "Copy value").as_deref(),
        Some("CN=Example Root")
    );
    // Timing: a phase, and the whole table tab-separated.
    assert_eq!(copy(&w, cx, "timing:0", "Copy DNS").as_deref(), Some("3.2 ms"));
    let table = copy(&w, cx, "timing", "Copy timing as table").unwrap();
    assert!(
        table.contains("Total\t184 ms\nRoute\tDirect\nConnected to\t93.184.215.14:443"),
        "{table}"
    );
}

#[gpui_kit::test]
fn the_raw_tab_rebuilds_the_message_and_copies_its_parts(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    let mut detail = sample_detail(3);
    detail.response_body.size = 11;
    detail.response_body.captured = 11;
    cx.update(|_, cx| {
        let caches = keys::caches(cx);
        let key = BodyKey {
            flow: FlowKey::new(0, 3),
            part: BodyPart::Response,
        };
        caches
            .body
            .update(cx, |c, cx| c.set(key, Arc::new(b"{\"ok\":true}".to_vec()), cx));
    });
    inspect(&w, cx, 3, Some(detail));
    cx.update(|_, cx| {
        w.inspector.update(cx, |i, cx| {
            i.set_request_tab(RequestTab::Raw, cx);
            i.set_response_tab(ResponseTab::Raw, cx);
        })
    });
    cx.run_until_parked();
    assert!(cx.update(|window, _| window.try_find("raw-response").is_some()));
    let all = copy(&w, cx, "raw:response", "Copy all").unwrap();
    assert!(
        all.starts_with("HTTP/2 200 OK\nContent-Type: application/json; charset=utf-8\n"),
        "{all}"
    );
    assert!(all.ends_with("\n\n{\"ok\":true}"), "{all}");
    assert_eq!(
        copy(&w, cx, "raw:response", "Copy status line").as_deref(),
        Some("HTTP/2 200 OK")
    );
    let request = copy(&w, cx, "raw:request", "Copy all").unwrap();
    assert!(
        request.starts_with("GET https://api.example.com/v1/users?page=2&q=a+b HTTP/2\n:authority: api.example.com\n")
    );
    assert!(request.ends_with("\n\n"), "no request body");
}

#[gpui_kit::test]
fn rule_badges_and_the_failure_banner(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    let mut detail = sample_detail(4);
    detail.response = None;
    detail.error = Some("connecting to api.example.com:443 timed out".into());
    detail.error_kind = Some(ErrorKind::Timeout);
    detail.rules.applied = vec![AppliedRule {
        rule_id: "r-7".into(),
        name: "Slow down".into(),
        origin: RuleOrigin::User,
        phase: Phase::Request,
    }];
    inspect(&w, cx, 4, Some(detail));
    assert_eq!(copy(&w, cx, "rule:0", "Copy rule id").as_deref(), Some("r-7"));
    assert_eq!(
        copy(&w, cx, "failure", "Copy title and error").as_deref(),
        Some("The server didn't answer in time\nconnecting to api.example.com:443 timed out")
    );
    assert!(cx.update(|window, _| window.try_find("failure-banner").is_some()));
    // The fix opens Settings › Advanced (G12's window says so until then).
    cx.update(|window, cx| window.click("fix-timeouts", cx));
    cx.run_until_parked();
    assert_eq!(last_toast(cx).0, "Settings › Advanced");
    // A badge's click opens the rule (G7's window).
    cx.update(|window, cx| window.click(("rule-badge", 0usize), cx));
    cx.run_until_parked();
    assert_eq!(last_toast(cx).0, "Rules");
}

#[gpui_kit::test]
fn enter_and_escape_move_the_keyboard_between_the_table_and_the_inspector(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    inspect(&w, cx, 3, Some(sample_detail(3)));
    let grid = cx.update(|_, cx| w.table.read(cx).grid().clone());
    cx.update(|window, cx| window.focus(&grid.focus_handle(cx), cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let in_inspector = cx.update(|window, cx| w.inspector.focus_handle(cx).contains_focused(window, cx));
    assert!(in_inspector, "↩ moves into the inspector");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        cx.update(|window, cx| grid.focus_handle(cx).is_focused(window)),
        "Escape returns"
    );
    // The palette's "Focus the inspector" does the same.
    cx.update(|window, cx| assert!(proxymate_native::features::commands::run("inspector", window, cx)));
    cx.run_until_parked();
    assert!(cx.update(|window, cx| w.inspector.focus_handle(cx).contains_focused(window, cx)));
}

#[gpui_kit::test]
fn the_flow_row_menu_copies_cells_and_filters(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    inspect(&w, cx, 3, Some(sample_detail(3)));
    let row_menu = |cx: &mut VisualTestContext, column: Option<ColumnId>| {
        cx.update(|_, cx| w.table.read(cx).row_menu_for(3, column.as_ref(), cx))
            .expect("the row menu")
    };
    let menu = row_menu(cx, ColumnId::parse("status"));
    assert_eq!(
        menu.labels(),
        [
            "Copy Status  200",
            "Copy URL",
            "Copy as cURL",
            "Copy",
            "Filter by",
            "—",
            "Pin",
            "—",
            "Export HAR…",
            "Add api.example.com to SSL bypass",
            "Never record api.example.com",
            "—",
            "Delete"
        ]
    );
    cx.update(|window, cx| menu.run("Copy Status  200", window, cx));
    cx.run_until_parked();
    assert_eq!(clipboard(cx).as_deref(), Some("200"));
    assert_eq!(last_toast(cx).0, "Copied status");
    cx.update(|window, cx| menu.run("Copy › Host", window, cx));
    cx.run_until_parked();
    assert_eq!(clipboard(cx).as_deref(), Some("api.example.com"));
    cx.update(|window, cx| menu.run("Filter by › App: Safari", window, cx));
    cx.run_until_parked();
    let text = cx.update(|_, cx| w.workspace.read(cx).filter.text.clone());
    assert_eq!(text, "app:Safari");
    // From the "…" button there is no cell to copy; an active flow can't be deleted.
    let menu = row_menu(cx, None);
    assert_eq!(menu.labels()[0], "Copy URL");
    assert!(menu.is_enabled("Delete"));
    // A failed flow has no status to copy or filter by.
    let failed = cx
        .update(|_, cx| w.table.read(cx).row_menu_for(4, ColumnId::parse("status").as_ref(), cx))
        .unwrap();
    assert_eq!(failed.labels()[0], "Copy URL");
    assert!(failed.find("Filter by › Status: 200").is_none());
}
