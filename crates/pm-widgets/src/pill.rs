//! A method, status or state pill: saturated text on its tinted `-bg`
//! token, which `pm-theme`'s contrast test keeps at 4.5:1 over the table.

use gpui_kit::{App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px};
use pm_theme::Rgba;

#[derive(IntoElement)]
pub struct Pill {
    label: SharedString,
    fg: Rgba,
    bg: Rgba,
}

impl Pill {
    /// `fg` is the token (`m_get`), `bg` its tint (`m_get_bg`).
    pub fn new(label: impl Into<SharedString>, fg: Rgba, bg: Rgba) -> Self {
        Self {
            label: label.into(),
            fg,
            bg,
        }
    }
}

impl RenderOnce for Pill {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex_none()
            .px(px(5.))
            .h(px(16.))
            .flex()
            .items_center()
            .rounded(px(3.))
            .text_size(px(11.))
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .bg(self.bg.hsla())
            .text_color(self.fg.hsla())
            .child(self.label)
    }
}
