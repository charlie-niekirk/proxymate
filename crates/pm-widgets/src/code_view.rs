//! CodeView: read-only text of any size (G0 spike a). Bodies over the
//! editor's 2 MB limit, "Load all" up to 20 MB, and the Raw tab.
//!
//! The text is cut into rows on the background executor once (lines, long
//! lines cut every [`NOWRAP_CHUNK`] bytes, or at the view's width in
//! characters when wrapping, the font being monospaced), and a
//! `uniform_list` draws only the rows on screen, each highlighted then by
//! [`pm_view::body::highlight`]. Rows are runs of one selection document,
//! so a drag selects across them; ⌘A marks the whole text (⌘C, or the
//! menu's Copy, then copies all of it, which the rows on screen alone
//! couldn't).
//!
//! ⌘F opens a find bar; without it, the filter's body terms
//! ([`CodeView::set_terms`]) are marked and stepped through. Matches are
//! found on the background executor, up to [`MATCH_LIMIT`]. A right-click
//! hands the owner what the menu needs ([`MenuRequest`]); the actions
//! ([`Find`], [`SelectAllText`]) are for its items.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::{SelectableText, TestSupportExt as _, TextSelectionHandle};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight, HighlightStyle, Hsla, KeyBinding, MouseButton,
    MouseDownEvent, Pixels, Point, Role, ScrollStrategy, SharedString, Subscription, Task, UniformListScrollHandle,
    Window, canvas, div, px, uniform_list,
};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::body::highlight::{self, Lang, Tone};
use pm_view::body::search::{self, Search};

gpui_kit::actions!(
    code_view,
    [
        /// Opens the find bar (⌘F).
        Find,
        /// The next match (↩ in the find bar, ⌘G).
        FindNext,
        /// The previous match (⇧↩, ⇧⌘G).
        FindPrevious,
        /// Closes the find bar (Escape).
        CloseFind,
        /// Marks the whole text (⌘A).
        SelectAllText,
        /// Copies the selection, or the whole text after ⌘A (⌘C).
        CopySelection
    ]
);

pub const KEY_CONTEXT: &str = "CodeView";
const FIND_CONTEXT: &str = "CodeViewFind";
/// A row's height.
pub const ROW_HEIGHT: f32 = 18.;
const FONT_SIZE: f32 = 12.;
/// Without wrapping, a line longer than this is cut into rows: shaping one
/// row of 20 MB each frame would never keep up.
pub const NOWRAP_CHUNK: usize = 2000;
/// Texts larger than this are cut into rows on the background executor.
const SYNC_INDEX_MAX: usize = 256 * 1024;

pub fn init(cx: &mut App) {
    let primary = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
    let ctx = Some(KEY_CONTEXT);
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    cx.bind_keys([
        KeyBinding::new(leak(format!("{primary}-f")), Find, ctx),
        KeyBinding::new(leak(format!("{primary}-g")), FindNext, ctx),
        KeyBinding::new(leak(format!("{primary}-shift-g")), FindPrevious, ctx),
        KeyBinding::new(leak(format!("{primary}-a")), SelectAllText, ctx),
        KeyBinding::new(leak(format!("{primary}-c")), CopySelection, ctx),
        KeyBinding::new("escape", CloseFind, ctx),
        KeyBinding::new("enter", FindNext, Some(FIND_CONTEXT)),
        KeyBinding::new("shift-enter", FindPrevious, Some(FIND_CONTEXT)),
    ]);
}

/// How the text is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Highlight {
    Lang(Lang),
    /// An HTTP message: the head's first `head_lines` lines, then a body in
    /// `body`.
    Http {
        head_lines: usize,
        body: Lang,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    /// 0-based line number.
    line: usize,
    /// Bytes of the text, without the line break.
    range: Range<usize>,
    /// The line's first row (it shows the line number).
    first: bool,
}

struct Rows {
    rows: Vec<Row>,
    lines: usize,
    /// The widest row, in characters.
    widest: usize,
}

