//! Pieces the integration surfaces share (`DeviceSetup/parts.tsx`): a line
//! of a live checklist, a monospace value with a copy button and a copy
//! menu, numbered steps, a settings path, a section and a labelled field.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, ElementId, FontWeight, MouseButton, Role, SharedString, div, px};
use pm_theme::Tokens;
use pm_view::devices::{Check, CheckState};

use crate::features::clipboard;
use crate::features::context_menu::Menu;

pub const MONO: &str = "Menlo";

/// One line of a live checklist: an icon by state, the label, the detail.
pub fn check(c: &Check, t: &Tokens) -> AnyElement {
    check_with(c, None, t)
}

/// [`check`] with more under the detail (a button).
pub fn check_with(c: &Check, extra: Option<AnyElement>, t: &Tokens) -> AnyElement {
    let (icon, colour) = match c.state {
        CheckState::Ok => (IconName::CircleCheck, t.s_2xx),
        CheckState::Warn => (IconName::CircleAlert, t.s_4xx),
        CheckState::Pending => (IconName::LoaderCircle, t.muted_foreground),
        CheckState::Idle => (IconName::CircleAlert, t.muted_foreground),
    };
    let state = match c.state {
        CheckState::Ok => "done",
        CheckState::Warn => "needs attention",
        CheckState::Pending => "in progress",
        CheckState::Idle => "not started",
    };
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .id(ElementId::Name(format!("check-{}", c.label).into()))
        .role(Role::ListItem)
        .aria_label(SharedString::from(format!("{}: {state}", c.label)))
        .child(
            div()
                .flex_none()
                .mt(px(1.))
                .child(pm_widgets::icon(icon, colour.hsla(), 16.)),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().font_weight(FontWeight::MEDIUM).child(c.label.clone()))
                .when_some(c.detail.clone(), |d, detail| {
                    d.child(div().text_color(t.muted_foreground.hsla()).child(detail))
                })
                .children(extra),
        )
        .into_any_element()
}

/// A checklist: the lines in a bordered box.
pub fn checklist(lines: Vec<AnyElement>, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border.hsla())
        .id("checklist")
        .role(Role::List)
        .children(lines)
        .into_any_element()
}

/// Monospace text with a copy button; right-click copies too, plus `more`.
pub fn copy_field(
    id: impl Into<SharedString>,
    value: String,
    label: &'static str,
    what: &'static str,
    multiline: bool,
    more: Vec<(SharedString, String, &'static str)>,
    t: &Tokens,
) -> AnyElement {
    let id: SharedString = id.into();
    let menu_value = value.clone();
    let copy_value = value.clone();
    let text = div()
        .min_w_0()
        .flex_1()
        .py(px(2.))
        .font_family(MONO)
        .text_size(px(11.))
        .line_height(px(16.))
        .when(!multiline, |d| d.truncate())
        .when(multiline, |d| d.whitespace_normal())
        .child(value);
    div()
        .id(ElementId::Name(format!("{id}-field").into()))
        .flex()
        .items_start()
        .gap(px(4.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border.hsla())
        .bg(t.muted.hsla())
        .child(text)
        .child(
            Button::new(ElementId::Name(id))
                .ghost()
                .xsmall()
                .accessibility_label(label)
                .tooltip(label)
                .child(pm_widgets::icon(IconName::Copy, t.muted_foreground.hsla(), 13.))
                .on_click(move |_, _, cx| clipboard::copy(&copy_value, Some(what), cx)),
        )
        .on_mouse_down(MouseButton::Right, move |e, window, cx| {
            field_menu(label, &menu_value, what, &more).show(e.position, window, cx);
        })
        .into_any_element()
}

/// A copy field's menu: its value, then `more`.
pub fn field_menu(label: &str, value: &str, what: &str, more: &[(SharedString, String, &'static str)]) -> Menu {
    more.iter().fold(
        Menu::new().copy(label.to_owned(), value.to_owned(), what.to_owned()),
        |m, (l, v, w)| m.copy(l.clone(), v.clone(), *w),
    )
}

/// A numbered list of steps.
pub fn steps(items: Vec<AnyElement>, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .id("steps")
        .role(Role::List)
        .children(items.into_iter().enumerate().map(|(i, item)| {
            div()
                .flex()
                .items_start()
                .gap(px(6.))
                .id(("step", i))
                .role(Role::ListItem)
                .child(
                    div()
                        .flex_none()
                        .w(px(14.))
                        .text_color(t.muted_foreground.hsla())
                        .child(format!("{}.", i + 1)),
                )
                .child(div().min_w_0().flex_1().flex().flex_col().gap(px(4.)).child(item))
        }))
        .into_any_element()
}

/// A run of text and highlighted settings paths: `[("Open ", false),
/// ("Settings › Wi-Fi", true)]`.
pub fn rich(parts: &[(&str, bool)], t: &Tokens) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .text_color(t.muted_foreground.hsla())
        .children(parts.iter().flat_map(|(text, strong)| {
            // Words wrap one by one, so a long path breaks like prose.
            let strong = *strong;
            let colour = if strong { t.foreground } else { t.muted_foreground };
            text.split_inclusive(' ').map(move |word| {
                div()
                    .text_color(colour.hsla())
                    .when(strong, |d| d.font_weight(FontWeight::MEDIUM))
                    .child(word.replace(' ', "\u{a0}"))
            })
        }))
        .into_any_element()
}

/// A titled section.
pub fn section(title: Option<&'static str>, children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .when_some(title, |d, title| {
            d.child(
                div()
                    .id("section-title")
                    .role(Role::Heading)
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
        })
        .children(children)
        .into_any_element()
}

/// A label column and a value.
pub fn field(label: &'static str, width: f32, value: impl IntoElement, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .flex_none()
                .w(px(width))
                .text_color(t.muted_foreground.hsla())
                .child(label),
        )
        .child(div().min_w_0().flex_1().child(value))
        .into_any_element()
}

/// [`field`] beside a [`copy_field`]: the label on its text's line.
pub fn copy_row(label: &'static str, width: f32, value: impl IntoElement, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .flex_none()
                .w(px(width))
                .pt(px(7.))
                .text_color(t.muted_foreground.hsla())
                .child(label),
        )
        .child(div().min_w_0().flex_1().child(value))
        .into_any_element()
}

/// An error line.
pub fn error(text: impl Into<SharedString>, t: &Tokens) -> AnyElement {
    div()
        .id("error")
        .role(Role::Alert)
        .text_color(t.s_failed.hsla())
        .child(text.into())
        .into_any_element()
}

/// A small dot in `colour`.
pub fn dot(colour: pm_theme::Rgba) -> AnyElement {
    div()
        .flex_none()
        .size(px(6.))
        .rounded_full()
        .bg(colour.hsla())
        .into_any_element()
}

/// A switch inside a box the dev remote and tests can click by `id`
/// (gpui-kit's `Switch` isn't findable by id itself).
pub fn switch(id: &'static str, switch: gpui_kit::component::switch::Switch) -> AnyElement {
    use gpui_kit::base::TestSupportExt as _;
    div().id(id).flex_none().child(switch).test_support().into_any_element()
}
