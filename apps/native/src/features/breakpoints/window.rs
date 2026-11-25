//! The Breakpoint Editor's window: the queue of paused flows (oldest
//! first, each with its phase, method or message direction, URL, rule and
//! countdown) beside the shown flow's form. One window at a time.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, Global, MouseButton,
    MouseDownEvent, Render, Role, SharedString, Subscription, Task, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowOptions, div, px, size,
};
use pm_model::{BreakpointId, BreakpointInfo, Phase, WsDirection};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::breakpoints::{self as logic, QueueItem};

use super::form::HttpForm;
use super::message::MessageForm;
use super::{AbortBreakpoint, ContinueBreakpoint, ShowPausedFlow};
use crate::bridge;
use crate::bridge::stream::Latest;
use crate::features::chrome::OpenRule;
use crate::features::context_menu::Menu;
use crate::features::editing::method_pill;

pub const DEFAULT_SIZE: (f32, f32) = (1080., 720.);
pub const MIN_SIZE: (f32, f32) = (760., 480.);
pub const QUEUE_WIDTH: f32 = 256.;

/// The shown flow's form.
#[derive(Clone)]
pub enum Pane {
    Http(Entity<HttpForm>),
    Message(Entity<MessageForm>),
}

struct Open {
    window: AnyWindowHandle,
    view: WeakEntity<BreakpointsWindow>,
}

impl Global for Open {}

pub struct BreakpointsWindow {
    queue: Entity<Latest<Vec<BreakpointInfo>>>,
    chosen: Option<BreakpointId>,
    pane: Option<(BreakpointId, Pane)>,
    /// Milliseconds since the epoch, for the countdowns (ticks each second).
    now_ms: f64,
    focus: FocusHandle,
    _ticker: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

pub fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0., |d| d.as_secs_f64() * 1000.)
}

/// Opens the window, or brings it forward.
pub fn open_or_focus(cx: &mut App) {
    if let Some((window, _)) = BreakpointsWindow::global(cx) {
        let _ = window.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let queue = bridge::breakpoints(cx);
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Breakpoints".into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)),
            cx,
        ))),
        window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        app_id: Some(pm_app::BUNDLE_ID.into()),
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| {
            let mut w = BreakpointsWindow::new(queue, window, cx);
            w.start_ticking(cx);
            w
        });
        BreakpointsWindow::register(window.window_handle(), &view, cx);
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
    });
    match opened {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Err(err) => eprintln!("can't open the Breakpoint Editor: {err}"),
    }
}

/// Closes the window, if it is open.
pub fn close(cx: &mut App) {
    if let Some((window, _)) = BreakpointsWindow::global(cx) {
        let _ = window.update(cx, |_, window, _| window.remove_window());
        cx.remove_global::<Open>();
    }
}

