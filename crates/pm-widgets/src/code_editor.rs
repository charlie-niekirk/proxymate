//! CodeEditor: gpui-kit's code editor (tree-sitter highlighting, folding,
//! line numbers, its own ⌘F search) for bodies up to the 2 MB limit and for
//! every editable one (G8's editors), read-only or editable.
//!
//! Two things it adds over the bare editor:
//! - Escape stays inside it (with [`CodeEditor::keep_escape`]), so a dialog
//!   or the inspector holding it doesn't close or lose focus on the
//!   Escape that dismisses its search or extra cursors.
//! - A large text is set after the first frame (G0 spike a: `set_value` of
//!   2 MB costs a few hundred ms on the foreground), with a placeholder
//!   until then.
//!
//! The filter's body terms are marked with decorations and stepped through
//! like the CodeView's ([`CodeEditor::set_terms`], [`CodeEditor::step`]).

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::base::input::{EditorState, Position, TextDecoration, TextDecorationCollection};
use gpui_kit::component::input::Editor;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, ElementId, Entity, FocusHandle, Focusable, HighlightStyle, KeyBinding, MouseButton, MouseDownEvent,
    Pixels, Point, Role, SharedString, Window, div, px,
};
use pm_theme::kit::TokensExt as _;
use pm_view::body::search::{self, Search};

gpui_kit::actions!(
    code_editor,
    [
        /// Escape, swallowed once the editor has had it.
        KeepEscape
    ]
);

pub const KEY_CONTEXT: &str = "CodeEditor";
/// Texts larger than this are set after the first frame.
pub const DEFER_ABOVE: usize = 128 * 1024;

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", KeepEscape, Some("CodeEditor && keep_escape"))]);
}

/// The editor's language for a body viewer: one of the grammars the app
/// enables (`json`, `javascript`, `html`, `css`, `graphql`), XML through
/// the HTML grammar, or plain text.
pub fn language_for(viewer: pm_view::body::registry::ViewerId) -> &'static str {
    use pm_view::body::registry::ViewerId::*;
    match viewer {
        Json | JsonTree => "json",
        Javascript => "javascript",
        Css => "css",
        Html | Xml | HtmlPreview => "html",
        Graphql => "graphql",
        _ => "text",
    }
}

pub struct MenuRequest {
    pub selection: Option<String>,
    pub text: SharedString,
    pub position: Point<Pixels>,
}

pub type OnMenu = Rc<dyn Fn(MenuRequest, &mut Window, &mut App)>;

pub struct CodeEditor {
    id: SharedString,
    label: SharedString,
    state: Entity<EditorState>,
    read_only: bool,
    keep_escape: bool,
    wrap: bool,
    text: SharedString,
    /// The text waits for the first frame (its generation, so a newer
    /// text wins).
    pending: Option<u64>,
    generation: u64,
    decorations: Option<TextDecorationCollection>,
    terms: Vec<String>,
    matches: Vec<Range<usize>>,
    current: Option<usize>,
    on_menu: Option<OnMenu>,
}

