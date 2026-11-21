//! The inspector (G3; `DetailPane.tsx` and `components/Inspector/`): the
//! focused flow's URL bar, notes, OpenAPI operation, rule badges and the
//! pinned banner over its Request and Response side by side, each with its
//! tabs (Request: Header, Query, Body, Raw; Response: Header, Body, Raw,
//! TLS, Timing). The chosen tabs stay across flows.
//!
//! The flow comes from the bridge's `inspected` cache (`bridge::keys`),
//! which the flow table's deltas invalidate; the OpenAPI line and the Raw
//! tab's bodies have caches of their own. Nothing here calls the engine on
//! the foreground thread.
//!
//! WebSocket and SSE flows get [`streams::StreamPanel`] under the
//! messages (40/60, G6).
//!
//! Extension points for later phases: the Body tabs ([`panes`]'s
//! `body_tab`, G4), and the stubs in `features::chrome` for the
//! dialogs the buttons open (Rewrite…: G9, Comment…: G11, Open rule: G7,
//! Certificate wizard…: G13).

pub mod actions;
pub mod menus;
mod panes;
mod parts;
mod raw;
pub mod streams;

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, DragMoveEvent, Entity, FocusHandle, Focusable, KeyBinding, MouseButton, Role, SharedString,
    Subscription, Window, div, px,
};
use pm_model::{BodyPart, FlowId, StoreId};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::inspector::StreamKind;

use crate::bridge::keys::{self, Caches, FlowKey, Inspected};
use crate::bridge::resource::Load;
use crate::features::commands;
use crate::features::context_menu::Menu;
use crate::features::flow_table::FlowTable;
use crate::workspace::Workspace;
use actions::{FocusInspector, FocusTable};

/// The inspector's key context: Escape returns to the flow table.
pub const KEY_CONTEXT: &str = "Inspector";
/// Each message pane is at least this wide.
const PANE_MIN_WIDTH: f32 = 240.;
/// A split handle's thickness.
const HANDLE: f32 = 5.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestTab {
    Header,
    Query,
    Body,
    Raw,
}

impl RequestTab {
    pub const ALL: [RequestTab; 4] = [RequestTab::Header, RequestTab::Query, RequestTab::Body, RequestTab::Raw];

    pub fn label(self) -> &'static str {
        match self {
            RequestTab::Header => "Header",
            RequestTab::Query => "Query",
            RequestTab::Body => "Body",
            RequestTab::Raw => "Raw",
        }
    }

    pub fn parse(s: &str) -> Option<RequestTab> {
        RequestTab::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseTab {
    Header,
    Body,
    Raw,
    Tls,
    Timing,
}

impl ResponseTab {
    pub const ALL: [ResponseTab; 5] = [
        ResponseTab::Header,
        ResponseTab::Body,
        ResponseTab::Raw,
        ResponseTab::Tls,
        ResponseTab::Timing,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ResponseTab::Header => "Header",
            ResponseTab::Body => "Body",
            ResponseTab::Raw => "Raw",
            ResponseTab::Tls => "TLS",
            ResponseTab::Timing => "Timing",
        }
    }

    pub fn parse(s: &str) -> Option<ResponseTab> {
        ResponseTab::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(s))
    }
}

pub fn register(cx: &mut App) {
    actions::register(cx);
    streams::register(cx);
    cx.bind_keys([KeyBinding::new("escape", FocusTable, Some(KEY_CONTEXT))]);
    cx.on_action(|_: &FocusTable, cx| in_window(cx, crate::shell::focus_table));
    cx.on_action(|_: &FocusInspector, cx| in_window(cx, crate::shell::focus_inspector));
    commands::add("table", commands::always, crate::shell::focus_table, cx);
    commands::add(
        "inspector",
        |cx| focused(cx).is_some(),
        crate::shell::focus_inspector,
        cx,
    );
    commands::add(
        "pin",
        |cx| focused(cx).is_some() && Workspace::global(cx).is_some_and(|w| w.read(cx).is_live()),
        |_, cx| {
            let Some((store, id)) = focused(cx) else { return };
            let pinned = focused_row(cx).is_some_and(|r| r.20);
            let flow = actions::FlowRef { store, id };
            cx.dispatch_action(&actions::PinFlow { flow, pinned: !pinned });
        },
        cx,
    );
    commands::add(
        "copy-url",
        |cx| focused(cx).is_some(),
        |_, cx| {
            let Some(table) = crate::features::flow_table::first(cx) else {
                return;
            };
            let (store, grid) = (table.read(cx).store(), table.read(cx).grid().read(cx));
            let (count, selection, id) = (grid.selection_count(), grid.flow_selection(), grid.selected().focused);
            if count > 1 {
                actions::copy_urls(store, selection, cx);
            } else if let Some(id) = id {
                actions::copy_url(actions::FlowRef { store, id }, cx);
            }
        },
        cx,
    );
    commands::add(
        "copy-curl",
        |cx| focused(cx).is_some(),
        |_, cx| {
            if let Some((store, id)) = focused(cx) {
                actions::copy_curl(actions::FlowRef { store, id }, cx);
            }
        },
        cx,
    );
    commands::add(
        "delete",
        |cx| crate::features::flow_table::first(cx).is_some_and(|t| t.read(cx).grid().read(cx).selection_count() > 0),
        |_, cx| {
            if let Some(table) = crate::features::flow_table::first(cx) {
                table.update(cx, |t, cx| t.delete_selection(cx));
            }
        },
        cx,
    );
}

