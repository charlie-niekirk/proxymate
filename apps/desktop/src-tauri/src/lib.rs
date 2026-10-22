//! Proxymate desktop app. Every command here is a thin adapter over
//! [`pm_core::Engine`]; no engine logic lives in this crate.

mod body_scheme;
mod commands;
mod menu;
mod updater;
mod window;

use std::path::PathBuf;
use std::sync::Arc;

use pm_core::{Engine, EngineConfig, synth};
use pm_model::MCP_DEFAULT_PORT;
use tauri::Manager;

/// The tauri-specta builder: the single list of commands, shared by the
/// invoke handler and the TypeScript bindings export.
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            commands::create_view,
            commands::set_filter,
            commands::set_sort,
            commands::drop_view,
            commands::get_rows,
            commands::get_flow,
            commands::subscribe_view,
            commands::get_flow_detail,
            commands::get_body,
            commands::save_body,
            commands::get_websocket_messages,
            commands::get_websocket_payload,
            commands::get_sse_events,
            commands::clear_flows,
            commands::set_recording,
            commands::get_proxy_status,
            commands::get_ca_info,
            commands::regenerate_ca,
            commands::get_ssl_settings,
            commands::set_ssl_settings,
            commands::add_ssl_bypass,
            commands::set_insecure_upstream,
            commands::subscribe_sidebar,
            commands::set_app_ssl_excluded,
            commands::get_trust_status,
            commands::install_ca_trust,
            commands::uninstall_ca_trust,
            commands::get_system_proxy_status,
            commands::set_system_proxy,
            commands::get_terminal_env,
            commands::list_rules,
            commands::create_rule,
            commands::update_rule,
            commands::set_rule_enabled,
            commands::delete_rule,
            commands::delete_agent_rules,
            commands::reorder_rules,
            commands::validate_rule,
            commands::draft_rule,
            commands::test_rule,
            commands::get_rule_settings,
            commands::set_rule_settings,
            commands::list_breakpoints,
            commands::subscribe_breakpoints,
            commands::get_breakpoint_body,
            commands::resolve_breakpoint,
            commands::get_device_access,
            commands::set_lan_access,
            commands::set_lan_interface,
            commands::subscribe_devices,
            commands::decide_device,
            commands::forget_device,
            commands::get_simulator_status,
            commands::install_simulator_cert,
            commands::get_android_status,
            commands::set_android_proxy,
            commands::push_android_cert,
            commands::subscribe_rules,
            commands::get_mcp_status,
            commands::subscribe_mcp,
            commands::set_mcp_settings,
            commands::get_mcp_token,
            commands::regenerate_mcp_token,
            commands::get_mcp_audit,
            commands::clear_mcp_audit,
            commands::send_request,
            commands::replay_flow,
            commands::get_request_spec,
            commands::get_curl,
            commands::list_documents,
            commands::open_document,
            commands::close_document,
            commands::save_session,
            commands::export_har,
            commands::get_storage_settings,
            commands::set_storage_settings,
            commands::get_storage_status,
            commands::subscribe_storage,
            commands::pin_flows,
            commands::get_proxy_settings,
            commands::set_proxy_settings,
            commands::set_sidebar_store,
            commands::get_update_settings,
            commands::set_update_settings,
            commands::check_for_updates,
        ])
        // Sizes are u64 but stay far below 2^53.
        .dangerously_cast_bigints_to_number()
}

/// Path of the generated bindings, relative to this crate.
pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/src/bindings.ts");

pub fn export_bindings(builder: &tauri_specta::Builder<tauri::Wry>) {
    builder
        .export(
            specta_typescript::Typescript::default().header("// @ts-nocheck\n/* eslint-disable */"),
            BINDINGS_PATH,
        )
        .expect("export TypeScript bindings");
}

