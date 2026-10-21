//! Proxymate desktop app. Every command here is a thin adapter over
//! [`pm_core::Engine`]; no engine logic lives in this crate.

mod body_scheme;
mod commands;
mod window;

use std::sync::Arc;

use pm_core::{Engine, EngineConfig, ProxyConfig, synth};
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
            commands::clear_flows,
            commands::set_recording,
            commands::get_proxy_status,
            commands::get_ca_info,
            commands::regenerate_ca,
            commands::get_ssl_settings,
            commands::set_ssl_settings,
            commands::add_ssl_bypass,
            commands::set_insecure_upstream,
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
        .setup(move |app| {
            builder.mount_events(app);
            app.manage(start_engine(app.path().data_dir().ok()));
            if let Some(main) = app.get_webview_window("main") {
                window::style_main_window(&main);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Proxymate")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Removes this session's live segment files.
                app.state::<Arc<Engine>>().shutdown();
            }
        });
}

/// Starts the engine and the proxy on 9090 (or the next free port up to
/// 9099). A proxy that can't bind is reported in the toolbar, not fatal.
///
/// For UI and performance work, `PROXYMATE_SYNTH_FLOWS` seeds synthetic
/// flows and `PROXYMATE_SYNTH_RATE` adds that many live ones per second.
/// `PROXYMATE_EXTRA_UPSTREAM_CA` names a PEM file of extra CAs to trust for
/// upstreams (e.g. the pm-testkit CA, or a corporate CA).
fn start_engine(data_dir: Option<std::path::PathBuf>) -> Arc<Engine> {
    let engine = Engine::with_config(EngineConfig {
        data_dir: data_dir.map(|dir| dir.join("Proxymate")),
        extra_upstream_roots: extra_upstream_roots(),
    });
    if let Err(err) = tauri::async_runtime::block_on(engine.start_proxy(ProxyConfig::default())) {
        eprintln!("{err}");
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
}
