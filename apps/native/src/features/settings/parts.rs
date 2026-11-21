//! The pages' building blocks (`Settings/parts.tsx`): a titled section with
//! a control on its right, labelled number fields, list editors, switches,
//! pickers (native menus), buttons, copyable fields and the notes under a
//! form. Every control is a gpui-kit control, or a div with a role and a
//! label; each has an element id the dev remote can click.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, FontWeight, MouseButton, MouseDownEvent, Role, SharedString, Window, div, px};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;

use super::SettingsWindow;
use super::window::NAV_WIDTH;

/// Controls are 26 px high: the Tauri pages' compact density.
pub const CONTROL_H: f32 = 26.;
/// A number field's label column (`grid-cols-[180px_1fr]`).
pub const LABEL_W: f32 = 180.;

/// How a button looks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Primary,
    Outline,
    Ghost,
    Danger,
}

/// A button at its own width, wherever it is placed (a page's sections
/// stretch their children across).
fn hug(button: AnyElement) -> AnyElement {
    div().flex().flex_none().child(button).into_any_element()
}

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}

impl Pipe for AnyElement {}

/// A titled block of a page, with an optional control on the right.
pub fn section(
    title: impl Into<SharedString>,
    description: Option<&str>,
    aside: Option<AnyElement>,
    body: Vec<AnyElement>,
    t: &Tokens,
) -> AnyElement {
    let title = title.into();
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(12.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .id(SharedString::from(format!("section-{title}")))
                                .role(Role::Heading)
                                .aria_label(title.clone())
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.foreground.hsla())
                                .child(title.clone()),
                        )
                        .when_some(description, |d, text| {
                            d.child(
                                div()
                                    .mt(px(2.))
                                    .text_size(px(12.))
                                    .line_height(px(16.))
                                    .text_color(t.muted_foreground.hsla())
                                    .child(text.to_owned()),
                            )
                        }),
                )
                .when_some(aside, |d, aside| d.child(div().flex_none().child(aside))),
        )
        .children(body)
        .into_any_element()
}

/// Muted text (a hint, an empty list).
pub fn muted(text: impl Into<SharedString>, t: &Tokens) -> AnyElement {
    div()
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(t.muted_foreground.hsla())
        .child(text.into())
        .into_any_element()
}

/// An inline error (`Note error`), announced as an alert.
pub fn error(id: &str, text: impl Into<SharedString>, t: &Tokens) -> AnyElement {
    let text = text.into();
    div()
        .id(SharedString::from(format!("error-{id}")))
        .role(Role::Alert)
        .aria_label(text.clone())
        .text_size(px(12.))
        .text_color(t.s_failed.hsla())
        .child(text)
        .into_any_element()
}

/// A confirmation under a form (`Note saved`).
pub fn saved(text: impl Into<SharedString>, t: &Tokens) -> AnyElement {
    div()
        .text_size(px(12.))
        .text_color(t.s_2xx.hsla())
        .child(text.into())
        .into_any_element()
}

/// A bordered box of rows (`ul.divide-y.rounded-md.border`).
pub fn rows(id: &str, label: &str, children: Vec<AnyElement>, t: &Tokens) -> AnyElement {
    let n = children.len();
    div()
        .id(SharedString::from(id.to_owned()))
        .role(Role::List)
        .aria_label(SharedString::from(label.to_owned()))
        .flex()
        .flex_col()
        .rounded(px(6.))
        .border_1()
        .border_color(t.border.hsla())
        .bg(t.muted.hsla().opacity(0.3))
        .children(children.into_iter().enumerate().map(|(i, row)| {
            div()
                .px(px(8.))
                .py(px(6.))
                .when(i + 1 < n, |d| d.border_b_1().border_color(t.border.hsla()))
                .child(row)
        }))
        .into_any_element()
}

/// An empty list's box (`italic` muted text inside the border).
pub fn empty_rows(text: &str, t: &Tokens) -> AnyElement {
    div()
        .rounded(px(6.))
        .border_1()
        .border_color(t.border.hsla())
        .bg(t.muted.hsla().opacity(0.3))
        .px(px(8.))
        .py(px(6.))
        .text_size(px(12.))
        .italic()
        .text_color(t.muted_foreground.hsla())
        .child(text.to_owned())
        .into_any_element()
}

