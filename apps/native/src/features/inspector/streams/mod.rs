//! The stream panels under WebSocket and SSE flows (G6; `WebSocketPanel.tsx`,
//! `SsePanel.tsx` and `StreamShell.tsx`): a toolbar (title, live status,
//! count, filters, search, Close) over a virtualised list of messages or
//! events beside the selected one's detail. WebSocket flows add the
//! composer under the list while a live connection is open.
//!
//! Everything is read through the bridge's caches (`ws_head`, `ws_rows`,
//! `ws_payload`, `sse_head`, `sse_rows`): a list's head (counts, open,
//! close) and its pages of [`LIST_PAGE`] rows, fetched for the rows on
//! screen. The flow table's deltas make a flow's heads and partial pages
//! stale (`keys::streams_changed`), so a live list grows a tail page at a
//! time with no polling. Writes (send, Resend, close) go through
//! `bridge::mutations`.
//!
//! One panel entity per flow: selecting another flow starts afresh.

mod payload;
mod sse;
mod websocket;

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, DragMoveEvent, Entity, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, Pixels,
    Point, Role, ScrollStrategy, SharedString, Subscription, UniformListScrollHandle, Window, div, px, uniform_list,
};
use pm_model::{LIVE_STORE, SseEvent, SsePage, WsDirection, WsMessage, WsOpcode, WsPage};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::inspector::StreamKind;
use pm_view::stream::{self as view, ComposeMode, LIST_PAGE, MenuItem, PayloadFormat, page_of};

use crate::bridge::keys::{self, Caches, FlowKey, PageKey, SseListKey, WsListKey};
use crate::bridge::mutations;
use crate::bridge::resource::Load;
use crate::features::clipboard::copy_later;
use crate::features::context_menu::Menu;
use crate::features::inspector::actions::FlowRef;
use crate::workspace::Workspace;

pub use payload::Viewer;

/// The list's rows are this tall.
pub const ROW_HEIGHT: f32 = 22.;
/// The toolbar's height.
pub const TOOLBAR_HEIGHT: f32 = 32.;
/// The list takes at least this much of the width; the detail at least
/// [`DETAIL_MIN`].
const LIST_MIN: f32 = 240.;
const DETAIL_MIN: f32 = 200.;
const HANDLE: f32 = 5.;

/// Copies a WebSocket payload (or what arrived, for an edited one) in a
/// format, fetched whole: a row only has a preview.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct CopyWsPayload {
    pub flow: FlowRef,
    pub index: u32,
    pub format: PayloadFormat,
}

/// Resend: a recorded message again, in its direction.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ResendWsMessage {
    pub flow: FlowRef,
    pub index: u32,
    pub direction: WsDirection,
    pub opcode: WsOpcode,
}

/// Close: a normal close frame to each side of a live connection.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct CloseWebSocket {
    pub flow: FlowRef,
}

pub fn register(cx: &mut App) {
    cx.on_action(|a: &CopyWsPayload, cx| {
        let Some(engine) = Workspace::global(cx).and_then(|w| w.read(cx).engine().cloned()) else {
            return;
        };
        let (flow, index, format) = (a.flow, a.index, a.format);
        copy_later(
            move || {
                let bytes = engine
                    .websocket_payload_in(flow.store, flow.id, index)
                    .map_err(|e| e.to_string())?;
                view::payload_as(&bytes, format).map(Some).map_err(str::to_owned)
            },
            format.what(),
            cx,
        );
    });
    cx.on_action(|a: &ResendWsMessage, cx| {
        let flow = FlowKey::new(a.flow.store, a.flow.id);
        mutations::ws_resend(flow, a.index, a.direction, a.opcode, cx);
    });
    cx.on_action(|a: &CloseWebSocket, cx| mutations::ws_close(FlowKey::new(a.flow.store, a.flow.id), cx));
}

/// The message or event shown in the detail.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Ws(WsMessage),
    Sse(SseEvent),
}

impl Row {
    fn index(&self) -> u32 {
        match self {
            Row::Ws(m) => m.index,
            Row::Sse(e) => e.index,
        }
    }
}

