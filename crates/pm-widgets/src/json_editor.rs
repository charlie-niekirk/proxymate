//! JsonEditor (`JsonEditor.tsx`): the editable JSON tree over
//! [`pm_view::json::editor`]'s model. Virtualised rows with typed inline
//! editors (one gpui-kit input reused for the row being edited), a change
//! type menu, add / remove / rename / duplicate / reset, undo and redo, and
//! rows tinted where they differ from the original.
//!
//! Keys: ↑↓ move, ←→ collapse and expand, ↩ edit the value, F2 rename,
//! ⌘↩ add after, ⌘D duplicate, ⌫ remove, ⌘Z / ⇧⌘Z undo and redo. Escape
//! cancels an inline edit and stays inside the editor, so a window or
//! dialog holding it doesn't close.
//!
//! The row menu is data ([`MenuRequest::spec`]); the owner shows it, and
//! its entries dispatch this module's actions, which the focused editor
//! handles on its selected row.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable, Hsla, KeyDownEvent, MouseButton,
    MouseDownEvent, Pixels, Point, Role, ScrollStrategy, SharedString, Subscription, UniformListScrollHandle, Window,
    div, px, uniform_list,
};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::json::doc::{Id, JType, Node, RowStatus};
use pm_view::json::editor::{Field, JsonEditorModel, Key, Label, MenuSpec, Row};

pub const ROW_HEIGHT: f32 = 20.;
const INDENT: f32 = 14.;
pub const KEY_CONTEXT: &str = "JsonEditor";
pub const HINT: &str = "↩ edit · F2 rename · ⌘↩ add · ⌫ remove";

/// "Change type ▸" a type, on the selected row.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = json_editor, no_json)]
pub struct ChangeType {
    pub t: SharedString,
}

/// "Add key / item ▸" a type, at the end of the selected container.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = json_editor, no_json)]
pub struct AddChild {
    pub t: SharedString,
}

/// "Add after ▸" a type, after the selected row.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = json_editor, no_json)]
pub struct AddAfter {
    pub t: SharedString,
}

gpui_kit::actions!(
    json_editor,
    [
        EditValue,
        Rename,
        Duplicate,
        ResetRow,
        RemoveRow,
        Undo,
        Redo,
        ExpandAll,
        CollapseAll
    ]
);

/// A right-click: the menu for the row under the pointer (now selected).
pub struct MenuRequest {
    pub spec: MenuSpec,
    pub position: Point<Pixels>,
}

pub type OnMenu = Rc<dyn Fn(MenuRequest, &mut Window, &mut App)>;

/// The document changed (an edit, undo or redo): read [`JsonEditor::root`].
pub struct Changed;

