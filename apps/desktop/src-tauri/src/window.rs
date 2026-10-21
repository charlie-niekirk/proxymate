//! Native window chrome. The overlay title bar and traffic-light inset are set
//! in `tauri.conf.json`; this adds the sidebar vibrancy behind the webview.

use tauri::WebviewWindow;

#[cfg(target_os = "macos")]
pub fn style_main_window(window: &WebviewWindow) {
    use window_vibrancy::{NSVisualEffectMaterial, NSVisualEffectState, apply_vibrancy};

    // The window is transparent; the UI paints every pane opaque except the
    // sidebar, which lets this material show through.
    if let Err(err) = apply_vibrancy(
        window,
        NSVisualEffectMaterial::Sidebar,
        Some(NSVisualEffectState::FollowsWindowActiveState),
        None,
    ) {
        eprintln!("vibrancy unavailable: {err}");
    }
}

/// Linux and Windows get a solid sidebar colour from the UI instead.
#[cfg(not(target_os = "macos"))]
pub fn style_main_window(_window: &WebviewWindow) {}
