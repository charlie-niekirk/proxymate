//! The Request and Response panes (`RequestPane`, `ResponsePane`) and
//! their tabs: Header (with trailers), Query, Raw, TLS (`TlsView.tsx`),
//! Timing (`TimingView.tsx`), and the Body tabs (G4's `body::BodyPane`).

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, FontWeight, MouseButton, MouseDownEvent, Role, SharedString, Window, div, px};
use pm_model::{BodyPart, FlowDetail, FlowState, FlowSummary, TlsStatus};
use pm_theme::Tokens;
use pm_view::flow::{format_duration, format_ms};
use pm_view::http::{header_rows, query_pairs, raw_message};
use pm_view::inspector::{self as view, KvKind, RawBody};
use pm_view::tls::{PhaseTone, cert_rows, connection_rows, route_rows, timing_phases};
use pm_widgets::KvTable;

use super::raw::address;
use super::{Inspector, RequestTab, ResponseTab, Rows, menus, parts, shared_rows};
use crate::bridge::keys::{BodyKey, FlowKey, Inspected};
use crate::bridge::resource::Load;
use crate::features::body;
use crate::features::context_menu::selected_text;

/// A table of the inspected flow by id: its rows, kind and side.
pub fn table_rows(table: &str, insp: &Inspected) -> Option<(Rows, KvKind, Option<BodyPart>)> {
    let detail = insp.detail.as_ref()?;
    match table {
        "request-headers" => Some((
            header_rows(&detail.request.headers),
            KvKind::Header,
            Some(BodyPart::Request),
        )),
        "query-params" => Some((query_pairs(&detail.request.url), KvKind::Query, None)),
        "response-headers" => Some((
            header_rows(&detail.response.as_ref()?.headers),
            KvKind::Header,
            Some(BodyPart::Response),
        )),
        "response-trailers" => Some((
            header_rows(detail.trailers.as_ref()?),
            KvKind::Header,
            Some(BodyPart::Response),
        )),
        "tls-connection" => Some((connection_rows(detail)?, KvKind::Plain, None)),
        cert => {
            let index: usize = cert.strip_prefix("cert-")?.parse().ok()?;
            let chain = &detail.tls.as_ref()?.chain;
            Some((
                cert_rows(chain.get(index)?, insp.flow.started_at_ms),
                KvKind::Plain,
                None,
            ))
        }
    }
}

/// The Timing tab as label/value rows: the phases, Total, the route.
pub fn timing_table(flow: &FlowSummary, detail: &FlowDetail) -> Rows {
    let mut rows: Rows = timing_phases(&detail.timing)
        .into_iter()
        .map(|p| (p.label.to_owned(), format_ms(Some(p.ms))))
        .collect();
    rows.push(("Total".into(), format_duration(flow.duration_ms)));
    rows.extend(route_rows(detail.via.as_ref()));
    rows
}

fn placeholder(text: impl Into<SharedString>, t: &Tokens) -> AnyElement {
    div()
        .p(px(16.))
        .w_full()
        .flex()
        .justify_center()
        .text_size(px(12.))
        .text_color(t.muted_foreground.hsla())
        .child(text.into())
        .into_any_element()
}

/// A tab whose content scrolls as a whole (headers, TLS, timing).
fn scroll(id: &'static str, content: impl IntoElement) -> AnyElement {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child(content)
        .into_any_element()
}

fn section_title(text: &'static str, t: &Tokens) -> impl IntoElement {
    div()
        .px(px(8.))
        .py(px(4.))
        .border_b_1()
        .border_color(t.border.hsla())
        .bg(t.pane_header.hsla())
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.muted_foreground.hsla())
        .child(text)
}

/// A name/value table whose right-click opens its menu, keyed by row.
fn kv_table(
    id: impl Into<SharedString>,
    label: &'static str,
    rows: Rows,
    kind: KvKind,
    part: Option<BodyPart>,
) -> KvTable {
    let shared = shared_rows(&rows);
    let rows = Rc::new(rows);
    KvTable::new(id, label, shared).on_menu(move |index, position, window, cx| {
        let selected = selected_text(window, cx);
        menus::kv(&rows, kind, part, index, selected).show(position, window, cx);
    })
}

