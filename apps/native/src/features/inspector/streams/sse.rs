//! The SSE panel (`SsePanel.tsx`): the events of a `text/event-stream`
//! response, parsed by the engine as the body streams, so the list grows
//! while the response is open; the selected event's fields and data.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, SharedString, Window, div, px};
use pm_model::SseEvent;
use pm_theme::Tokens;
use pm_view::inspector::KvKind;
use pm_view::stream as view;
use pm_widgets::KvTable;

use super::payload::centered;
use super::{Row, StreamPanel};
use crate::features::context_menu::selected_text;
use crate::features::inspector::menus;

/// The columns' widths: #, Time, Event, Id; Data takes the rest.
const COLUMNS: [f32; 4] = [40., 92., 96., 64.];

fn cell(width: f32) -> gpui_kit::Div {
    div().flex_none().w(px(width)).overflow_hidden().whitespace_nowrap()
}

/// An event row's cells.
pub(super) fn row(
    base: gpui_kit::Stateful<gpui_kit::Div>,
    e: &SseEvent,
    selected: bool,
    t: &Tokens,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let muted = if selected {
        t.selection_fg.hsla()
    } else {
        t.muted_foreground.hsla()
    };
    let label = format!("Event {}, {}", e.index + 1, view::event_name(e));
    base.aria_label(SharedString::from(label))
        .child(
            cell(COLUMNS[0])
                .text_right()
                .text_color(muted)
                .child((e.index + 1).to_string()),
        )
        .child(cell(COLUMNS[1]).child(pm_view::flow::format_time(e.time_ms)))
        .child(cell(COLUMNS[2]).truncate().child(view::event_name(e).to_owned()))
        .child(cell(COLUMNS[3]).truncate().child(e.id.clone().unwrap_or_default()))
        .child(div().flex_1().min_w_0().truncate().child(view::data_preview(&e.data)))
}

fn header(t: &Tokens) -> AnyElement {
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
        .child(head(COLUMNS[0], "#", true))
        .child(head(COLUMNS[1], "TIME", false))
        .child(head(COLUMNS[2], "EVENT", false))
        .child(head(COLUMNS[3], "ID", false))
        .child(div().flex_1().child("DATA"))
        .into_any_element()
}

impl StreamPanel {
    pub(super) fn render_sse(&mut self, t: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (len, total) = (self.len() as u32, self.total() as u32);
        let open = self.is_open();
        let toolbar = self
            .toolbar(
                view::sse_status(open).into(),
                view::count_label(len, total, "events"),
                vec![self.search_box(t, cx)],
                t,
            )
            .into_any_element();
        let list = self.list(header(t), view::sse_empty(total, open), t, cx);
        let detail = match self.shown(cx) {
            Some(Row::Sse(e)) => self.event_detail(&e, t, window, cx),
            _ => centered("Select an event", t),
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

    fn event_detail(&mut self, e: &SseEvent, t: &Tokens, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let fields = view::sse_fields(e);
        let shared = Rc::new(
            fields
                .iter()
                .map(|(k, v)| (SharedString::from(k.clone()), SharedString::from(v.clone())))
                .collect::<Vec<_>>(),
        );
        let rows = Rc::new(fields);
        let table = KvTable::new("sse-fields", "Event fields", shared).on_menu(move |index, position, window, cx| {
            let selected = selected_text(window, cx);
            menus::kv(&rows, KvKind::Plain, None, index, selected).show(position, window, cx);
        });
        let bytes = match &self.sse_bytes {
            Some((index, bytes)) if *index == e.index => bytes.clone(),
            _ => {
                let bytes = Arc::new(e.data.clone().into_bytes());
                self.sse_bytes = Some((e.index, bytes.clone()));
                bytes
            }
        };
        let cut = e.truncated.then(|| {
            div()
                .flex_none()
                .px(px(8.))
                .py(px(4.))
                .border_b_1()
                .border_color(t.border.hsla())
                .text_size(px(11.))
                .text_color(t.muted_foreground.hsla())
                .child("Data cut at 1 MB")
        });
        let payload = self.payload_view("sse", bytes, false, t, cx);
        div()
            .id("sse-event-detail")
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .border_b_1()
                    .border_color(t.border.hsla())
                    .child(table),
            )
            .children(cut)
            .child(div().flex_1().min_h_0().child(payload))
            .test_support()
            .into_any_element()
    }
}
