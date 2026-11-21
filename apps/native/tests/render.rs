//! The main window's shell with the flow table (G1), its chrome (G2:
//! toolbar, sidebar, document tabs, filter bar, footer; the ⌘K palette)
//! and the inspector (G3: a decrypted flow with notes, an OpenAPI line and
//! rule badges on its TLS tab; a failed flow with its banner on Timing),
//! the stream panels (G6: a WebSocket's messages, badges, composer and an
//! edited message; an event stream's events and fields), and the Rules
//! window (G7: groups, origin badges, a rule with every kind of action
//! form and a dry run), rendered by gpui's real
//! offscreen renderer (Metal on macOS, WGPU on Linux; CI uses Mesa's
//! lavapipe) in both themes, from injected snapshots: no engine, no proxy.
//! The pane and row pixels must be their tokens, and each capture is
//! written to `target/native-snapshots/shell-<theme>.png` for review (CI
//! uploads them).
//!
//! It fails, rather than passing vacuously, when no renderer is available.
//! Its own harness (`harness = false`): the renderer wants the main thread.

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    render::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("render: skipped; gpui has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod render {
    use std::path::PathBuf;
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, HeadlessAppContext, WindowBounds, WindowOptions, point, px, size};
    use pm_model::{
        AppCount, ContentGroup, ContentKind, DocumentInfo, DocumentKind, DomainNode, FlowLabel, FlowRow, FlowState,
        HostCount, HttpVersion, ProxyStatus, SaveProgress, Scheme, SentBy, SidebarSnapshot, StorageStatus, TlsStatus,
    };
    use pm_model::{AppliedRule, OperationMatch, Phase, RuleOrigin};
    use pm_theme::{Mode, Rgba};
    use pm_view::columns::{HEADER_HEIGHT, ROW_HEIGHT};
    use pm_view::sample;
    use proxymate_native::bridge::keys::{self, FlowKey, Inspected};
    use proxymate_native::bridge::stream::Latest;
    use proxymate_native::features::flow_table::FlowTable;
    use proxymate_native::features::inspector::{RequestTab, ResponseTab};
    use proxymate_native::shell::{self, MainWindow, Parts, filter_bar, tabs};
    use proxymate_native::workspace::{SaveJob, Workspace};

    const SIZE: (f32, f32) = (1200., 760.);

    fn context() -> HeadlessAppContext {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(pm_widgets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            proxymate_native::prefs::init(None, cx);
            proxymate_native::features::register(cx);
        });
        cx
    }

    /// Rows like the zoo's: every method, status class, state and badge.
    fn rows() -> Vec<FlowRow> {
        let methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD"];
        let statuses = [Some(200), Some(304), Some(404), Some(500), Some(101), None];
        let hosts = ["api.example.com", "cdn.example.net", "localhost", "auth.example.org"];
        (0..60u32)
            .map(|i| {
                let i_ = i as usize;
                let state = match i % 11 {
                    5 => FlowState::Failed,
                    7 => FlowState::Active,
                    9 => FlowState::Tunnel,
                    _ => FlowState::Completed,
                };
                FlowRow(
                    i,
                    state,
                    if i % 4 == 2 { Scheme::Http } else { Scheme::Https },
                    methods[i_ % methods.len()].into(),
                    hosts[i_ % hosts.len()].into(),
                    if i % 4 == 2 { 3000 } else { 443 },
                    format!("/v1/items/{i}"),
                    if i % 3 == 0 { format!("page={i}") } else { String::new() },
                    statuses[i_ % statuses.len()],
                    ContentKind::Json,
                    Some(["Safari", "node", "Firefox"][i_ % 3].into()),
                    1_700_000_000_000.0 + i as f64 * 37.0,
                    Some(i * 13),
                    (i as u64 % 3) * 700,
                    i as u64 * 1500,
                    i % 6 == 0,
                    if i % 2 == 0 {
                        HttpVersion::Http2
                    } else {
                        HttpVersion::Http11
                    },
                    [
                        TlsStatus::Intercepted,
                        TlsStatus::Passthrough,
                        TlsStatus::None,
                        TlsStatus::LikelyPinned,
                    ][i_ % 4],
                    (i % 8 == 1).then(|| "GetViewer".into()),
                    [None, Some(SentBy::User), None, Some(SentBy::Agent)][i_ % 4],
                    i % 10 == 0,
                    (i % 9 == 4).then_some(5),
                    (i % 5 == 3).then_some(FlowLabel::Blue),
                    (i % 7 == 2).then(|| "check".into()),
                    vec![],
                )
            })
            .collect()
    }

    /// Apps and domains like a browsing session's.
    fn sidebar() -> SidebarSnapshot {
        let app = |name: &str, count| AppCount {
            name: name.into(),
            bundle_id: None,
            has_icon: false,
            remote: false,
            count,
        };
        SidebarSnapshot {
            store: 0,
            total: 60,
            apps: vec![app("Firefox", 20), app("node", 20), app("Safari", 20)],
            domains: vec![
                DomainNode {
                    domain: "example.com".into(),
                    count: 45,
                    hosts: vec![
                        HostCount {
                            host: "api.example.com".into(),
                            count: 30,
                        },
                        HostCount {
                            host: "cdn.example.net".into(),
                            count: 15,
                        },
                    ],
                },
                DomainNode {
                    domain: "localhost".into(),
                    count: 15,
                    hosts: vec![],
                },
            ],
        }
    }

    fn snapshot_dir() -> PathBuf {
        // `target/tmp` is Cargo's scratch dir for integration tests; its
        // sibling holds the captures (CARGO_TARGET_DIR is honoured).
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("../native-snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn assert_pixel(image: &image::RgbaImage, scale: f32, at: (f32, f32), want: Rgba, what: &str) {
        let (x, y) = ((at.0 * scale) as u32, (at.1 * scale) as u32);
        let px = image.get_pixel(x, y).0;
        let got = [px[0], px[1], px[2]];
        let want = [want.r, want.g, want.b].map(|c| (c * 255.0).round() as i32);
        let close = (0..3).all(|i| (got[i] as i32 - want[i]).abs() <= 3);
        assert!(close, "{what} at {at:?}: got {got:?}, want {want:?}");
    }

    /// `fg` (maybe translucent, like `--pane-row-alt`) composited over `bg`.
    fn over(fg: Rgba, bg: Rgba) -> Rgba {
        let mix = |f: f32, b: f32| f * fg.a + b * (1. - fg.a);
        Rgba {
            r: mix(fg.r, bg.r),
            g: mix(fg.g, bg.g),
            b: mix(fg.b, bg.b),
            a: 1.,
        }
    }

    fn shell_renders(mode: Mode) {
        let mut cx = context();
        cx.update(|cx| {
            use proxymate_native::prefs::{self, Appearance};
            let forced = if mode.is_dark() {
                Appearance::Dark
            } else {
                Appearance::Light
            };
            prefs::update(cx, |p| p.appearance = forced);
            pm_theme::kit::apply(mode, cx);
        });
        let handle = cx
            .update(|cx| {
                let storage = cx.new(|_| Latest {
                    value: Some(StorageStatus {
                        flows: 1234,
                        pinned: 3,
                        disk_bytes: 5 << 20,
                        unrecorded: 12,
                        ..Default::default()
                    }),
                });
                let sidebar = cx.new(|_| Latest { value: Some(sidebar()) });
                let proxy = ProxyStatus {
                    listening: Some("127.0.0.1:9090".into()),
                    recording: true,
                    ..Default::default()
                };
                let workspace = cx.new(|cx| {
                    let mut ws = Workspace::new(None, proxy, cx);
                    ws.documents = vec![DocumentInfo {
                        store: 1,
                        kind: DocumentKind::Har,
                        name: "checkout.har".into(),
                        path: "/tmp/checkout.har".into(),
                        flows: 42,
                    }];
                    ws.filter.content = vec![ContentGroup::Json];
                    ws.filter.client = Some("Safari".into());
                    ws.set_save(
                        Some(SaveJob::new(
                            "Saving session",
                            SaveProgress {
                                done: 60,
                                total: 60,
                                finished: true,
                                error: None,
                                path: "/tmp/Proxymate 2026-11-20 10.00.proxymate".into(),
                            },
                        )),
                        cx,
                    );
                    ws
                });
                Workspace::set_global(workspace.clone(), cx);
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                        origin: point(px(0.), px(0.)),
                        size: size(px(SIZE.0), px(SIZE.1)),
                    })),
                    focus: false,
                    show: false,
                    ..Default::default()
                };
                cx.open_window(options, |window, cx| {
                    let table = cx.new(|cx| FlowTable::snapshot(rows(), 60, cx));
                    let parts = Parts {
                        workspace,
                        storage,
                        sidebar,
                        engine: None,
                    };
                    let view = cx.new(|cx| MainWindow::new(parts, table, window, cx));
                    cx.new(|cx| shell::root(view, window, cx))
                })
            })
            .expect("open the window");
        // Let the rows load, then select row 1 (flow #2).
        cx.run_until_parked();
        cx.update(|cx| {
            let table = proxymate_native::features::flow_table::first(cx).expect("a table");
            let grid = table.read(cx).grid().clone();
            grid.update(cx, |g, cx| g.select_index(1, cx));
        });
        cx.run_until_parked();
        let scale = cx
            .update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.scale_factor()
            })
            .unwrap();
        let image = cx
            .capture_screenshot(handle.into())
            .expect("gpui's offscreen renderer must be available");
        let name = if mode.is_dark() { "dark" } else { "light" };
        let path = snapshot_dir().join(format!("shell-{name}.png"));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());

        let t = mode.tokens();
        let main_x = shell::SIDEBAR_WIDTH + 40.;
        assert_pixel(&image, scale, (main_x, 8.), t.pane_toolbar, "toolbar");
        // The table: its header, then rows 0 (plain), 1 (selected) and 3 (odd),
        // sampled in the # column's empty left part.
        // The document tabs (a document is open) and the filter bar.
        let tabs_y = shell::TOOLBAR_HEIGHT;
        assert_pixel(
            &image,
            scale,
            (main_x + 400., tabs_y + 4.),
            t.pane_toolbar,
            "document tabs",
        );
        let bar_y = tabs_y + tabs::TABS_HEIGHT;
        assert_pixel(&image, scale, (main_x + 380., bar_y + 4.), t.pane_toolbar, "filter bar");
        // The All protocol chip is on; JSON is the content chip on.
        assert_pixel(
            &image,
            scale,
            (shell::SIDEBAR_WIDTH + 11., bar_y + filter_bar::BAR_HEIGHT / 2.),
            over(t.primary_chip_bg, t.pane_toolbar),
            "the All protocol chip",
        );
        let top = bar_y + filter_bar::BAR_HEIGHT;
        let row_y = |i: f32| top + HEADER_HEIGHT + ROW_HEIGHT * i + ROW_HEIGHT / 2.;
        let x = shell::SIDEBAR_WIDTH + 6.;
        assert_pixel(
            &image,
            scale,
            (x, top + HEADER_HEIGHT / 2.),
            over(t.pane_header, t.pane_table),
            "table header",
        );
        assert_pixel(&image, scale, (x, row_y(0.)), t.pane_table, "row 0");
        assert_pixel(
            &image,
            scale,
            (x, row_y(1.)),
            over(t.selection_bg, t.pane_table),
            "selected row 1",
        );
        assert_pixel(
            &image,
            scale,
            (x, row_y(3.)),
            over(t.pane_row_alt, t.pane_table),
            "odd row 3",
        );
        // The inspector placeholder under the table's 58%.
        let content = SIZE.1 - top - shell::FOOTER_HEIGHT;
        assert_pixel(
            &image,
            scale,
            (main_x, top + content * shell::TABLE_SHARE + 40.),
            t.background,
            "inspector",
        );
        assert_pixel(&image, scale, (main_x + 300., SIZE.1 - 3.), t.pane_toolbar, "footer");
        if !cfg!(target_os = "macos") {
            // macOS leaves the sidebar clear for the vibrancy behind the window.
            // Under the sections, above the filter box at the bottom.
            assert_pixel(&image, scale, (4., SIZE.1 - 60.), t.sidebar, "sidebar");
        }

        inspector_renders(&mut cx, handle.into(), scale, mode, top, content);
        stream_renders(&mut cx, handle.into(), scale, mode);

        // The ⌘K palette over the shell, with the commands G2 registers.
        cx.update_window(handle.into(), |_, window, cx| {
            proxymate_native::features::commands::open_palette(window, cx)
        })
        .unwrap();
        for _ in 0..30 {
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
        let image = cx.capture_screenshot(handle.into()).expect("a palette frame");
        let path = snapshot_dir().join(format!("palette-{name}.png"));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
        palette_checks(&image, scale, &t);
    }

    /// Shows `flow` (with `detail`) in the inspector by putting it in the
    /// cache and selecting its row.
    fn inspect(
        cx: &mut HeadlessAppContext,
        flow: pm_model::FlowSummary,
        detail: pm_model::FlowDetail,
        tabs: (RequestTab, ResponseTab),
        operation: Option<pm_model::OperationMatch>,
    ) {
        cx.update(|cx| {
            let caches = keys::caches(cx);
            let key = FlowKey::new(0, flow.id);
            let id = flow.id;
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
            caches.operation.update(cx, |c, cx| c.set(key, operation, cx));
            let inspector = shell::parts(cx)
                .and_then(|p| p.inspector.upgrade())
                .expect("the inspector");
            inspector.update(cx, |i, cx| {
                i.set_request_tab(tabs.0, cx);
                i.set_response_tab(tabs.1, cx);
            });
            let table = proxymate_native::features::flow_table::first(cx).expect("a table");
            let grid = table.read(cx).grid().clone();
            grid.update(cx, |g, cx| g.select(Some(id), cx));
        });
        cx.run_until_parked();
    }

    fn capture(cx: &mut HeadlessAppContext, handle: gpui_kit::AnyWindowHandle, file: String) -> image::RgbaImage {
        for _ in 0..3 {
            cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                .unwrap();
            cx.run_until_parked();
        }
        let image = cx.capture_screenshot(handle).expect("an inspector frame");
        let path = snapshot_dir().join(file);
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
        image
    }

    fn inspector_renders(
        cx: &mut HeadlessAppContext,
        handle: gpui_kit::AnyWindowHandle,
        scale: f32,
        mode: Mode,
        top: f32,
        content: f32,
    ) {
        let t = mode.tokens();
        let name = if mode.is_dark() { "dark" } else { "light" };
        // Under the table's 58%, its 1 px border and the 5 px handle.
        let inspector_top = top + content * shell::TABLE_SHARE + 6.;

        // A decrypted flow with a label, a comment, an OpenAPI operation and
        // two rules, on the TLS tab.
        let mut flow = sample::summary();
        flow.id = 1;
        flow.label = Some(FlowLabel::Blue);
        flow.comment = Some("check the page size".into());
        let mut detail = sample::detail();
        detail.id = 1;
        detail.rules.applied = vec![
            AppliedRule {
                rule_id: "r1".into(),
                name: "Mock users".into(),
                origin: RuleOrigin::User,
                phase: Phase::Response,
            },
            AppliedRule {
                rule_id: "r2".into(),
                name: "Slow down".into(),
                origin: RuleOrigin::Agent {
                    client: "claude".into(),
                    session: None,
                    created_at_ms: 0.,
                },
                phase: Phase::Request,
            },
        ];
        detail.rules.mocked = true;
        let operation = OperationMatch {
            spec_id: 1,
            spec_name: "Users API".into(),
            operation_id: Some("listUsers".into()),
            summary: Some("List the users, a page at a time".into()),
            method: "GET".into(),
            path: "/v1/users".into(),
        };
        inspect(
            cx,
            flow,
            detail,
            (RequestTab::Header, ResponseTab::Tls),
            Some(operation),
        );
        let image = capture(cx, handle, format!("inspector-{name}.png"));
        // The URL bar (its plain background between the URL and the pills).
        assert_pixel(&image, scale, (700., inspector_top + 16.), t.background, "the URL bar");
        // The first rule badge, under the URL bar (32), notes (24) and
        // OpenAPI line (24), above its text.
        assert_pixel(
            &image,
            scale,
            (300., inspector_top + 80. + 6.),
            over(t.rule_bg, t.background),
            "a rule badge",
        );

        // A failed flow: the banner above the Timing tab.
        let mut failed = sample::summary();
        failed.id = 5;
        failed.state = FlowState::Failed;
        failed.status = None;
        failed.resp_size = 0;
        let mut detail = sample::detail();
        detail.id = 5;
        detail.response = None;
        detail.error = Some("connecting to api.example.com:443 timed out".into());
        detail.error_kind = Some(pm_model::ErrorKind::Timeout);
        inspect(cx, failed, detail, (RequestTab::Raw, ResponseTab::Timing), None);
        let image = capture(cx, handle, format!("inspector-failed-{name}.png"));
        // The Response pane starts past the Request pane's half and the
        // 5 px handle; its banner is under the URL bar and the tab strip.
        let response_left = shell::SIDEBAR_WIDTH + (SIZE.0 - shell::SIDEBAR_WIDTH) * 0.5 + 5.;
        assert_pixel(
            &image,
            scale,
            (response_left + 320., inspector_top + 64. + 3.),
            over(t.s_5xx_bg, t.background),
            "the failure banner",
        );
    }

    /// Where element `id` was drawn.
    fn bounds_of(
        cx: &mut HeadlessAppContext,
        handle: gpui_kit::AnyWindowHandle,
        id: &'static str,
    ) -> (f32, f32, f32, f32) {
        cx.update_window(handle, |_, window, _| {
            let b = window.find(id).bounds();
            (
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(b.size.width),
                f32::from(b.size.height),
            )
        })
        .unwrap()
    }

    /// The stream panels (G6), with the table shrunk so the inspector has
    /// room: an open WebSocket with its flags, the selected edited message
    /// and the composer; then an event stream with an event's fields.
    fn stream_renders(cx: &mut HeadlessAppContext, handle: gpui_kit::AnyWindowHandle, scale: f32, mode: Mode) {
        use proxymate_native::bridge::keys::{PageKey, SseListKey, WsListKey, WsPayloadKey};
        use proxymate_native::features::inspector::streams::StreamPanel;

        let t = mode.tokens();
        let name = if mode.is_dark() { "dark" } else { "light" };
        let main = cx.update(|cx| {
            shell::parts(cx)
                .and_then(|p| p.main.upgrade())
                .expect("the main window")
        });
        cx.update(|cx| main.update(cx, |m, cx| m.set_table_share(0.2, cx)));
        let panel = |cx: &mut HeadlessAppContext| -> gpui_kit::Entity<StreamPanel> {
            cx.update(|cx| {
                let inspector = shell::parts(cx)
                    .and_then(|p| p.inspector.upgrade())
                    .expect("the inspector");
                inspector.read(cx).stream_panel(cx).expect("a stream panel")
            })
        };

        // A WebSocket: every kind of message, open, message 4 (edited) selected.
        let messages = sample::ws_messages();
        cx.update(|cx| {
            let caches = keys::caches(cx);
            let flow = FlowKey::new(0, 6);
            let list = WsListKey {
                flow,
                direction: None,
                text: String::new(),
            };
            caches.ws_head.update(cx, |c, cx| {
                c.set(list.clone(), Arc::new(sample::ws_page(&messages, true)), cx)
            });
            let page = PageKey {
                list,
                page: 0,
                dropped: 0,
            };
            caches
                .ws_rows
                .update(cx, |c, cx| c.set(page, Arc::new(messages.clone()), cx));
            let payload = WsPayloadKey {
                flow,
                index: 4,
                original: false,
            };
            caches.ws_payload.update(cx, |c, cx| {
                c.set(payload, Some(Arc::new(br#"{"price":999}"#.to_vec())), cx)
            });
        });
        inspect(
            cx,
            sample::ws_summary(),
            sample::ws_detail(),
            (RequestTab::Header, ResponseTab::Header),
            None,
        );
        let ws = panel(cx);
        cx.update(|cx| ws.update(cx, |p, cx| p.select_index(4, cx)));
        cx.run_until_parked();
        let image = capture(cx, handle, format!("websocket-{name}.png"));
        let (x, y, w, h) = bounds_of(cx, handle, "stream-toolbar");
        assert_pixel(
            &image,
            scale,
            (x + w * 0.45, y + h / 2.),
            t.pane_toolbar,
            "the stream toolbar",
        );
        // The list follows its end; the badges say where rows 3 and 4 are.
        let (x, _, _, _) = bounds_of(cx, handle, "stream-list");
        let (_, ey, _, eh) = bounds_of(cx, handle, "ws-flag-12");
        assert_pixel(
            &image,
            scale,
            (x + 3., ey + eh / 2.),
            over(t.selection_bg, t.pane_table),
            "the selected message",
        );
        let (_, _, w, _) = bounds_of(cx, handle, "stream-list");
        assert_pixel(
            &image,
            scale,
            (x + w - 4., ey + eh / 2.),
            over(t.selection_bg, t.pane_table),
            "the selection's full width",
        );
        let (fx, fy, _, fh) = bounds_of(cx, handle, "ws-flag-9");
        assert_pixel(
            &image,
            scale,
            (fx + 1., fy + fh / 2.),
            over(t.diff_add_bg, over(t.pane_row_alt, t.pane_table)),
            "the Injected badge",
        );
        let (x, y, w, _) = bounds_of(cx, handle, "ws-composer");
        assert_pixel(&image, scale, (x + w * 0.6, y + 3.), t.pane_toolbar, "the composer");
        let (x, y, w, _) = bounds_of(cx, handle, "ws-edited-bar");
        assert_pixel(
            &image,
            scale,
            (x + w * 0.5, y + 2.),
            t.background,
            "the edited message's bar",
        );

        // An event stream, event 1 selected.
        let events = sample::sse_events();
        cx.update(|cx| {
            let caches = keys::caches(cx);
            let list = SseListKey {
                flow: FlowKey::new(0, 7),
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
            caches
                .sse_rows
                .update(cx, |c, cx| c.set(page, Arc::new(events.clone()), cx));
        });
        inspect(
            cx,
            sample::sse_summary(),
            sample::sse_detail(),
            (RequestTab::Header, ResponseTab::Header),
            None,
        );
        let sse = panel(cx);
        cx.update(|cx| sse.update(cx, |p, cx| p.select_index(1, cx)));
        cx.run_until_parked();
        let image = capture(cx, handle, format!("sse-{name}.png"));
        let (x, y, w, h) = bounds_of(cx, handle, "stream-toolbar");
        assert_pixel(
            &image,
            scale,
            (x + w * 0.45, y + h / 2.),
            t.pane_toolbar,
            "the SSE toolbar",
        );
        let (x, y, _, _) = bounds_of(cx, handle, "stream-list");
        assert_pixel(
            &image,
            scale,
            (x + 3., y + 22. + 22. + 11.),
            over(t.selection_bg, t.pane_table),
            "the selected event",
        );
        assert_pixel(&image, scale, (x + 3., y + 22. + 11.), t.pane_table, "event 0");
        cx.update(|cx| main.update(cx, |m, cx| m.set_table_share(shell::TABLE_SHARE, cx)));
        cx.run_until_parked();
    }

    /// The palette's panel is drawn in the popover colour, centred under
    /// the toolbar.
    fn palette_checks(image: &image::RgbaImage, scale: f32, t: &pm_theme::Tokens) {
        // The panel's left padding (520 px wide, centred, 80 px down).
        let x = (SIZE.0 - 520.) / 2. + 6.;
        let px = image.get_pixel((x * scale) as u32, (300. * scale) as u32).0;
        let close = |want: pm_theme::Rgba| {
            let want = [want.r, want.g, want.b].map(|c| (c * 255.0).round() as i32);
            (0..3).all(|i| (px[i] as i32 - want[i]).abs() <= 3)
        };
        assert!(
            close(t.background) || close(t.popover),
            "palette panel at ({x}, 300): got {:?}, want the background or popover token",
            &px[..3]
        );
    }

    /// Rules in a group and out of one, from the user, a preset and an
    /// agent; the agent's rule selected, with a form for many action kinds
    /// and a matched dry run.
    fn rule_book() -> pm_model::RuleBook {
        use pm_model::{Action, MapLocalSource, Pattern, PresetKind, Rule, RuleGroup, WsOp};
        use pm_view::rules::{default_action, new_rule};
        let rule = |id: &str, name: &str, origin, group: Option<&str>, enabled| Rule {
            id: id.into(),
            name: name.into(),
            enabled,
            origin,
            group: group.map(Into::into),
            ..new_rule()
        };
        let mut agent = rule(
            "r-agent",
            "Mock the cart",
            RuleOrigin::Agent {
                client: "claude-code".into(),
                session: None,
                created_at_ms: 0.,
            },
            None,
            true,
        );
        agent.matcher.methods = vec!["GET".into(), "POST".into()];
        agent.matcher.host = Some(Pattern::glob("*.example.com"));
        agent.matcher.path = Some(Pattern::exact("/v1/cart"));
        agent.actions = vec![
            Action::MapLocal {
                source: MapLocalSource::Inline {
                    body: "{\"items\": []}".into(),
                },
                status: 200,
                headers: vec![],
                content_type: Some("application/json".into()),
            },
            default_action("JsonSet").unwrap(),
            Action::WsMessage {
                direction: None,
                filter: None,
                op: WsOp::Drop,
            },
            default_action("ServeFrom").unwrap(),
            default_action("Comment").unwrap(),
        ];
        let mut preset = rule(
            "r-preset",
            "Map Local · GET api.example.com/v1/users",
            RuleOrigin::Preset {
                preset: PresetKind::MapLocal,
            },
            None,
            true,
        );
        preset.actions = vec![default_action("MapLocal").unwrap()];
        let mut off = rule("r-off", "Slow 3G", RuleOrigin::User, Some("g1"), false);
        off.actions = vec![default_action("Throttle").unwrap()];
        let mut on = rule("r-on", "Strip cookies", RuleOrigin::User, Some("g1"), true);
        on.actions = vec![default_action("RemoveHeader").unwrap()];
        pm_model::RuleBook {
            rules: vec![agent, preset, off, on],
            groups: vec![RuleGroup {
                id: "g1".into(),
                name: "Ticket 42".into(),
                enabled: true,
                order: 1,
                origin: RuleOrigin::User,
            }],
        }
    }

    fn rules_renders(mode: Mode) {
        use proxymate_native::features::rules::form::TestState;
        use proxymate_native::features::rules::window::{LIST_WIDTH, RulesWindow, Target};

        let mut cx = context();
        cx.update(|cx| {
            use proxymate_native::prefs::{self, Appearance};
            prefs::update(cx, |p| {
                p.appearance = if mode.is_dark() {
                    Appearance::Dark
                } else {
                    Appearance::Light
                }
            });
            pm_theme::kit::apply(mode, cx);
        });
        let handle = cx
            .update(|cx| {
                let book = proxymate_native::bridge::rule_book(cx);
                book.update(cx, |b, _| b.value = Some(rule_book()));
                let settings = keys::rules_caches(cx).settings;
                settings.update(cx, |c, cx| c.set((), pm_model::RuleSettings::default(), cx));
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                        origin: point(px(0.), px(0.)),
                        size: size(px(SIZE.0), px(SIZE.1)),
                    })),
                    focus: false,
                    show: false,
                    ..Default::default()
                };
                cx.open_window(options, |window, cx| {
                    let view = cx.new(|cx| {
                        let mut w = RulesWindow::new(book, None, window, cx);
                        w.go(Target::Rule("r-agent".into()), window, cx);
                        w
                    });
                    cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
                })
            })
            .expect("open the Rules window");
        cx.run_until_parked();
        let form = cx
            .update_window(handle.into(), |root, _, cx| {
                let root = root.downcast::<gpui_kit::base::Root>().unwrap();
                let view = root.read(cx).view().clone().downcast::<RulesWindow>().unwrap();
                view.read(cx).form().cloned()
            })
            .unwrap()
            .expect("the agent rule's form");
        cx.update(|cx| {
            form.update(cx, |f, cx| {
                let request = pm_model::RequestHead {
                    method: "GET".into(),
                    url: "https://api.example.com/v1/cart".into(),
                    version: pm_model::HttpVersion::Http11,
                    headers: vec![],
                };
                f.set_test(
                    TestState {
                        result: Some(pm_model::RuleTestResult {
                            matched: true,
                            mismatch: None,
                            response_matched: Some(true),
                            request,
                            request_body: None,
                            response: None,
                            response_body: None,
                            notes: vec!["Answered by Map Local; the upstream isn't contacted.".into()],
                            request_body_truncated: false,
                            response_body_truncated: false,
                        }),
                        error: None,
                        busy: false,
                    },
                    cx,
                );
            });
        });
        let scale = cx
            .update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.scale_factor()
            })
            .unwrap();
        let name = if mode.is_dark() { "dark" } else { "light" };
        let image = capture(&mut cx, handle.into(), format!("rules-{name}.png"));
        let t = mode.tokens();
        // The form's pane: its plain background right of the name field.
        assert_pixel(&image, scale, (SIZE.0 - 30., 75.), t.background, "the form");
        // The list: its tinted background, a selected row and a group header.
        let list = over(
            Rgba {
                a: t.muted.a * 0.2,
                ..t.muted
            },
            t.background,
        );
        assert_pixel(&image, scale, (LIST_WIDTH / 2., SIZE.1 - 90.), list, "the rule list");
        let column = |want: Rgba| {
            (60..SIZE.1 as u32 - 60).any(|y| {
                let px = image.get_pixel((4. * scale) as u32, (y as f32 * scale) as u32).0;
                let want = [want.r, want.g, want.b].map(|c| (c * 255.0).round() as i32);
                (0..3).all(|i| (px[i] as i32 - want[i]).abs() <= 3)
            })
        };
        assert!(column(over(t.selection_bg, list)), "a selected rule row");
        assert!(
            column(over(
                Rgba {
                    a: t.muted.a * 0.6,
                    ..t.muted
                },
                list
            )),
            "a group header"
        );
    }

    pub fn run() {
        for mode in [Mode::Dark, Mode::Light] {
            println!("running shell_renders ({mode:?})");
            shell_renders(mode);
            println!("passed shell_renders ({mode:?})");
            println!("running rules_renders ({mode:?})");
            rules_renders(mode);
            println!("passed rules_renders ({mode:?})");
        }
        println!(
            "render: 4 passed (real offscreen renderer; shell, inspector, stream panels, palette and Rules window in each theme)"
        );
    }
}
