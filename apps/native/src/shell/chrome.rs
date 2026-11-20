//! Native chrome helpers: strips of the transparent title bar that move the
//! window (the window owns title bar drags, see `window_options`).

use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, MouseButton, Stateful, div};

/// A strip that moves the window when dragged and zooms (or minimises, per
/// the system setting) on a double click. A control inside it that takes
/// the press (prevents the default) keeps its click.
pub fn drag(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).on_mouse_down(MouseButton::Left, |event, window, _| {
        if window.default_prevented() {
            return;
        }
        if event.click_count == 2 {
            window.titlebar_double_click();
        } else {
            window.start_window_move();
        }
    })
}