/// The text cut into rows: lines, each cut every `cols` characters (or
/// [`NOWRAP_CHUNK`] bytes without wrapping), on character boundaries.
fn cut_rows(text: &str, cols: Option<usize>) -> Rows {
    let mut rows = Vec::new();
    let mut widest = 0;
    let mut line = 0;
    let mut start = 0;
    let bytes = text.as_bytes();
    loop {
        let end = bytes[start..].iter().position(|b| *b == b'\n').map(|p| start + p);
        let line_end = end.unwrap_or(text.len());
        let content_end = if line_end > start && bytes[line_end - 1] == b'\r' {
            line_end - 1
        } else {
            line_end
        };
        let mut piece = start;
        let mut first = true;
        loop {
            let cut = match cols {
                Some(cols) => {
                    // `cols` characters on, or the line's end.
                    let s = &text[piece..content_end];
                    if s.len() <= cols {
                        content_end
                    } else {
                        s.char_indices().nth(cols).map_or(content_end, |(i, _)| piece + i)
                    }
                }
                None => {
                    let mut cut = (piece + NOWRAP_CHUNK).min(content_end);
                    while !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    cut
                }
            };
            let chars = if text[piece..cut].is_ascii() {
                cut - piece
            } else {
                text[piece..cut].chars().count()
            };
            widest = widest.max(chars);
            rows.push(Row {
                line,
                range: piece..cut,
                first,
            });
            first = false;
            piece = cut;
            if piece >= content_end {
                break;
            }
        }
        line += 1;
        match end {
            Some(e) => start = e + 1,
            None => break,
        }
    }
    Rows {
        rows,
        lines: line,
        widest,
    }
}

/// What a right-click asks of the owner's menu.
pub struct MenuRequest {
    /// The selected text (all of it after ⌘A), if any.
    pub selection: Option<String>,
    /// The whole text, for "Copy all".
    pub text: SharedString,
    pub position: Point<Pixels>,
}

pub type OnMenu = Rc<dyn Fn(MenuRequest, &mut Window, &mut App)>;

pub struct CodeView {
    id: SharedString,
    label: SharedString,
    text: SharedString,
    highlight: Option<Highlight>,
    wrap: bool,
    rows: Arc<Vec<Row>>,
    lines: usize,
    widest: usize,
    /// The columns the rows were cut for (wrapping), and the width that gave.
    cols: Option<usize>,
    width: Pixels,
    char_width: Pixels,
    indexing: Option<Task<()>>,
    scroll: UniformListScrollHandle,
    hscroll: gpui_kit::ScrollHandle,
    selection: TextSelectionHandle,
    all_selected: bool,
    focus: FocusHandle,
    terms: Vec<String>,
    find: Option<Entity<InputState>>,
    find_open: bool,
    matches: Arc<Vec<Range<usize>>>,
    current: Option<usize>,
    searching: Option<Task<()>>,
    on_menu: Option<OnMenu>,
    _subscriptions: Vec<Subscription>,
}

