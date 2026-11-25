//! The Settings window (G12) driven like a user would, against a real
//! engine (in memory: no data dir, so nothing on disk is shared; the proxy
//! listens on loopback only when a test starts it): each page's values
//! change through its fields, switches and buttons, reach the engine, and
//! come back when the page is opened again. Menus are run and their copies
//! read from the test clipboard. Async engine writes (the port, Network)
//! run on a tokio runtime the test owns, as the bridge's would.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{ProxyStatus, UpstreamProxy};
use proxymate_native::bridge::{self, keys};
use proxymate_native::features::settings::window::SettingsWindow;
use proxymate_native::features::settings::{self as feature, sample};
use proxymate_native::workspace::Workspace;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(path)
}

struct Harness<'a> {
    engine: Option<Arc<Engine>>,
    view: Entity<SettingsWindow>,
    cx: &'a mut VisualTestContext,
    _runtime: Option<tokio::runtime::Runtime>,
}

/// The app's features over `engine` (or none: sample data), and a Settings
/// window at `page`.
fn open<'a>(cx: &'a mut TestAppContext, page: &'static str, engine: Option<Arc<Engine>>) -> Harness<'a> {
    let runtime = engine.as_ref().map(|_| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    });
    cx.executor().allow_parking();
    let e = engine.clone();
    let handle = runtime.as_ref().map(|r| r.handle().clone());
    cx.update(|cx| {
        gpui_kit::init(cx);
        proxymate_native::prefs::init(None, cx);
        proxymate_native::features::register(cx);
        if let Some(handle) = handle {
            bridge::set_runtime(handle, cx);
        }
        let status = e.as_ref().map(|e| e.proxy_status()).unwrap_or_default();
        let ws = cx.new(|cx| Workspace::new(e.clone(), status, cx));
        Workspace::set_global(ws, cx);
        if e.is_none() {
            sample::seed(cx);
        }
    });
    let (view, cx) = cx.add_window_view(move |window, cx| SettingsWindow::new(page, window, cx));
    let v = view.clone();
    cx.update(|window, cx| SettingsWindow::register(window.window_handle(), &v, cx));
    let mut h = Harness {
        engine,
        view,
        cx,
        _runtime: runtime,
    };
    h.settle();
    h
}

fn engine() -> Arc<Engine> {
    Engine::with_config(EngineConfig::default())
}