/// A pane's title and tab strip.
fn tab_header(
    id: &'static str,
    title: &'static str,
    labels: Vec<&'static str>,
    selected: usize,
    on_click: impl Fn(&usize, &mut Window, &mut gpui_kit::App) + 'static,
    t: &Tokens,
) -> impl IntoElement {
    div()
        .flex_none()
        .h(px(32.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(t.border.hsla())
        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(title))
        .child(
            TabBar::new(id)
                .xsmall()
                .selected_index(selected)
                .children(labels.into_iter().map(|l| Tab::new().label(l)))
                .on_click(on_click),
        )
}

impl Inspector {
    pub(super) fn request_pane(
        &mut self,
        key: FlowKey,
        insp: &Arc<Inspected>,
        t: &Tokens,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = RequestTab::ALL.iter().position(|x| *x == self.request_tab).unwrap_or(0);
        let header = tab_header(
            "request-tabs",
            "Request",
            RequestTab::ALL.iter().map(|x| x.label()).collect(),
            selected,
            cx.listener(|this, ix: &usize, _, cx| {
                if let Some(tab) = RequestTab::ALL.get(*ix) {
                    this.set_request_tab(*tab, cx);
                }
            }),
            t,
        );
        let content = match insp.detail.as_ref() {
            None => no_detail(t),
            Some(detail) => match self.request_tab {
                RequestTab::Header => scroll(
                    "request-header-tab",
                    kv_table(
                        "request-headers",
                        "Request headers",
                        header_rows(&detail.request.headers),
                        KvKind::Header,
                        Some(BodyPart::Request),
                    ),
                ),
                RequestTab::Query => {
                    let pairs = query_pairs(&detail.request.url);
                    if pairs.is_empty() {
                        placeholder("No query parameters", t)
                    } else {
                        scroll(
                            "request-query-tab",
                            kv_table("query-params", "Query parameters", pairs, KvKind::Query, None),
                        )
                    }
                }
                RequestTab::Body => {
                    let finished = insp.flow.state != FlowState::Active || detail.request_body.complete;
                    self.body_tab(key, insp, BodyPart::Request, finished, cx)
                }
                RequestTab::Raw => {
                    let finished = insp.flow.state != FlowState::Active || detail.request_body.complete;
                    self.raw_tab(key, insp, BodyPart::Request, finished, t, cx)
                }
            },
        };
        pane(header, Vec::new(), content, t)
    }

    pub(super) fn response_pane(
        &mut self,
        key: FlowKey,
        insp: &Arc<Inspected>,
        t: &Tokens,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = ResponseTab::ALL
            .iter()
            .position(|x| *x == self.response_tab)
            .unwrap_or(0);
        let header = tab_header(
            "response-tabs",
            "Response",
            ResponseTab::ALL.iter().map(|x| x.label()).collect(),
            selected,
            cx.listener(|this, ix: &usize, _, cx| {
                if let Some(tab) = ResponseTab::ALL.get(*ix) {
                    this.set_response_tab(*tab, cx);
                }
            }),
            t,
        );
        let flow = &insp.flow;
        let Some(detail) = insp.detail.as_ref() else {
            return pane(header, Vec::new(), no_detail(t), t);
        };
        let mut banners = Vec::new();
        if let Some(help) = parts::failure_help(flow, Some(detail)) {
            banners.push(self.failure_banner(flow, help, t, cx));
        }
        if flow.tls == TlsStatus::Passthrough {
            banners.push(parts::tunnel_note(flow, t));
        }
        let no_response = || {
            placeholder(
                if detail.error.is_some() {
                    "No response"
                } else {
                    "Waiting for the response…"
                },
                t,
            )
        };
        let content = match self.response_tab {
            ResponseTab::Header => match &detail.response {
                None => no_response(),
                Some(response) => {
                    let trailers = detail.trailers.as_ref().filter(|t| !t.is_empty()).map(|trailers| {
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .px(px(8.))
                                    .py(px(4.))
                                    .border_y_1()
                                    .border_color(t.border.hsla())
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(t.muted_foreground.hsla())
                                    .child("Trailers"),
                            )
                            .child(kv_table(
                                "response-trailers",
                                "Response trailers",
                                header_rows(trailers),
                                KvKind::Header,
                                Some(BodyPart::Response),
                            ))
                    });
                    scroll(
                        "response-header-tab",
                        div()
                            .flex()
                            .flex_col()
                            .child(kv_table(
                                "response-headers",
                                "Response headers",
                                header_rows(&response.headers),
                                KvKind::Header,
                                Some(BodyPart::Response),
                            ))
                            .children(trailers),
                    )
                }
            },
            ResponseTab::Body => match &detail.response {
                None => no_response(),
                Some(response) => {
                    let finished = flow.state != FlowState::Active || response.status == 101;
                    self.body_tab(key, insp, BodyPart::Response, finished, cx)
                }
            },
            ResponseTab::Raw => match &detail.response {
                None => no_response(),
                Some(response) => {
                    // A 101 has no body: the connection carries WebSocket messages instead.
                    let finished = flow.state != FlowState::Active || response.status == 101;
                    self.raw_tab(key, insp, BodyPart::Response, finished, t, cx)
                }
            },
            ResponseTab::Tls => scroll("response-tls-tab", tls_view(flow, detail, t)),
            ResponseTab::Timing => scroll("response-timing-tab", timing_view(flow, detail, t)),
        };
        pane(header, banners, content, t)
    }

    /// The Raw tab: a note, then the message rebuilt from its parts.
    fn raw_tab(
        &mut self,
        key: FlowKey,
        insp: &Arc<Inspected>,
        part: BodyPart,
        finished: bool,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(detail) = insp.detail.as_ref() else {
            return no_detail(t);
        };
        let info = match part {
            BodyPart::Request => &detail.request_body,
            BodyPart::Response => &detail.response_body,
        };
        let rule = view::raw_body(info, finished);
        let body = if rule == RawBody::Fetch {
            let body_key = BodyKey { flow: key, part };
            match self.caches.body.update(cx, |c, cx| c.get(&body_key, cx)) {
                Load::Ready(bytes) => Some(bytes),
                _ => None,
            }
        } else {
            None
        };
        let source = (address(insp), body.as_ref().map_or(0, address));
        let (start_line, headers) = match part {
            BodyPart::Request => (view::request_line(detail), detail.request.headers.clone()),
            BodyPart::Response => (
                view::status_line(detail).unwrap_or_default(),
                detail.response.as_ref().map(|r| r.headers.clone()).unwrap_or_default(),
            ),
        };
        let size = info.size;
        let doc = self.raw.update(
            part,
            source,
            info.kind,
            move || {
                let body_text = match rule {
                    RawBody::None => None,
                    RawBody::Note(note) => Some(note),
                    RawBody::Fetch => body.map(|b| view::raw_body_text(&b, size)),
                };
                let text = raw_message(&start_line, &headers, body_text.as_deref());
                (start_line, headers, text)
            },
            cx,
        );
        let terms = body_terms(part, cx);
        let raw_view = doc.view.clone();
        raw_view.update(cx, |v, cx| v.set_terms(terms, cx));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .py(px(4.))
                    .border_b_1()
                    .border_color(t.border.hsla())
                    .text_size(px(11.))
                    .text_color(t.muted_foreground.hsla())
                    .child("Rebuilt from the parsed message, not the exact bytes on the wire"),
            )
            .child(div().flex_1().min_h_0().child(raw_view))
            .into_any_element()
    }

    /// The Body tab: G4's viewers over the message's body.
    fn body_tab(
        &mut self,
        key: FlowKey,
        insp: &Arc<Inspected>,
        part: BodyPart,
        finished: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(detail) = insp.detail.as_ref() else {
            return div().into_any_element();
        };
        let (info, headers) = match part {
            BodyPart::Request => (detail.request_body.clone(), detail.request.headers.clone()),
            BodyPart::Response => (
                detail.response_body.clone(),
                detail.response.as_ref().map(|r| r.headers.clone()).unwrap_or_default(),
            ),
        };
        let input = body::BodyInput {
            key,
            part,
            info,
            finished,
            headers,
            url: detail.request.url.clone(),
            terms: body_terms(part, cx),
        };
        let pane = self.body_pane(part).clone();
        pane.update(cx, |p, cx| p.set_input(input, cx));
        div().flex_1().min_h_0().child(pane).into_any_element()
    }
}

