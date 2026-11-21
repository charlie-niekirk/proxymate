//! JsonTree (`JsonViews.tsx`'s `Tree`): a parsed document as a virtualised,
//! collapsible tree with type-coloured values and child counts, a JSONPath
//! or JSON pointer filter, the filter's body terms marked in keys and
//! values (stepped through, revealing collapsed matches), and right-click
//! copies through the owner's menu ([`MenuRequest`]). Values come from
//! [`pm_view::body::json_doc`], so numbers show (and copy) exactly as sent.
//! G5's decoded viewers add `badges` after the values they note.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::{SelectableText, TestSupportExt as _, TextSelectionHandle};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, ElementId, Entity, FocusHandle, Focusable, HighlightStyle, Hsla, MouseButton,
    MouseDownEvent, Pixels, Point, Role, ScrollStrategy, SharedString, Subscription, Task, UniformListScrollHandle,
    Window, div, px, uniform_list,
};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::body::json_doc::JNode;
use pm_view::body::json_path::{self, Seg};
use pm_view::body::json_tree::{self as tree, Label, RowValue, TreeRow};
use pm_view::body::search::{self, Search};

const ROW_HEIGHT: f32 = 20.;
const INDENT: f32 = 14.;

/// Expands the container at `ptr` and everything below it.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = json_tree, no_json)]
pub struct ExpandAllBelow {
    pub ptr: SharedString,
}

gpui_kit::actions!(
    json_tree,
    [
        /// Collapses every container.
        CollapseAll
    ]
);

pub struct MenuRequest {
    /// The row under the pointer.
    pub row: Option<TreeRow>,
    /// That row's value as "Copy value" copies it (a string's own text).
    pub value: Option<String>,
    pub selection: Option<String>,
    pub position: Point<Pixels>,
}

pub type OnMenu = Rc<dyn Fn(MenuRequest, &mut Window, &mut App)>;

pub struct JsonTree {
    id: SharedString,
    doc: Arc<JNode>,
    filter: Entity<InputState>,
    filter_result: Result<Option<usize>, String>,
    expanded: HashSet<String>,
    rows: Rc<Vec<TreeRow>>,
    badges: HashMap<String, Vec<SharedString>>,
    terms: Vec<String>,
    search: Option<Search>,
    hits: Arc<Vec<String>>,
    current: Option<usize>,
    hunting: Option<Task<()>>,
    scroll: UniformListScrollHandle,
    selection: TextSelectionHandle,
    focus: FocusHandle,
    on_menu: Option<OnMenu>,
    _subscriptions: Vec<Subscription>,
}