pub fn run() {
    let builder = specta_builder();
    #[cfg(debug_assertions)]
    export_bindings(&builder);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .menu(menu::build)
        .on_menu_event(menu::on_event)
        .invoke_handler(builder.invoke_handler())
        .manage(body_scheme::BodyCache::default())
        .register_asynchronous_uri_scheme_protocol(body_scheme::SCHEME, |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            // Reading and decoding a body can take a while; keep it off the main thread.
            tauri::async_runtime::spawn_blocking(move || {
                let engine = app.state::<Arc<Engine>>();
                let cache = app.state::<body_scheme::BodyCache>();
                responder.respond(body_scheme::handle(&engine, &cache, &request));
            });
        })
        .register_asynchronous_uri_scheme_protocol("pmicon", |ctx, request, responder| {
            let engine = ctx.app_handle().state::<Arc<Engine>>().inner().clone();
            let name = percent_decode(request.uri().path().trim_start_matches('/'));
            // Rendering an icon the first time takes a few ms of AppKit work.
            std::thread::spawn(move || responder.respond(icon_response(&engine, &name)));
        })
        .setup(move |app| {
            builder.mount_events(app);
            let engine = start_engine(app.path().data_dir().ok());
            install_exit_guards(&engine);
            // MCP follows its settings (enabled, read-only) from here on.
            let port = env_or("PROXYMATE_MCP_PORT", MCP_DEFAULT_PORT);
            let mcp_engine = engine.clone();
            tauri::async_runtime::spawn(async move { pm_mcp::spawn_managed(mcp_engine, port).await });
            app.manage(engine);
            if let Some(main) = app.get_webview_window("main") {
                window::style_main_window(&main);
            }
            updater::check_at_launch(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Proxymate")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let engine = app.state::<Arc<Engine>>();
                restore_system_proxy(&engine);
                // Removes this session's live segment files.
                engine.shutdown();
            }
        });
}

/// `proxymate --sysproxy-guardian <pid> --data-dir <dir>`: waits for the app
/// to exit, then restores the system proxy if the app left it pointing at
/// itself (a crash or SIGKILL). Returns the process exit code.
pub fn run_guardian(args: &[String]) -> i32 {
    let pid = args.first().and_then(|p| p.parse::<u32>().ok());
    let dir = args
        .iter()
        .position(|a| a == "--data-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    let (Some(pid), Some(dir)) = (pid, dir) else {
        eprintln!("usage: proxymate --sysproxy-guardian <pid> --data-dir <dir>");
        return 2;
    };
    match Engine::run_system_proxy_guardian(&dir, pid) {
        Ok(services) if services.is_empty() => 0,
        Ok(services) => {
            eprintln!(
                "proxymate guardian: restored the system proxy of {}",
                services.join(", ")
            );
            0
        }
        Err(err) => {
            eprintln!("proxymate guardian: {err}");
            1
        }
    }
}

/// `proxymate mcp-stdio [--port N]`: relays a stdio MCP client to the
/// running app's endpoint, starting the app first if nothing listens.
pub fn run_mcp_stdio(args: &[String]) -> i32 {
    let port = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(MCP_DEFAULT_PORT);
    let Some(data_dir) = pm_platform::fs::app_data_dir() else {
        eprintln!("proxymate mcp-stdio: can't find the app's data directory");
        return 1;
    };
    let config = pm_mcp::bridge::BridgeConfig {
        addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port)),
        data_dir,
        launch: Some(Box::new(|| {
            pm_platform::process::launch_app(BUNDLE_ID).map_err(std::io::Error::other)
        })),
    };
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    match runtime.block_on(pm_mcp::bridge::run(config)) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("proxymate mcp-stdio: {err}");
            1
        }
    }
}

/// `identifier` in `tauri.conf.json`.
const BUNDLE_ID: &str = "dev.proxymate.desktop";