/// The composer's state (WebSocket flows only).
pub struct Composer {
    pub input: Entity<TextareaState>,
    pub direction: WsDirection,
    pub mode: ComposeMode,
    pub error: Option<SharedString>,
}

/// Drags of the list | detail handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ListSplit;

struct NoGhost;

impl Render for NoGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub struct StreamPanel {
    flow: FlowKey,
    kind: StreamKind,
    caches: Caches,
    focus: FocusHandle,
    search: Entity<InputState>,
    text: String,
    direction: Option<WsDirection>,
    /// The clicked row and its position in the list.
    selected: Option<(usize, Row)>,
    /// A position to select once its row loads (the dev remote, keys).
    pending: Option<usize>,
    scroll: UniformListScrollHandle,
    /// Follows the end as rows arrive, until scrolled up.
    following: bool,
    check_follow: bool,
    last_len: usize,
    /// The list's share of the width.
    split: f32,
    /// An edited message shows what arrived.
    original: bool,
    /// The viewer picked for the shown payload.
    viewer: Option<Viewer>,
    doc: Option<payload::PayloadDoc>,
    /// The SSE event whose data `doc` shows, as bytes.
    sse_bytes: Option<(u32, Arc<Vec<u8>>)>,
    composer: Option<Composer>,
    ws_head: Option<Arc<WsPage>>,
    sse_head: Option<Arc<SsePage>>,
    head_error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl gpui_kit::Focusable for StreamPanel {
    /// The list's: ↑/↓, Home and End move the selection.
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl StreamPanel {
    pub fn new(flow: FlowKey, kind: StreamKind, window: &mut Window, cx: &mut Context<Self>) -> StreamPanel {
        let caches = keys::caches(cx);
        let placeholder = match kind {
            StreamKind::WebSocket => "Search payloads",
            StreamKind::Sse => "Search events",
        };
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let mut subscriptions = vec![
            cx.subscribe(&search, |this: &mut Self, input, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.text = input.read(cx).value().to_string();
                    this.filters_changed(cx);
                }
            }),
            cx.observe(&caches.ws_head, |_, _, cx| cx.notify()),
            cx.observe(&caches.ws_rows, |_, _, cx| cx.notify()),
            cx.observe(&caches.ws_payload, |_, _, cx| cx.notify()),
            cx.observe(&caches.sse_head, |_, _, cx| cx.notify()),
            cx.observe(&caches.sse_rows, |_, _, cx| cx.notify()),
        ];
        let composer = (kind == StreamKind::WebSocket).then(|| {
            let input = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(2)
                    .placeholder(placeholder_for(ComposeMode::Text))
            });
            // ⌘↩ sends (the textarea's secondary Enter never inserts a newline).
            subscriptions.push(cx.subscribe(&input, |this: &mut Self, _, e: &InputEvent, cx| {
                if let InputEvent::PressEnter { secondary: true, .. } = e {
                    this.send(cx);
                }
            }));
            Composer {
                input,
                direction: WsDirection::Sent,
                mode: ComposeMode::Text,
                error: None,
            }
        });
        StreamPanel {
            flow,
            kind,
            caches,
            focus: cx.focus_handle(),
            search,
            text: String::new(),
            direction: None,
            selected: None,
            pending: None,
            scroll: UniformListScrollHandle::new(),
            following: true,
            check_follow: false,
            last_len: 0,
            split: 0.55,
            original: false,
            viewer: None,
            doc: None,
            sse_bytes: None,
            composer,
            ws_head: None,
            sse_head: None,
            head_error: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn flow(&self) -> FlowKey {
        self.flow
    }

    pub fn kind(&self) -> StreamKind {
        self.kind
    }

    fn flow_ref(&self) -> FlowRef {
        FlowRef {
            store: self.flow.store,
            id: self.flow.id,
        }
    }

    fn ws_key(&self) -> WsListKey {
        WsListKey {
            flow: self.flow,
            direction: self.direction,
            text: self.text.clone(),
        }
    }

    fn sse_key(&self) -> SseListKey {
        SseListKey {
            flow: self.flow,
            text: self.text.clone(),
        }
    }

    // ---- Reading the caches ----

    /// The list's head, fetching it when stale; the last one stands in
    /// while a new filter's loads.
    fn refresh_head(&mut self, cx: &mut Context<Self>) {
        self.head_error = None;
        match self.kind {
            StreamKind::WebSocket => {
                let key = self.ws_key();
                match self.caches.ws_head.update(cx, |c, cx| c.get(&key, cx)) {
                    Load::Ready(head) => self.ws_head = Some(head),
                    Load::Failed(err) => self.head_error = Some(err),
                    Load::Loading => {}
                }
            }
            StreamKind::Sse => {
                let key = self.sse_key();
                match self.caches.sse_head.update(cx, |c, cx| c.get(&key, cx)) {
                    Load::Ready(head) => self.sse_head = Some(head),
                    Load::Failed(err) => self.head_error = Some(err),
                    Load::Loading => {}
                }
            }
        }
    }

    /// Rows matching the filter, as the head counts them.
    pub fn len(&self) -> usize {
        match self.kind {
            StreamKind::WebSocket => self.ws_head.as_ref().map_or(0, |h| h.len as usize),
            StreamKind::Sse => self.sse_head.as_ref().map_or(0, |h| h.len as usize),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every row of the flow, filtered or not.
    pub fn total(&self) -> usize {
        match self.kind {
            StreamKind::WebSocket => self.ws_head.as_ref().map_or(0, |h| h.total as usize),
            StreamKind::Sse => self.sse_head.as_ref().map_or(0, |h| h.total as usize),
        }
    }

    /// True while the connection or response is open.
    pub fn is_open(&self) -> bool {
        match self.kind {
            StreamKind::WebSocket => self.ws_head.as_ref().is_some_and(|h| h.open),
            StreamKind::Sse => self.sse_head.as_ref().is_some_and(|h| h.open),
        }
    }

    /// Only an open connection of the live capture takes messages.
    pub fn sendable(&self) -> bool {
        self.kind == StreamKind::WebSocket && self.flow.store == LIVE_STORE && self.is_open()
    }

    fn dropped(&self) -> u32 {
        self.ws_head.as_ref().map_or(0, |h| h.dropped)
    }

    /// Row `index`, fetching its page when missing or stale.
    fn row(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Row> {
        let slot = index % LIST_PAGE as usize;
        match self.kind {
            StreamKind::WebSocket => {
                let key = PageKey {
                    list: self.ws_key(),
                    page: page_of(index),
                    dropped: self.dropped(),
                };
                let page = self.caches.ws_rows.update(cx, |c, cx| c.get(&key, cx));
                page.ready()?.get(slot).cloned().map(Row::Ws)
            }
            StreamKind::Sse => {
                let key = PageKey {
                    list: self.sse_key(),
                    page: page_of(index),
                    dropped: 0,
                };
                let page = self.caches.sse_rows.update(cx, |c, cx| c.get(&key, cx));
                page.ready()?.get(slot).cloned().map(Row::Sse)
            }
        }
    }

    /// Row `index` if it is loaded (menus, tests).
    pub fn peek_row(&self, index: usize, cx: &App) -> Option<Row> {
        let slot = index % LIST_PAGE as usize;
        match self.kind {
            StreamKind::WebSocket => {
                let key = PageKey {
                    list: self.ws_key(),
                    page: page_of(index),
                    dropped: self.dropped(),
                };
                self.caches.ws_rows.read(cx).peek(&key)?.get(slot).cloned().map(Row::Ws)
            }
            StreamKind::Sse => {
                let key = PageKey {
                    list: self.sse_key(),
                    page: page_of(index),
                    dropped: 0,
                };
                self.caches
                    .sse_rows
                    .read(cx)
                    .peek(&key)?
                    .get(slot)
                    .cloned()
                    .map(Row::Sse)
            }
        }
    }

    /// The row in the detail: the clicked one or, with Auto Select on, the
    /// newest until one is clicked.
    pub fn shown(&self, cx: &App) -> Option<Row> {
        if let Some((_, row)) = &self.selected {
            return Some(row.clone());
        }
        let len = self.len();
        (auto_select(cx) && len > 0)
            .then(|| self.peek_row(len - 1, cx))
            .flatten()
    }

    // ---- Changes ----

    fn filters_changed(&mut self, cx: &mut Context<Self>) {
        // Positions belong to the old filter; the row itself stays shown.
        if let Some((pos, _)) = &mut self.selected {
            *pos = usize::MAX;
        }
        self.pending = None;
        self.following = true;
        cx.notify();
    }

    pub fn set_direction(&mut self, direction: Option<WsDirection>, cx: &mut Context<Self>) {
        if self.direction != direction {
            self.direction = direction;
            self.filters_changed(cx);
        }
    }

    pub fn direction(&self) -> Option<WsDirection> {
        self.direction
    }

    /// Types into the search box (the dev remote, tests).
    pub fn set_search(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx));
        self.text = text.to_owned();
        self.filters_changed(cx);
    }

    pub fn search_text(&self) -> &str {
        &self.text
    }

    /// Selects the row at `index` of the list, now if it is loaded, else
    /// once it is.
    pub fn select_index(&mut self, index: usize, cx: &mut Context<Self>) {
        // A row picked away from the end stays in view as rows arrive.
        self.following = index + 1 >= self.len();
        self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        match self.peek_row(index, cx) {
            Some(row) => self.select(index, row, cx),
            None => {
                self.pending = Some(index);
                cx.notify();
            }
        }
    }

    fn select(&mut self, index: usize, row: Row, cx: &mut Context<Self>) {
        if self.selected.as_ref().map(|(_, r)| r.index()) != Some(row.index()) {
            self.original = false;
            self.viewer = None;
        }
        self.selected = Some((index, row));
        self.pending = None;
        cx.notify();
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected.as_ref().map(|(i, _)| *i).filter(|i| *i != usize::MAX)
    }

    pub fn set_original(&mut self, original: bool, cx: &mut Context<Self>) {
        self.original = original;
        cx.notify();
    }

    pub fn original(&self) -> bool {
        self.original
    }

    pub fn set_viewer(&mut self, viewer: Viewer, cx: &mut Context<Self>) {
        self.viewer = Some(viewer);
        cx.notify();
    }

    /// The viewer the shown payload is in, once it has loaded.
    pub fn viewer(&self) -> Option<Viewer> {
        self.doc.as_ref().map(|d| self.viewer.unwrap_or(d.viewer))
    }

    pub fn composer(&self) -> Option<&Composer> {
        self.composer.as_ref()
    }

    /// Fills the composer (the dev remote, tests).
    pub fn compose(
        &mut self,
        text: &str,
        direction: Option<WsDirection>,
        mode: Option<ComposeMode>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(c) = &mut self.composer else { return };
        if let Some(direction) = direction {
            c.direction = direction;
        }
        if let Some(mode) = mode {
            c.mode = mode;
            c.input
                .update(cx, |s, cx| s.set_placeholder(placeholder_for(mode), window, cx));
        }
        c.input.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx));
        cx.notify();
    }

    /// Sends what the composer holds (Send, ⌘↩); a message that can't be
    /// read or sent shows its error inline.
    pub fn send(&mut self, cx: &mut Context<Self>) {
        let flow = self.flow;
        let Some(c) = &mut self.composer else { return };
        let input = c.input.read(cx).value().to_string();
        match view::ws_send_of(c.direction, c.mode, &input) {
            Err(err) => c.error = Some(err.into()),
            Ok(message) => {
                c.error = None;
                let weak = cx.weak_entity();
                mutations::ws_send(
                    flow,
                    message,
                    move |answer, cx| {
                        if let Err(err) = answer {
                            let _ = weak.update(cx, |this, cx| {
                                if let Some(c) = &mut this.composer {
                                    c.error = Some(err.into());
                                }
                                cx.notify();
                            });
                        }
                    },
                    cx,
                );
            }
        }
        cx.notify();
    }

    // ---- Menus ----

    /// A context menu by name (tests, the dev remote): `stream:<row>` (a
    /// row of the list) or `payload` (the shown payload or data).
    pub fn menu_for(&self, name: &str, selected: Option<String>, cx: &App) -> Option<Menu> {
        let (kind, arg) = name.split_once(':').map_or((name, None), |(k, a)| (k, Some(a)));
        match kind {
            "stream" => {
                let row = self.peek_row(arg?.parse().ok()?, cx)?;
                Some(self.row_menu(&row))
            }
            "payload" => {
                let bytes = self.doc.as_ref()?.bytes().clone();
                let what = match self.kind {
                    StreamKind::WebSocket => "payload",
                    StreamKind::Sse => "data",
                };
                Some(payload_menu(&bytes, what, selected))
            }
            _ => None,
        }
    }

    fn row_menu(&self, row: &Row) -> Menu {
        let flow = self.flow_ref();
        let items = match row {
            Row::Ws(m) => view::ws_menu(m, self.sendable()),
            Row::Sse(e) => view::sse_menu(e),
        };
        let mut menu = Menu::new();
        for item in items {
            menu = match item {
                MenuItem::Copy { label, text, what } => menu.copy(label, text, what),
                MenuItem::CopyPayload { label, format, enabled } => {
                    let index = row.index();
                    let action = CopyWsPayload { flow, index, format };
                    if enabled {
                        menu.item(label, action)
                    } else {
                        menu.disabled(label, action)
                    }
                }
                MenuItem::Separator => menu.separator(),
                MenuItem::Resend(direction) => match row {
                    Row::Ws(m) => menu.item(
                        view::resend_label(direction),
                        ResendWsMessage {
                            flow,
                            index: m.index,
                            direction,
                            opcode: m.opcode,
                        },
                    ),
                    Row::Sse(_) => menu,
                },
            };
        }
        menu
    }

    fn show_row_menu(&mut self, index: usize, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.peek_row(index, cx) {
            self.row_menu(&row).show(position, window, cx);
        }
    }

    // ---- Rendering ----

    fn on_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let len = self.len();
        if len == 0 || e.keystroke.modifiers.modified() {
            return;
        }
        let current = self.selected_index();
        let next = match e.keystroke.key.as_str() {
            "down" => current.map_or(0, |i| (i + 1).min(len - 1)),
            "up" => current.map_or(len - 1, |i| i.saturating_sub(1)),
            "home" => 0,
            "end" => len - 1,
            _ => return,
        };
        cx.stop_propagation();
        self.select_index(next, cx);
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::Stateful<gpui_kit::Div>> {
        let t = *cx.tokens();
        let shown = self.shown(cx).map(|r| r.index());
        range
            .map(|index| {
                let row = self.row(index, cx);
                let selected = row.as_ref().is_some_and(|r| Some(r.index()) == shown);
                let base = div()
                    .id(("stream-row", index))
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .map(|d| {
                        if selected {
                            d.bg(t.selection_bg.hsla()).text_color(t.selection_fg.hsla())
                        } else {
                            d.when(index % 2 == 1, |d| d.bg(t.pane_row_alt.hsla()))
                                .hover(|s| s.bg(t.pane_row_hover.hsla()))
                        }
                    });
                let Some(row) = row else {
                    return base.role(Role::ListBoxOption).aria_label("Loading");
                };
                let base = base
                    .role(Role::ListBoxOption)
                    .aria_selected(selected)
                    .on_click(cx.listener({
                        let row = row.clone();
                        move |this, _, window, cx| {
                            window.focus(&this.focus, cx);
                            this.select(index, row.clone(), cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            this.show_row_menu(index, e.position, window, cx)
                        }),
                    );
                match &row {
                    Row::Ws(m) => websocket::row(base, m, selected, &t),
                    Row::Sse(e) => sse::row(base, e, selected, &t),
                }
            })
            .collect()
    }

    fn toolbar(&self, status: String, count: String, controls: Vec<AnyElement>, t: &Tokens) -> impl IntoElement {
        let live = self.is_open();
        let title = match self.kind {
            StreamKind::WebSocket => "WebSocket",
            StreamKind::Sse => "Events",
        };
        div()
            .id("stream-toolbar")
            .flex_none()
            .h(px(TOOLBAR_HEIGHT))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(t.border.hsla())
            .bg(t.pane_toolbar.hsla())
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .id("stream-status")
                    .role(Role::Status)
                    .aria_label(SharedString::from(format!("{status}, {count}")))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(t.muted_foreground.hsla())
                    .child(div().flex_none().size(px(6.)).rounded_full().bg(if live {
                        t.s_active.hsla()
                    } else {
                        t.s_blocked.hsla()
                    }))
                    .child(div().min_w_0().truncate().child(status))
                    .child(div().flex_none().child(format!("· {count}"))),
            )
            .children(controls)
            .test_support()
    }

    fn search_box(&self, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let clear = (!self.text.is_empty()).then(|| {
            div()
                .id("stream-search-clear")
                .role(Role::Button)
                .aria_label("Clear search")
                .child(pm_widgets::icon(IconName::X, t.muted_foreground.hsla(), 12.))
                .on_click(cx.listener(|this, _, window, cx| this.set_search("", window, cx)))
                .test_support()
        });
        let label = match self.kind {
            StreamKind::WebSocket => "Search payloads",
            StreamKind::Sse => "Search events",
        };
        div()
            .flex_none()
            .w(px(192.))
            .child(
                Input::new(&self.search)
                    .id("stream-search")
                    .aria_label(label)
                    .h(px(24.))
                    .prefix(pm_widgets::icon(IconName::Search, t.muted_foreground.hsla(), 12.))
                    .suffix(div().flex().items_center().children(clear)),
            )
            .into_any_element()
    }

    fn list(&mut self, header: AnyElement, empty: &'static str, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let len = self.len();
        let label = match self.kind {
            StreamKind::WebSocket => "Messages",
            StreamKind::Sse => "Events",
        };
        let body = if len == 0 {
            let text: SharedString = match &self.head_error {
                Some(err) => format!("Couldn't load the list: {err}").into(),
                None => empty.into(),
            };
            div()
                .id("stream-empty")
                .flex_1()
                .p(px(16.))
                .flex()
                .justify_center()
                .text_size(px(12.))
                .text_color(t.muted_foreground.hsla())
                .child(text)
                .test_support()
                .into_any_element()
        } else {
            uniform_list("stream-rows", len, cx.processor(Self::render_rows))
                .track_scroll(&self.scroll)
                .flex_1()
                .w_full()
                .into_any_element()
        };
        div()
            .id("stream-list")
            .key_context("StreamList")
            .track_focus(&self.focus)
            .role(Role::ListBox)
            .aria_label(label)
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll(cx.listener(|this, _, _, cx| {
                this.check_follow = true;
                cx.notify();
            }))
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.pane_table.hsla())
            .font_family("Menlo")
            .child(header)
            .child(body)
            .test_support()
            .into_any_element()
    }

    /// Follows the end: after a scroll, whether the list is at its end
    /// decides; then new rows scroll into view while following.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let len = self.len();
        if self.check_follow {
            self.check_follow = false;
            let handle = self.scroll.0.borrow();
            let (offset, max) = (handle.base_handle.offset().y, handle.base_handle.max_offset().y);
            self.following = f32::from(max + offset) < ROW_HEIGHT * 2.;
        }
        if len != self.last_len {
            self.last_len = len;
            if self.following && len > 0 {
                self.scroll.scroll_to_item(len - 1, ScrollStrategy::Bottom);
            }
        }
        if let Some(index) = self.pending.filter(|i| *i < len)
            && let Some(row) = self.row(index, cx)
        {
            self.select(index, row, cx);
        }
    }

    /// List | detail, side by side; the handle drags, a double-click resets.
    fn split_view(&self, list: AnyElement, detail: AnyElement, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let share = self.split;
        div()
            .id("stream-split-view")
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_row()
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<ListSplit>, _, cx| {
                let left = f32::from(e.bounds.origin.x);
                let width = f32::from(e.bounds.size.width).max(1.);
                let max = (width - DETAIL_MIN - HANDLE).max(LIST_MIN);
                let x = (f32::from(e.event.position.x) - left).clamp(LIST_MIN, max);
                this.split = x / width;
                cx.notify();
            }))
            .child(
                div()
                    .h_full()
                    .flex_none()
                    .w(gpui_kit::relative(share))
                    .min_w(px(LIST_MIN))
                    .child(list),
            )
            .child(
                div()
                    .id("stream-split")
                    .role(Role::Splitter)
                    .aria_label("Resize the list and the detail")
                    .aria_orientation(gpui_kit::Orientation::Vertical)
                    .aria_numeric_value((share * 100.) as f64)
                    .flex_none()
                    .w(px(HANDLE))
                    .h_full()
                    .cursor_col_resize()
                    .border_l_1()
                    .border_color(t.border.hsla())
                    .on_drag(ListSplit, |_, _, _, cx| cx.new(|_| NoGhost))
                    .on_click(cx.listener(|this, e: &gpui_kit::ClickEvent, _, cx| {
                        if e.click_count() >= 2 {
                            this.split = 0.55;
                            cx.notify();
                        }
                    })),
            )
            .child(
                div()
                    .id("stream-detail")
                    .h_full()
                    .flex_1()
                    .min_w(px(DETAIL_MIN))
                    .bg(t.background.hsla())
                    .child(detail),
            )
            .into_any_element()
    }

    /// The payload view over `bytes`, its doc rebuilt when they change.
    fn payload_view(
        &mut self,
        id: &'static str,
        bytes: Arc<Vec<u8>>,
        binary: bool,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.doc.as_ref().is_some_and(|d| d.shows(&bytes)) {
            self.doc = Some(payload::PayloadDoc::new(bytes, binary, cx));
        }
        let doc = self.doc.as_mut().expect("just set");
        if let Some(v) = self.viewer {
            doc.viewer = v;
        }
        let weak = cx.weak_entity();
        let what = match self.kind {
            StreamKind::WebSocket => "payload",
            StreamKind::Sse => "data",
        };
        let menu_bytes = doc.bytes().clone();
        div()
            .id("stream-payload")
            .size_full()
            .min_h_0()
            .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                let selected = crate::features::context_menu::selected_text(window, cx);
                payload_menu(&menu_bytes, what, selected).show(e.position, window, cx);
            })
            .child(doc.element(id, t, move |v, _, cx| {
                let _ = weak.update(cx, |this, cx| this.set_viewer(v, cx));
            }))
            .into_any_element()
    }
}