impl Harness<'_> {
    fn engine(&self) -> &Arc<Engine> {
        self.engine.as_ref().unwrap()
    }

    /// Lets writes land: the background executor, then the runtime's.
    fn settle(&mut self) {
        for _ in 0..5 {
            self.cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        self.cx.run_until_parked();
    }

    /// Settles until `done` holds (async writes on the runtime).
    fn settle_until(&mut self, what: &str, done: impl Fn(&mut Self) -> bool) {
        for _ in 0..400 {
            self.settle();
            if done(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    fn go(&mut self, page: &str) {
        let page = page.to_owned();
        self.view.update_in(self.cx, |w, window, cx| w.go(&page, window, cx));
        self.settle();
    }

    fn type_text(&mut self, key: &str, value: &str) {
        let (key, value) = (key.to_owned(), value.to_owned());
        self.view
            .update_in(self.cx, |w, window, cx| w.type_text(&key, &value, window, cx));
        self.settle();
    }

    fn choose(&mut self, key: &str, value: &str) {
        let (key, value) = (key.to_owned(), value.to_owned());
        self.view
            .update_in(self.cx, |w, window, cx| w.choose(&key, &value, window, cx));
        self.settle();
    }

    fn press(&mut self, id: &str) {
        let id = id.to_owned();
        self.view.update_in(self.cx, |w, window, cx| w.press(&id, window, cx));
        self.settle();
    }

    fn text(&mut self, key: &str) -> String {
        let key = key.to_owned();
        self.cx.update(|_, cx| self.view.read(cx).text(&key, cx))
    }

    fn error(&mut self, form: &str) -> Option<String> {
        let form = form.to_owned();
        self.cx
            .update(|_, cx| self.view.read(cx).error(&form).map(str::to_owned))
    }

    fn menu(&mut self, key: &str) -> Option<proxymate_native::features::context_menu::Menu> {
        let key = key.to_owned();
        self.cx.update(|_, cx| self.view.read(cx).menu_for(&key, cx))
    }

    /// Runs a menu entry; answers with the clipboard.
    fn run_menu(&mut self, key: &str, label: &str) -> Option<String> {
        let menu = self.menu(key).unwrap_or_else(|| panic!("no menu {key}"));
        let label = label.to_owned();
        assert!(
            self.cx.update(|window, cx| menu.run(&label, window, cx)),
            "no entry {label} in {key}: {:?}",
            menu.labels()
        );
        self.settle();
        self.cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()))
    }

    /// Forgets every cached setting and opens the window again on `page`,
    /// so what shows was read back from the engine.
    fn reopen(&mut self, page: &'static str) {
        self.cx.update(|_, cx| {
            let c = keys::settings_caches(cx);
            macro_rules! stale {
                ($($e:expr),*) => { $( $e.update(cx, |c, cx| { c.invalidate(|_, _| true, cx); }); )* };
            }
            stale!(
                c.proxy,
                c.network,
                c.storage,
                c.recording,
                c.autosave,
                c.proto_schemas,
                c.proto_mappings,
                c.openapi_specs,
                c.scripts,
                c.mcp_token,
                c.updates
            );
            let r = keys::rules_caches(cx);
            r.settings.update(cx, |c, cx| {
                c.invalidate(|_, _| true, cx);
            });
            r.mock_sources.update(cx, |c, cx| {
                c.invalidate(|_, _| true, cx);
            });
        });
        // Read back (as the new window's first render would), then a new
        // window over them: its fields start from what the engine answered.
        self.cx.update(|_, cx| {
            let c = keys::settings_caches(cx);
            macro_rules! fetch {
                ($($e:expr),*) => { $( $e.update(cx, |c, cx| { c.get(&(), cx); }); )* };
            }
            fetch!(
                c.proxy,
                c.network,
                c.storage,
                c.recording,
                c.autosave,
                c.scripts,
                c.updates
            );
            let r = keys::rules_caches(cx);
            r.settings.update(cx, |c, cx| {
                c.get(&(), cx);
            });
        });
        self.settle();
        let view = self.cx.update(|window, cx| {
            let view = cx.new(|cx| SettingsWindow::new(page, window, cx));
            SettingsWindow::register(window.window_handle(), &view, cx);
            view
        });
        self.view = view;
        self.settle();
    }
}

#[gpui_kit::test]
fn opens_at_a_page_and_lists_eleven(cx: &mut TestAppContext) {
    let mut h = open(cx, "", None);
    assert_eq!(h.cx.update(|_, cx| h.view.read(cx).page()), "proxy");
    for (page, _) in pm_view::commands::SETTINGS_PAGES {
        h.go(page);
        assert_eq!(h.cx.update(|_, cx| h.view.read(cx).page()), page);
    }
    assert_eq!(pm_view::commands::SETTINGS_PAGES.len(), 11);
    h.go("nope");
    assert_eq!(h.cx.update(|_, cx| h.view.read(cx).page()), "proxy");
}

#[gpui_kit::test]
fn the_proxy_port_rebinds(cx: &mut TestAppContext) {
    let free = || {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    };
    let (first, second) = (free(), free());
    let engine = engine();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let e = engine.clone();
    runtime
        .block_on(async move {
            e.start_proxy(ProxyConfig {
                ip: std::net::Ipv4Addr::LOCALHOST.into(),
                ports: first..=first,
            })
            .await
        })
        .unwrap();
    let mut h = open(cx, "proxy", Some(engine.clone()));
    // The Port field starts at the setting (the default, 9090); type ours.
    h.type_text("proxy.port", "80");
    h.press("proxy.apply");
    assert_eq!(h.error("proxy").as_deref(), Some("Enter 1,024 to 65,535"));
    h.type_text("proxy.port", &second.to_string());
    h.press("proxy.apply");
    let want = format!("127.0.0.1:{second}");
    h.settle_until("the listener to move", |h| {
        h.engine().proxy_status().listening.as_deref() == Some(want.as_str())
            && !h.cx.update(|_, cx| h.view.read(cx).is_busy("proxy"))
    });
    assert_eq!(h.error("proxy"), None);
    assert_eq!(h.engine().proxy_settings().port, second);
    // The workspace (the toolbar's pill) follows, and the page shows it again.
    let shown =
        h.cx.update(|_, cx| Workspace::global(cx).unwrap().read(cx).proxy.listening.clone());
    assert_eq!(shown.as_deref(), Some(want.as_str()));
    assert_eq!(h.run_menu("listening", "Copy address").as_deref(), Some(want.as_str()));
    h.reopen("proxy");
    assert_eq!(h.text("proxy.port"), second.to_string());
    engine.shutdown();
    drop(runtime);
}

#[gpui_kit::test]
fn network_is_one_draft_with_revert_and_save(cx: &mut TestAppContext) {
    let mut h = open(cx, "network", Some(engine()));
    let draft = |h: &mut Harness| h.cx.update(|_, cx| h.view.read(cx).network_draft().cloned().unwrap());
    assert!(!draft(&mut h).dirty());
    h.press("net.add.up");
    h.type_text("net.up.0.hosts", "*.corp.example, api.test");
    h.type_text("net.up.0.host", "proxy.corp.example");
    h.type_text("net.up.0.port", "99999");
    assert!(draft(&mut h).field_errors.contains_key("up.0.port"));
    h.type_text("net.up.0.port", "3129");
    h.type_text("net.up.0.password", "s3cret");
    h.choose("net.up.0.kind", "socks5");
    assert!(draft(&mut h).dirty());
    // Revert drops it all.
    h.press("net.revert");
    assert!(!draft(&mut h).dirty());
    assert!(draft(&mut h).draft.upstream.rules.is_empty());

    h.press("net.add.up");
    h.type_text("net.up.0.host", "proxy.corp.example");
    h.type_text("net.up.0.password", "s3cret");
    h.press("net.add.dns");
    h.type_text("net.dns.0.host", "api.staging.test");
    h.type_text("net.dns.0.ip", "10.0.0.7");
    h.type_text("list.net.bypass", "*.internal.example");
    h.press("list.net.bypass.add");
    h.press("net.save");
    h.settle_until("Network to save", |h| {
        !h.cx.update(|_, cx| h.view.read(cx).is_busy("network"))
    });
    assert_eq!(h.error("network"), None);
    let saved = h.engine().network_settings();
    assert_eq!(saved.dns[0].ip, "10.0.0.7");
    assert!(saved.upstream.bypass.contains(&"*.internal.example".to_owned()));
    let UpstreamProxy::Http(server) = &saved.upstream.rules[0].proxy else {
        panic!("{saved:?}")
    };
    // The password went to the secret store: only "saved" comes back.
    assert_eq!((server.password.as_deref(), server.has_password), (None, true));
    assert!(!draft(&mut h).dirty(), "the draft starts again from what was saved");
    h.reopen("network");
    assert_eq!(h.text("net.up.0.host"), "proxy.corp.example");
    assert_eq!(h.text("net.up.0.password"), "", "a saved password is never shown");
    assert_eq!(h.text("net.dns.0.host"), "api.staging.test");
    // A bad save says why and keeps the draft.
    h.press("net.add.rev");
    h.type_text("net.rev.0.target", "not a url");
    h.press("net.save");
    h.settle_until("the refusal", |h| h.error("network").is_some());
    assert!(draft(&mut h).dirty());
}

#[gpui_kit::test]
fn ssl_and_recording_lists_add_and_remove(cx: &mut TestAppContext) {
    let mut h = open(cx, "ssl", Some(engine()));
    h.type_text("list.ssl.bypass", "pinned.example.com");
    h.press("list.ssl.bypass.add");
    assert!(
        h.engine()
            .ssl_settings()
            .bypass
            .contains(&"pinned.example.com".to_owned())
    );
    assert_eq!(h.text("list.ssl.bypass"), "", "the box empties");
    assert_eq!(
        h.run_menu("list:ssl.bypass:pinned.example.com", "Copy pinned.example.com")
            .as_deref(),
        Some("pinned.example.com")
    );
    h.run_menu("list:ssl.bypass:pinned.example.com", "Remove");
    assert!(h.engine().ssl_settings().bypass.is_empty());
    // The sidebar's ticks read the same lists.
    h.type_text("list.ssl.insecure_upstream", "localhost");
    h.press("list.ssl.insecure_upstream.add");
    assert!(h.cx.update(|_, cx| Workspace::global(cx).unwrap().read(cx).is_insecure("localhost")));

    h.go("storage");
    h.type_text("list.rec.never", "*.apple.com");
    h.press("list.rec.never.add");
    h.type_text("list.rec.only", "api.example.com");
    h.press("list.rec.only.add");
    let lists = h.engine().recording_settings();
    assert_eq!(
        (lists.never, lists.only),
        (vec!["*.apple.com".to_owned()], vec!["api.example.com".to_owned()])
    );
    h.reopen("storage");
    assert!(h.menu("list:rec.never:*.apple.com").is_some());
}

#[gpui_kit::test]
fn storage_limits_and_auto_save_round_trip(cx: &mut TestAppContext) {
    let mut h = open(cx, "storage", Some(engine()));
    h.type_text("storage.max_flows", "1000");
    h.type_text("storage.max_ws_messages", "50");
    h.press("storage.save");
    // The engine's own checks come back under the form.
    assert_eq!(
        h.error("storage").as_deref(),
        Some("keep at least 100 WebSocket messages per connection (or 0 for no limit)")
    );
    h.type_text("storage.max_ws_messages", "500");
    h.press("storage.save");
    assert_eq!(h.error("storage"), None);
    assert_eq!(h.engine().storage_settings().max_flows, 1000);
    assert_eq!(
        h.cx.update(|_, cx| h.view.read(cx).note("storage").map(str::to_owned))
            .as_deref(),
        Some("Saved. The capture was trimmed to the new limits.")
    );
    h.type_text("storage.max_disk_mb", "lots");
    h.press("storage.save");
    assert_eq!(h.error("storage").as_deref(), Some("Enter a whole number"));
    h.press("storage.revert");
    assert_eq!(
        h.text("storage.max_disk_mb"),
        h.engine().storage_settings().max_disk_mb.to_string()
    );

    let folder = tempdir("autosave");
    h.type_text("autosave.folder", &folder.display().to_string());
    h.type_text("autosave.interval", "15");
    h.type_text("autosave.keep", "3");
    h.press("autosave.save");
    h.choose("autosave.enabled", "true");
    assert_eq!(h.error("autosave"), None);
    let saved = h.engine().autosave_settings();
    assert_eq!((saved.enabled, saved.interval_minutes, saved.keep), (true, 15, 3));
    h.reopen("storage");
    assert_eq!(h.text("storage.max_flows"), "1000");
    assert_eq!(h.text("storage.max_ws_messages"), "500");
    assert_eq!(h.text("autosave.interval"), "15");
    assert_eq!(h.text("autosave.folder"), folder.display().to_string());
    let _ = std::fs::remove_dir_all(folder);
}

#[gpui_kit::test]
fn advanced_and_updates_round_trip(cx: &mut TestAppContext) {
    let mut h = open(cx, "advanced", Some(engine()));
    h.type_text("adv.bp", "45");
    h.press("adv.bp.save");
    h.type_text("adv.traffic_ms", "5");
    h.press("adv.scripts.save");
    assert_eq!(h.error("scripts").as_deref(), Some("Enter 10 to 60,000"));
    h.type_text("adv.traffic_ms", "750");
    h.type_text("adv.view_ms", "2500");
    h.press("adv.scripts.save");
    assert_eq!(h.engine().rule_settings().breakpoint_timeout_secs, 45);
    let scripts = h.engine().script_settings();
    assert_eq!((scripts.traffic_timeout_ms, scripts.view_timeout_ms), (750, 2500));

    h.go("updates");
    h.choose("updates.launch", "false");
    assert!(!h.engine().update_settings().check_at_launch);

    h.reopen("advanced");
    assert_eq!(h.text("adv.bp"), "45");
    assert_eq!(h.text("adv.view_ms"), "2500");
}

#[gpui_kit::test]
fn protobuf_and_mocking_libraries(cx: &mut TestAppContext) {
    let data = tempdir("mocking");
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(data.clone()),
        ..Default::default()
    });
    let mut h = open(cx, "protobuf", Some(engine));
    h.cx.update(|_, cx| {
        feature::window::import_proto(vec![repo("crates/pm-testkit/proto/shop.proto")], cx);
    });
    h.settle_until("the schema", |h| !h.engine().proto_schemas().is_empty());
    let id = h.engine().proto_schemas()[0].id;
    h.press(&format!("proto.remove:{id}"));
    assert!(h.engine().proto_schemas().is_empty());

    h.go("mocking");
    h.cx.update(|_, cx| {
        feature::mock_from(
            bridge::mutations::MockFrom::File(repo("crates/pm-core/tests/fixtures/chrome-devtools.har")),
            cx,
        );
    });
    h.settle_until("the recording", |h| !h.engine().mock_sources().is_empty());
    // The unsaved Serve from rule opens in the Rules window.
    h.settle_until("the Rules window", |h| {
        h.cx.update(|_, cx| {
            proxymate_native::features::rules::RulesWindow::global(cx).is_some_and(|(_, w)| w.read(cx).is_draft())
        })
    });
    let source = h.engine().mock_sources()[0].id.clone();
    h.press(&format!("mock.remove:{source}"));
    assert!(h.engine().mock_sources().is_empty());

    h.cx.update(|_, cx| {
        feature::import_openapi_path(repo("crates/pm-core/tests/fixtures/openapi/petstore.yaml"), cx);
    });
    h.settle_until("the spec", |h| !h.engine().openapi_specs().is_empty());
    let spec = h.engine().openapi_specs()[0].clone();
    h.press(&format!("openapi.generate:{}", spec.id));
    let generated = h
        .engine()
        .rule_book()
        .rules
        .into_iter()
        .filter(|r| !r.enabled && r.name.contains(""))
        .count();
    assert!(generated >= spec.operations as usize, "{generated} rules");
    let toast =
        h.cx.update(|_, cx| proxymate_native::features::clipboard::Toasts::last(cx).map(|t| t.title));
    assert!(toast.is_some_and(|t| t.ends_with("added, turned off until you review them")));
    h.press(&format!("openapi.remove:{}", spec.id));
    assert!(h.engine().openapi_specs().is_empty());
    let _ = std::fs::remove_dir_all(data);
}