impl CodeEditor {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        language: &str,
        read_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = language.to_owned();
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .folding(true)
                .searchable(true)
                .context_menu(false)
        });
        CodeEditor {
            id: id.into(),
            label: label.into(),
            state,
            read_only,
            keep_escape: !read_only,
            wrap: false,
            text: SharedString::default(),
            pending: None,
            generation: 0,
            decorations: None,
            terms: Vec::new(),
            matches: Vec::new(),
            current: None,
            on_menu: None,
        }
    }

    /// Whether Escape stays inside (on by default when editable).
    pub fn keep_escape(mut self, keep: bool) -> Self {
        self.keep_escape = keep;
        self
    }

    pub fn on_menu(mut self, f: impl Fn(MenuRequest, &mut Window, &mut App) + 'static) -> Self {
        self.on_menu = Some(Rc::new(f));
        self
    }

    pub fn state(&self) -> &Entity<EditorState> {
        &self.state
    }

    /// The text as last set (an editable editor's edits are in [`Self::value`]).
    pub fn text(&self) -> &SharedString {
        &self.text
    }

    pub fn value(&self, cx: &App) -> SharedString {
        self.state.read(cx).value()
    }

    /// Whether the text is still waiting for the first frame.
    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn set_wrap(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.wrap != wrap {
            self.wrap = wrap;
            self.state.update(cx, |s, cx| s.set_soft_wrap(wrap, window, cx));
        }
    }

    /// Sets the text: at once when small, after the next frame when large.
    pub fn set_text(&mut self, text: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        if text == self.text && self.pending.is_none() {
            return;
        }
        self.text = text.clone();
        self.generation += 1;
        if text.len() <= DEFER_ABOVE {
            self.pending = None;
            self.apply_text(text, window, cx);
            return;
        }
        // Two frames: the one that draws the placeholder, then the set.
        let generation = self.generation;
        self.pending = Some(generation);
        cx.on_next_frame(window, move |_, window, cx| {
            cx.on_next_frame(window, move |this, window, cx| {
                if this.pending == Some(generation) {
                    this.pending = None;
                    this.apply_text(text, window, cx);
                }
            });
        });
        cx.notify();
    }

    fn apply_text(&mut self, text: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
        self.remark(cx);
        cx.notify();
    }

    /// The filter's body terms to mark.
    pub fn set_terms(&mut self, terms: Vec<String>, cx: &mut Context<Self>) {
        if self.terms != terms {
            self.terms = terms;
            self.remark(cx);
        }
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    pub fn current_match(&self) -> Option<usize> {
        self.current
    }

    fn remark(&mut self, cx: &mut Context<Self>) {
        self.current = None;
        self.matches = match Search::new(&self.terms) {
            Some(search) if self.pending.is_none() => search.ranges(&self.text),
            _ => Vec::new(),
        };
        self.paint_marks(cx);
    }

    fn paint_marks(&mut self, cx: &mut Context<Self>) {
        let t = *cx.tokens();
        let decorations = self
            .decorations
            .get_or_insert_with(|| {
                self.state
                    .update(cx, |s, cx| s.create_decorations_collection(Vec::new(), cx))
            })
            .clone();
        let marks = self
            .matches
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let bg = if Some(i) == self.current {
                    t.code_match_current
                } else {
                    t.code_match
                };
                TextDecoration::new(
                    r.clone(),
                    HighlightStyle {
                        background_color: Some(bg.hsla()),
                        ..Default::default()
                    },
                )
            })
            .collect();
        decorations.set(marks, cx);
        cx.notify();
    }

    /// Steps to the next (or previous) mark, selecting and revealing it.
    pub fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.current = search::step(self.current, self.matches.len(), forward);
        if let Some(range) = self.current.and_then(|i| self.matches.get(i)).cloned() {
            let before = &self.text[..range.start];
            let line = before.bytes().filter(|b| *b == b'\n').count();
            let line_start = before.rfind('\n').map_or(0, |i| i + 1);
            let character = before[line_start..].chars().count();
            self.state.update(cx, |s, cx| {
                s.set_cursor_position(Position::new(line as u32, character as u32), window, cx);
                s.set_selected_range(range, cx);
            });
        }
        self.paint_marks(cx);
    }

    /// Opens the editor's own search (⌘F).
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| {
            s.focus(window, cx);
            s.open_search(false, cx);
        });
    }

    pub fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
    }

    pub fn selected_text(&self, cx: &App) -> Option<String> {
        let text = self.state.read(cx).selected_value().to_string();
        (!text.is_empty()).then_some(text)
    }
}

impl Focusable for CodeEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.read(cx).focus_handle(cx)
    }
}

impl Render for CodeEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let context = if self.keep_escape {
            "CodeEditor keep_escape"
        } else {
            "CodeEditor"
        };
        let editor = Editor::new(&self.state)
            .readonly(self.read_only)
            .bordered(false)
            .appearance(false)
            .aria_label(self.label.clone())
            .h_full();
        let menu = self.on_menu.clone();
        div()
            .id(ElementId::Name(self.id.clone()))
            .key_context(context)
            .role(Role::Group)
            .aria_label(self.label.clone())
            .size_full()
            .relative()
            .bg(t.background.hsla())
            .on_action(|_: &KeepEscape, _, _| {})
            .when_some(menu, |d, menu| {
                // Our menu replaces the editor's (turned off in `new`).
                d.capture_any_mouse_down(cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    if e.button != MouseButton::Right {
                        return;
                    }
                    cx.stop_propagation();
                    let request = MenuRequest {
                        selection: this.selected_text(cx),
                        text: this.text.clone(),
                        position: e.position,
                    };
                    let menu = menu.clone();
                    cx.defer_in(window, move |_, window, cx| menu(request, window, cx));
                }))
            })
            .child(editor)
            .when(self.pending.is_some(), |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(t.background.hsla())
                        .text_size(px(12.))
                        .text_color(t.muted_foreground.hsla())
                        .child("Loading…"),
                )
            })
            .test_support()
    }
}