/// Runs `f` on the active window after the current dispatch.
fn in_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    if let Some(handle) = cx.active_window().or_else(|| cx.windows().first().copied()) {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| f(window, cx));
        });
    }
}

/// The flow the table focuses, in the store it shows.
pub fn focused(cx: &App) -> Option<(StoreId, FlowId)> {
    let table = crate::features::flow_table::first(cx)?;
    let t = table.read(cx);
    Some((t.store(), t.grid().read(cx).selected().focused?))
}

/// The focused flow's table row, if it is loaded.
fn focused_row(cx: &App) -> Option<pm_model::FlowRow> {
    let table = crate::features::flow_table::first(cx)?;
    let grid = table.read(cx).grid().read(cx);
    let id = grid.selected().focused?;
    (0..grid.len())
        .filter_map(|i| grid.row(i))
        .find(|r| r.id() == id)
        .map(|r| r.row.clone())
}

/// Drags of the Request | Response handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PaneSplit;

struct NoGhost;

impl Render for NoGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub struct Inspector {
    table: Entity<FlowTable>,
    caches: Caches,
    focus: FocusHandle,
    request_tab: RequestTab,
    response_tab: ResponseTab,
    /// The Request pane's share of the width.
    split: f32,
    raw: raw::RawDocs,
    request_body: Entity<crate::features::body::BodyPane>,
    response_body: Entity<crate::features::body::BodyPane>,
    /// The WebSocket or SSE panel of the flow shown, if it has one.
    streams: Option<Entity<streams::StreamPanel>>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Inspector {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Inspector {
    pub fn new(workspace: Entity<Workspace>, table: Entity<FlowTable>, cx: &mut Context<Self>) -> Self {
        let caches = keys::caches(cx);
        let subscriptions = vec![
            cx.observe(&table, |_, _, cx| cx.notify()),
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            cx.observe(&caches.inspected, |_, _, cx| cx.notify()),
            cx.observe(&caches.operation, |_, _, cx| cx.notify()),
            cx.observe(&caches.body, |_, _, cx| cx.notify()),
        ];
        let response_tab = crate::dev_env::value("PROXYMATE_DEV_RESPONSE_TAB")
            .and_then(|t| ResponseTab::parse(&t))
            .unwrap_or(ResponseTab::Header);
        Inspector {
            table,
            caches,
            focus: cx.focus_handle(),
            request_tab: RequestTab::Header,
            response_tab,
            split: 0.5,
            raw: raw::RawDocs::default(),
            request_body: cx.new(|cx| crate::features::body::BodyPane::new(BodyPart::Request, cx)),
            response_body: cx.new(|cx| crate::features::body::BodyPane::new(BodyPart::Response, cx)),
            streams: None,
            _subscriptions: subscriptions,
        }
    }

    /// The Body tab of one side (tests, the dev remote).
    pub fn body_pane(&self, part: BodyPart) -> &Entity<crate::features::body::BodyPane> {
        match part {
            BodyPart::Request => &self.request_body,
            BodyPart::Response => &self.response_body,
        }
    }

    /// The Raw tab's view of one side, once shown.
    pub fn raw_view(&self, part: BodyPart) -> Option<&Entity<pm_widgets::code_view::CodeView>> {
        self.raw.get(part).map(|d| &d.view)
    }

    pub fn request_tab(&self) -> RequestTab {
        self.request_tab
    }

    pub fn response_tab(&self) -> ResponseTab {
        self.response_tab
    }

    pub fn set_request_tab(&mut self, tab: RequestTab, cx: &mut Context<Self>) {
        self.request_tab = tab;
        cx.notify();
    }

    pub fn set_response_tab(&mut self, tab: ResponseTab, cx: &mut Context<Self>) {
        self.response_tab = tab;
        cx.notify();
    }

    /// The flow shown: the table's focused flow, in its store.
    pub fn key(&self, cx: &App) -> Option<FlowKey> {
        let t = self.table.read(cx);
        let id = t.grid().read(cx).selected().focused?;
        Some(FlowKey::new(t.store(), id))
    }

    /// The inspected flow, if it has loaded.
    pub fn inspected(&self, cx: &App) -> Option<Arc<Inspected>> {
        let key = self.key(cx)?;
        self.caches.inspected.read(cx).peek(&key).cloned().flatten()
    }

    /// The stream panel of the flow shown, while it is a WebSocket or SSE
    /// flow.
    pub fn stream_panel(&self, cx: &App) -> Option<Entity<streams::StreamPanel>> {
        let key = self.key(cx)?;
        self.streams.clone().filter(|p| p.read(cx).flow() == key)
    }

    /// The panel for `key`, made afresh for another flow or kind.
    fn stream_panel_for(
        &mut self,
        key: FlowKey,
        kind: StreamKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<streams::StreamPanel> {
        if let Some(panel) = &self.streams
            && panel.read(cx).flow() == key
            && panel.read(cx).kind() == kind
        {
            return panel.clone();
        }
        let panel = cx.new(|cx| streams::StreamPanel::new(key, kind, window, cx));
        self.streams = Some(panel.clone());
        panel
    }

    /// Moves the keyboard into the inspector (↩ in the table, "Focus the
    /// inspector").
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// A context menu of the inspected flow by name (the dev remote and
    /// tests): `url`, `kv:<table>[:<row>]` (`request-headers`,
    /// `query-params`, `response-headers`, `response-trailers`,
    /// `tls-connection`, `cert-<n>`), `rule:<n>`, `cert:<n>`,
    /// `timing[:<row>]`, `failure`, `raw:request|response`, and the stream
    /// panel's `stream:<row>` and `payload`.
    pub fn menu_for(&self, name: &str, selected: Option<String>, cx: &App) -> Option<Menu> {
        if name == "payload" || name.starts_with("stream:") {
            return self.stream_panel(cx)?.read(cx).menu_for(name, selected, cx);
        }
        let insp = self.inspected(cx)?;
        let key = self.key(cx)?;
        let (kind, arg) = name.split_once(':').map_or((name, None), |(k, a)| (k, Some(a)));
        let flow = &insp.flow;
        let detail = insp.detail.as_ref();
        match kind {
            "url" => Some(menus::url_bar(key.store, flow, detail)),
            "kv" => {
                let (table, row) = arg?.split_once(':').map_or((arg?, None), |(t, r)| (t, r.parse().ok()));
                let (rows, kind, part) = panes::table_rows(table, &insp)?;
                Some(menus::kv(&rows, kind, part, row, selected))
            }
            "rule" => {
                let rule = detail?.rules.applied.get(arg?.parse::<usize>().ok()?)?;
                Some(menus::rule_badge(rule))
            }
            "cert" => {
                let chain = &detail?.tls.as_ref()?.chain;
                Some(menus::certificate(chain.get(arg?.parse::<usize>().ok()?)?))
            }
            "timing" => {
                let table = panes::timing_table(flow, detail?);
                Some(menus::timing(&table, arg.and_then(|a| a.parse().ok())))
            }
            "failure" => Some(menus::failure(&parts::failure_help(flow, detail)?)),
            "raw" => {
                let part = match arg? {
                    "request" => BodyPart::Request,
                    "response" => BodyPart::Response,
                    _ => return None,
                };
                let doc = self.raw.get(part)?;
                Some(crate::features::body::menus::raw(
                    part,
                    &doc.start_line,
                    &doc.headers,
                    doc.text.clone(),
                    selected,
                ))
            }
            _ => None,
        }
    }

    /// The whole pane, holding one centred message (nothing selected, an
    /// error, a gone flow).
    fn message(&self, t: &Tokens, alert: bool, children: Vec<gpui_kit::AnyElement>) -> gpui_kit::AnyElement {
        div()
            .id("inspector-message")
            .when(alert, |d| d.role(Role::Alert))
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .bg(t.background.hsla())
            .text_size(px(13.))
            .text_color(t.muted_foreground.hsla())
            .children(children)
            .test_support()
            .into_any_element()
    }

    fn render_flow(
        &mut self,
        key: FlowKey,
        insp: Arc<Inspected>,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let flow = &insp.flow;
        let detail = insp.detail.as_ref();
        let stream = pm_view::inspector::stream_kind(flow, detail);
        let operation = (&*flow.method != "CONNECT")
            .then(|| self.caches.operation.update(cx, |c, cx| c.get(&key, cx)))
            .and_then(|l| l.ready().cloned().flatten());
        let messages = self.messages(key, &insp, t, window, cx);
        let panel = stream.map(|kind| self.stream_panel_for(key, kind, window, cx));
        if panel.is_none() {
            self.streams = None;
        }
        let body = match panel {
            None => messages.into_any_element(),
            // WebSocket messages and SSE events take the lower 60%.
            Some(panel) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(div().h(gpui_kit::relative(0.4)).min_h(px(96.)).flex().child(messages))
                .child(panel)
                .into_any_element(),
        };
        div()
            .id("inspector-flow")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background.hsla())
            .child(self.url_bar(key, &insp, t, cx))
            .children(self.notes_bar(key, flow, t))
            .children(operation.map(|op| parts::operation_line(&op, t)))
            .children(detail.and_then(|d| parts::rule_badges(&d.rules, t)))
            .children(self.pinned_banner(flow, t, cx))
            .child(body)
            .into_any_element()
    }

    /// Request | Response, side by side, each at least 240 px.
    fn messages(
        &mut self,
        key: FlowKey,
        insp: &Arc<Inspected>,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let share = self.split;
        let request = self.request_pane(key, insp, t, window, cx);
        let response = self.response_pane(key, insp, t, window, cx);
        div()
            .id("inspector-messages")
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_row()
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<PaneSplit>, _, cx| {
                let left = f32::from(e.bounds.origin.x);
                let width = f32::from(e.bounds.size.width).max(1.);
                let max = (width - PANE_MIN_WIDTH - HANDLE).max(PANE_MIN_WIDTH);
                let x = (f32::from(e.event.position.x) - left).clamp(PANE_MIN_WIDTH, max);
                this.split = x / width;
                cx.notify();
            }))
            .child(
                div()
                    .id("request-pane")
                    .h_full()
                    .flex_none()
                    .w(gpui_kit::relative(share))
                    .min_w(px(PANE_MIN_WIDTH))
                    .child(request),
            )
            .child(
                div()
                    .id("inspector-split")
                    .role(Role::Splitter)
                    .aria_label("Resize the request and response")
                    .aria_orientation(gpui_kit::Orientation::Vertical)
                    .aria_numeric_value((share * 100.) as f64)
                    .flex_none()
                    .w(px(HANDLE))
                    .h_full()
                    .cursor_col_resize()
                    .border_l_1()
                    .border_color(t.border.hsla())
                    .hover(|s| s.bg(t.primary.hsla().opacity(0.3)))
                    .on_drag(PaneSplit, |_, _, _, cx| cx.new(|_| NoGhost))
                    .on_click(cx.listener(|this, e: &gpui_kit::ClickEvent, _, cx| {
                        if e.click_count() >= 2 {
                            this.split = 0.5;
                            cx.notify();
                        }
                    })),
            )
            .child(
                div()
                    .id("response-pane")
                    .h_full()
                    .flex_1()
                    .min_w(px(PANE_MIN_WIDTH))
                    .child(response),
            )
    }
}