/// The filter's body terms for one side (M17's body search).
fn body_terms(part: BodyPart, cx: &App) -> Vec<String> {
    crate::workspace::Workspace::global(cx)
        .map(|w| pm_view::dsl::body_search_terms(&w.read(cx).filter.text, part))
        .unwrap_or_default()
}

/// A pane: its tab strip, any banners, then the tab.
fn pane(header: impl IntoElement, banners: Vec<AnyElement>, content: AnyElement, t: &Tokens) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(t.background.hsla())
        .child(header)
        .children(banners)
        .child(div().flex_1().min_h_0().flex().flex_col().child(content))
        .into_any_element()
}

fn no_detail(t: &Tokens) -> AnyElement {
    placeholder("Synthetic flow: no recorded messages", t)
}

/// The TLS tab: how both legs were negotiated, and the upstream's chain.
fn tls_view(flow: &FlowSummary, detail: &FlowDetail, t: &Tokens) -> AnyElement {
    if flow.tls == TlsStatus::Passthrough {
        return placeholder(
            "Not decrypted: this host is in the SSL exclude or bypass list, so the tunnel was relayed as is.",
            t,
        );
    }
    let Some(rows) = connection_rows(detail) else {
        return placeholder("Plain HTTP: no TLS", t);
    };
    let chain = detail.tls.as_ref().map(|tls| tls.chain.clone()).unwrap_or_default();
    let cards: Vec<AnyElement> =
        if chain.is_empty() {
            vec![placeholder("The upstream presented no certificate", t)]
        } else {
            chain
                .iter()
                .enumerate()
                .map(|(i, cert)| {
                    let menu_cert = cert.clone();
                    let id = format!("cert-{i}");
                    div()
                        .border_b_1()
                        .border_color(t.border.hsla())
                        .child(
                            div()
                                .id(("cert-title", i))
                                .flex()
                                .items_baseline()
                                .gap(px(8.))
                                .px(px(8.))
                                .pt(px(6.))
                                .text_size(px(12.))
                                .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                                    menus::certificate(&menu_cert).show(e.position, window, cx);
                                })
                                .child(div().text_color(t.muted_foreground.hsla()).child(i.to_string()))
                                .child(div().font_weight(FontWeight::MEDIUM).child(
                                    gpui_kit::base::SelectableText::new(("cert-subject", i), cert.subject.clone()),
                                ))
                                .when(i == 0, |d| {
                                    d.child(div().text_color(t.muted_foreground.hsla()).child("leaf"))
                                }),
                        )
                        .child(kv_table(
                            id,
                            "Certificate",
                            cert_rows(cert, flow.started_at_ms),
                            KvKind::Plain,
                            None,
                        ))
                        .into_any_element()
                })
                .collect()
        };
    div()
        .id("tls-view")
        .flex()
        .flex_col()
        .child(section_title("CONNECTION", t))
        .child(kv_table("tls-connection", "TLS connection", rows, KvKind::Plain, None))
        .child(section_title("UPSTREAM CERTIFICATE CHAIN", t))
        .children(cards)
        .test_support()
        .into_any_element()
}

