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
use pm_model::{IpcError, IpcErrorKind};
use tauri::State;

type EngineState<'a> = State<'a, Arc<Engine>>;

/// Runs a command's engine work on the blocking pool through
/// [`Engine::blocking`] (R2): a sync command runs on the main thread, so a
/// refilter, a body read or a clear waiting for the store lock behind an
/// ingest batch would stall the UI. Saving a script evaluates it, so those
/// commands use it too.
async fn blocking<T: Send + 'static, E: Into<Ipc>>(
    engine: &Arc<Engine>,
    f: impl FnOnce(&Arc<Engine>) -> Result<T, E> + Send + 'static,
) -> Result<T, IpcError> {
    engine
        .blocking(move |engine| f(engine).map_err(ipc))
        .await
        .map_err(|e| IpcError::internal(format!("the command failed: {e}")))?
}

/// [`blocking`] for commands without an error to return; a panic in `f`
/// gives the default.
async fn blocking_or_default<T: Default + Send + 'static>(
    engine: &Arc<Engine>,
    f: impl FnOnce(&Arc<Engine>) -> T + Send + 'static,
) -> T {
    engine.blocking(move |engine| f(engine)).await.unwrap_or_default()
}

/// The engine, for async commands that take the app handle (which
/// commands without a `Result` must, rather than a borrowed `State`).
fn engine_of(app: &tauri::AppHandle) -> Arc<Engine> {
    use tauri::Manager;
    app.state::<Arc<Engine>>().inner().clone()
}

/// An error on its way to the webview: engine errors keep their kind
/// (`pm_core::Error::kind`); anything else (a dialog, a join) is
/// `internal`.
pub(crate) struct Ipc(IpcError);

impl From<pm_core::Error> for Ipc {
    fn from(err: pm_core::Error) -> Ipc {
        Ipc(err.into())
    }
}

impl From<IpcError> for Ipc {
    fn from(err: IpcError) -> Ipc {
        Ipc(err)
    }
}

impl From<tauri::Error> for Ipc {
    fn from(err: tauri::Error) -> Ipc {
        Ipc(IpcError::internal(err))
    }
}

impl From<tokio::task::JoinError> for Ipc {
    fn from(err: tokio::task::JoinError) -> Ipc {
        Ipc(IpcError::internal(err))
    }
}

/// Writing a file the user picked.
impl From<std::io::Error> for Ipc {
    fn from(err: std::io::Error) -> Ipc {
        Ipc(IpcError::new(IpcErrorKind::Storage, err.to_string()))
    }
}

fn ipc(err: impl Into<Ipc>) -> IpcError {
    err.into().0
}
