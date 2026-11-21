//! The WebSocket panel (`WebSocketPanel.tsx`): the direction filter,
//! Close, the message rows with their flags, the composer and the
//! message detail with Resend and the Forwarded/Original toggle.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Role, SharedString, Window, div, px};
use pm_model::{WsDirection, WsMessage};
use pm_theme::Tokens;
use pm_view::stream::{self as view, ComposeMode, Flag};

use super::payload::{centered, chip};
use super::{CloseWebSocket, ResendWsMessage, Row, StreamPanel};
use crate::bridge::keys::WsPayloadKey;
use crate::bridge::resource::Load;

/// The columns' widths: arrow, #, Time, Type, Size; Payload takes the rest.
const COLUMNS: [f32; 5] = [14., 40., 92., 48., 60.];

fn arrow(direction: WsDirection, t: &Tokens) -> gpui_kit::Svg {
    match direction {
        WsDirection::Sent => pm_widgets::icon(IconName::ArrowUp, t.ws_sent.hsla(), 12.),
        WsDirection::Received => pm_widgets::icon(IconName::ArrowDown, t.ws_received.hsla(), 12.),
    }
}

fn cell(width: f32) -> gpui_kit::Div {
    div().flex_none().w(px(width)).overflow_hidden().whitespace_nowrap()
}

fn flag(flag: Flag, id: SharedString, t: &Tokens) -> impl IntoElement {
    let (fg, bg) = match flag {
        Flag::Injected => (t.diff_add, t.diff_add_bg),
        Flag::Edited => (t.diff_change, t.diff_change_bg),
        Flag::Dropped => (t.diff_del, t.diff_del_bg),
    };
    div()
        .id(id)
        .flex_none()
        .px(px(4.))
        .rounded(px(2.))
        .text_size(px(10.))
        .line_height(px(16.))
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .text_color(fg.hsla())
        .bg(bg.hsla())
        .child(flag.label())
        .tooltip(move |window, cx| Tooltip::new(flag.title()).build(window, cx))
        .test_support()
}

/// A message row's cells.
pub(super) fn row(
    base: gpui_kit::Stateful<gpui_kit::Div>,
    m: &WsMessage,
    selected: bool,
    t: &Tokens,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let muted = if selected {
        t.selection_fg.hsla()
    } else {
        t.muted_foreground.hsla()
    };
    let i = m.index as usize;
    let flags: Vec<_> = view::message_flags(m)
        .into_iter()
        .enumerate()
        .map(|(k, f)| flag(f, format!("ws-flag-{}", i * 3 + k).into(), t).into_any_element())
        .collect();
    base.aria_label(SharedString::from(view::row_label(m)))
        .child(cell(COLUMNS[0]).flex().items_center().child(arrow(m.direction, t)))
        .child(
            cell(COLUMNS[1])
                .text_right()
                .text_color(muted)
                .child((m.index + 1).to_string()),
        )
        .child(cell(COLUMNS[2]).child(pm_view::flow::format_time(Some(m.time_ms))))
        .child(cell(COLUMNS[3]).child(view::opcode_label(m.opcode)))
        .child(cell(COLUMNS[4]).text_right().child(view::size_label(m.len)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(4.))
                .children(flags)
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .when(m.dropped, |d| d.line_through().text_color(muted))
                        .child(view::row_preview(m)),
                ),
        )
}

