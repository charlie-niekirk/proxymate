//! The Body and Raw tabs (G4) driven like a user would, from injected
//! bodies (no engine): the viewer follows the body's kind and the choice
//! sticks per kind, the menus copy the right text into the test clipboard,
//! the filter's body terms are marked, ⌘F searches a CodeView, the JSON
//! tree filters, big bodies get their notes, and Escape stays inside an
//! editable editor.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div,
};
use gpui_kit::{InteractiveElement as _, prelude::FluentBuilder as _};
use pm_model::{BodyPart, ContentKind, FlowSummary, ProxyStatus, SidebarSnapshot, StorageStatus};
use pm_view::body::LOAD_LIMIT;
use pm_view::body::registry::ViewerId;
use pm_view::sample;
use pm_widgets::code_editor::CodeEditor;
use proxymate_native::bridge::keys::{self, BodyKey, DecodedBody, DecodedKey, FlowKey, Inspected};
use proxymate_native::bridge::stream::Latest;
use proxymate_native::features::body::{self, BodyPane};
use proxymate_native::features::flow_table::FlowTable;
use proxymate_native::features::inspector::{Inspector, RequestTab, ResponseTab};
use proxymate_native::shell::{self, MainWindow, Parts};
use proxymate_native::workspace::Workspace;

const JSON: &str = r#"{"user":{"id":12345678901234567890,"name":"alice","price":1.50},"tags":["admin","alice"]}"#;

fn flows() -> Vec<FlowSummary> {
    (0..5u32)
        .map(|id| {
            let mut f = sample::summary();
            f.id = id;
            f
        })
        .collect()
}

struct Win {
    workspace: Entity<Workspace>,
    table: Entity<FlowTable>,
    inspector: Entity<Inspector>,
}

fn open(cx: &mut TestAppContext) -> (Win, &mut VisualTestContext) {
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
        Win {
            workspace,
            table,
            inspector,
        },
        cx,
    )
}

/// Shows flow `id` with a JSON response body of `bytes` on the Body tab.
fn inspect_body(w: &Win, cx: &mut VisualTestContext, id: u32, bytes: &[u8], captured: Option<u64>) {
    let mut detail = sample::detail();
    detail.id = id;
    let size = bytes.len() as u64;
    detail.response_body.size = captured.unwrap_or(size);
    detail.response_body.captured = captured.unwrap_or(size);
    detail.response_body.kind = ContentKind::Json;
    let flow = flows().remove(id as usize);
    let max = captured.filter(|c| *c > LOAD_LIMIT).map(|_| LOAD_LIMIT);
    cx.update(|_, cx| {
        let caches = keys::caches(cx);
        let key = FlowKey::new(0, id);
        caches.inspected.update(cx, |c, cx| {
            c.set(
                key,
                Some(Arc::new(Inspected {
                    flow,
                    detail: Some(detail),
                })),
                cx,
            )
        });
        let body = Arc::new(DecodedBody {
            bytes: Arc::new(bytes.to_vec()),
            ..Default::default()
        });
        caches.decoded.update(cx, |c, cx| {
            c.set(
                DecodedKey {
                    flow: key,
                    part: BodyPart::Response,
                    max,
                },
                body,
                cx,
            )
        });
        caches.body.update(cx, |c, cx| {
            c.set(
                BodyKey {
                    flow: key,
                    part: BodyPart::Response,
                },
                Arc::new(bytes.to_vec()),
                cx,
            )
        });
        w.inspector.update(cx, |i, cx| {
            i.set_request_tab(RequestTab::Header, cx);
            i.set_response_tab(ResponseTab::Body, cx);
        });
        let grid = w.table.read(cx).grid().clone();
        grid.update(cx, |g, cx| g.select(Some(id), cx));
    });
    settle(cx);
}

/// Lets the body load, its viewer be prepared and drawn.
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

fn pane(w: &Win, cx: &mut VisualTestContext) -> Entity<BodyPane> {
    cx.update(|_, cx| w.inspector.read(cx).body_pane(BodyPart::Response).clone())
}

fn pane_in(w: &Win, cx: &gpui_kit::App) -> Entity<BodyPane> {
    w.inspector.read(cx).body_pane(BodyPart::Response).clone()
}

fn state(w: &Win, cx: &mut VisualTestContext) -> serde_json::Value {
    let pane = pane(w, cx);
    cx.update(|_, cx| pane.read(cx).describe(cx))
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}

