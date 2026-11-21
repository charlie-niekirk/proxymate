//! The main window's chrome (G2) driven like a user would, from injected
//! snapshots (no engine): its context menus copy the right text, ⌘K opens
//! the palette, ⌘F and the search box's autocomplete keys, the chips, the
//! sidebar's rows and ⌘L.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use pm_model::{
    AppCount, ContentGroup, DocumentInfo, DocumentKind, DomainNode, FlowRow, FlowState, HostCount, HttpVersion,
    ProtocolFilter, ProxyStatus, SaveProgress, Scheme, SidebarSnapshot, StorageStatus, TlsStatus,
};
use proxymate_native::bridge::stream::Latest;
use proxymate_native::features::clipboard::Toasts;
use proxymate_native::features::commands::{self, MAC};
use proxymate_native::features::flow_table::FlowTable;
use proxymate_native::shell::{self, MainWindow, Parts, footer, sidebar, tabs, toolbar};
use proxymate_native::workspace::{SaveJob, Workspace};

fn primary(key: &str) -> String {
    format!("{}-{key}", if MAC { "cmd" } else { "ctrl" })
}

fn rows() -> Vec<FlowRow> {
    (0..5u32)
        .map(|i| {
            FlowRow(
                i,
                FlowState::Completed,
                Scheme::Https,
                "GET".into(),
                "api.example.com".into(),
                443,
                format!("/v1/{i}"),
                String::new(),
                Some(200),
                pm_model::ContentKind::Json,
                Some("Safari".into()),
                1_700_000_000_000.0,
                Some(10),
                0,
                100,
                false,
                HttpVersion::Http2,
                TlsStatus::Intercepted,
                None,
                None,
                false,
                None,
                None,
                None,
                vec![],
            )
        })
        .collect()
}

fn snapshot() -> SidebarSnapshot {
    SidebarSnapshot {
        store: 0,
        total: 5,
        apps: vec![
            AppCount {
                name: "Safari".into(),
                bundle_id: Some("com.apple.Safari".into()),
                has_icon: false,
                remote: false,
                count: 4,
            },
            AppCount {
                name: "curl".into(),
                bundle_id: None,
                has_icon: false,
                remote: false,
                count: 1,
            },
        ],
        domains: vec![DomainNode {
            domain: "example.com".into(),
            count: 5,
            hosts: vec![HostCount {
                host: "api.example.com".into(),
                count: 5,
            }],
        }],
    }
}

struct Shell {
    workspace: Entity<Workspace>,
    main: Entity<MainWindow>,
}

fn open(cx: &mut TestAppContext) -> (Shell, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        proxymate_native::prefs::init(None, cx);
        proxymate_native::features::register(cx);
    });
    let workspace = cx.new(|cx| {
        let mut ws = Workspace::new(
            None,
            ProxyStatus {
                listening: Some("127.0.0.1:9090".into()),
                recording: true,
                ..Default::default()
            },
            cx,
        );
        ws.documents = vec![DocumentInfo {
            store: 1,
            kind: DocumentKind::Session,
            name: "Login.proxymate".into(),
            path: "/Users/me/Login.proxymate".into(),
            flows: 7,
        }];
        ws
    });
    cx.update(|cx| Workspace::set_global(workspace.clone(), cx));
    let storage = cx.new(|_| Latest {
        value: Some(StorageStatus {
            flows: 5,
            ..Default::default()
        }),
    });
    let sidebar = cx.new(|_| Latest {
        value: Some(snapshot()),
    });
    let ws = workspace.clone();
    let (root, cx) = cx.add_window_view(|window, cx| {
        let table = cx.new(|cx| FlowTable::snapshot(rows(), 5, cx));
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
    let _ = root;
    (Shell { workspace, main }, cx)
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}

fn last_toast(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| Toasts::last(cx).map(|t| t.title.to_string()).unwrap_or_default())
}

/// Runs the menu entry `label` as choosing it would, then reads what it
/// copied.
fn run(
    cx: &mut VisualTestContext,
    menu: impl Fn(&gpui_kit::App) -> Option<proxymate_native::features::context_menu::Menu>,
    label: &str,
) -> Option<String> {
    let ran = cx.update(|window, cx| {
        let menu = menu(cx).expect("the menu");
        menu.run(label, window, cx)
    });
    assert!(ran, "no entry {label:?}");
    cx.run_until_parked();
    clipboard(cx)
}

