//! Proxymate desktop app. Every command here is a thin adapter over
//! [`pm_core::Engine`]; no engine logic lives in this crate.

mod commands;
mod window;

use std::sync::Arc;

use pm_core::{Engine, synth};
use tauri::Manager;

/// Synthetic traffic until the proxy lands in M1. Override with
/// `PROXYMATE_SYNTH_FLOWS` (initial flows) and `PROXYMATE_SYNTH_RATE` (new flows/s).
const DEFAULT_SYNTH_FLOWS: usize = 100_000;
const DEFAULT_SYNTH_RATE: u32 = 20;

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
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            app.manage(start_engine());
            if let Some(main) = app.get_webview_window("main") {
                window::style_main_window(&main);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Proxymate");
}

fn start_engine() -> Arc<Engine> {
    let flows = env_or("PROXYMATE_SYNTH_FLOWS", DEFAULT_SYNTH_FLOWS);
    let rate = env_or("PROXYMATE_SYNTH_RATE", DEFAULT_SYNTH_RATE);
    let engine = Engine::new();
    synth::seed(&engine, flows, 1);
    if rate > 0 {
        synth::spawn_live(Arc::downgrade(&engine), rate, 2);
    }
    engine
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