#[gpui_kit::test]
fn the_viewer_follows_the_kind_and_the_choice_sticks_per_kind(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    inspect_body(&w, cx, 3, JSON.as_bytes(), None);
    let s = state(&w, cx);
    assert_eq!(s["viewer"], "json");
    assert_eq!(s["viewers"], serde_json::json!(["json", "json-tree", "text", "hex"]));
    assert_eq!(s["shown"], "code-editor", "{s}");
    let pretty = cx.update(|_, cx| pane_in(&w, cx).read(cx).surface().map(|s| s.text(cx).to_string()));
    assert!(
        pretty.unwrap().contains("\"id\": 12345678901234567890"),
        "re-indented, the 64-bit id exact"
    );
    // The dropdown picks the tree; the next JSON body opens in it too.
    cx.update(|window, cx| {
        window.dispatch_action(
            Box::new(body::ChooseViewer {
                kind: ContentKind::Json,
                viewer: ViewerId::JsonTree,
            }),
            cx,
        )
    });
    settle(cx);
    assert_eq!(state(&w, cx)["shown"], "json-tree");
    inspect_body(&w, cx, 2, b"[1,2,3]", None);
    let s = state(&w, cx);
    assert_eq!(s["viewer"], "json-tree");
    assert_eq!(s["detail"]["rows"], 4, "the root and its three items");
    // Hex is always there.
    cx.update(|_, cx| body::choose(ContentKind::Json, ViewerId::Hex, cx));
    settle(cx);
    assert_eq!(state(&w, cx)["shown"], "hex");
    cx.update(|_, cx| body::choose(ContentKind::Json, ViewerId::Json, cx));
}

#[gpui_kit::test]
fn the_body_menu_copies_the_body_as_text_and_base64(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    inspect_body(&w, cx, 3, JSON.as_bytes(), None);
    let menu = cx.update(|_, cx| pane_in(&w, cx).read(cx).menu(None, cx));
    assert_eq!(
        menu.labels(),
        [
            "Copy all",
            "Select all",
            "Find…",
            "—",
            "Copy body",
            "Copy body as Base64",
            "Save body…"
        ]
    );
    assert!(menu.copy_text("Copy all").unwrap().starts_with("{\n  \"user\": {"));
    cx.update(|window, cx| assert!(menu.run("Copy body", window, cx)));
    settle(cx);
    assert_eq!(clipboard(cx).as_deref(), Some(JSON));
    cx.update(|window, cx| assert!(menu.run("Copy body as Base64", window, cx)));
    settle(cx);
    assert_eq!(
        clipboard(cx).as_deref(),
        Some(pm_view::copy_formats::base64(JSON.as_bytes()).as_str())
    );
}

#[gpui_kit::test]
fn the_filters_body_terms_are_marked_and_stepped(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    cx.update(|_, cx| w.workspace.update(cx, |ws, cx| ws.set_text("body:alice", cx)));
    inspect_body(&w, cx, 3, JSON.as_bytes(), None);
    let s = state(&w, cx);
    assert_eq!(s["detail"]["matches"], 2, "{s}");
    let surface = cx.update(|_, cx| pane_in(&w, cx).read(cx).surface().cloned()).unwrap();
    let proxymate_native::features::body::Surface::Editor(editor) = surface else {
        panic!("a small body is in the editor")
    };
    cx.update(|window, cx| editor.update(cx, |e, cx| e.step(true, window, cx)));
    assert_eq!(cx.update(|_, cx| editor.read(cx).current_match()), Some(0));
    cx.update(|window, cx| editor.update(cx, |e, cx| e.step(false, window, cx)));
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).current_match()),
        Some(1),
        "round to the last"
    );
    // The tree marks keys and values, and reveals what it steps to.
    cx.update(|_, cx| body::choose(ContentKind::Json, ViewerId::JsonTree, cx));
    settle(cx);
    let tree = cx.update(|_, cx| pane_in(&w, cx).read(cx).tree().cloned()).unwrap();
    assert_eq!(cx.update(|_, cx| tree.read(cx).hit_count()), 2);
    cx.update(|window, cx| tree.update(cx, |t, cx| t.step(false, window, cx)));
    let shown: Vec<String> = cx.update(|_, cx| tree.read(cx).rows().iter().map(|r| r.ptr.clone()).collect());
    assert!(
        shown.contains(&"/tags/1".to_owned()),
        "the last hit's array opened: {shown:?}"
    );
    // A JSONPath filter.
    cx.update(|window, cx| tree.update(cx, |t, cx| t.set_filter("$..id", window, cx)));
    assert_eq!(cx.update(|_, cx| tree.read(cx).filter_result().clone()), Ok(Some(1)));
    let first = cx.update(|_, cx| tree.read(cx).rows()[0].clone());
    assert_eq!(first.value_text(), "12345678901234567890");
    cx.update(|_, cx| body::choose(ContentKind::Json, ViewerId::Json, cx));
}