#[gpui_kit::test]
fn mcp_switches_and_token(cx: &mut TestAppContext) {
    let mut h = open(cx, "mcp", Some(engine()));
    // The status is a push; set it as the publisher would.
    let status = h.engine().mcp_status();
    h.cx.update(|_, cx| bridge::mcp(cx).update(cx, |m, _| m.value = Some(status)));
    let before = h.engine().mcp_token();
    h.choose("mcp.read_only", "true");
    assert!(h.engine().mcp_settings().read_only);
    h.press("mcp.regenerate");
    assert!(h.cx.update(|_, cx| h.view.read(cx).confirming_regenerate()));
    h.press("mcp.regenerate.cancel");
    assert_eq!(h.engine().mcp_token(), before);
    h.press("mcp.regenerate");
    h.press("mcp.regenerate.confirm");
    let after = h.engine().mcp_token();
    assert_ne!(after, before);
    let copied = h.run_menu("copy:mcp-claude", "Copy bearer token");
    assert_eq!(copied.as_deref(), Some(after.as_str()), "menus copy the token whole");
}

#[gpui_kit::test]
fn menus_copy_the_right_text(cx: &mut TestAppContext) {
    let mut h = open(cx, "mcp", None);
    assert_eq!(
        h.run_menu("copy:mcp-claude", "Copy the claude mcp add command")
            .as_deref(),
        Some(pm_view::settings::claude_add_command("http://127.0.0.1:9095/mcp", sample::TOKEN).as_str())
    );
    let shown =
        h.cx.update(|_, cx| h.view.read(cx).copy_field("mcp-claude", cx).unwrap().display.unwrap());
    assert!(shown.ends_with("Bearer ••••••••cdef\""), "{shown}");
    assert_eq!(h.run_menu("audit:2", "Copy error").as_deref(), Some("read-only mode"));
    let json = h.run_menu("audit:3", "Copy entry as JSON").unwrap();
    assert!(json.contains("\"tool\": \"find_flows\""), "{json}");
    assert_eq!(
        h.run_menu("mcp-client:a1b2c3", "Copy name and version").as_deref(),
        Some("claude-code 2.4.0")
    );
    assert_eq!(
        h.run_menu("mcp-serving", "Copy URL").as_deref(),
        Some("http://127.0.0.1:9095/mcp")
    );
    assert_eq!(
        h.run_menu("listening", "Copy address").as_deref(),
        Some("127.0.0.1:9090")
    );
    assert_eq!(
        h.run_menu("lan-interface", "Copy IP address").as_deref(),
        Some("192.168.1.10")
    );
    assert_eq!(
        h.run_menu("copy:socks-curl", "Copy curl example").as_deref(),
        Some("curl --socks5-hostname 127.0.0.1:9090 https://example.com/")
    );
    // A device's menu: copies, then what applies to its decision.
    let labels = h.menu("device:192.168.1.42").unwrap().labels();
    assert_eq!(labels, ["Copy IP address", "Copy nickname", "—", "Deny", "Forget"]);
    let labels = h.menu("device:192.168.1.57").unwrap().labels();
    assert_eq!(labels, ["Copy IP address", "—", "Allow", "Deny", "Forget"]);
    assert_eq!(
        h.run_menu("storage-status", "Copy location").as_deref(),
        Some("/Users/me/Library/Application Support/Proxymate/live/3f2a")
    );
    // Pickers are menus of choices.
    let picker = h.menu("picker:net.up.0.kind").unwrap();
    assert_eq!(picker.labels(), ["HTTP", "SOCKS5", "Direct"]);
}

