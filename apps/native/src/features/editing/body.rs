//! A message body being edited (`BodyEditor` and `TreeBody` in
//! `BreakpointEditor.tsx`): G4's code editor, highlighted for the live
//! Content-Type, or, for a JSON body with an original to compare with, the
//! JsonEditor tree (its changes tinted). The text follows every tree edit,
//! written back pretty if the original was.

use gpui_kit::component::Selectable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString, Subscription, Window, div, px,
};
use pm_theme::kit::TokensExt as _;
use pm_view::json::editor::{JsonEditorModel, is_pretty};
use pm_widgets::code_editor::CodeEditor;
use pm_widgets::json_editor::{self, JsonEditor};

use super::menus;
use crate::features::context_menu::Menu;

/// The text changed: read [`BodyEditor::text`].
pub struct BodyChanged;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyMode {
    Text,
    Tree,
}

pub struct BodyEditor {
    id: SharedString,
    label: SharedString,
    language: &'static str,
    read_only: bool,
    /// The paused body, for the tree's tints; `None` offers no tree.
    original: Option<String>,
    text: String,
    mode: BodyMode,
    /// The text isn't JSON, so the tree couldn't open.
    invalid: bool,
    code: Entity<CodeEditor>,
    tree: Option<Entity<JsonEditor>>,
    _code_events: Subscription,
    _tree_events: Option<Subscription>,
}

impl EventEmitter<BodyChanged> for BodyEditor {}

impl BodyEditor {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        text: String,
        original: Option<String>,
        language: &'static str,
        read_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (id, label) = (id.into(), label.into());
        let (code, events) = Self::make_code(&id, &label, language, read_only, &text, window, cx);
        BodyEditor {
            id,
            label,
            language,
            read_only,
            original,
            text,
            mode: BodyMode::Text,
            invalid: false,
            code,
            tree: None,
            _code_events: events,
            _tree_events: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_code(
        id: &SharedString,
        label: &SharedString,
        language: &'static str,
        read_only: bool,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<CodeEditor>, Subscription) {
        let id = SharedString::from(format!("{id}-code"));
        let label = label.clone();
        let text = SharedString::from(text.to_owned());
        let code = cx.new(|cx| {
            let mut editor = CodeEditor::new(id, label, language, read_only, window, cx).on_menu(|req, window, cx| {
                Menu::new()
                    .selection(req.selection)
                    .copy("Copy body", req.text.to_string(), "body")
                    .show(req.position, window, cx);
            });
            editor.set_text(text, window, cx);
            editor
        });
        let state = code.read(cx).state().clone();
        let events = cx.subscribe(&state, |this, state, e: &gpui_kit::component::input::InputEvent, cx| {
            if matches!(e, gpui_kit::component::input::InputEvent::Change) && this.mode == BodyMode::Text {
                let value = state.read(cx).value().to_string();
                if value != this.text {
                    this.text = value;
                    cx.emit(BodyChanged);
                }
            }
        });
        (code, events)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn mode(&self) -> BodyMode {
        self.mode
    }

    pub fn tree(&self) -> Option<&Entity<JsonEditor>> {
        self.tree.as_ref()
    }

    pub fn code(&self) -> &Entity<CodeEditor> {
        &self.code
    }

    /// Whether Tree is offered: an editable JSON body with an original.
    pub fn can_tree(&self) -> bool {
        !self.read_only
            && self
                .original
                .as_deref()
                .is_some_and(pm_view::breakpoints::is_json)
    }

    /// Follows the Content-Type: a new editor in the language, same text.
    pub fn set_language(&mut self, language: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if language == self.language {
            return;
        }
        self.language = language;
        let (code, events) = Self::make_code(&self.id, &self.label, language, self.read_only, &self.text, window, cx);
        self.code = code;
        self._code_events = events;
        cx.notify();
    }

    /// Replaces the text (the dev remote, Compose's "Replace with text").
    pub fn set_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.text = text.clone();
        self.code.update(cx, |c, cx| c.set_text(text.into(), window, cx));
        if self.mode == BodyMode::Tree {
            self.mode = BodyMode::Text;
            self.tree = None;
            self._tree_events = None;
        }
        cx.emit(BodyChanged);
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: BodyMode, window: &mut Window, cx: &mut Context<Self>) {
        if mode == self.mode {
            return;
        }
        match mode {
            BodyMode::Text => {
                // Back from the tree: the editor starts from the tree's text.
                let text = SharedString::from(self.text.clone());
                self.code.update(cx, |c, cx| c.set_text(text, window, cx));
                self.tree = None;
                self._tree_events = None;
                self.mode = BodyMode::Text;
            }
            BodyMode::Tree => {
                let Ok(model) = JsonEditorModel::for_text(&self.text, self.original.as_deref()) else {
                    self.invalid = true;
                    return cx.notify();
                };
                self.invalid = false;
                let pretty = self.original.as_deref().is_some_and(is_pretty);
                let id = SharedString::from(format!("{}-tree", self.id));
                let label = self.label.clone();
                let tree = cx.new(|cx| {
                    JsonEditor::new(id, label, model, window, cx).on_menu(|req, window, cx| {
                        menus::json_row(&req.spec).show(req.position, window, cx);
                    })
                });
                self._tree_events = Some(cx.subscribe(&tree, move |this, tree, _: &json_editor::Changed, cx| {
                    this.text = tree.read(cx).model().text(pretty);
                    cx.emit(BodyChanged);
                }));
                self.tree = Some(tree);
                self.mode = BodyMode::Tree;
            }
        }
        cx.notify();
    }
}

impl Focusable for BodyEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.tree {
            Some(tree) => tree.focus_handle(cx),
            None => self.code.focus_handle(cx),
        }
    }
}

impl Render for BodyEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let can_tree = self.can_tree();
        let mode = self.mode;
        let id = self.id.clone();
        let tab = |m: BodyMode, label: &'static str, cx: &mut Context<Self>| {
            let on = mode == m;
            let button = Button::new(SharedString::from(format!("{id}-{}", label.to_lowercase())))
                .xsmall()
                .label(label)
                .selected(on)
                .accessibility_label(SharedString::from(format!("Edit as {label}")))
                .on_click(cx.listener(move |this, _, window, cx| this.set_mode(m, window, cx)));
            if on { button.primary() } else { button.ghost() }
        };
        div()
            .id(self.id.clone())
            .role(Role::Group)
            .aria_label(self.label.clone())
            .size_full()
            .flex()
            .flex_col()
            .gap(px(4.))
            .when(can_tree, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(4.))
                        .when(self.invalid, |d| {
                            d.child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(t.s_failed.hsla())
                                    .child("The text isn't valid JSON"),
                            )
                        })
                        .child(tab(BodyMode::Text, "Text", cx))
                        .child(tab(BodyMode::Tree, "Tree", cx)),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(t.border.hsla())
                    .overflow_hidden()
                    .map(|d| match &self.tree {
                        Some(tree) => d.child(tree.clone()),
                        None => d.child(self.code.clone()),
                    }),
            )
    }
}
