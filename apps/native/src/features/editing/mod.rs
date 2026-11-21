//! What the editing surfaces share (G8): the Breakpoint Editor and Compose
//! edit headers with `pm_widgets`'s KvEditor and bodies with [`body`]'s
//! BodyEditor (G4's code editor, or the JsonEditor tree for JSON); their
//! menus are [`menus`] builders over the widgets' menu data.

pub mod body;
pub mod menus;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, FontWeight, SharedString, div, px};
use pm_theme::Tokens;
use pm_view::flow::MethodTone;
use pm_widgets::Pill;
use pm_widgets::flow_grid::row::method_colors;

pub use body::{BodyChanged, BodyEditor, BodyInit};

/// A method pill in the table's colours.
pub fn method_pill(method: &str, t: &Tokens) -> Pill {
    let (fg, bg) = method_colors(t, MethodTone::of(method));
    Pill::new(method.to_owned(), fg, bg)
}

/// A section heading over a form's part ("Headers", "Body · 12 bytes").
pub fn heading(text: impl Into<SharedString>, t: &Tokens) -> AnyElement {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.muted_foreground.hsla())
        .child(text.into())
        .into_any_element()
}

/// A single-line mono input, 28 px high.
pub fn line_input(state: &Entity<InputState>, id: &str, label: &str, cx: &App) -> AnyElement {
    let mono = gpui_kit::component::ActiveTheme::theme(cx).mono_font_family.clone();
    div()
        .flex_1()
        .min_w(px(60.))
        .font_family(mono)
        .text_size(px(12.))
        .child(gpui_kit::Styled::h(
            Input::new(state)
                .id(SharedString::from(id.to_owned()))
                .small()
                .aria_label(SharedString::from(label.to_owned())),
            px(28.),
        ))
        .into_any_element()
}

/// The line under a form for an error from the engine.
pub fn error_line(id: &'static str, error: Option<&str>, t: &Tokens) -> AnyElement {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(px(12.))
        .text_color(t.s_failed.hsla())
        .when_some(error, |d, e| {
            d.role(gpui_kit::Role::Alert)
                .aria_label(SharedString::from(e.to_owned()))
                .child(e.to_owned())
        })
        .into_any_element()
}