pub struct JsonEditor {
    id: SharedString,
    label: SharedString,
    model: JsonEditorModel,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    input: Entity<InputState>,
    /// The edit the input holds (so it is filled once per edit).
    input_for: Option<(Vec<Id>, Field)>,
    /// The inline edit's error, with the text it was about.
    input_error: Option<(SharedString, String)>,
    on_menu: Option<OnMenu>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Changed> for JsonEditor {}

impl JsonEditor {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        model: JsonEditorModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::PressEnter { .. } => this.commit_input(window, cx),
            // An error stays until the text changes (the input also says
            // Change when nothing did).
            InputEvent::Change => {
                let text = this.input.read(cx).value();
                if this.input_error.as_ref().is_some_and(|(at, _)| *at != text) {
                    this.input_error = None;
                    cx.notify();
                }
            }
            // Clicking away commits a valid edit and drops an invalid one.
            InputEvent::Blur => {
                if this.model.editing().is_some() {
                    let text = this.input.read(cx).value().to_string();
                    let before = this.model.root().clone();
                    if this.model.validate(&text).is_some() || this.model.commit(&text).is_err() {
                        this.model.cancel_edit();
                    }
                    this.after(before, false, window, cx);
                }
            }
            InputEvent::Focus => {}
        });
        JsonEditor {
            id: id.into(),
            label: label.into(),
            model,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            input,
            input_for: None,
            input_error: None,
            on_menu: None,
            _subscriptions: vec![subscription],
        }
    }

    pub fn on_menu(mut self, f: impl Fn(MenuRequest, &mut Window, &mut App) + 'static) -> Self {
        self.on_menu = Some(Rc::new(f));
        self
    }

    pub fn model(&self) -> &JsonEditorModel {
        &self.model
    }

    pub fn root(&self) -> &Arc<Node> {
        self.model.root()
    }

    /// The inline edit's error ("Not a JSON number"), if any.
    pub fn input_error(&self) -> Option<&str> {
        self.input_error.as_ref().map(|(_, e)| e.as_str())
    }

    /// The inline input's text (while editing).
    pub fn input_text(&self, cx: &App) -> Option<String> {
        self.model.editing().map(|_| self.input.read(cx).value().to_string())
    }

    /// Changes the model (tests, the dev remote and the owner use this; the
    /// editor itself does too), then follows the selection and the edit.
    pub fn update_model(&mut self, f: impl FnOnce(&mut JsonEditorModel), window: &mut Window, cx: &mut Context<Self>) {
        let before = self.model.root().clone();
        f(&mut self.model);
        self.after(before, true, window, cx);
    }

    /// Types into the inline input (as the keyboard would).
    pub fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx));
    }

    /// Presses ↩ in the inline input.
    pub fn commit_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value();
        let text = value.to_string();
        if let Some(problem) = self.model.validate(&text) {
            self.input_error = Some((value, problem));
            return cx.notify();
        }
        let before = self.model.root().clone();
        match self.model.commit(&text) {
            Ok(()) => self.after(before, true, window, cx),
            Err(problem) => {
                self.input_error = Some((value, problem));
                cx.notify();
            }
        }
    }

    pub fn cancel_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_model(JsonEditorModel::cancel_edit, window, cx);
    }

    /// A key, as if pressed in the focused tree.
    pub fn press(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut handled = false;
        self.update_model(|m| handled = m.key(key), window, cx);
        handled
    }

    fn after(&mut self, before: Arc<Node>, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !Arc::ptr_eq(&before, self.model.root()) {
            cx.emit(Changed);
        }
        let editing = self.model.editing().map(|e| (e.path.clone(), e.field));
        if editing != self.input_for {
            self.input_for = editing.clone();
            self.input_error = None;
            if editing.is_some() {
                let text = self.model.edit_text().unwrap_or_default();
                self.input.update(cx, |s, cx| {
                    s.set_value(text, window, cx);
                    s.focus(window, cx);
                    s.select_all(window, cx);
                });
            } else if refocus && self.input.read(cx).focus_handle(cx).is_focused(window) {
                window.focus(&self.focus, cx);
            }
        }
        if let Some(i) = self.model.selected_index() {
            self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    fn on_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The inline input takes its own keys.
        if !self.focus.is_focused(window) {
            return;
        }
        let ks = &e.keystroke;
        let primary = ks.modifiers.platform || ks.modifiers.control;
        let key = match (primary, ks.modifiers.shift, ks.key.as_str()) {
            (true, true, "z") => Key::Redo,
            (true, false, "z") | (true, false, "Z") => Key::Undo,
            (true, _, "y") => Key::Redo,
            (true, false, "d") => Key::Duplicate,
            (true, false, "enter") => Key::AddAfter,
            (false, _, "down") => Key::Down,
            (false, _, "up") => Key::Up,
            (false, _, "home") => Key::Home,
            (false, _, "end") => Key::End,
            (false, _, "left") => Key::Left,
            (false, _, "right") => Key::Right,
            (false, false, "enter") => Key::Enter,
            (false, _, "f2") => Key::Rename,
            (false, false, "backspace" | "delete") => Key::Remove,
            _ => return,
        };
        if self.press(key, window, cx) {
            cx.stop_propagation();
        }
    }

    fn selected_or_root(&self) -> usize {
        self.model.selected_index().unwrap_or(0)
    }

    fn on_row_down(&mut self, ix: usize, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.click_count >= 2 {
            self.update_model(|m| _ = m.start_edit(ix, Field::Value), window, cx);
            return;
        }
        window.focus(&self.focus, cx);
        self.update_model(|m| m.select_row(ix), window, cx);
    }

    fn open_menu(&mut self, ix: usize, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.update_model(|m| m.select_row(ix), window, cx);
        let (Some(menu), Some(spec)) = (self.on_menu.clone(), self.model.menu(ix)) else {
            return;
        };
        let request = MenuRequest {
            spec,
            position: e.position,
        };
        cx.defer_in(window, move |_, window, cx| menu(request, window, cx));
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        let t = *cx.tokens();
        let selected = self.model.selected_index();
        let editing = self.model.editing_row();
        range
            .filter_map(|ix| {
                let row = self.model.rows().get(ix)?.clone();
                let edit = editing.filter(|(i, _)| *i == ix).map(|(_, f)| f);
                Some(self.render_row(ix, &row, selected == Some(ix), edit, &t, cx))
            })
            .collect()
    }

    fn render_row(
        &self,
        ix: usize,
        row: &Row,
        selected: bool,
        edit: Option<Field>,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let expandable = row.expandable();
        let id = row.id;
        let caret = if expandable {
            let icon = if row.expanded {
                gpui_kit::assets::IconName::ChevronDown
            } else {
                gpui_kit::assets::IconName::ChevronRight
            };
            div()
                .id(("json-edit-caret", ix))
                .flex_none()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.update_model(|m| m.toggle(id, None), window, cx);
                    }),
                )
                .child(crate::icon(icon, t.muted_foreground.hsla(), 12.))
                .into_any_element()
        } else {
            div().w(px(12.)).flex_none().into_any_element()
        };
        let label = row.label_text().map(|text| {
            let color = if matches!(row.label, Label::Index(_)) {
                t.syntax_punct
            } else {
                t.syntax_key
            };
            let key = if edit == Some(Field::Key) {
                self.inline_input(t, cx).into_any_element()
            } else {
                div()
                    .id(("json-edit-key", ix))
                    .flex_none()
                    .text_color(color.hsla())
                    .when(row.status == RowStatus::Renamed, |d| d.underline())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            if e.click_count >= 2 {
                                cx.stop_propagation();
                                this.update_model(|m| _ = m.start_edit(ix, Field::Key), window, cx);
                            }
                        }),
                    )
                    .child(text)
                    .into_any_element()
            };
            div()
                .flex()
                .flex_none()
                .items_center()
                .child(key)
                .child(div().text_color(t.syntax_punct.hsla()).child(":"))
        });
        let value_color: Hsla = match &*row.node {
            Node::Obj(_) | Node::Arr(_) => t.syntax_punct,
            Node::Str(_) => t.syntax_string,
            Node::Num(_) => t.syntax_number,
            Node::Bool(_) | Node::Null => t.syntax_literal,
        }
        .hsla();
        let value = if edit == Some(Field::Value) {
            self.inline_input(t, cx).into_any_element()
        } else {
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap(px(4.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(value_color)
                        .child(row.value_text()),
                )
                .children(row.size_text().map(|s| {
                    div()
                        .flex_none()
                        .text_size(px(10.5))
                        .text_color(t.muted_foreground.hsla())
                        .child(s)
                }))
                .into_any_element()
        };
        let bg = if selected {
            Some(t.accent)
        } else {
            match row.status {
                RowStatus::Added => Some(t.diff_add_bg),
                RowStatus::Changed | RowStatus::Renamed => Some(t.diff_change_bg),
                RowStatus::Same => None,
            }
        };
        div()
            .id(("json-edit-row", ix))
            .role(Role::TreeItem)
            .aria_label(SharedString::from(row.accessible_label()))
            .aria_level(row.depth + 1)
            .aria_selected(selected)
            .when(expandable, |d| d.aria_expanded(row.expanded))
            .h(px(ROW_HEIGHT))
            .w_full()
            .pl(px(6. + row.depth as f32 * INDENT))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .whitespace_nowrap()
            .overflow_hidden()
            .when_some(bg, |d, bg| d.bg(bg.hsla()))
            .when(selected, |d| d.border_l_2().border_color(t.primary.hsla()))
            .when(bg.is_none(), |d| d.hover(|s| s.bg(t.code_row_hover.hsla())))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| this.on_row_down(ix, e, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(ix, e, window, cx);
                }),
            )
            .child(caret)
            .children(label)
            .child(value)
            .when(row.removed > 0, |d| {
                d.child(
                    div()
                        .id(("json-edit-removed", ix))
                        .flex_none()
                        .ml(px(4.))
                        .px(px(4.))
                        .rounded(px(2.))
                        .bg(t.diff_del_bg.hsla())
                        .text_size(px(10.))
                        .text_color(t.diff_del.hsla())
                        .role(Role::Status)
                        .aria_label(SharedString::from(format!(
                            "{} original members or items removed",
                            row.removed
                        )))
                        .child(format!("−{}", row.removed)),
                )
            })
            .into_any_element()
    }

    fn inline_input(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let invalid = self.input_error.is_some();
        div()
            .id("json-edit-input")
            .key_context("JsonEditorEdit")
            .flex_1()
            .min_w(px(96.))
            .h(px(18.))
            .rounded(px(2.))
            .border_1()
            .border_color(if invalid { t.destructive } else { t.ring }.hsla())
            .bg(t.background.hsla())
            // The input lets Escape through: it cancels the edit here and
            // goes no further (a window or dialog stays open).
            .on_action(cx.listener(|this, _: &Escape, window, cx| this.cancel_input(window, cx)))
            .child(
                Input::new(&self.input)
                    .xsmall()
                    .appearance(false)
                    .bordered(false)
                    .aria_label(if let Some(e) = self.input_error() {
                        SharedString::from(e.to_owned())
                    } else {
                        SharedString::from("Edit")
                    }),
            )
    }
}