/// Starts the engine and the proxy on the configured port (9090 by default,
/// or the next free one of the nine after it). A proxy that can't bind is
/// reported in the toolbar, not fatal.
///
/// For UI and performance work, `PROXYMATE_SYNTH_FLOWS` seeds synthetic
/// flows and `PROXYMATE_SYNTH_RATE` adds that many live ones per second.
/// `PROXYMATE_EXTRA_UPSTREAM_CA` names a PEM file of extra CAs to trust for
/// upstreams (e.g. the pm-testkit CA, or a corporate CA).
/// `PROXYMATE_SYSTEM_PROXY=1` turns the system proxy on at launch.
fn start_engine(data_dir: Option<PathBuf>) -> Arc<Engine> {
    let engine = Engine::with_config(EngineConfig {
        data_dir: data_dir.map(|dir| dir.join("Proxymate")),
        extra_upstream_roots: extra_upstream_roots(),
        guardian_exe: std::env::current_exe().ok(),
    });
    // A run that died without its guardian may have left the proxy on.
    match engine.restore_stale_system_proxy() {
        Ok(services) if !services.is_empty() => {
            eprintln!(
                "restored the system proxy left by an earlier run: {}",
                services.join(", ")
            );
        }
        Ok(_) => {}
        Err(err) => eprintln!("can't restore an earlier run's system proxy: {err}"),
    }
    if let Err(err) = tauri::async_runtime::block_on(engine.start_proxy(engine.configured_proxy())) {
        eprintln!("{err}");
    }
    if env_or("PROXYMATE_SYSTEM_PROXY", 0u8) == 1 {
        let status = engine.set_system_proxy(true);
        if let Some(err) = status.error {
            eprintln!("can't turn on the system proxy: {err}");
        }
    }
    let flows = env_or("PROXYMATE_SYNTH_FLOWS", 0usize);
    let rate = env_or("PROXYMATE_SYNTH_RATE", 0u32);
    if flows > 0 {
        synth::seed(&engine, flows, 1);
    }
    if rate > 0 {
        synth::spawn_live(Arc::downgrade(&engine), rate, 2);
    }
    engine
}

/// The system proxy must never be left pointing at a dead Proxymate.
/// Quit restores in `RunEvent::Exit`; this covers SIGINT/SIGTERM/SIGHUP
/// (which also removes the live segment files) and panics. SIGKILL and
/// crashes are the guardian's job.
fn install_exit_guards(engine: &Arc<Engine>) {
    let weak = Arc::downgrade(engine);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(engine) = weak.upgrade() {
            restore_system_proxy(&engine);
        }
        previous(info);
    }));

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let engine = engine.clone();
        tauri::async_runtime::spawn(async move {
            let (Ok(mut int), Ok(mut term), Ok(mut hup)) = (
                signal(SignalKind::interrupt()),
                signal(SignalKind::terminate()),
                signal(SignalKind::hangup()),
            ) else {
                eprintln!("can't install signal handlers");
                return;
            };
            let code = tokio::select! {
                _ = int.recv() => 130,
                _ = term.recv() => 143,
                _ = hup.recv() => 129,
            };
            restore_system_proxy(&engine);
            engine.shutdown();
            std::process::exit(code);
        });
    }
}

fn restore_system_proxy(engine: &Engine) {
    match engine.restore_system_proxy() {
        Ok(services) if !services.is_empty() => eprintln!("restored the system proxy: {}", services.join(", ")),
        Ok(_) => {}
        Err(err) => eprintln!("can't restore the system proxy: {err}"),
    }
}

fn icon_response(engine: &Engine, name: &str) -> tauri::http::Response<Vec<u8>> {
    let builder = tauri::http::Response::builder().header("Access-Control-Allow-Origin", "*");
    let response = match engine.app_icon(name) {
        Some(png) => builder
            .header("Content-Type", "image/png")
            .header("Cache-Control", "max-age=86400")
            .body(png.as_ref().clone()),
        None => builder.status(404).body(Vec::new()),
    };
    response.unwrap_or_default()
}

/// `%XX` escapes in a URI path segment, as `convertFileSrc` writes them.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn extra_upstream_roots() -> Vec<Vec<u8>> {
    let Some(path) = std::env::var_os("PROXYMATE_EXTRA_UPSTREAM_CA") else {
        return Vec::new();
    };
    pm_core::tls::certs_from_pem_file(path.as_ref()).unwrap_or_else(|err| {
        eprintln!("PROXYMATE_EXTRA_UPSTREAM_CA: {err}");
        Vec::new()
    })
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    /// Regenerates `ui/src/bindings.ts`. CI runs this and fails on a diff.
    #[test]
    fn export_bindings() {
        super::export_bindings(&super::specta_builder());
    }

    #[test]
    fn icon_names_are_percent_decoded() {
        assert_eq!(super::percent_decode("Google%20Chrome"), "Google Chrome");
        assert_eq!(super::percent_decode("caf%C3%A9"), "café");
        assert_eq!(super::percent_decode("50%"), "50%");
    }
}