impl CodeView {
    /// `id` names the view (rows are `<id>-<n>`), `label` is what assistive
    /// tools read.
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        text: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = text.into();
        let mut view = CodeView {
            id: id.into(),
            label: label.into(),
            selection: TextSelectionHandle::new(String::new(), cx),
            text: SharedString::default(),
            highlight: None,
            wrap: false,
            rows: Arc::new(Vec::new()),
            lines: 0,
            widest: 0,
            cols: None,
            width: px(0.),
            char_width: px(7.2),
            indexing: None,
            scroll: UniformListScrollHandle::new(),
            hscroll: gpui_kit::ScrollHandle::new(),
            all_selected: false,
            focus: cx.focus_handle(),
            terms: Vec::new(),
            find: None,
            find_open: false,
            matches: Arc::new(Vec::new()),
            current: None,
            searching: None,
            on_menu: None,
            _subscriptions: Vec::new(),
        };
        view.set_text(text, cx);
        view
    }

    pub fn highlight(mut self, highlight: Option<Highlight>) -> Self {
        self.highlight = highlight;
        self
    }

    pub fn wrap(mut self, wrap: bool, cx: &mut Context<Self>) -> Self {
        self.set_wrap(wrap, cx);
        self
    }

    pub fn on_menu(mut self, f: impl Fn(MenuRequest, &mut Window, &mut App) + 'static) -> Self {
        self.on_menu = Some(Rc::new(f));
        self
    }

    pub fn set_on_menu(&mut self, f: impl Fn(MenuRequest, &mut Window, &mut App) + 'static) {
        self.on_menu = Some(Rc::new(f));
    }

    pub fn text(&self) -> &SharedString {
        &self.text
    }

    pub fn line_count(&self) -> usize {
        self.lines
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Whether the rows are still being cut.
    pub fn is_indexing(&self) -> bool {
        self.indexing.is_some()
    }

    pub fn is_wrapping(&self) -> bool {
        self.wrap
    }

    pub fn scroll_handle(&self) -> &UniformListScrollHandle {
        &self.scroll
    }

    /// Replaces the text (the selection and the current match go).
    pub fn set_text(&mut self, text: SharedString, cx: &mut Context<Self>) {
        if text == self.text && !self.rows.is_empty() {
            return;
        }
        self.text = text.clone();
        self.selection = TextSelectionHandle::new(text.to_string(), cx);
        self.all_selected = false;
        self.reindex(cx);
        self.research(cx);
    }

    pub fn set_highlight(&mut self, highlight: Option<Highlight>, cx: &mut Context<Self>) {
        if self.highlight != highlight {
            self.highlight = highlight;
            cx.notify();
        }
    }

    pub fn set_wrap(&mut self, wrap: bool, cx: &mut Context<Self>) {
        if self.wrap != wrap {
            self.wrap = wrap;
            self.reindex(cx);
        }
    }

    /// The filter's body terms to mark (lower-cased, longest first).
    pub fn set_terms(&mut self, terms: Vec<String>, cx: &mut Context<Self>) {
        if self.terms != terms {
            self.terms = terms;
            if !self.find_open {
                self.research(cx);
            }
        }
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    pub fn current_match(&self) -> Option<usize> {
        self.current
    }

    pub fn is_find_open(&self) -> bool {
        self.find_open
    }

    /// The query of the find bar, when it is open.
    pub fn find_query(&self, cx: &App) -> Option<String> {
        self.find_open
            .then(|| self.find.as_ref().map(|f| f.read(cx).value().to_string()))
            .flatten()
    }

    fn columns(&self) -> Option<usize> {
        if !self.wrap || self.width <= px(0.) {
            return None;
        }
        let gutter = self.gutter_width();
        let usable = self.width - gutter - px(16.);
        Some(((usable / self.char_width).floor() as usize).max(20))
    }

    fn gutter_width(&self) -> Pixels {
        let digits = self.lines.max(1).to_string().len().max(2);
        self.char_width * (digits as f32 + 2.)
    }

    fn reindex(&mut self, cx: &mut Context<Self>) {
        let cols = self.columns();
        self.cols = cols;
        let text = self.text.clone();
        if text.len() <= SYNC_INDEX_MAX {
            let rows = cut_rows(&text, cols);
            self.apply_rows(rows, cx);
            return;
        }
        let task = cx.background_spawn(async move { cut_rows(&text, cols) });
        self.indexing = Some(cx.spawn(async move |this, cx| {
            let rows = task.await;
            let _ = this.update(cx, |this, cx| {
                this.indexing = None;
                this.apply_rows(rows, cx);
            });
        }));
        cx.notify();
    }

    fn apply_rows(&mut self, rows: Rows, cx: &mut Context<Self>) {
        self.rows = Arc::new(rows.rows);
        self.lines = rows.lines;
        self.widest = rows.widest;
        cx.notify();
    }

    /// The width changed: rows cut for wrapping are cut again.
    fn set_width(&mut self, width: Pixels, char_width: Pixels, cx: &mut Context<Self>) {
        let changed = (width - self.width).abs() > px(0.5) || (char_width - self.char_width).abs() > px(0.01);
        if !changed {
            return;
        }
        self.width = width;
        self.char_width = char_width;
        if self.wrap && self.columns() != self.cols {
            self.reindex(cx);
        }
    }

    fn query_terms(&self, cx: &App) -> Vec<String> {
        match self.find_query(cx) {
            Some(q) if !q.is_empty() => vec![q],
            Some(_) => Vec::new(),
            None => self.terms.clone(),
        }
    }

    /// Finds the matches again (the text, the terms or the query changed).
    fn research(&mut self, cx: &mut Context<Self>) {
        let terms = self.query_terms(cx);
        self.current = None;
        let Some(search) = Search::new(&terms) else {
            self.matches = Arc::new(Vec::new());
            self.searching = None;
            cx.notify();
            return;
        };
        let text = self.text.clone();
        let find = self.find_open;
        let task = cx.background_spawn(async move { search.ranges(&text) });
        self.searching = Some(cx.spawn(async move |this, cx| {
            let ranges = task.await;
            let _ = this.update(cx, |this, cx| {
                this.searching = None;
                this.matches = Arc::new(ranges);
                // The find bar jumps to the first match as you type.
                if find && !this.matches.is_empty() {
                    this.step(true, cx);
                }
                cx.notify();
            });
        }));
    }

    /// Steps to the next (or previous) match and scrolls to it.
    pub fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.current = search::step(self.current, self.matches.len(), forward);
        if let Some(at) = self.current.and_then(|i| self.matches.get(i)) {
            let start = at.start;
            let row = self
                .rows
                .partition_point(|r| r.range.end < start || (r.range.end == start && r.range.start < start));
            self.scroll
                .scroll_to_item(row.min(self.rows.len().saturating_sub(1)), ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// Opens the find bar, its field focused.
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = match &self.find {
            Some(input) => input.clone(),
            None => {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
                self._subscriptions
                    .push(cx.subscribe_in(&input, window, |this, _, e: &InputEvent, _, cx| {
                        if matches!(e, InputEvent::Change) {
                            this.research(cx);
                        }
                    }));
                self.find = Some(input.clone());
                input
            }
        };
        self.find_open = true;
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        self.research(cx);
    }

    /// Closes the find bar; the body terms are marked again.
    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find_open {
            cx.propagate();
            return;
        }
        self.find_open = false;
        window.focus(&self.focus, cx);
        self.research(cx);
    }

    /// Marks the whole text, for copying.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.all_selected = true;
        cx.notify();
    }

    pub fn is_all_selected(&self) -> bool {
        self.all_selected
    }

    /// The selected text: all of it after ⌘A, else the drag selection.
    pub fn selected_text(&self, window: &mut Window, cx: &mut App) -> Option<String> {
        if self.all_selected {
            return Some(self.text.to_string());
        }
        let text = gpui_kit::base::TextSelection::selected_text(window, cx);
        (!text.is_empty()).then_some(text)
    }

    fn copy_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_text(window, cx) {
            Some(text) => cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text)),
            None => cx.propagate(),
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        let t = *cx.tokens();
        let gutter = self.gutter_width();
        let rows = self.rows.clone();
        let matches = self.matches.clone();
        let current = self.current.and_then(|i| matches.get(i).cloned());
        let mark = HighlightStyle {
            background_color: Some(t.code_match.hsla()),
            ..Default::default()
        };
        let mark_current = HighlightStyle {
            background_color: Some(t.code_match_current.hsla()),
            ..Default::default()
        };
        range
            .filter_map(|ix| {
                let row = rows.get(ix)?;
                let piece = &self.text[row.range.clone()];
                let syntax: Vec<(Range<usize>, HighlightStyle)> = match self.highlight {
                    None => Vec::new(),
                    Some(Highlight::Lang(lang)) => styled(highlight::highlight_line(lang, piece), &t),
                    Some(Highlight::Http { head_lines, body }) => {
                        let spans = if row.line < head_lines {
                            highlight::highlight_http_head(piece, row.line == 0)
                        } else {
                            highlight::highlight_line(body, piece)
                        };
                        styled(spans, &t)
                    }
                };
                let marks: Vec<(Range<usize>, HighlightStyle)> = search::ranges_in(&matches, row.range.clone())
                    .into_iter()
                    .map(|r| {
                        let abs = row.range.start + r.start;
                        let is_current = current.as_ref().is_some_and(|c| c.start <= abs && abs < c.end);
                        (r, if is_current { mark_current } else { mark })
                    })
                    .collect();
                let runs = highlight::layer(&syntax, &marks, |a, b| a.highlight(*b));
                let id = ElementId::Name(format!("{}-{ix}", self.id).into());
                let text =
                    SelectableText::with_handle(id, self.selection.clone(), SharedString::from(piece.to_owned()))
                        .document_order(ix as u64)
                        .highlights(runs);
                Some(
                    div()
                        .id(("code-row", ix))
                        .h(px(ROW_HEIGHT))
                        .flex()
                        .items_center()
                        .whitespace_nowrap()
                        .when(self.all_selected, |d| d.bg(t.code_selection.hsla()))
                        .child(
                            div()
                                .w(gutter)
                                .flex_none()
                                .pr(px(8.))
                                .flex()
                                .justify_end()
                                .text_color(t.code_gutter_fg.hsla())
                                .when(row.first, |d| d.child((row.line + 1).to_string())),
                        )
                        .child(div().pl(px(4.)).pr(px(8.)).child(text))
                        .into_any_element(),
                )
            })
            .collect()
    }

    fn render_bar(&self, t: &Tokens, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let count = self.matches.len();
        let label = if self.searching.is_some() {
            "Searching…".to_owned()
        } else {
            search::match_label(count, self.current)
        };
        let step = |id: &'static str, icon: gpui_kit::assets::IconName, forward: bool, tip: &'static str| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip(tip)
                .disabled(count == 0)
                .on_click(cx.listener(move |this, _, _, cx| this.step(forward, cx)))
        };
        let steppers = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                div()
                    .id(ElementId::Name(format!("{}-match-count", self.id).into()))
                    .role(Role::Status)
                    .aria_label(SharedString::from(label.clone()))
                    .mr(px(4.))
                    .text_color(t.muted_foreground.hsla())
                    .child(label),
            )
            .child(step(
                "match-prev",
                gpui_kit::assets::IconName::ChevronUp,
                false,
                "Previous match · ⇧↩",
            ))
            .child(step(
                "match-next",
                gpui_kit::assets::IconName::ChevronDown,
                true,
                "Next match · ↩",
            ));
        let bar = div()
            .flex_none()
            .h(px(28.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(t.border.hsla())
            .bg(t.pane_header.hsla())
            .text_size(px(11.));
        if self.find_open {
            let input = self.find.clone()?;
            return Some(
                bar.key_context(FIND_CONTEXT)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&input).xsmall().aria_label("Find in body")),
                    )
                    .child(steppers)
                    .child(
                        Button::new("find-close")
                            .ghost()
                            .xsmall()
                            .icon(gpui_kit::assets::IconName::X)
                            .tooltip("Close · Esc")
                            .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
                    )
                    .into_any_element(),
            );
        }
        if self.terms.is_empty() {
            return None;
        }
        Some(
            bar.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(t.muted_foreground.hsla())
                    .child(SharedString::from(format!("Body search: {}", self.terms.join(", ")))),
            )
            .child(steppers)
            .into_any_element(),
        )
    }
}