/// The payload's menu: the selection, then the whole payload as text,
/// Base64 and hex (the body viewers' fallback menu).
fn payload_menu(bytes: &[u8], what: &'static str, selected: Option<String>) -> Menu {
    let mut menu = Menu::new().selection(selected);
    if let Ok(text) = view::payload_as(bytes, PayloadFormat::Text) {
        menu = menu.copy(format!("Copy {what}"), text, what);
    }
    menu.copy(
        format!("Copy {what} as Base64"),
        view::payload_as(bytes, PayloadFormat::Base64).unwrap_or_default(),
        format!("{what} as Base64"),
    )
    .copy(
        format!("Copy {what} as hex"),
        view::payload_as(bytes, PayloadFormat::Hex).unwrap_or_default(),
        format!("{what} as hex"),
    )
}

fn placeholder_for(mode: ComposeMode) -> &'static str {
    match mode {
        ComposeMode::Text => "Message text, e.g. {\"type\":\"ping\"} · ⌘↩ sends",
        ComposeMode::Hex => "Bytes in hex, e.g. 0a ff 3c · ⌘↩ sends",
    }
}

/// Auto Select is on in the flow table.
fn auto_select(cx: &App) -> bool {
    crate::features::flow_table::first(cx).is_some_and(|t| t.read(cx).grid().read(cx).auto_select())
}

impl Render for StreamPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        self.refresh_head(cx);
        self.follow(cx);
        let body = match self.kind {
            StreamKind::WebSocket => self.render_websocket(&t, window, cx),
            StreamKind::Sse => self.render_sse(&t, window, cx),
        };
        div()
            .id("stream-panel")
            .role(Role::Region)
            .aria_label(match self.kind {
                StreamKind::WebSocket => "WebSocket messages",
                StreamKind::Sse => "Server-sent events",
            })
            .flex_1()
            .min_h(px(120.))
            .w_full()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.border.hsla())
            .bg(t.background.hsla())
            .child(body)
            .test_support()
    }
}