trait Opacity {
    fn opacity(self, a: f32) -> Self;
}

impl Opacity for gpui_kit::Hsla {
    fn opacity(mut self, a: f32) -> Self {
        self.a *= a;
        self
    }
}

impl Render for Inspector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let key = self.key(cx);
        let content: gpui_kit::AnyElement = match key {
            None => self.message(&t, false, vec!["Select a flow to inspect it".into_any_element()]),
            Some(key) => match self.caches.inspected.update(cx, |c, cx| c.get(&key, cx)) {
                Load::Failed(err) => self.message(
                    &t,
                    true,
                    vec![
                        "Couldn't load this flow".into_any_element(),
                        div().max_w(px(420.)).text_size(px(12.)).child(err).into_any_element(),
                        gpui_kit::component::button::Button::new("inspector-retry")
                            .label("Try again")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.caches.inspected.update(cx, |c, cx| c.retry(&key, cx));
                            }))
                            .into_any_element(),
                    ],
                ),
                Load::Ready(None) => {
                    self.message(&t, false, vec!["This flow is no longer available".into_any_element()])
                }
                Load::Ready(Some(insp)) if insp.flow.id == key.id => self.render_flow(key, insp, &t, window, cx),
                // Still loading: a blank pane rather than a message flashing
                // on every arrow key.
                _ => div().size_full().bg(t.background.hsla()).into_any_element(),
            },
        };
        div()
            .id("inspector")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .role(Role::Region)
            .aria_label("Inspector")
            .size_full()
            .min_h_0()
            .text_color(t.foreground.hsla())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if !this.focus.contains_focused(window, cx) {
                        window.focus(&this.focus, cx);
                    }
                }),
            )
            .child(content)
    }
}

/// The text of a table's rows, for `menu_for` and the panes.
type Rows = Vec<(String, String)>;

/// Display-cased header rows as shared strings for a [`pm_widgets::KvTable`].
fn shared_rows(rows: &Rows) -> std::rc::Rc<Vec<(SharedString, SharedString)>> {
    std::rc::Rc::new(
        rows.iter()
            .map(|(k, v)| (SharedString::from(k.clone()), SharedString::from(v.clone())))
            .collect(),
    )
}