impl BreakpointsWindow {
    pub fn new(queue: Entity<Latest<Vec<BreakpointInfo>>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe_in(&queue, window, |this, _, window, cx| this.sync(window, cx))];
        let mut view = BreakpointsWindow {
            queue,
            chosen: None,
            pane: None,
            now_ms: now_ms(),
            focus: cx.focus_handle(),
            _ticker: None,
            _subscriptions: subscriptions,
        };
        view.sync(window, cx);
        view
    }

    /// Makes `view` the window the actions and the dev remote act on.
    pub fn register(window: AnyWindowHandle, view: &Entity<BreakpointsWindow>, cx: &mut App) {
        cx.set_global(Open {
            window,
            view: view.downgrade(),
        });
    }

    pub fn global(cx: &App) -> Option<(AnyWindowHandle, Entity<BreakpointsWindow>)> {
        let open = cx.try_global::<Open>()?;
        Some((open.window, open.view.upgrade()?))
    }

    /// Runs `f` on the open window's view, in its window (deferred).
    pub fn update_global(
        cx: &mut App,
        f: impl FnOnce(&mut BreakpointsWindow, &mut Window, &mut Context<BreakpointsWindow>) + 'static,
    ) {
        cx.defer(move |cx| {
            let Some((window, view)) = BreakpointsWindow::global(cx) else {
                return;
            };
            let _ = window.update(cx, |_, window, cx| view.update(cx, |w, cx| f(w, window, cx)));
        });
    }

    fn start_ticking(&mut self, cx: &mut Context<Self>) {
        self._ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update(cx, |w, cx| {
                    w.now_ms = now_ms();
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    /// The clock the countdowns use (offscreen renders pin it).
    pub fn set_now(&mut self, now_ms: f64, cx: &mut Context<Self>) {
        self.now_ms = now_ms;
        cx.notify();
    }

    pub fn queue(&self, cx: &App) -> Vec<BreakpointInfo> {
        self.queue.read(cx).value.clone().unwrap_or_default()
    }

    /// The paused flow shown.
    pub fn shown(&self, cx: &App) -> Option<BreakpointInfo> {
        logic::shown(&self.queue(cx), self.chosen).cloned()
    }

    pub fn pane(&self) -> Option<&Pane> {
        self.pane.as_ref().map(|(_, p)| p)
    }

    pub fn select(&mut self, id: BreakpointId, window: &mut Window, cx: &mut Context<Self>) {
        self.chosen = Some(id);
        self.sync(window, cx);
    }

    /// Follows the queue: the shown flow's form is kept while it waits, and
    /// the next one is shown once it's resolved.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.shown(cx);
        match (&shown, &self.pane) {
            (Some(bp), Some((id, _))) if bp.id == *id => {}
            (Some(bp), _) => {
                let bp = bp.clone();
                let pane = if bp.message.is_some() {
                    Pane::Message(cx.new(|cx| MessageForm::new(bp.clone(), window, cx)))
                } else {
                    Pane::Http(cx.new(|cx| HttpForm::new(bp.clone(), window, cx)))
                };
                self.pane = Some((bp.id, pane));
            }
            (None, _) => self.pane = None,
        }
        cx.notify();
    }

    /// A queue row's menu, by breakpoint id (also the dev remote's).
    pub fn menu_for(&self, id: BreakpointId, cx: &App) -> Option<Menu> {
        let bp = self.queue(cx).into_iter().find(|b| b.id == id)?;
        Some(
            logic::queue_menu(&bp)
                .into_iter()
                .fold(Menu::new(), |menu, item| match item {
                    QueueItem::Copy { label, text, what } => menu.copy(label, text, what),
                    QueueItem::ShowFlow(flow) => {
                        menu.item(format!("Show flow #{}", flow + 1), ShowPausedFlow { id: flow })
                    }
                    QueueItem::OpenRule { id, name } => {
                        menu.item(format!("Open rule “{name}”"), OpenRule { id: id.into() })
                    }
                    QueueItem::Separator => menu.separator(),
                    QueueItem::Continue => menu.item("Continue unmodified", ContinueBreakpoint { id }),
                    QueueItem::Abort(label) => menu.item(label, AbortBreakpoint { id }),
                }),
        )
    }

    fn queue_row(&self, bp: &BreakpointInfo, selected: bool, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let id = bp.id;
        let (fg, muted) = if selected {
            (t.selection_fg, t.selection_fg)
        } else {
            (t.foreground, t.muted_foreground)
        };
        let what = match &bp.message {
            Some(m) => {
                let (icon, color) = match m.direction {
                    _ if selected => (
                        if m.direction == WsDirection::Sent {
                            gpui_kit::assets::IconName::ArrowUp
                        } else {
                            gpui_kit::assets::IconName::ArrowDown
                        },
                        t.selection_fg,
                    ),
                    WsDirection::Sent => (gpui_kit::assets::IconName::ArrowUp, t.ws_sent),
                    WsDirection::Received => (gpui_kit::assets::IconName::ArrowDown, t.ws_received),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted.hsla())
                    .child(pm_widgets::icon(icon, color.hsla(), 12.))
                    .child("WebSocket")
                    .into_any_element()
            }
            None if selected => {
                pm_widgets::Pill::new(bp.request.method.clone(), t.selection_fg, t.pill_on_selection).into_any_element()
            }
            None => method_pill(&bp.request.method, t).into_any_element(),
        };
        // On the selection, pills turn to its colours (the table's rows do).
        let phase = if selected {
            pm_widgets::Pill::new(logic::phase_label(bp.phase), t.selection_fg, t.pill_on_selection)
        } else {
            phase_pill(bp.phase, t)
        };
        let mono = cx.theme().mono_font_family.clone();
        div()
            .id(SharedString::from(format!("bp-queue-{id}")))
            .role(Role::ListItem)
            .aria_label(SharedString::from(logic::queue_label(bp, self.now_ms)))
            .aria_selected(selected)
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(fg.hsla())
            .when(selected, |d| d.bg(t.selection_bg.hsla()))
            .when(!selected, |d| d.hover(|s| s.bg(t.accent.hsla())))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| this.select(id, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.select(id, window, cx);
                    if let Some(menu) = this.menu_for(id, cx) {
                        menu.show(e.position, window, cx);
                    }
                }),
            )
            .child(
                div().flex().items_center().gap(px(6.)).child(phase).child(what).child(
                    div()
                        .ml_auto()
                        .font_family(mono.clone())
                        .text_size(px(11.))
                        .text_color(muted.hsla())
                        .children(logic::flow_number(bp)),
                ),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(mono.clone())
                    .text_size(px(11.))
                    .child(bp.request.url.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(11.))
                    .text_color(muted.hsla())
                    .child(div().min_w_0().truncate().child(bp.rule_name.clone()))
                    .child(
                        div()
                            .ml_auto()
                            .flex_none()
                            .font_family(mono)
                            .child(logic::countdown(bp.deadline_ms, self.now_ms)),
                    ),
            )
            .into_any_element()
    }
}