#[gpui_kit::test]
fn context_menus_copy_the_right_text(cx: &mut TestAppContext) {
    let (shell, cx) = open(cx);
    let listening = toolbar::current_listening_menu;
    assert_eq!(run(cx, listening, "Copy address").as_deref(), Some("127.0.0.1:9090"));
    assert_eq!(last_toast(cx), "Copied proxy address");
    assert_eq!(
        run(cx, listening, "Copy curl example").as_deref(),
        Some("curl -x http://127.0.0.1:9090 http://example.com/")
    );
    assert_eq!(
        run(cx, listening, "Copy SOCKS5 address").as_deref(),
        Some("socks5h://127.0.0.1:9090")
    );

    let app = |cx: &gpui_kit::App| sidebar::menu_for("app:Safari", cx);
    assert_eq!(run(cx, app, "Copy name").as_deref(), Some("Safari"));
    assert_eq!(last_toast(cx), "Copied app name");
    assert_eq!(run(cx, app, "Copy bundle id").as_deref(), Some("com.apple.Safari"));
    let domain = |cx: &gpui_kit::App| sidebar::menu_for("domain:example.com", cx);
    assert_eq!(run(cx, domain, "Copy *.example.com").as_deref(), Some("*.example.com"));
    assert_eq!(last_toast(cx), "Copied domain pattern");
    let host = |cx: &gpui_kit::App| sidebar::menu_for("host:api.example.com", cx);
    assert_eq!(
        run(cx, host, "Copy api.example.com").as_deref(),
        Some("api.example.com")
    );
    let doc = |cx: &gpui_kit::App| sidebar::menu_for("doc:1", cx);
    assert_eq!(run(cx, doc, "Copy path").as_deref(), Some("/Users/me/Login.proxymate"));

    let tab = |cx: &gpui_kit::App| tabs::current_tab_menu(1, cx);
    assert_eq!(run(cx, tab, "Copy name").as_deref(), Some("Login.proxymate"));

    shell.workspace.update(cx, |w, cx| {
        w.set_save(
            Some(SaveJob::new(
                "Exporting HAR",
                SaveProgress {
                    done: 5,
                    total: 5,
                    finished: true,
                    error: None,
                    path: "/tmp/out/trip.har".into(),
                },
            )),
            cx,
        )
    });
    assert_eq!(
        run(cx, footer::current_save_menu, "Copy file name").as_deref(),
        Some("trip.har")
    );
    // Dismiss is the menu's last entry, and it clears the status.
    let _ = run(cx, footer::current_save_menu, "Dismiss");
    assert!(shell.workspace.read_with(cx, |w, _| w.save.is_none()));

    // Menu items that aren't copies act: "Show only Safari" scopes the table.
    let _ = run(cx, app, "Show only Safari");
    let client = shell.workspace.read_with(cx, |w, _| w.filter.client.clone());
    assert_eq!(client.as_deref(), Some("Safari"));
    let table_client = shell
        .main
        .read_with(cx, |m, cx| m.table().read(cx).filter().client.clone());
    assert_eq!(table_client.as_deref(), Some("Safari"), "the table follows the scope");
}

#[gpui_kit::test]
fn the_palette_opens_with_cmd_k_and_runs_a_command(cx: &mut TestAppContext) {
    let (shell, cx) = open(cx);
    cx.simulate_keystrokes(&primary("k"));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| commands::is_palette_open(cx)));
    let titles: Vec<String> = cx.update(|_, cx| {
        commands::palette_groups(cx)
            .into_iter()
            .flat_map(|(_, c)| c.into_iter().map(|c| c.title.into_owned()))
            .collect()
    });
    assert!(titles.contains(&"Stop recording".to_owned()), "{titles:?}");
    assert!(titles.contains(&"Show pinned flows".to_owned()));
    assert!(
        !titles.contains(&"Show live capture".to_owned()),
        "only on a document tab"
    );
    cx.simulate_input("pinned");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!cx.update(|_, cx| commands::is_palette_open(cx)));
    let text = shell.workspace.read_with(cx, |w, _| w.filter.text.clone());
    assert_eq!(text, "is:pinned");
    // ⌘E stops recording.
    cx.simulate_keystrokes(&primary("e"));
    cx.run_until_parked();
    assert!(!shell.workspace.read_with(cx, |w, _| w.proxy.recording));
    cx.simulate_keystrokes(&primary("e"));
    cx.run_until_parked();
    assert!(shell.workspace.read_with(cx, |w, _| w.proxy.recording));
}

#[gpui_kit::test]
fn the_search_box_autocompletes_and_hands_back_to_the_table(cx: &mut TestAppContext) {
    let (shell, cx) = open(cx);
    cx.simulate_keystrokes(&primary("f"));
    cx.run_until_parked();
    let bar = shell.main.read_with(cx, |m, _| m.filter_bar().clone());
    assert!(cx.update(|window, cx| bar.read(cx).is_focused(window, cx)));
    cx.simulate_input("ho");
    cx.run_until_parked();
    let labels = |cx: &mut VisualTestContext| -> Vec<String> {
        cx.update(|_, cx| bar.read(cx).suggestions(cx).into_iter().map(|s| s.label).collect())
    };
    assert_eq!(labels(cx), ["host:"]);
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| bar.read(cx).text(cx)), "host:");
    cx.simulate_input("exa");
    cx.run_until_parked();
    assert_eq!(labels(cx), ["*.example.com", "api.example.com"]);
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| bar.read(cx).text(cx)), "host:api.example.com ");
    assert_eq!(
        shell.workspace.read_with(cx, |w, _| w.filter.text.clone()),
        "host:api.example.com "
    );
    // No list after a whole term: Escape goes back to the table.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let table_focused = cx.update(|window, cx| {
        let grid = shell.main.read(cx).table().read(cx).grid().clone();
        gpui_kit::Focusable::focus_handle(grid.read(cx), cx).is_focused(window)
    });
    assert!(table_focused);
    // The typed filter reaches the table after its debounce.
    cx.executor()
        .advance_clock(proxymate_native::features::flow_table::FILTER_DEBOUNCE * 2);
    cx.run_until_parked();
    let applied = shell
        .main
        .read_with(cx, |m, cx| m.table().read(cx).filter().text.clone());
    assert_eq!(applied, "host:api.example.com ");
}