/// A status line with a coloured dot (Listening, MCP serving).
pub fn status_line(
    id: &str,
    dot: pm_theme::Rgba,
    text: impl Into<SharedString>,
    t: &Tokens,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let text = text.into();
    div()
        .id(SharedString::from(id.to_owned()))
        .role(Role::Status)
        .aria_label(text.clone())
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border.hsla())
        .px(px(8.))
        .py(px(6.))
        .child(div().size(px(6.)).flex_none().rounded_full().bg(dot.hsla()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.))
                .text_color(t.foreground.hsla())
                .child(text),
        )
}

/// A labelled row: the label in a 180 px column, then the controls, then
/// an optional hint under them.
pub fn labelled(label: &str, controls: Vec<AnyElement>, hint: Option<&str>, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .w(px(LABEL_W))
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(t.foreground.hsla())
                        .child(label.to_owned()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .children(controls),
                ),
        )
        .when_some(hint, |d, hint| {
            d.child(
                div()
                    .pl(px(LABEL_W + 12.))
                    .text_size(px(11.))
                    .line_height(px(15.))
                    .text_color(t.muted_foreground.hsla())
                    .child(hint.to_owned()),
            )
        })
        .into_any_element()
}

/// A field over its small label (Network's rows: `Field`).
pub fn over(label: &str, control: AnyElement, width: Option<f32>, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .map(|d| match width {
            Some(w) => d.w(px(w)).flex_none(),
            None => d.flex_1().min_w(px(160.)),
        })
        .child(
            div()
                .text_size(px(11.))
                .text_color(t.muted_foreground.hsla())
                .child(label.to_owned()),
        )
        .child(control)
        .into_any_element()
}