/// The Timing tab: a waterfall of the connection and transfer phases, the
/// total, and the route the connection took.
fn timing_view(flow: &FlowSummary, detail: &FlowDetail, t: &Tokens) -> AnyElement {
    let phases = timing_phases(&detail.timing);
    if phases.is_empty() {
        return placeholder(
            if flow.state == FlowState::Active {
                "Waiting for the upstream…"
            } else {
                "No timings recorded"
            },
            t,
        );
    }
    let table = Rc::new(timing_table(flow, detail));
    let last = phases.last().expect("not empty");
    let total = (last.offset + last.ms).max(0.001);
    let route = route_rows(detail.via.as_ref());
    let menu = |index: Option<usize>| {
        let table = table.clone();
        move |e: &MouseDownEvent, window: &mut Window, cx: &mut gpui_kit::App| {
            if index.is_some() {
                cx.stop_propagation();
            }
            menus::timing(&table, index).show(e.position, window, cx);
        }
    };
    let n = phases.len();
    let color = |tone: PhaseTone| match tone {
        PhaseTone::Dns => t.timing_dns,
        PhaseTone::Connect => t.timing_connect,
        PhaseTone::Tls => t.timing_tls,
        PhaseTone::Wait => t.timing_wait,
        PhaseTone::Receive => t.timing_receive,
    };
    let row = |id: (&'static str, usize)| div().id(id).flex().items_center().gap(px(8.));
    div()
        .id("timing-view")
        .role(Role::Table)
        .aria_label("Timing")
        .flex()
        .flex_col()
        .gap(px(4.))
        .p(px(8.))
        .text_size(px(12.))
        .on_mouse_down(MouseButton::Right, menu(None))
        .children(phases.iter().enumerate().map(|(i, p)| {
            let left = (p.offset / total) as f32;
            let width = ((p.ms / total) as f32).max(0.005);
            row(("timing-phase", i))
                .role(Role::Row)
                .aria_label(SharedString::from(format!("{} {}", p.label, format_ms(Some(p.ms)))))
                .on_mouse_down(MouseButton::Right, menu(Some(i)))
                .child(
                    div()
                        .w(px(110.))
                        .flex_none()
                        .text_color(t.muted_foreground.hsla())
                        .child(p.label),
                )
                .child(
                    div()
                        .flex_1()
                        .h(px(10.))
                        .rounded(px(2.))
                        .bg(t.muted.hsla())
                        .relative()
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(gpui_kit::relative(left))
                                .w(gpui_kit::relative(width))
                                .rounded(px(2.))
                                .bg(color(p.tone).hsla()),
                        ),
                )
                .child(
                    div()
                        .w(px(72.))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .child(format_ms(Some(p.ms))),
                )
        }))
        .child(
            row(("timing-total", 0))
                .role(Role::Row)
                .mt(px(4.))
                .pt(px(6.))
                .border_t_1()
                .border_color(t.border.hsla())
                .on_mouse_down(MouseButton::Right, menu(Some(n)))
                .child(
                    div()
                        .w(px(110.))
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Total"),
                )
                .child(
                    div()
                        .flex_1()
                        .text_color(t.muted_foreground.hsla())
                        .when(detail.timing.reused_connection, |d| {
                            d.child("Reused an open upstream connection")
                        }),
                )
                .child(
                    div()
                        .w(px(72.))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .child(format_duration(flow.duration_ms)),
                ),
        )
        .when(!route.is_empty(), |d| {
            d.child(
                div()
                    .id("timing-route")
                    .mt(px(4.))
                    .pt(px(6.))
                    .border_t_1()
                    .border_color(t.border.hsla())
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(route.into_iter().enumerate().map(|(i, (label, value))| {
                        row(("timing-route-row", i))
                            .role(Role::Row)
                            .on_mouse_down(MouseButton::Right, menu(Some(n + 1 + i)))
                            .child(
                                div()
                                    .w(px(110.))
                                    .flex_none()
                                    .text_color(t.muted_foreground.hsla())
                                    .child(label),
                            )
                            .child(div().flex_1().min_w_0().truncate().text_size(px(11.)).child(value))
                    })),
            )
        })
        .test_support()
        .into_any_element()
}
