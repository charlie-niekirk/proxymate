//! System, Light and Dark. A forced mode sets the app's native appearance
//! (window chrome, vibrancy and traffic lights follow it); System clears it.
//! The theme always follows what the window reports, so a system change
//! reaches every window.

use gpui_kit::{App, Window, WindowAppearance};
use pm_theme::Mode;

use crate::prefs::{self, Appearance};

pub fn init(cx: &mut App) {
    let appearance = prefs::get(cx).appearance;
    apply(appearance, cx);
}

/// Settings › Appearance (and the View menu, the dev remote's `theme`).
pub fn set(appearance: Appearance, cx: &mut App) {
    prefs::update(cx, |p| p.appearance = appearance);
    apply(appearance, cx);
}

fn apply(appearance: Appearance, cx: &mut App) {
    let forced = match appearance {
        Appearance::System => None,
        Appearance::Light => Some(WindowAppearance::Light),
        Appearance::Dark => Some(WindowAppearance::Dark),
    };
    cx.set_window_appearance(forced);
    pm_theme::kit::apply(mode_of(forced.unwrap_or_else(|| cx.window_appearance())), cx);
    crate::menus::refresh(cx);
}

/// Follows the window's appearance when the setting is System: call from a
/// window's `observe_window_appearance`. A forced mode stays put.
pub fn window_changed(window: &Window, cx: &mut App) {
    if prefs::get(cx).appearance != Appearance::System {
        return;
    }
    let mode = mode_of(window.appearance());
    if pm_theme::kit::TokensExt::theme_mode(cx) != mode {
        pm_theme::kit::apply(mode, cx);
    }
}

pub fn mode_of(appearance: WindowAppearance) -> Mode {
    match appearance {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Mode::Dark,
        WindowAppearance::Light | WindowAppearance::VibrantLight => Mode::Light,
    }
}