impl SettingsWindow {
    /// A single-line input bound to `key`, `width` wide (or filling).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn input(
        &mut self,
        key: &str,
        placeholder: &str,
        label: &str,
        width: Option<f32>,
        mono: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.input_with(key, placeholder, label, width, mono, false, window, cx)
    }

    /// A password input: what is typed is hidden.
    pub(crate) fn password(
        &mut self,
        key: &str,
        placeholder: &str,
        label: &str,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.input_with(key, placeholder, label, Some(width), false, true, window, cx)
    }

    #[allow(clippy::too_many_arguments)]
    fn input_with(
        &mut self,
        key: &str,
        placeholder: &str,
        label: &str,
        width: Option<f32>,
        mono: bool,
        masked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.field(key, placeholder, masked, window, cx);
        let failed = self.field_failed(key);
        let font = if mono {
            cx.theme().mono_font_family.clone()
        } else {
            cx.theme().font_family.clone()
        };
        let t = *cx.tokens();
        div()
            .map(|d| match width {
                Some(w) => d.w(px(w)).flex_none(),
                None => d.flex_1().min_w(px(80.)),
            })
            .font_family(font)
            .text_size(px(12.))
            .when(failed, |d| d.rounded(px(6.)).border_1().border_color(t.s_failed.hsla()))
            .child(gpui_kit::Styled::px(
                gpui_kit::Styled::h(
                    Input::new(&state)
                        .id(SharedString::from(format!("field-{key}")))
                        .small()
                        .aria_label(SharedString::from(label.to_owned())),
                    px(CONTROL_H),
                ),
                px(8.),
            ))
            .into_any_element()
    }

    fn field_failed(&self, key: &str) -> bool {
        key.strip_prefix("net.")
            .zip(self.network.as_ref())
            .is_some_and(|(rest, n)| n.field_errors.contains_key(rest))
    }

    /// A number field in a labelled row, with its unit and hint.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn number(
        &mut self,
        key: &str,
        label: &str,
        unit: Option<&str>,
        hint: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.tokens();
        let input = self.input(key, "", label, Some(112.), true, window, cx);
        let mut controls = vec![input];
        if let Some(unit) = unit {
            controls.push(muted(unit.to_owned(), &t));
        }
        labelled(label, controls, hint, &t)
    }

    /// A switch bound to `key` (its change is a [`super::SettingsChoice`]).
    pub(crate) fn switch(
        &self,
        key: &str,
        on: bool,
        label: &str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let k = key.to_owned();
        Switch::new(SharedString::from(format!("switch-{key}")))
            .checked(on)
            .small()
            .disabled(disabled)
            .accessibility_label(label.to_owned())
            .on_click(cx.listener(move |this, value: &bool, window, cx| {
                let value = if *value { "true" } else { "false" };
                this.choose(&k, value, window, cx);
            }))
            .into_any_element()
    }

    /// A button pressing `id` (see [`super::SettingsPress`]).
    pub(crate) fn button(
        &self,
        id: &str,
        label: impl Into<SharedString>,
        look: Look,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = label.into();
        let press = id.to_owned();
        let button = Button::new(SharedString::from(id.to_owned()));
        let button = match look {
            Look::Primary => button.primary(),
            Look::Outline => button.outline(),
            Look::Ghost => button.ghost(),
            Look::Danger => button.danger(),
        };
        button
            .small()
            .h(px(CONTROL_H))
            .px(px(10.))
            .disabled(disabled)
            .label(label.clone())
            .accessibility_label(label)
            .on_click(cx.listener(move |this, _, window, cx| this.press(&press, window, cx)))
            .into_any_element()
            .pipe(hug)
    }

    /// A button with an icon before its label.
    pub(crate) fn icon_label_button(
        &self,
        id: &str,
        icon: IconName,
        label: &str,
        look: Look,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.tokens();
        let press = id.to_owned();
        let button = Button::new(SharedString::from(id.to_owned()));
        let button = match look {
            Look::Primary => button.primary(),
            Look::Outline => button.outline(),
            Look::Ghost => button.ghost(),
            Look::Danger => button.danger(),
        };
        let fg = if look == Look::Primary {
            t.primary_foreground
        } else {
            t.foreground
        };
        button
            .small()
            .h(px(CONTROL_H))
            .px(px(10.))
            .disabled(disabled)
            .accessibility_label(label.to_owned())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(pm_widgets::icon(icon, fg.hsla(), 13.))
                    .child(label.to_owned()),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.press(&press, window, cx)))
            .into_any_element()
            .pipe(hug)
    }

    /// An icon-only button (Remove), named for assistive tools.
    pub(crate) fn icon_button(
        &self,
        id: &str,
        icon: IconName,
        label: &str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.tokens();
        let press = id.to_owned();
        Button::new(SharedString::from(id.to_owned()))
            .ghost()
            .xsmall()
            .disabled(disabled)
            .tooltip(label.to_owned())
            .accessibility_label(label.to_owned())
            .child(pm_widgets::icon(icon, t.muted_foreground.hsla(), 13.))
            .on_click(cx.listener(move |this, _, window, cx| this.press(&press, window, cx)))
            .into_any_element()
    }

    /// A dropdown: its current label; a click shows `options` as a native
    /// menu whose entries are [`super::SettingsChoice`]s.
    pub(crate) fn picker(
        &self,
        key: &str,
        options: &[(String, String)],
        current: &str,
        label: &str,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shown = options
            .iter()
            .find(|(v, _)| v == current)
            .map_or_else(|| current.to_owned(), |(_, l)| l.clone());
        let k = key.to_owned();
        let t = *cx.tokens();
        Button::new(SharedString::from(format!("picker-{key}")))
            .outline()
            .small()
            .h(px(CONTROL_H))
            .px(px(8.))
            .accessibility_label(SharedString::from(format!("{label}: {shown}")))
            .child(
                div()
                    .w(px(width))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(4.))
                    .child(div().truncate().child(shown))
                    .child(pm_widgets::icon(IconName::ChevronDown, t.muted_foreground.hsla(), 12.)),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(menu) = this.menu_for(&format!("picker:{k}"), cx) {
                    menu.show(window.mouse_position(), window, cx);
                }
            }))
            .into_any_element()
    }

    /// Right-clicking `element` shows menu `key` (`menu_for`).
    pub(crate) fn with_menu(
        &self,
        element: gpui_kit::Stateful<gpui_kit::Div>,
        key: String,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        element.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                if let Some(menu) = this.menu_for(&key, cx) {
                    menu.show(e.position, window, cx);
                }
            }),
        )
    }

    /// An editable list of strings (`ListEditor`): each row with its Remove
    /// button and menu, then the add box. `list_id` names the list
    /// (`ssl.include`, `rec.never`, `net.bypass`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn list_editor(
        &mut self,
        list_id: &str,
        label: &str,
        placeholder: &str,
        empty: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.tokens();
        let items = self.list_items(list_id, cx);
        let list = if items.is_empty() {
            empty_rows(empty, &t)
        } else {
            let mono = cx.theme().mono_font_family.clone();
            let rows_el: Vec<AnyElement> = items
                .iter()
                .map(|item| {
                    let row = div()
                        .id(SharedString::from(format!("list-{list_id}-{item}")))
                        .role(Role::ListItem)
                        .aria_label(SharedString::from(item.clone()))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(mono.clone())
                                .text_size(px(11.))
                                .text_color(t.foreground.hsla())
                                .child(item.clone()),
                        )
                        .child(self.icon_button(
                            &format!("list.{list_id}.remove:{item}"),
                            IconName::X,
                            &format!("Remove {item}"),
                            false,
                            cx,
                        ));
                    self.with_menu(row, format!("list:{list_id}:{item}"), cx)
                        .test_support()
                        .into_any_element()
                })
                .collect();
            div()
                .id(SharedString::from(format!("list-{list_id}")))
                .max_h(px(160.))
                .overflow_y_scroll()
                .child(rows(&format!("rows-{list_id}"), label, rows_el, &t))
                .into_any_element()
        };
        let field = format!("list.{list_id}");
        let draft_empty = self.text(&field, cx).trim().is_empty();
        let input = self.input(&field, placeholder, &format!("Add to {label}"), None, true, window, cx);
        let add = self.icon_label_button(
            &format!("list.{list_id}.add"),
            IconName::Plus,
            "Add",
            Look::Outline,
            draft_empty,
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(list)
            .child(div().flex().items_center().gap(px(6.)).child(input).child(add))
            .into_any_element()
    }

    /// A value to copy (`CopyField`): shown in mono, with a Copy button and
    /// a menu; `display` is shown in its place (a masked token).
    pub(crate) fn copy_box(&self, id: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let field = self.copy_field(id, cx)?;
        let t = *cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        let shown = field.display.clone().unwrap_or_else(|| field.value.clone());
        let body = div()
            .id(SharedString::from(format!("copy-{id}")))
            .role(Role::Group)
            .aria_label(SharedString::from(field.label.clone()))
            .flex()
            .items_start()
            .gap(px(4.))
            .rounded(px(6.))
            .border_1()
            .border_color(t.border.hsla())
            .bg(t.muted.hsla().opacity(0.5))
            .px(px(8.))
            .py(px(4.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(mono)
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .text_color(t.foreground.hsla())
                    .when(!field.multiline, |d| d.truncate())
                    .child(shown),
            )
            .child(self.icon_button(&format!("copy:{id}"), IconName::Copy, &field.label, false, cx));
        Some(
            self.with_menu(body, format!("copy:{id}"), cx)
                .test_support()
                .into_any_element(),
        )
    }
}

