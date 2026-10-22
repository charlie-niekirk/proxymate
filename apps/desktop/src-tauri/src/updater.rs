//! App updates (M11): tauri-plugin-updater against the signed `latest.json`
//! on GitHub Releases (`plugins.updater` in `tauri.conf.json`). A quiet
//! check runs at launch unless Settings › Updates turns it off; "Check for
//! Updates…" checks and reports either way. Every download is verified
//! against the updater public key before it replaces the app.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pm_core::Engine;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::{Error, Update, UpdaterExt};

/// The launch check waits for the window and the proxy to settle first.
const LAUNCH_DELAY: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(30);
/// Release notes beyond this are cut in the dialog (they're on GitHub).
const NOTES_MAX: usize = 600;

/// One check (and its dialog) at a time, so a slow launch check and a
/// menu click never stack two prompts.
static BUSY: AtomicBool = AtomicBool::new(false);

/// The quiet launch check: only an available update is shown. Debug builds
/// skip it (`tauri dev` has no installed bundle to replace) unless testing.
pub fn check_at_launch<R: Runtime>(app: &AppHandle<R>) {
    if !test_mode() && (cfg!(debug_assertions) || !app.state::<Arc<Engine>>().update_settings().check_at_launch) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(LAUNCH_DELAY).await;
        check(app, false).await;
    });
}

/// "Check for Updates…": says so when there's nothing new or it failed.
pub fn check_now<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(check(app, true));
}

async fn check<R: Runtime>(app: AppHandle<R>, interactive: bool) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    match find(&app).await {
        Ok(Some(update)) => return offer(app, update),
        Ok(None) if interactive => message(
            &app,
            MessageDialogKind::Info,
            "You're up to date",
            &format!("Proxymate {} is the latest version.", app.package_info().version),
        ),
        Ok(None) => {}
        Err(err) if interactive => message(
            &app,
            MessageDialogKind::Error,
            "Can't check for updates",
            &err.to_string(),
        ),
        Err(err) => eprintln!("update check: {err}"),
    }
    BUSY.store(false, Ordering::SeqCst);
}

async fn find<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Update>, Error> {
    // Never through the system proxy: that may be Proxymate itself.
    let updater = app.updater_builder().no_proxy().timeout(TIMEOUT).build()?;
    match updater.check().await {
        // No published release yet (the endpoint 404s).
        Err(Error::ReleaseNotFound) => Ok(None),
        other => other,
    }
}

/// Asks before installing; the app restarts through the normal exit path,
/// so the system proxy is restored and the live files removed first.
fn offer<R: Runtime>(app: AppHandle<R>, update: Update) {
    let mut text = format!(
        "Proxymate {} is available. You have {}.",
        update.version, update.current_version
    );
    if let Some(notes) = update.body.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        text.push_str("\n\n");
        text.push_str(&cut(notes, NOTES_MAX));
    }
    text.push_str("\n\nProxymate restarts after installing.");
    if test_mode() {
        eprintln!("PROXYMATE_UPDATE_TEST: installing {}", update.version);
        return install(app, update);
    }
    let handle = app.clone();
    app.dialog()
        .message(text)
        .title("Update available")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install and Restart".into(),
            "Later".into(),
        ))
        .show(move |install| {
            if install {
                self::install(handle, update);
            } else {
                BUSY.store(false, Ordering::SeqCst);
            }
        });
}

fn install<R: Runtime>(app: AppHandle<R>, update: Update) {
    tauri::async_runtime::spawn(async move {
        match update.download_and_install(|_, _| {}, || {}).await {
            Ok(()) => app.request_restart(),
            Err(err) => {
                eprintln!("update install: {err}");
                message(
                    &app,
                    MessageDialogKind::Error,
                    "Can't install the update",
                    &err.to_string(),
                );
                BUSY.store(false, Ordering::SeqCst);
            }
        }
    });
}

/// `PROXYMATE_UPDATE_TEST=1` in a debug bundle (`tauri build --debug`):
/// check at launch and install without asking, to exercise the updater
/// end to end from a shell (docs/RELEASING.md).
fn test_mode() -> bool {
    cfg!(debug_assertions) && std::env::var_os("PROXYMATE_UPDATE_TEST").is_some()
}

fn message<R: Runtime>(app: &AppHandle<R>, kind: MessageDialogKind, title: &str, text: &str) {
    app.dialog().message(text).title(title).kind(kind).show(|_| {});
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
}
