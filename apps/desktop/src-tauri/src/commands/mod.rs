//! Tauri commands: argument plumbing only, everything delegates to `Engine`.
//! One module per domain; `lib.rs` lists every command.

pub mod capture;
pub mod devices;
pub mod dialogs;
pub mod documents;
pub mod flows;
pub mod mcp;
pub mod mocks;
pub mod protos;
pub mod proxy;
pub mod rules;
pub mod scripts;
pub mod settings;
pub mod workflow;

use std::sync::Arc;

use pm_core::Engine;
use tauri::State;

type EngineState<'a> = State<'a, Arc<Engine>>;

/// Runs a command's engine work on the blocking pool (R2). A sync command
/// runs on the main thread, so a refilter, a body read or a clear that
/// waits for the store lock behind an ingest batch would stall the UI.
async fn run_blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("the command failed: {e}"))?
}

/// [`run_blocking`] for commands without an error to return; a panic in `f`
/// gives the default.
async fn run_blocking_or_default<T: Default + Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tauri::async_runtime::spawn_blocking(f).await.unwrap_or_default()
}

/// The engine, for async commands that take the app handle (which
/// commands without a `Result` must, rather than a borrowed `State`).
fn engine_of(app: &tauri::AppHandle) -> Arc<Engine> {
    use tauri::Manager;
    app.state::<Arc<Engine>>().inner().clone()
}

/// Saving a script evaluates it once (up to the traffic timeout), so the
/// mutating commands run off the IPC thread.
async fn blocking<T: Send + 'static>(
    engine: &EngineState<'_>,
    f: impl FnOnce(&Engine) -> Result<T, pm_core::Error> + Send + 'static,
) -> Result<T, String> {
    let engine = engine.inner().clone();
    tokio::task::spawn_blocking(move || f(&engine))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
