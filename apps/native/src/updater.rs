//! App updates (G15), as the Tauri app's `updater.rs` does them: the signed
//! `latest.json` on GitHub Releases ([`feed`]), a quiet check 5 s after
//! launch unless Settings › Updates turns it off, and "Check for Updates…",
//! which reports either way. Every download is verified against the
//! updater's public key before it replaces the app ([`install`]), and the
//! app then restarts through the normal quit path (gpui's `restart` runs the
//! quit callbacks, which restore the system proxy, then reopens the bundle
//! once this process has gone, so the single-instance lock is free).
//!
//! One check, with its prompt, runs at a time. Network work runs on the
//! bridge's tokio runtime, unpacking on its blocking pool; the prompts are
//! gpui's (`window.prompt`).

pub mod feed;
pub mod install;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::{App, AsyncApp, PromptLevel};

use crate::bridge::Bridge;
use feed::{Config, Update};

/// The launch check waits for the window and the proxy to settle first.
const LAUNCH_DELAY: Duration = Duration::from_secs(5);
/// Release notes beyond this are cut in the prompt (they're on GitHub).
const NOTES_MAX: usize = 600;

/// One check (and its prompt) at a time, so a slow launch check and a menu
/// click never stack two prompts.
static BUSY: AtomicBool = AtomicBool::new(false);

/// This build's version: the workspace's, or the one `cargo xtask
/// bundle-macos --version` compiled in (it writes the same to Info.plist).
pub const VERSION: &str = match option_env!("PROXYMATE_APP_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The quiet launch check: only an available update is shown. Debug builds
/// skip it (`cargo run` has no installed bundle to replace) unless testing.
pub fn init(cx: &mut App) {
    let test = test_mode();
    if feed::target().is_none() || (test.is_none() && cfg!(debug_assertions)) {
        return;
    }
    let engine = Bridge::engine(cx);
    let runtime = Bridge::runtime(cx);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(LAUNCH_DELAY).await;
        let wanted = runtime
            .spawn_blocking(move || engine.update_settings().check_at_launch)
            .await;
        if test.is_some() || wanted.unwrap_or(false) {
            cx.update(|cx| check(test == Some(TestMode::Check), cx));
        }
    })
    .detach();
}

/// "Check for Updates…" (and Settings › Updates › Check Now, G12): says so
/// when there's nothing new or the check failed.
pub fn check_now(cx: &mut App) {
    check(true, cx);
}

fn check(interactive: bool, cx: &mut App) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let task = Bridge::runtime(cx).spawn(async {
        let client = feed::client()?;
        feed::check(&client, &Config::app(), VERSION).await
    });
    cx.spawn(async move |cx| {
        let result = task.await.unwrap_or_else(|e| Err(feed::Error::Network(e.to_string())));
        cx.update(|cx| match result {
            Ok(Some(update)) => offer(update, cx),
            Ok(None) => {
                if interactive {
                    tell(
                        cx,
                        PromptLevel::Info,
                        "You're up to date",
                        format!("Proxymate {VERSION} is the latest version."),
                    );
                }
                done();
            }
            Err(err) => {
                if interactive {
                    tell(cx, PromptLevel::Critical, "Can't check for updates", err.to_string());
                } else {
                    eprintln!("update check: {err}");
                }
                done();
            }
        });
    })
    .detach();
}

/// Asks before installing ("Install and Restart" / "Later").
fn offer(update: Update, cx: &mut App) {
    if test_mode() == Some(TestMode::Install) {
        eprintln!("PROXYMATE_UPDATE_TEST: installing {}", update.version);
        return install(update, cx);
    }
    let mut text = format!(
        "Proxymate {} is available. You have {}.",
        update.version, update.current
    );
    if let Some(notes) = update.notes.as_deref().map(str::trim) {
        text.push_str("\n\n");
        text.push_str(&cut(notes, NOTES_MAX));
    }
    text.push_str("\n\nProxymate restarts after installing.");
    let Some(answer) = ask(
        cx,
        PromptLevel::Info,
        "Update available",
        text,
        &["Install and Restart", "Later"],
    ) else {
        return done();
    };
    cx.spawn(async move |cx| {
        let install_it = answer.await == Some(0);
        cx.update(|cx| if install_it { install(update, cx) } else { done() });
    })
    .detach();
}

fn install(update: Update, cx: &mut App) {
    let runtime = Bridge::runtime(cx);
    let task = runtime.clone().spawn(async move {
        let bundle = std::env::current_exe()
            .ok()
            .and_then(|exe| install::bundle_of(&exe))
            .ok_or("Proxymate isn't running from its app bundle, so it can't replace itself.")?;
        let client = feed::client().map_err(|e| e.to_string())?;
        let bytes = feed::download(&client, &update, &Config::app().public_key)
            .await
            .map_err(|e| e.to_string())?;
        let target = bundle.clone();
        runtime
            .spawn_blocking(move || install::install(&target, &bytes))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        Ok::<PathBuf, String>(bundle)
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = task.await.unwrap_or_else(|e| Err(e.to_string()));
        cx.update(|cx| match result {
            Ok(bundle) => {
                eprintln!("installed the update in {}; restarting", bundle.display());
                cx.set_restart_path(bundle);
                cx.restart();
            }
            Err(err) => {
                eprintln!("update install: {err}");
                tell(cx, PromptLevel::Critical, "Can't install the update", err);
                done();
            }
        });
    })
    .detach();
}

fn done() {
    BUSY.store(false, Ordering::SeqCst);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestMode {
    /// Check at launch and install without asking (as the Tauri app's test).
    Install,
    /// Run "Check for Updates…" at launch: every prompt shows, nothing
    /// installs unless "Install and Restart" is clicked.
    Check,
}

/// `PROXYMATE_UPDATE_TEST` in a debug bundle (`cargo xtask bundle-macos
/// --debug`), to exercise the updater end to end from a shell
/// (docs/RELEASING.md): `1` installs without asking, `check` shows the
/// prompts. The restarted app is opened by LaunchServices, without it.
fn test_mode() -> Option<TestMode> {
    if !cfg!(debug_assertions) {
        return None;
    }
    match std::env::var("PROXYMATE_UPDATE_TEST").ok()?.as_str() {
        "check" => Some(TestMode::Check),
        _ => Some(TestMode::Install),
    }
}

/// A prompt on the frontmost window, answering with the button's index;
/// `None` when there's no window.
fn ask(
    cx: &mut App,
    level: PromptLevel,
    title: &str,
    detail: String,
    buttons: &[&str],
) -> Option<impl Future<Output = Option<usize>> + 'static> {
    let window = cx.active_window().or_else(|| cx.windows().first().copied())?;
    let answer = window
        .update(cx, |_, window, cx| {
            window.prompt(level, title, Some(&detail), buttons, cx)
        })
        .ok()?;
    Some(async move { answer.await.ok() })
}

fn tell(cx: &mut App, level: PromptLevel, title: &str, detail: String) {
    if let Some(answer) = ask(cx, level, title, detail, &["OK"]) {
        cx.background_executor()
            .spawn(async move {
                let _ = answer.await;
            })
            .detach();
    }
}

fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn long_notes_are_cut_on_a_char_boundary() {
        assert_eq!(super::cut("short", 10), "short");
        assert_eq!(super::cut("héllo wörld", 6), "héllo…");
    }

    #[test]
    fn the_version_is_the_workspaces_unless_the_bundle_set_one() {
        let expected = option_env!("PROXYMATE_APP_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
        assert_eq!(super::VERSION, expected);
        assert!(semver::Version::parse(super::VERSION).is_ok());
    }
}