/// The phase pill, in the paused colours.
pub fn phase_pill(phase: Phase, t: &Tokens) -> pm_widgets::Pill {
    pm_widgets::Pill::new(logic::phase_label(phase), t.s_paused, t.s_paused_bg)
}

impl Focusable for BreakpointsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for BreakpointsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let queue = self.queue(cx);
        let shown = logic::shown(&queue, self.chosen).map(|b| b.id);
        let rows: Vec<AnyElement> = queue
            .iter()
            .map(|bp| self.queue_row(bp, Some(bp.id) == shown, &t, cx))
            .collect();
        let now = self.now_ms;
        div()
            .id("breakpoint-editor")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background.hsla())
            .text_color(t.foreground.hsla())
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px(px(16.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(t.border.hsla())
                    .child(
                        div()
                            .id("breakpoints-title")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .role(Role::Heading)
                            .child(pm_widgets::icon(
                                gpui_kit::assets::IconName::OctagonPause,
                                t.s_paused.hsla(),
                                16.,
                            ))
                            .child("Breakpoints"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.muted_foreground.hsla())
                            .child(logic::DESCRIPTION),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .id("breakpoint-queue")
                            .role(Role::List)
                            .aria_label("Paused flows")
                            .w(px(QUEUE_WIDTH))
                            .flex_none()
                            .overflow_y_scroll()
                            .border_r_1()
                            .border_color(t.border.hsla())
                            .bg(t.muted.hsla())
                            .p(px(4.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .when(rows.is_empty(), |d| {
                                d.child(
                                    div()
                                        .p(px(8.))
                                        .text_size(px(12.))
                                        .text_color(t.muted_foreground.hsla())
                                        .child(logic::EMPTY_QUEUE),
                                )
                            })
                            .children(rows),
                    )
                    .child(div().flex_1().min_w_0().map(|d| {
                        match &self.pane {
                            Some((_, Pane::Http(form))) => {
                                form.update(cx, |f, _| f.now_ms = now);
                                d.child(form.clone())
                            }
                            Some((_, Pane::Message(form))) => {
                                form.update(cx, |f, _| f.now_ms = now);
                                d.child(form.clone())
                            }
                            None => d.flex().items_center().justify_center().p(px(24.)).child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.muted_foreground.hsla())
                                    .child(logic::EMPTY),
                            ),
                        }
                    })),
            )
            .test_support()
    }
}