impl JsonTree {
    pub fn new(id: impl Into<SharedString>, doc: Arc<JNode>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter: $.items[*].id  or  /items/0"));
        let subscription = cx.subscribe_in(&filter, window, |this, _, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::Change) {
                this.reflatten(cx);
            }
        });
        let mut view = JsonTree {
            id: id.into(),
            expanded: tree::initial_expansion(&doc),
            doc,
            filter,
            filter_result: Ok(None),
            rows: Rc::new(Vec::new()),
            badges: HashMap::new(),
            terms: Vec::new(),
            search: None,
            hits: Arc::new(Vec::new()),
            current: None,
            hunting: None,
            scroll: UniformListScrollHandle::new(),
            selection: TextSelectionHandle::new(String::new(), cx),
            focus: cx.focus_handle(),
            on_menu: None,
            _subscriptions: vec![subscription],
        };
        view.reflatten(cx);
        view
    }

    pub fn on_menu(mut self, f: impl Fn(MenuRequest, &mut Window, &mut App) + 'static) -> Self {
        self.on_menu = Some(Rc::new(f));
        self
    }

    /// Notes after values, by JSON pointer (G5: wire types, `bytes`, tags).
    pub fn set_badges(&mut self, badges: HashMap<String, Vec<SharedString>>, cx: &mut Context<Self>) {
        self.badges = badges;
        cx.notify();
    }

    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub fn filter_result(&self) -> &Result<Option<usize>, String> {
        &self.filter_result
    }

    pub fn hit_count(&self) -> usize {
        self.hits.len()
    }

    pub fn current_hit(&self) -> Option<usize> {
        self.current
    }

    /// Sets the filter (a JSONPath or pointer), as typing it would.
    pub fn set_filter(&mut self, filter: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |s, cx| s.set_value(filter.to_owned(), window, cx));
        self.reflatten(cx);
    }

    fn reflatten(&mut self, cx: &mut Context<Self>) {
        let filter = self.filter.read(cx).value().to_string();
        let (roots, result) = tree::roots(&self.doc, &filter);
        self.rows = Rc::new(tree::flatten(&roots, &self.expanded));
        self.filter_result = result;
        cx.notify();
    }

    /// Opens or closes `ptr`; `deep` opens everything below it.
    pub fn toggle(&mut self, ptr: &str, deep: bool, cx: &mut Context<Self>) {
        if deep {
            if let Some(m) = json_path::pointer(&self.doc, ptr) {
                let mut path = m.path.clone();
                tree::expanded_to(m.value, 64, &mut path, &mut self.expanded);
            }
        } else if !self.expanded.remove(ptr) {
            self.expanded.insert(ptr.to_owned());
        }
        self.reflatten(cx);
    }

    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.reflatten(cx);
    }

    /// The filter's body terms to mark.
    pub fn set_terms(&mut self, terms: Vec<String>, cx: &mut Context<Self>) {
        if self.terms == terms {
            return;
        }
        self.terms = terms;
        self.search = Search::new(&self.terms);
        self.current = None;
        self.hits = Arc::new(Vec::new());
        let Some(search) = self.search.clone() else {
            self.hunting = None;
            cx.notify();
            return;
        };
        let doc = self.doc.clone();
        let task = cx.background_spawn(async move { tree::tree_matches(&doc, &search) });
        self.hunting = Some(cx.spawn(async move |this, cx| {
            let hits = task.await;
            let _ = this.update(cx, |this, cx| {
                this.hunting = None;
                this.hits = Arc::new(hits);
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Steps to the next (or previous) hit: clears the filter, opens its
    /// ancestors and scrolls to it.
    pub fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.current = search::step(self.current, self.hits.len(), forward);
        let Some(ptr) = self.current.and_then(|i| self.hits.get(i)).cloned() else {
            return;
        };
        self.expanded.extend(tree::ancestors(&ptr));
        self.filter.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        self.reflatten(cx);
        if let Some(ix) = self.rows.iter().position(|r| r.ptr == ptr) {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
    }

    fn value_of(&self, row: &TreeRow) -> Option<String> {
        json_path::pointer(&self.doc, &row.ptr).map(|m| m.value.value_text())
    }

    fn menu(&mut self, row: Option<usize>, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.contains_focused(window, cx) {
            window.focus(&self.focus, cx);
        }
        let Some(menu) = self.on_menu.clone() else { return };
        let row = row.and_then(|i| self.rows.get(i)).cloned();
        let text = gpui_kit::base::TextSelection::selected_text(window, cx);
        let request = MenuRequest {
            value: row.as_ref().and_then(|r| self.value_of(r)),
            row,
            selection: (!text.is_empty()).then_some(text),
            position: e.position,
        };
        cx.defer_in(window, move |_, window, cx| menu(request, window, cx));
    }

    fn marks(&self, text: &str, current: bool, t: &Tokens) -> Vec<(Range<usize>, HighlightStyle)> {
        let Some(search) = &self.search else {
            return Vec::new();
        };
        let bg = if current { t.code_match_current } else { t.code_match };
        search
            .ranges_up_to(text, 64)
            .into_iter()
            .map(|r| {
                (
                    r,
                    HighlightStyle {
                        background_color: Some(bg.hsla()),
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    fn render_rows(&mut self, range: Range<usize>, _window: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let t = *cx.tokens();
        let current_ptr = self.current.and_then(|i| self.hits.get(i)).cloned();
        let rows = self.rows.clone();
        range
            .filter_map(|ix| {
                let row = rows.get(ix)?;
                let is_current = current_ptr.as_deref() == Some(row.ptr.as_str()) && !matches!(row.label, Label::Match(_));
                let label = row.label_text();
                let value = row.value_text();
                let label_color: Hsla = match row.label {
                    Label::Match(_) => t.muted_foreground,
                    Label::Index(_) => t.syntax_punct,
                    _ => t.syntax_key,
                }
                .hsla();
                let value_color: Hsla = match row.value {
                    RowValue::Object(_) | RowValue::Array(_) => t.syntax_punct,
                    RowValue::String(_) => t.syntax_string,
                    RowValue::Number(_) => t.syntax_number,
                    RowValue::Literal(_) => t.syntax_literal,
                }
                .hsla();
                let expandable = row.expandable();
                let ptr: SharedString = row.ptr.clone().into();
                let caret = if expandable {
                    let icon = if row.expanded {
                        gpui_kit::assets::IconName::ChevronDown
                    } else {
                        gpui_kit::assets::IconName::ChevronRight
                    };
                    crate::icon(icon, t.muted_foreground.hsla(), 12.).into_any_element()
                } else {
                    div().w(px(12.)).flex_none().into_any_element()
                };
                let name = (!label.is_empty()).then(|| {
                    let marks = if matches!(row.label, Label::Key(_)) {
                        self.marks(&label, is_current, &t)
                    } else {
                        Vec::new()
                    };
                    div()
                        .flex()
                        .flex_none()
                        .child(
                            div().text_color(label_color).child(
                                SelectableText::with_handle(
                                    ElementId::Name(format!("{}-k{ix}", self.id).into()),
                                    self.selection.clone(),
                                    label.clone(),
                                )
                                .document_order(ix as u64 * 2)
                                .highlights(marks),
                            ),
                        )
                        .child(div().text_color(t.syntax_punct.hsla()).child(":"))
                });
                let value_marks = if row.is_container() {
                    Vec::new()
                } else {
                    self.marks(&value, is_current, &t)
                };
                let badges = self.badges.get(&row.ptr).cloned().unwrap_or_default();
                let aria: SharedString = format!("{label} {value}").trim().to_owned().into();
                Some(
                    div()
                        .id(("json-row", ix))
                        .role(Role::TreeItem)
                        .aria_label(aria)
                        .when(expandable, |d| d.aria_expanded(row.expanded))
                        .aria_level(row.depth + 1)
                        .h(px(ROW_HEIGHT))
                        .w_full()
                        .pl(px(6. + row.depth as f32 * INDENT))
                        .pr(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .hover(|s| s.bg(t.code_row_hover.hsla()))
                        .when(is_current, |d| d.bg(t.code_active_line.hsla()))
                        .when(expandable, |d| {
                            let ptr = ptr.clone();
                            d.cursor_pointer().on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                                this.toggle(&ptr, e.modifiers().alt, cx);
                            }))
                        })
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.menu(Some(ix), e, window, cx);
                            }),
                        )
                        .child(caret)
                        .children(name)
                        .child(
                            div().min_w_0().truncate().text_color(value_color).child(
                                SelectableText::with_handle(
                                    ElementId::Name(format!("{}-v{ix}", self.id).into()),
                                    self.selection.clone(),
                                    value,
                                )
                                .document_order(ix as u64 * 2 + 1)
                                .highlights(value_marks),
                            ),
                        )
                        .children(row.size_text().map(|s| {
                            div()
                                .flex_none()
                                .ml(px(4.))
                                .text_size(px(10.5))
                                .text_color(t.muted_foreground.hsla())
                                .child(s)
                        }))
                        .children(badges.into_iter().map(|b| {
                            div()
                                .flex_none()
                                .ml(px(4.))
                                .px(px(4.))
                                .rounded(px(2.))
                                .border_1()
                                .border_color(t.border.hsla())
                                .text_size(px(10.))
                                .text_color(t.muted_foreground.hsla())
                                .child(b)
                        }))
                        .into_any_element(),
                )
            })
            .collect()
    }
}

impl Focusable for JsonTree {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for JsonTree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        let status: Option<(String, Hsla)> = match &self.filter_result {
            Err(e) => Some((e.clone(), t.s_failed.hsla())),
            Ok(Some(n)) => Some((
                format!("{n} match{}", if *n == 1 { "" } else { "es" }),
                t.muted_foreground.hsla(),
            )),
            Ok(None) => None,
        };
        let hits = self.hits.len();
        let terms = self.search.as_ref().map(|_| {
            let label = if self.hunting.is_some() {
                "Searching…".to_owned()
            } else {
                search::match_label(hits, self.current)
            };
            let step = |id: &'static str, icon: gpui_kit::assets::IconName, forward: bool, tip: &'static str| {
                Button::new(id)
                    .ghost()
                    .xsmall()
                    .icon(icon)
                    .tooltip(tip)
                    .disabled(hits == 0)
                    .on_click(cx.listener(move |this, _, window, cx| this.step(forward, window, cx)))
            };
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(2.))
                .child(
                    div()
                        .id("json-tree-match-count")
                        .role(Role::Status)
                        .aria_label(SharedString::from(label.clone()))
                        .mr(px(4.))
                        .text_color(t.muted_foreground.hsla())
                        .child(label),
                )
                .child(step("json-match-prev", gpui_kit::assets::IconName::ChevronUp, false, "Previous match"))
                .child(step("json-match-next", gpui_kit::assets::IconName::ChevronDown, true, "Next match"))
        });
        let len = self.rows.len();
        div()
            .id(ElementId::Name(self.id.clone()))
            .track_focus(&self.focus)
            .role(Role::Tree)
            .aria_label("JSON tree")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background.hsla())
            .text_color(t.foreground.hsla())
            .on_action(cx.listener(|this, a: &ExpandAllBelow, _, cx| this.toggle(&a.ptr, true, cx)))
            .on_action(cx.listener(|this, _: &CollapseAll, _, cx| this.collapse_all(cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &MouseDownEvent, window, cx| this.menu(None, e, window, cx)),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(28.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .border_b_1()
                    .border_color(t.border.hsla())
                    .text_size(px(11.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(mono.clone())
                            .child(Input::new(&self.filter).xsmall().aria_label("Filter the JSON tree")),
                    )
                    .children(status.map(|(s, c)| div().flex_none().text_color(c).child(s)))
                    .children(terms),
            )
            .child(
                uniform_list(ElementId::Name(format!("{}-rows", self.id).into()), len, cx.processor(Self::render_rows))
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .font_family(mono)
                    .text_size(px(11.5)),
            )
            .test_support()
    }
}

/// A row's path as segments (for tests and menus).
pub fn path_of(row: &TreeRow) -> &[Seg] {
    &row.path
}
