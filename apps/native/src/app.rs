//! Application start-up: the bridge (engine and tokio runtime) before gpui,
//! then gpui-kit, the theme, menus, features and the main window; and quit,
//! which restores the system proxy and shuts the engine down.

use std::path::PathBuf;

use gpui_kit::App;
use pm_app::single_instance::{self, Lock};

use crate::bridge::Bridge;

/// The data dir: the Tauri app's, or `PROXYMATE_DATA_DIR` (a dev instance
/// beside the installed app).
pub fn data_dir() -> Option<PathBuf> {
    pm_app::data_dir()
}

pub fn run(data_dir: Option<PathBuf>, lock: Option<Lock>) {
    let bridge = Bridge::start(data_dir.clone());
    gpui_kit::application()
        .with_assets(pm_widgets::Assets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            cx.set_global(bridge);
            crate::prefs::init(data_dir.clone(), cx);
            crate::appearance::init(cx);
            crate::menus::init(cx);
            crate::features::register(cx);
            serve_later_launches(lock, cx);
            install_quit(data_dir, cx);
            crate::shell::open_main_window(cx);
            crate::features::rules::open_from_env(cx);
            crate::features::settings::open_from_env(cx);
            crate::features::integration::open_from_env(cx);
            crate::features::breakpoints::open_from_env(cx);
            crate::features::compose::open_from_env(cx);
            #[cfg(feature = "dev-harness")]
            crate::dev::init(cx);
            #[cfg(feature = "dev-harness")]
            crate::dev::zoo::start(cx);
            crate::perf_probe::init(cx);
            crate::updater::init(cx);
            cx.activate(true);
        });
}

/// A later launch (see `main`) asks this copy to come forward.
fn serve_later_launches(lock: Option<Lock>, cx: &mut App) {
    let Some(lock) = lock else { return };
    let (tx, rx) = async_channel::unbounded::<()>();
    lock.serve(move || {
        let _ = tx.try_send(());
    });
    cx.spawn(async move |cx| {
        while rx.recv().await.is_ok() {
            cx.update(|cx| {
                cx.activate(true);
                match cx.windows().first() {
                    Some(window) => {
                        let _ = window.update(cx, |_, window, _| window.activate_window());
                    }
                    None => crate::shell::open_main_window(cx),
                }
            });
        }
    })
    .detach();
}

/// Quit (⌘Q, the menu, or closing the last window, as in the Tauri app)
/// restores the system proxy, then shuts the engine down. It runs in the
/// quit callback itself, not its future, which gpui may cut short.
fn install_quit(data_dir: Option<PathBuf>, cx: &mut App) {
    cx.on_app_quit(move |cx| {
        crate::prefs::flush(cx);
        pm_app::quit(&Bridge::engine(cx));
        if let Some(dir) = &data_dir {
            single_instance::release(dir);
        }
        async {}
    })
    .detach();
    // Closing the main window quits, as in the Tauri app (whose Rules
    // Editor was a dialog in it), even with the Rules window open.
    cx.on_window_closed(|cx, _| {
        let main_gone = crate::shell::parts(cx).is_some_and(|p| !cx.windows().contains(&p.window));
        if cx.windows().is_empty() || main_gone {
            cx.quit();
        }
    })
    .detach();
}