fn type_of(t: &SharedString) -> JType {
    JType::from_id(t).unwrap_or(JType::Str)
}

impl Focusable for JsonEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for JsonEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        let (can_undo, can_redo) = (self.model.can_undo(), self.model.can_redo());
        let tool = |id: &'static str, icon: gpui_kit::assets::IconName, tip: &'static str| {
            Button::new(SharedString::from(format!("{}-{id}", self.id)))
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip(tip)
        };
        let len = self.model.rows().len();
        let error = self.input_error().map(str::to_owned);
        div()
            .id(ElementId::Name(self.id.clone()))
            .key_context(KEY_CONTEXT)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background.hsla())
            .text_color(t.foreground.hsla())
            .on_action(cx.listener(|this, _: &Undo, window, cx| _ = this.press(Key::Undo, window, cx)))
            .on_action(cx.listener(|this, _: &Redo, window, cx| _ = this.press(Key::Redo, window, cx)))
            .on_action(
                cx.listener(|this, _: &ExpandAll, window, cx| {
                    this.update_model(JsonEditorModel::expand_all, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &CollapseAll, window, cx| {
                this.update_model(JsonEditorModel::collapse_all, window, cx)
            }))
            .on_action(cx.listener(|this, _: &EditValue, window, cx| {
                let i = this.selected_or_root();
                this.update_model(|m| _ = m.start_edit(i, Field::Value), window, cx)
            }))
            .on_action(cx.listener(|this, _: &Rename, window, cx| {
                let i = this.selected_or_root();
                this.update_model(|m| _ = m.start_edit(i, Field::Key), window, cx)
            }))
            .on_action(cx.listener(|this, a: &ChangeType, window, cx| {
                let (i, t) = (this.selected_or_root(), type_of(&a.t));
                this.update_model(|m| m.change_type(i, t), window, cx)
            }))
            .on_action(cx.listener(|this, a: &AddChild, window, cx| {
                let (i, t) = (this.selected_or_root(), type_of(&a.t));
                this.update_model(|m| m.add_child(i, t), window, cx)
            }))
            .on_action(cx.listener(|this, a: &AddAfter, window, cx| {
                let (i, t) = (this.selected_or_root(), type_of(&a.t));
                this.update_model(|m| m.add_after(i, t), window, cx)
            }))
            .on_action(cx.listener(|this, _: &Duplicate, window, cx| {
                let i = this.selected_or_root();
                this.update_model(|m| m.duplicate_row(i), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ResetRow, window, cx| {
                let i = this.selected_or_root();
                this.update_model(|m| m.reset(i), window, cx)
            }))
            .on_action(cx.listener(|this, _: &RemoveRow, window, cx| {
                let i = this.selected_or_root();
                this.update_model(|m| m.remove_row(i), window, cx)
            }))
            .child(
                div()
                    .flex_none()
                    .h(px(28.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .border_b_1()
                    .border_color(t.border.hsla())
                    .text_size(px(11.))
                    .child(
                        tool("undo", gpui_kit::assets::IconName::Undo2, "Undo (⌘Z)")
                            .disabled(!can_undo)
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| _ = this.press(Key::Undo, window, cx)),
                            ),
                    )
                    .child(
                        tool("redo", gpui_kit::assets::IconName::Redo2, "Redo (⇧⌘Z)")
                            .disabled(!can_redo)
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| _ = this.press(Key::Redo, window, cx)),
                            ),
                    )
                    .child(
                        tool("expand", gpui_kit::assets::IconName::ChevronsUpDown, "Expand all").on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                this.update_model(JsonEditorModel::expand_all, window, cx)
                            },
                        )),
                    )
                    .child(
                        tool("collapse", gpui_kit::assets::IconName::ChevronsDownUp, "Collapse all").on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.update_model(JsonEditorModel::collapse_all, window, cx)
                            }),
                        ),
                    )
                    .child(
                        div()
                            .ml(px(4.))
                            .min_w_0()
                            .truncate()
                            .text_color(t.muted_foreground.hsla())
                            .child(HINT),
                    )
                    .children(error.map(|e| {
                        div()
                            .id("json-edit-error")
                            .ml_auto()
                            .flex_none()
                            .role(Role::Alert)
                            .aria_label(SharedString::from(e.clone()))
                            .text_color(t.s_failed.hsla())
                            .child(e)
                    })),
            )
            .child(
                div()
                    .id(ElementId::Name(format!("{}-tree", self.id).into()))
                    .track_focus(&self.focus)
                    .role(Role::Tree)
                    .aria_label(self.label.clone())
                    .flex_1()
                    .min_h_0()
                    .on_key_down(cx.listener(Self::on_key_down))
                    .child(
                        uniform_list(
                            ElementId::Name(format!("{}-rows", self.id).into()),
                            len,
                            cx.processor(Self::render_rows),
                        )
                        .track_scroll(&self.scroll)
                        .size_full()
                        .font_family(mono)
                        .text_size(px(11.5)),
                    ),
            )
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc as StdArc, Mutex};

    use gpui_kit::{KeyBinding, TestAppContext, VisualTestContext};
    use pm_view::json::doc;

    use super::*;

    gpui_kit::actions!(json_editor_test, [ParentEscape]);

    /// A window around the editor whose own Escape binding counts what
    /// reaches it (a dialog closing on Escape).
    struct Parent {
        editor: Entity<JsonEditor>,
        escapes: StdArc<Mutex<usize>>,
    }

    impl Render for Parent {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let escapes = self.escapes.clone();
            div()
                .key_context("Parent")
                .size_full()
                .on_action(cx.listener(move |_, _: &ParentEscape, _, _| *escapes.lock().unwrap() += 1))
                .child(self.editor.clone())
        }
    }

    struct Harness<'a> {
        editor: Entity<JsonEditor>,
        cx: &'a mut VisualTestContext,
        escapes: StdArc<Mutex<usize>>,
        changes: StdArc<Mutex<usize>>,
    }

    fn setup<'a>(cx: &'a mut TestAppContext, text: &str) -> Harness<'a> {
        cx.update(|cx| {
            gpui_kit::init(cx);
            pm_theme::kit::apply(pm_theme::Mode::Dark, cx);
            cx.bind_keys([KeyBinding::new("escape", ParentEscape, Some("Parent"))]);
        });
        let root = doc::parse(text).unwrap();
        let escapes: StdArc<Mutex<usize>> = StdArc::default();
        let changes: StdArc<Mutex<usize>> = StdArc::default();
        let (sink, counted) = (changes.clone(), escapes.clone());
        let (parent, cx) = cx.add_window_view(move |window, cx| {
            let editor = cx.new(|cx| {
                JsonEditor::new(
                    "test-json",
                    "JSON",
                    JsonEditorModel::new(root.clone(), Some(root)),
                    window,
                    cx,
                )
            });
            cx.subscribe(&editor, move |_, _, _: &Changed, _| *sink.lock().unwrap() += 1)
                .detach();
            Parent {
                editor,
                escapes: counted,
            }
        });
        cx.simulate_resize(gpui_kit::size(px(600.), px(400.)));
        let editor = parent.read_with(cx, |p, _| p.editor.clone());
        cx.update(|window, cx| {
            let focus = editor.focus_handle(cx);
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        Harness {
            editor,
            cx,
            escapes,
            changes,
        }
    }

    impl Harness<'_> {
        fn keys(&mut self, keys: &str) {
            self.cx.simulate_keystrokes(keys);
            self.cx.run_until_parked();
        }

        fn text(&mut self) -> String {
            self.editor.read_with(self.cx, |e, _| e.root().to_json(0))
        }

        fn selected(&mut self) -> Option<usize> {
            self.editor.read_with(self.cx, |e, _| e.model().selected_index())
        }

        fn editing(&mut self) -> Option<(usize, Field)> {
            self.editor.read_with(self.cx, |e, _| e.model().editing_row())
        }
    }

    #[gpui_kit::test]
    fn edits_a_number_inline_and_keeps_escape(cx: &mut TestAppContext) {
        let mut h = setup(cx, r#"{"a":1,"b":"x"}"#);
        h.keys("down enter");
        assert_eq!(h.editing(), Some((1, Field::Value)));
        h.cx.simulate_input("oops");
        h.keys("enter");
        assert_eq!(
            h.editor.read_with(h.cx, |e, _| e.input_error().map(str::to_owned)),
            Some("Not a JSON number".into())
        );
        assert_eq!(h.text(), r#"{"a":1,"b":"x"}"#);
        // Escape cancels the edit and goes no further.
        h.keys("escape");
        assert_eq!(h.editing(), None);
        assert_eq!(*h.escapes.lock().unwrap(), 0, "Escape stayed inside the editor");
        assert_eq!(h.text(), r#"{"a":1,"b":"x"}"#);
        h.keys("enter");
        h.cx.simulate_input("1.50");
        h.keys("enter");
        assert_eq!(h.text(), r#"{"a":1.50,"b":"x"}"#);
        assert_eq!(h.editing(), None);
        assert_eq!(*h.changes.lock().unwrap(), 1);
        // With no edit open, Escape belongs to the window.
        h.keys("escape");
        assert_eq!(*h.escapes.lock().unwrap(), 1);
    }

    #[gpui_kit::test]
    fn its_keys_add_duplicate_remove_rename_and_undo(cx: &mut TestAppContext) {
        let mut h = setup(cx, r#"{"a":1}"#);
        h.keys("down cmd-d");
        assert_eq!(h.text(), r#"{"a":1,"a_copy":1}"#);
        assert_eq!(h.selected(), Some(2));
        h.keys("backspace");
        assert_eq!(h.text(), r#"{"a":1}"#);
        h.keys("f2");
        assert_eq!(h.editing(), Some((1, Field::Key)));
        h.cx.simulate_input("name");
        h.keys("enter");
        assert_eq!(h.text(), r#"{"name":1}"#);
        h.keys("cmd-enter");
        assert_eq!(h.editing(), Some((2, Field::Key)), "a new member starts with its key");
        h.keys("escape");
        assert_eq!(h.text(), r#"{"name":1,"key":""}"#);
        h.keys("cmd-z");
        assert_eq!(h.text(), r#"{"name":1}"#);
        h.keys("cmd-z cmd-z cmd-z");
        assert_eq!(h.text(), r#"{"a":1}"#);
        h.keys("cmd-shift-z");
        assert_eq!(h.text(), r#"{"a":1,"a_copy":1}"#);
    }

    #[gpui_kit::test]
    fn the_menu_actions_act_on_the_selected_row(cx: &mut TestAppContext) {
        let mut h = setup(cx, r#"{"a":"5"}"#);
        h.keys("down");
        h.cx.dispatch_action(ChangeType { t: "num".into() });
        h.cx.run_until_parked();
        assert_eq!(h.text(), r#"{"a":5}"#);
        h.cx.dispatch_action(AddAfter { t: "bool".into() });
        h.cx.run_until_parked();
        h.keys("escape");
        assert_eq!(h.text(), r#"{"a":5,"key":false}"#);
        h.keys("up");
        h.cx.dispatch_action(ResetRow);
        h.cx.run_until_parked();
        assert_eq!(h.text(), r#"{"a":"5","key":false}"#);
        h.keys("home");
        h.cx.dispatch_action(ResetRow);
        h.cx.run_until_parked();
        assert_eq!(h.text(), r#"{"a":"5"}"#);
    }
}