#[gpui_kit::test]
fn the_raw_tab_highlights_and_searches_with_cmd_f(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    inspect_body(&w, cx, 3, JSON.as_bytes(), None);
    cx.update(|_, cx| w.inspector.update(cx, |i, cx| i.set_response_tab(ResponseTab::Raw, cx)));
    settle(cx);
    let raw = cx
        .update(|_, cx| w.inspector.read(cx).raw_view(BodyPart::Response).cloned())
        .expect("the raw view");
    let text = cx.update(|_, cx| raw.read(cx).text().to_string());
    assert!(text.starts_with("HTTP/2 200"), "{text}");
    assert!(text.ends_with(JSON));
    // ⌘F in the view opens its find bar rather than the flow search.
    cx.update(|window, cx| {
        let focus = gpui_kit::Focusable::focus_handle(raw.read(cx), cx);
        window.focus(&focus, cx);
    });
    cx.simulate_keystrokes("cmd-f");
    settle(cx);
    assert!(cx.update(|_, cx| raw.read(cx).is_find_open()), "⌘F opened the find bar");
    cx.simulate_input("alice");
    settle(cx);
    assert_eq!(cx.update(|_, cx| raw.read(cx).match_count()), 2);
    assert_eq!(
        cx.update(|_, cx| raw.read(cx).current_match()),
        Some(0),
        "typing jumps to the first"
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(cx.update(|_, cx| raw.read(cx).current_match()), Some(1));
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(!cx.update(|_, cx| raw.read(cx).is_find_open()), "Escape closed it");
    // ⌘A then the menu's Copy copies everything, rows off screen included.
    cx.simulate_keystrokes("cmd-a");
    assert!(cx.update(|_, cx| raw.read(cx).is_all_selected()));
    let selection = cx.update(|window, cx| raw.update(cx, |r, cx| r.selected_text(window, cx)));
    let menu = cx
        .update(|_, cx| w.inspector.read(cx).menu_for("raw:response", selection, cx))
        .unwrap();
    assert_eq!(menu.copy_text("Copy").map(str::len), Some(text.len()));
    assert!(menu.copy_text("Copy status line").unwrap().starts_with("HTTP/2 200"));
}

#[gpui_kit::test]
fn big_bodies_load_the_first_20_mb_until_load_all(cx: &mut TestAppContext) {
    let (w, cx) = open(cx);
    inspect_body(&w, cx, 3, JSON.as_bytes(), Some(LOAD_LIMIT + 1));
    assert!(cx.update(|window, _| window.try_find("large-body-note").is_some()));
    let pane = pane(&w, cx);
    cx.update(|_, cx| pane.update(cx, |p, cx| p.load_all(cx)));
    settle(cx);
    let s = state(&w, cx);
    assert_eq!(s["loadAll"], true);
    assert!(
        cx.update(|window, _| window.try_find("large-body-note").is_none()),
        "the note goes with Load all"
    );
}

/// An editable editor inside a parent that, like the inspector, turns
/// Escape into "back to the table".
struct EscapeHost {
    editor: Entity<CodeEditor>,
    escapes: usize,
}

gpui_kit::actions!(body_test, [Back]);

impl Render for EscapeHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Host")
            .size_full()
            .on_action(cx.listener(|this, _: &Back, _, _| this.escapes += 1))
            .child(self.editor.clone())
            .when(false, |d| d)
    }
}

#[gpui_kit::test]
fn escape_stays_inside_an_editable_editor(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        pm_widgets::code_editor::init(cx);
        cx.bind_keys([gpui_kit::KeyBinding::new("escape", Back, Some("Host"))]);
    });
    for (read_only, swallowed) in [(false, true), (true, false)] {
        let (host, cx) = cx.add_window_view(|window, cx| {
            let editor = cx.new(|cx| {
                let mut e = CodeEditor::new("ed", "Editor", "json", read_only, window, cx);
                e.set_text("{\"a\": 1}".into(), window, cx);
                e
            });
            EscapeHost { editor, escapes: 0 }
        });
        cx.update(|window, cx| {
            let focus: FocusHandle = gpui_kit::Focusable::focus_handle(host.read(cx).editor.read(cx), cx);
            window.focus(&focus, cx);
        });
        cx.simulate_keystrokes("escape");
        let escapes = cx.update(|_, cx| host.read(cx).escapes);
        assert_eq!(escapes == 0, swallowed, "read only: {read_only}");
    }
}