/// The page list (`nav`): one row per page, the current one marked.
pub fn nav(page: &str, cx: &mut Context<SettingsWindow>) -> AnyElement {
    let t = *cx.tokens();
    div()
        .id("settings-pages")
        .role(Role::Navigation)
        .aria_label("Settings pages")
        .w(px(NAV_WIDTH))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(2.))
        .p(px(8.))
        .bg(t.muted.hsla().opacity(0.3))
        .border_r_1()
        .border_color(t.border.hsla())
        .children(pm_view::commands::SETTINGS_PAGES.iter().map(|(id, title)| {
            let on = *id == page;
            let fg = if on { t.accent_foreground } else { t.foreground };
            Button::new(SharedString::from(format!("settings-page-{id}")))
                .ghost()
                .small()
                .h(px(28.))
                .w_full()
                .selected(on)
                .when(on, |b| b.bg(t.accent.hsla()))
                .accessibility_label(SharedString::from(format!(
                    "{title}{}",
                    if on { " (current page)" } else { "" }
                )))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(fg.hsla())
                        .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                        .child(pm_widgets::icon(page_icon(id), fg.hsla(), 14.))
                        .child(*title),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.go(id, window, cx)))
        }))
        .into_any_element()
}

pub fn page_icon(id: &str) -> IconName {
    match id {
        "proxy" => IconName::Wifi,
        "network" => IconName::Network,
        "ssl" => IconName::ShieldCheck,
        "clients" => IconName::Laptop,
        "storage" => IconName::HardDrive,
        "protobuf" => IconName::Braces,
        "mocking" => IconName::Boxes,
        "appearance" => IconName::Palette,
        "advanced" => IconName::SlidersHorizontal,
        "mcp" => IconName::Bot,
        _ => IconName::RefreshCw,
    }
}