/// The colour of a syntax tone.
pub fn tone_color(tone: Tone, t: &Tokens) -> Hsla {
    match tone {
        Tone::Key => t.syntax_key,
        Tone::String => t.syntax_string,
        Tone::Number => t.syntax_number,
        Tone::Literal => t.syntax_literal,
        Tone::Punct => t.syntax_punct,
        Tone::Keyword => t.syntax_keyword,
        Tone::Comment => t.syntax_comment,
        Tone::Tag => t.syntax_tag,
        Tone::Attr => t.syntax_attr,
        Tone::Meta => t.syntax_meta,
        Tone::Type => t.syntax_type,
        Tone::Variable => t.syntax_variable,
    }
    .hsla()
}

fn styled(spans: highlight::Spans, t: &Tokens) -> Vec<(Range<usize>, HighlightStyle)> {
    spans
        .into_iter()
        .filter(|(r, _)| !r.is_empty())
        .map(|(r, tone)| {
            let style = HighlightStyle {
                color: Some(tone_color(tone, t)),
                font_weight: matches!(tone, Tone::Key).then_some(FontWeight::NORMAL),
                ..Default::default()
            };
            (r, style)
        })
        .collect()
}

impl Focusable for CodeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CodeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        // The advance as the rows are drawn: a run shaped in the window's
        // text style with the mono family, the way the rows inherit it.
        let mut style = window.text_style();
        style.font_family = mono.clone();
        let sample: SharedString = "0".repeat(64).into();
        let run = gpui_kit::TextRun {
            len: sample.len(),
            font: style.font(),
            color: t.foreground.hsla(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let char_width = window
            .text_system()
            .shape_line(sample, px(FONT_SIZE), &[run], None)
            .width
            / 64.;
        let bar = self.render_bar(&t, cx);
        let entity = cx.entity();
        let measure = canvas(
            move |bounds, _, cx| {
                entity.update(cx, |this, cx| this.set_width(bounds.size.width, char_width, cx));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let len = self.rows.len();
        let content_width = self.gutter_width() + char_width * self.widest as f32 + px(24.);
        let list = uniform_list(self.id.clone(), len, cx.processor(Self::render_rows))
            .track_scroll(&self.scroll)
            .size_full();
        let body = if self.indexing.is_some() && len == 0 {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.muted_foreground.hsla())
                .child("Indexing lines…")
                .into_any_element()
        } else if self.wrap {
            list.into_any_element()
        } else {
            div()
                .id(ElementId::Name(format!("{}-scroll-x", self.id).into()))
                .size_full()
                .overflow_x_scroll()
                .track_scroll(&self.hscroll)
                .child(div().h_full().min_w(content_width).w_full().child(list))
                .into_any_element()
        };
        let menu = self.on_menu.clone();
        let focus = self.focus.clone();
        div()
            .id(ElementId::Name(self.id.clone()))
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .role(Role::Document)
            .aria_label(self.label.clone())
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background.hsla())
            .font_family(mono)
            .text_size(px(FONT_SIZE))
            .text_color(t.foreground.hsla())
            .on_action(cx.listener(|this, _: &Find, window, cx| this.open_find(window, cx)))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.step(true, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.step(false, cx)))
            .on_action(cx.listener(|this, _: &CloseFind, window, cx| this.close_find(window, cx)))
            .on_action(cx.listener(|this, _: &SelectAllText, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &CopySelection, window, cx| this.copy_selection(window, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    if !this.focus.contains_focused(window, cx) {
                        window.focus(&this.focus, cx);
                    }
                    if this.all_selected {
                        this.all_selected = false;
                        cx.notify();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if !focus.contains_focused(window, cx) {
                        window.focus(&focus, cx);
                    }
                    let Some(menu) = menu.clone() else { return };
                    let request = MenuRequest {
                        selection: this.selected_text(window, cx),
                        text: this.text.clone(),
                        position: e.position,
                    };
                    cx.defer_in(window, move |_, window, cx| menu(request, window, cx));
                }),
            )
            .children(bar)
            .child(div().relative().flex_1().min_h_0().child(measure).child(body))
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(text: &str, cols: Option<usize>) -> Vec<(usize, &str, bool)> {
        cut_rows(text, cols)
            .rows
            .into_iter()
            .map(|r| (r.line, &text[r.range], r.first))
            .collect()
    }

    #[test]
    fn lines_become_rows_without_their_breaks() {
        assert_eq!(
            pieces("a\r\nbc\n\nd", None),
            [(0, "a", true), (1, "bc", true), (2, "", true), (3, "d", true)]
        );
        assert_eq!(pieces("", None), [(0, "", true)]);
        assert_eq!(pieces("x\n", None), [(0, "x", true), (1, "", true)]);
        assert_eq!(cut_rows("ab\nc", None).lines, 2);
    }

    #[test]
    fn wrapping_cuts_at_the_column_count_on_characters() {
        assert_eq!(
            pieces("abcdé\nfg", Some(2)),
            [(0, "ab", true), (0, "cd", false), (0, "é", false), (1, "fg", true)]
        );
        let rows = cut_rows("abcdé", Some(2));
        assert_eq!(rows.widest, 2);
    }

    #[test]
    fn long_lines_are_cut_without_wrapping_too() {
        let line = "é".repeat(NOWRAP_CHUNK);
        let rows = cut_rows(&line, None).rows;
        assert!(rows.len() > 1);
        assert!(rows.iter().all(|r| r.range.len() <= NOWRAP_CHUNK));
        assert!(rows.iter().skip(1).all(|r| !r.first));
        let joined: String = rows.iter().map(|r| &line[r.range.clone()]).collect();
        assert_eq!(joined, line);
    }
}