#[gpui_kit::test]
fn chips_and_sidebar_rows_filter(cx: &mut TestAppContext) {
    let (shell, cx) = open(cx);
    cx.update(|window, cx| window.click("protocol-HTTPS", cx));
    cx.update(|window, cx| window.click("content-JSON", cx));
    cx.update(|window, cx| window.click("content-XML", cx));
    cx.run_until_parked();
    let f = shell.workspace.read_with(cx, |w, _| w.filter.clone());
    assert_eq!(f.protocol, ProtocolFilter::Https);
    assert_eq!(f.content, [ContentGroup::Json, ContentGroup::Xml]);
    // Chips apply at once, without the typing debounce.
    let table = shell.main.read_with(cx, |m, cx| m.table().read(cx).filter().clone());
    assert_eq!(table.content, [ContentGroup::Json, ContentGroup::Xml]);
    cx.update(|window, cx| window.click("content-All", cx));
    cx.update(|window, cx| window.click("sidebar-domain-example.com", cx));
    cx.run_until_parked();
    let f = shell.workspace.read_with(cx, |w, _| w.filter.clone());
    assert!(f.content.is_empty());
    assert_eq!(f.host.as_deref(), Some("*.example.com"));
    cx.update(|window, cx| window.click("sidebar-all", cx));
    cx.run_until_parked();
    assert_eq!(shell.workspace.read_with(cx, |w, _| w.filter.host.clone()), None);
    // The Saved section and the tabs switch stores.
    cx.update(|window, cx| window.click("saved-1", cx));
    cx.run_until_parked();
    assert_eq!(shell.workspace.read_with(cx, |w, _| w.store), 1);
    assert_eq!(shell.main.read_with(cx, |m, cx| m.table().read(cx).store()), 1);
    cx.update(|window, cx| window.click("tab-0", cx));
    cx.run_until_parked();
    assert_eq!(shell.workspace.read_with(cx, |w, _| w.store), 0);
}

#[gpui_kit::test]
fn cmd_l_filters_the_sidebar(cx: &mut TestAppContext) {
    let (shell, cx) = open(cx);
    cx.simulate_keystrokes(&primary("l"));
    cx.simulate_input("cur");
    cx.run_until_parked();
    let sidebar = shell.main.read_with(cx, |m, _| m.sidebar().clone());
    assert_eq!(cx.update(|_, cx| sidebar.read(cx).filter_text(cx)), "cur");
    let has = |cx: &mut VisualTestContext, id: &'static str| cx.update(|window, _| window.try_find(id).is_some());
    assert!(has(cx, "sidebar-app-curl"));
    assert!(!has(cx, "sidebar-app-Safari"), "filtered out");
    assert!(has(cx, "sidebar-no-domains"));
    // ⌘F from another text field still goes to the search box.
    cx.simulate_keystrokes(&primary("f"));
    cx.run_until_parked();
    let bar = shell.main.read_with(cx, |m, _| m.filter_bar().clone());
    assert!(cx.update(|window, cx| bar.read(cx).is_focused(window, cx)));
}

#[gpui_kit::test]
fn cmd_backspace_in_a_text_field_edits_the_text(cx: &mut TestAppContext) {
    let (shell, cx) = open(cx);
    let cleared = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = cleared.clone();
    cx.update(|_, cx| {
        commands::add("clear", |_| true, move |_, _| flag.set(true), cx);
    });
    cx.simulate_keystrokes(&primary("f"));
    cx.simulate_input("abc");
    cx.simulate_keystrokes(&primary("backspace"));
    cx.run_until_parked();
    let bar = shell.main.read_with(cx, |m, _| m.filter_bar().clone());
    assert_eq!(cx.update(|_, cx| bar.read(cx).text(cx)), "");
    assert!(!cleared.get(), "⌘⌫ in text deletes text, not flows");
    // In the table it clears.
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes(&primary("backspace"));
    cx.run_until_parked();
    assert!(cleared.get());
}

#[gpui_kit::test]
fn the_empty_state_copies_its_command(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        proxymate_native::prefs::init(None, cx);
        proxymate_native::features::register(cx);
    });
    let table = cx.new(|cx| FlowTable::snapshot(vec![], 0, cx));
    let menu = cx
        .update(|cx| toolbar::empty_state_menu(cx))
        .expect("the empty state's menu");
    assert_eq!(
        menu.copy_text("Copy command"),
        Some("curl -x http://127.0.0.1:9090 http://example.com/")
    );
    drop(table);
}