#[gpui_kit::test]
fn appearance_and_the_palette(cx: &mut TestAppContext) {
    let mut h = open(cx, "appearance", None);
    h.choose("appearance", "dark");
    assert_eq!(
        h.cx.update(|_, cx| proxymate_native::prefs::get(cx).appearance),
        proxymate_native::prefs::Appearance::Dark
    );
    h.choose("appearance", "system");
    // Settings: Protobuf and Settings: Updates are in the palette (the Tauri
    // palette left them out), and open their page.
    let ids: Vec<String> = h.cx.update(|_, cx| {
        proxymate_native::features::commands::available(cx)
            .into_iter()
            .map(|c| c.id.to_string())
            .filter(|id| id.starts_with("settings"))
            .collect()
    });
    for id in ["settings", "settings-protobuf", "settings-updates", "settings-mcp"] {
        assert!(ids.iter().any(|i| i == id), "{id} in {ids:?}");
    }
    h.cx.update(|window, cx| proxymate_native::features::commands::run("settings-updates", window, cx));
    h.settle();
    assert_eq!(h.cx.update(|_, cx| h.view.read(cx).page()), "updates");
}

#[gpui_kit::test]
fn clients_page_decides_devices(cx: &mut TestAppContext) {
    let mut h = open(cx, "clients", None);
    // Without an engine, buttons do nothing; their labels follow the device.
    let rows: Vec<Vec<&str>> = sample::devices()
        .devices
        .iter()
        .map(|d| {
            pm_view::settings::client_buttons(d, d.nickname.as_deref().unwrap_or(""))
                .into_iter()
                .map(|b| b.0)
                .collect()
        })
        .collect();
    assert_eq!(
        rows,
        [
            vec!["Deny", "Forget"],
            vec!["Allow", "Deny", "Forget"],
            vec!["Allow", "Forget"]
        ]
    );
    h.type_text("client.192.168.1.42", "Phone");
    let labels: Vec<&str> = {
        let nickname = h.text("client.192.168.1.42");
        pm_view::settings::client_buttons(&sample::devices().devices[0], &nickname)
            .into_iter()
            .map(|b| b.0)
            .collect()
    };
    assert_eq!(labels, ["Save", "Forget"], "a new nickname is saved with Save");
}

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-g12-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[allow(dead_code)]
fn status() -> ProxyStatus {
    sample::proxy_status()
}
