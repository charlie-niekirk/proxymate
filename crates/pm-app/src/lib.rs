//! Host glue shared by the Tauri app and the native (gpui) app, with no UI
//! dependency: the binary modes the app executable doubles as
//! ([`modes`]), starting the engine from the environment ([`start_engine`]),
//! the exit guards that put the system proxy back ([`install_exit_guards`]),
//! the managed MCP endpoint ([`spawn_mcp`]) and the native app's single
//! instance lock ([`single_instance`]).
//!
//! Every function that needs async work takes the caller's tokio runtime
//! [`Handle`]: Tauri passes its own runtime's, the native app the one its
//! bridge owns.

pub mod modes;
pub mod single_instance;

use std::path::PathBuf;
use std::sync::Arc;

use pm_core::{Engine, EngineConfig};
use pm_model::MCP_DEFAULT_PORT;
use tokio::runtime::Handle;

pub use modes::{run_guardian, run_mcp_stdio, run_mode};

/// The app's bundle identifier (`identifier` in `tauri.conf.json`); the
/// native app keeps it so updates and the data dir carry over.
pub const BUNDLE_ID: &str = "dev.proxymate.desktop";

/// The data directory both apps share: `~/Library/Application Support/Proxymate`
/// on macOS, `$XDG_DATA_HOME/Proxymate` on Linux. `PROXYMATE_DATA_DIR`
/// overrides it, as it does for `proxymate-cli mcp-stdio`: a dev instance
/// beside the installed app. (The Tauri app takes its own from Tauri; only
/// its `mcp-stdio` reads this.)
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("PROXYMATE_DATA_DIR")
        .map(PathBuf::from)
        .or_else(pm_platform::fs::app_data_dir)
}

/// Starts the engine and the proxy on the configured port (9090 by default,
/// or the next free one of the nine after it). A proxy that can't bind is
/// reported in the toolbar, not fatal. `data_dir` is the full data directory
/// (`…/Proxymate`), or `None` to keep everything in memory.
///
/// For UI and performance work, `PROXYMATE_SYNTH_FLOWS` seeds synthetic
/// flows and `PROXYMATE_SYNTH_RATE` adds that many live ones per second.
/// `PROXYMATE_EXTRA_UPSTREAM_CA` names a PEM file of extra CAs to trust for
/// upstreams (e.g. the pm-testkit CA, or a corporate CA).
/// `PROXYMATE_SYSTEM_PROXY=1` turns the system proxy on at launch.
pub fn start_engine(runtime: &Handle, data_dir: Option<PathBuf>) -> Arc<Engine> {
    let engine = Engine::with_config(EngineConfig {
        data_dir,
        extra_upstream_roots: extra_upstream_roots(),
        guardian_exe: std::env::current_exe().ok(),
        use_keychain: true,
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
    if let Err(err) = runtime.block_on(engine.start_proxy(engine.configured_proxy())) {
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
        engine.seed_synthetic(flows, 1);
    }
    if rate > 0 {
        engine.spawn_synthetic(rate, 2);
    }
    engine
}

/// Serves MCP on `PROXYMATE_MCP_PORT` (9095 by default). It follows its
/// settings (enabled, read-only) from here on.
pub fn spawn_mcp(runtime: &Handle, engine: Arc<Engine>) {
    let port = env_or("PROXYMATE_MCP_PORT", MCP_DEFAULT_PORT);
    runtime.spawn(async move { pm_mcp::spawn_managed(engine, port).await });
}

/// The system proxy must never be left pointing at a dead Proxymate.
/// Quit restores through [`quit`]; this covers SIGINT/SIGTERM/SIGHUP
/// (which also removes the live segment files) and panics. SIGKILL and
/// crashes are the guardian's job.
pub fn install_exit_guards(runtime: &Handle, engine: &Arc<Engine>) {
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
        runtime.spawn(async move {
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
            quit(&engine);
            std::process::exit(code);
        });
    }
    #[cfg(not(unix))]
    let _ = runtime;
}

/// What quitting does, in this order: put the system proxy back, then shut
/// the engine down (which removes this session's live segment files).
pub fn quit(engine: &Engine) {
    restore_system_proxy(engine);
    engine.shutdown();
}

/// Puts the system proxy back if it still points at us, and says so.
pub fn restore_system_proxy(engine: &Engine) {
    match engine.restore_system_proxy() {
        Ok(services) if !services.is_empty() => eprintln!("restored the system proxy: {}", services.join(", ")),
        Ok(_) => {}
        Err(err) => eprintln!("can't restore the system proxy: {err}"),
    }
}

fn extra_upstream_roots() -> Vec<Vec<u8>> {
    let Some(path) = std::env::var_os("PROXYMATE_EXTRA_UPSTREAM_CA") else {
        return Vec::new();
    };
    Engine::read_pem_certificates(path.as_ref()).unwrap_or_else(|err| {
        eprintln!("PROXYMATE_EXTRA_UPSTREAM_CA: {err}");
        Vec::new()
    })
}

/// `key` parsed as `T`, or `default` when it is unset or doesn't parse.
pub fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    parse_or(std::env::var(key).ok().as_deref(), default)
}

fn parse_or<T: std::str::FromStr>(value: Option<&str>, default: T) -> T {
    value.and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::parse_or;

    #[test]
    fn env_values_fall_back_when_unset_or_unparsable() {
        assert_eq!(parse_or(Some("42"), 7u32), 42);
        assert_eq!(parse_or(Some("x"), 7u32), 7);
        assert_eq!(parse_or(None, 7u32), 7);
    }
}
