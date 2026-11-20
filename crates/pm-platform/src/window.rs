//! Native window chrome for the native (gpui) app (G0): sidebar vibrancy
//! behind a transparent window, and window captures for the dev remote's
//! `screenshot`. macOS only; elsewhere vibrancy is unsupported (the app
//! paints the solid `--sidebar` token) and captures fail.

use std::path::Path;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::PlatformError;

/// Puts an `NSVisualEffectView` (the Sidebar material, following the
/// window's active state) behind the content of `window`, which must be
/// transparent and paint its sidebar transparent. Call it on the main
/// thread, once per window. `Ok(false)` where there is no vibrancy.
pub fn apply_sidebar_vibrancy(window: &impl HasWindowHandle) -> Result<bool, PlatformError> {
    match handle(window)? {
        #[cfg(target_os = "macos")]
        RawWindowHandle::AppKit(h) => crate::macos::window::apply_sidebar_vibrancy(h.ns_view).map(|()| true),
        _ => Ok(false),
    }
}

/// The window server's number for `window` (`NSWindow.windowNumber`).
pub fn window_number(window: &impl HasWindowHandle) -> Result<i64, PlatformError> {
    match handle(window)? {
        #[cfg(target_os = "macos")]
        RawWindowHandle::AppKit(h) => crate::macos::window::window_number(h.ns_view),
        _ => Err(PlatformError::Unsupported),
    }
}

/// Saves a PNG of window `number` as the window server composites it
/// (vibrancy included), without its shadow. On macOS this is
/// `screencapture -l`, which needs Screen Recording permission for the
/// process that started the app.
pub fn capture(number: i64, path: &Path) -> Result<(), PlatformError> {
    if !cfg!(target_os = "macos") {
        return Err(PlatformError::Unsupported);
    }
    let output = std::process::Command::new("/usr/sbin/screencapture")
        .arg("-x")
        .arg("-o")
        .arg(format!("-l{number}"))
        .arg(path)
        .output()?;
    if !output.status.success() {
        return Err(PlatformError::Os(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    // screencapture exits 0 with no file when it isn't allowed to record.
    if !path.exists() {
        return Err(PlatformError::Os(
            "screencapture wrote nothing (Screen Recording permission?)".into(),
        ));
    }
    Ok(())
}

fn handle(window: &impl HasWindowHandle) -> Result<RawWindowHandle, PlatformError> {
    window
        .window_handle()
        .map(|h| h.as_raw())
        .map_err(|err| PlatformError::Os(err.to_string()))
}