pub(super) fn header(t: &Tokens) -> AnyElement {
    let head = |w: f32, label: &'static str, right: bool| {
        cell(w).when(right, |d| d.text_right()).child(label).into_any_element()
    };
    div()
        .flex_none()
        .h(px(22.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(t.border.hsla())
        .bg(t.pane_header.hsla())
        .text_size(px(10.))
        .font_weight(gpui_kit::FontWeight::MEDIUM)
        .text_color(t.muted_foreground.hsla())
        .child(head(COLUMNS[0], "", false))
        .child(head(COLUMNS[1], "#", true))
        .child(head(COLUMNS[2], "TIME", false))
        .child(head(COLUMNS[3], "TYPE", false))
        .child(head(COLUMNS[4], "SIZE", true))
        .child(div().flex_1().child("PAYLOAD"))
        .into_any_element()
}

impl StreamPanel {
    pub(super) fn render_websocket(&mut self, t: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (len, total) = (self.len() as u32, self.total() as u32);
        let head = self.ws_head.clone();
        let status = view::ws_status(
            head.as_ref().is_some_and(|h| h.open),
            head.as_ref().and_then(|h| h.close.as_ref()),
        );
        let count = view::count_label(len, total, "messages");
        let sendable = self.sendable();

        let filters = [
            (None, "All", "ws-filter-all"),
            (Some(WsDirection::Sent), "Sent", "ws-filter-sent"),
            (Some(WsDirection::Received), "Received", "ws-filter-received"),
        ]
        .map(|(d, label, id)| {
            chip(
                id.into(),
                label,
                self.direction == d,
                d.map(|d| arrow(d, t)),
                t,
                cx.listener(move |this, _, _, cx| this.set_direction(d, cx)),
            )
        });
        let mut controls = vec![
            div()
                .id("ws-direction")
                .role(Role::RadioGroup)
                .aria_label("Direction")
                .flex()
                .gap(px(2.))
                .children(filters)
                .into_any_element(),
            self.search_box(t, cx),
        ];
        if sendable {
            let flow = self.flow_ref();
            controls.push(
                Button::new("ws-close")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Unplug)
                    .label("Close")
                    .tooltip("Close the connection: a normal close frame to each side")
                    .on_click(move |_, window, cx| window.dispatch_action(Box::new(CloseWebSocket { flow }), cx))
                    .into_any_element(),
            );
        }
        let toolbar = self.toolbar(status, count, controls, t).into_any_element();

        let list = self.list(header(t), view::ws_empty(total), t, cx);
        let list = div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(list))
            .children(sendable.then(|| self.composer_element(t, cx)))
            .into_any_element();
        let detail = match self.shown(cx) {
            Some(Row::Ws(m)) => self.message_detail(&m, sendable, t, window, cx),
            _ => centered("Select a message", t),
        };
        let split = self.split_view(list, detail, t, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(split)
            .into_any_element()
    }

    fn composer_element(&self, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let Some(c) = &self.composer else {
            return div().into_any_element();
        };
        let directions = [
            (WsDirection::Sent, "To server", "ws-compose-to-server"),
            (WsDirection::Received, "To client", "ws-compose-to-client"),
        ]
        .map(|(d, label, id)| {
            chip(
                id.into(),
                label,
                c.direction == d,
                Some(arrow(d, t)),
                t,
                cx.listener(move |this, _, _, cx| {
                    if let Some(c) = &mut this.composer {
                        c.direction = d;
                    }
                    cx.notify();
                }),
            )
        });
        let modes = [
            (ComposeMode::Text, "Text", "ws-compose-text"),
            (ComposeMode::Hex, "Binary (hex)", "ws-compose-hex"),
        ]
        .map(|(m, label, id)| {
            chip(
                id.into(),
                label,
                c.mode == m,
                None,
                t,
                cx.listener(move |this, _, window, cx| {
                    let text = this
                        .composer
                        .as_ref()
                        .map(|c| c.input.read(cx).value().to_string())
                        .unwrap_or_default();
                    this.compose(&text, None, Some(m), window, cx);
                }),
            )
        });
        div()
            .id("ws-composer")
            .role(Role::Group)
            .aria_label("Send a message")
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(6.))
            .border_t_1()
            .border_color(t.border.hsla())
            .bg(t.pane_toolbar.hsla())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .id("ws-compose-direction")
                            .role(Role::RadioGroup)
                            .aria_label("Send to")
                            .flex()
                            .gap(px(2.))
                            .children(directions),
                    )
                    .child(
                        div()
                            .id("ws-compose-mode")
                            .role(Role::RadioGroup)
                            .aria_label("Payload")
                            .flex()
                            .gap(px(2.))
                            .children(modes),
                    )
                    .child(
                        div()
                            .id("ws-compose-error")
                            .when(c.error.is_some(), |d| d.role(Role::Alert))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_right()
                            .text_size(px(11.))
                            .text_color(t.s_failed.hsla())
                            .children(c.error.clone())
                            .test_support(),
                    )
                    .child(
                        Button::new("ws-compose-send")
                            .primary()
                            .xsmall()
                            .icon(IconName::Send)
                            .label("Send")
                            .tooltip("Send (⌘↩)")
                            .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                    ),
            )
            .child(
                Textarea::new(&c.input)
                    .aria_label("Message to send")
                    .h(px(48.))
                    .font_family("Menlo")
                    .text_size(px(12.)),
            )
            .test_support()
            .into_any_element()
    }

    fn message_detail(
        &mut self,
        m: &WsMessage,
        sendable: bool,
        t: &Tokens,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let original = self.original && m.edited;
        let key = WsPayloadKey {
            flow: self.flow,
            index: m.index,
            original,
        };
        let load = self.caches.ws_payload.update(cx, |c, cx| c.get(&key, cx));
        let recorded = match &load {
            Load::Ready(Some(bytes)) => Some(bytes.len()),
            _ => None,
        };
        let lines = view::detail_lines(m, recorded);
        let flow = self.flow_ref();
        let mut resend = (sendable && view::can_resend(m)).then(|| {
            let action = ResendWsMessage {
                flow,
                index: m.index,
                direction: m.direction,
                opcode: m.opcode,
            };
            Button::new("ws-detail-resend")
                .ghost()
                .xsmall()
                .icon(IconName::RotateCcw)
                .label("Resend")
                .tooltip(view::resend_label(m.direction))
                .on_click(move |_, window, cx| window.dispatch_action(Box::new(action.clone()), cx))
        });
        let note = |i: usize, text: String| {
            div()
                .flex_none()
                .px(px(8.))
                .py(px(4.))
                .flex()
                .items_center()
                .gap(px(6.))
                .border_b_1()
                .border_color(t.border.hsla())
                .text_size(px(11.))
                .text_color(t.muted_foreground.hsla())
                .when(i == 0, |d| d.child(arrow(m.direction, t)))
                .child(div().flex_1().min_w_0().truncate().child(text))
        };
        let mut notes: Vec<AnyElement> = Vec::new();
        for (i, line) in lines.into_iter().enumerate() {
            let mut n = note(i, line);
            if i == 0
                && let Some(b) = resend.take()
            {
                n = n.child(b);
            }
            notes.push(n.into_any_element());
        }
        let edited = m.edited.then(|| {
            let toggle = [
                (false, "Forwarded", "ws-show-forwarded"),
                (true, "Original", "ws-show-original"),
            ]
            .map(|(o, label, id)| {
                chip(
                    id.into(),
                    label,
                    original == o,
                    None,
                    t,
                    cx.listener(move |this, _, _, cx| this.set_original(o, cx)),
                )
            });
            div()
                .id("ws-edited-bar")
                .flex_none()
                .px(px(8.))
                .py(px(4.))
                .flex()
                .items_center()
                .gap(px(6.))
                .border_b_1()
                .border_color(t.border.hsla())
                .text_size(px(11.))
                .child(
                    div()
                        .flex_1()
                        .text_color(t.muted_foreground.hsla())
                        .child("A rule, script or breakpoint changed this message."),
                )
                .child(
                    div()
                        .id("ws-original-toggle")
                        .role(Role::RadioGroup)
                        .aria_label("Payload shown")
                        .flex()
                        .gap(px(2.))
                        .children(toggle),
                )
                .test_support()
        });
        let body = match load {
            Load::Ready(Some(bytes)) => {
                let binary = m.opcode == pm_model::WsOpcode::Binary;
                self.payload_view("ws", bytes, binary, t, cx)
            }
            Load::Ready(None) => centered("No original was recorded for this message", t),
            Load::Failed(err) => centered(format!("Couldn't load the payload: {err}"), t),
            Load::Loading => centered("Loading…", t),
        };
        div()
            .id("ws-message-detail")
            .size_full()
            .flex()
            .flex_col()
            .children(notes)
            .children(edited)
            .child(div().flex_1().min_h_0().child(body))
            .test_support()
            .into_any_element()
    }
}
