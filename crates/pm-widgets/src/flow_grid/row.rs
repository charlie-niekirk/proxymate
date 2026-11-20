//! One flow table row, prepared once and drawn every frame. The text of
//! every cell (sizes, times, the URL and its colour runs, pill labels) is
//! formatted when the row's page lands, on the background executor, so a
//! frame only lays rows out: this is the native table's "memoised row".

use std::ops::Range;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, FontWeight, HighlightStyle, Hsla, ImageSource, Role, SharedString,
    StyledText, Window, div, img, pulsating_between, px, svg,
};
use pm_model::{FlowLabel, FlowRow, FlowState, SentBy, TlsStatus};
use pm_theme::Tokens;
use pm_view::columns::{Align, Builtin, ColumnId, PlacedColumn, ROW_HEIGHT};
use pm_view::flow::{self, MethodTone, StatusTone};
use pm_view::row_cache::CachedRow;

/// The URL's parts, which take their own colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlPart {
    Scheme,
    Host,
    Port,
    Path,
    Query,
}

/// A flow row with every cell's text ready to draw.
#[derive(Clone, Debug)]
pub struct GridRow {
    /// The engine's row, for menus and the inspector.
    pub row: FlowRow,
    /// `flow-row-<id>`: the row's element id (the dev remote and tests find
    /// rows by it).
    pub element_id: SharedString,
    pub number: SharedString,
    pub url: SharedString,
    pub url_runs: Vec<(Range<usize>, UrlPart)>,
    pub method: SharedString,
    pub method_tone: MethodTone,
    pub status: SharedString,
    pub status_tone: StatusTone,
    pub client: Option<SharedString>,
    pub time: SharedString,
    pub duration: SharedString,
    pub req: SharedString,
    pub resp: SharedString,
    pub proto: SharedString,
    pub graphql: Option<SharedString>,
    pub comment: Option<SharedString>,
    pub extra: Vec<Option<SharedString>>,
    /// What a screen reader reads for the row.
    pub a11y_label: SharedString,
}

impl CachedRow for GridRow {
    fn flow_id(&self) -> pm_model::FlowId {
        self.row.0
    }
}

impl GridRow {
    pub fn new(row: FlowRow) -> GridRow {
        let FlowRow(
            id,
            state,
            scheme,
            ref method,
            ref host,
            port,
            ref path,
            ref query,
            status,
            _kind,
            ref client,
            started_at_ms,
            duration_ms,
            req_size,
            resp_size,
            _edited,
            http_version,
            _tls,
            ref graphql_op,
            _sent_by,
            _pinned,
            grpc_status,
            _label,
            ref comment,
            ref extra,
        ) = row;
        let parts = flow::url_parts(scheme, host, port, path, query);
        let mut url = String::with_capacity(64);
        let mut url_runs = Vec::with_capacity(5);
        for (text, part) in [
            (&parts.scheme, UrlPart::Scheme),
            (&parts.host, UrlPart::Host),
            (&parts.port, UrlPart::Port),
            (&parts.path, UrlPart::Path),
            (&parts.query, UrlPart::Query),
        ] {
            if !text.is_empty() {
                url_runs.push((url.len()..url.len() + text.len(), part));
                url.push_str(text);
            }
        }
        let status_text = flow::status_label(status, state, grpc_status);
        let a11y_label = format!("{} {method} {url} {status_text}", id + 1).into();
        GridRow {
            element_id: format!("flow-row-{id}").into(),
            number: (id as u64 + 1).to_string().into(),
            url: url.into(),
            url_runs,
            method: SharedString::from(method.to_string()),
            method_tone: MethodTone::of(method),
            status: status_text.into(),
            status_tone: flow::status_tone(status, state, grpc_status),
            client: client.as_deref().map(|c| SharedString::from(c.to_owned())),
            time: flow::format_time(Some(started_at_ms)).into(),
            duration: flow::format_duration(duration_ms).into(),
            req: flow::format_size(req_size).into(),
            resp: flow::format_size(resp_size).into(),
            proto: flow::version_label(http_version).into(),
            graphql: graphql_op.as_deref().map(|g| SharedString::from(g.to_owned())),
            comment: comment.as_deref().map(|c| SharedString::from(c.to_owned())),
            extra: extra
                .iter()
                .map(|v| v.as_deref().map(|s| SharedString::from(s.to_owned())))
                .collect(),
            a11y_label,
            row,
        }
    }

    pub fn id(&self) -> pm_model::FlowId {
        self.row.0
    }

    pub fn state(&self) -> FlowState {
        self.row.1
    }
}

pub fn method_colors(t: &Tokens, tone: MethodTone) -> (pm_theme::Rgba, pm_theme::Rgba) {
    match tone {
        MethodTone::Get => (t.m_get, t.m_get_bg),
        MethodTone::Post => (t.m_post, t.m_post_bg),
        MethodTone::Put => (t.m_put, t.m_put_bg),
        MethodTone::Patch => (t.m_patch, t.m_patch_bg),
        MethodTone::Delete => (t.m_delete, t.m_delete_bg),
        MethodTone::Head => (t.m_head, t.m_head_bg),
        MethodTone::Options => (t.m_options, t.m_options_bg),
        MethodTone::Connect => (t.m_connect, t.m_connect_bg),
        MethodTone::Other => (t.m_other, t.m_other_bg),
    }
}

/// A status pill's colours; `None` for the background of the outlined
/// Failed pill.
pub fn status_colors(t: &Tokens, tone: StatusTone) -> (pm_theme::Rgba, Option<pm_theme::Rgba>) {
    match tone {
        StatusTone::S1xx => (t.s_1xx, Some(t.s_1xx_bg)),
        StatusTone::S2xx => (t.s_2xx, Some(t.s_2xx_bg)),
        StatusTone::S3xx => (t.s_3xx, Some(t.s_3xx_bg)),
        StatusTone::S4xx => (t.s_4xx, Some(t.s_4xx_bg)),
        StatusTone::S5xx => (t.s_5xx, Some(t.s_5xx_bg)),
        StatusTone::Active => (t.s_active, Some(t.s_active_bg)),
        StatusTone::Paused => (t.s_paused, Some(t.s_paused_bg)),
        StatusTone::Blocked => (t.s_blocked, Some(t.s_blocked_bg)),
        StatusTone::Tunnel => (t.s_tunnel, Some(t.s_tunnel_bg)),
        StatusTone::Failed => (t.s_failed, None),
    }
}

pub fn label_color(t: &Tokens, label: FlowLabel) -> pm_theme::Rgba {
    match label {
        FlowLabel::Red => t.label_red,
        FlowLabel::Orange => t.label_orange,
        FlowLabel::Yellow => t.label_yellow,
        FlowLabel::Green => t.label_green,
        FlowLabel::Blue => t.label_blue,
        FlowLabel::Purple => t.label_purple,
    }
}

fn state_dot_color(t: &Tokens, state: FlowState) -> pm_theme::Rgba {
    match state {
        FlowState::Active => t.s_active,
        FlowState::Paused => t.s_paused,
        FlowState::Failed => t.s_failed,
        FlowState::Blocked => t.s_blocked,
        FlowState::Tunnel => t.s_tunnel,
        FlowState::Completed => t.s_2xx,
    }
}

/// How a row is drawn this frame.
#[derive(Clone, Copy)]
pub struct RowLook {
    pub selected: bool,
    /// The focused row of a multi-selection (an inset outline).
    pub focused: bool,
    /// The focused flow's row: the grid's active descendant.
    pub current: bool,
    pub odd: bool,
}

/// Everything a cell needs from outside the row.
pub struct CellContext<'a> {
    pub t: &'a Tokens,
    pub look: RowLook,
    /// The client's icon, once loaded.
    pub icon: Option<ImageSource>,
    /// The UI font, for the cells that don't use the table's monospace one.
    pub sans: SharedString,
}

impl CellContext<'_> {
    fn fg(&self) -> Hsla {
        if self.look.selected {
            self.t.selection_fg.hsla()
        } else {
            self.t.foreground.hsla()
        }
    }

    /// Secondary text: muted, or the selection's foreground.
    fn dim(&self) -> Hsla {
        if self.look.selected {
            self.t.selection_fg.hsla()
        } else {
            self.t.muted_foreground.hsla()
        }
    }

    /// A token colour that turns into the selection's foreground on a
    /// selected row.
    fn tint(&self, c: pm_theme::Rgba) -> Hsla {
        if self.look.selected {
            self.t.selection_fg.hsla()
        } else {
            c.hsla()
        }
    }
}

/// A pill in a row: on a selected row it takes `--pill-on-selection`.
fn pill(label: SharedString, fg: Hsla, bg: Option<Hsla>, outline: Option<Hsla>, cx: &CellContext) -> AnyElement {
    let (fg, bg) = if cx.look.selected {
        (cx.t.selection_fg.hsla(), Some(cx.t.pill_on_selection.hsla()))
    } else {
        (fg, bg)
    };
    div()
        .flex_shrink(1.)
        .min_w_0()
        .max_w_full()
        .h(px(16.))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .overflow_hidden()
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg)
        .when_some(bg, |d, bg| d.bg(bg))
        .when_some(outline.filter(|_| !cx.look.selected), |d, c| {
            d.border_1().border_color(c)
        })
        .child(div().truncate().child(label))
        .into_any_element()
}

fn icon(name: gpui_kit::assets::IconName, color: Hsla, size: f32) -> AnyElement {
    svg()
        .path(name.path())
        .flex_none()
        .size(px(size))
        .text_color(color)
        .into_any_element()
}

/// Fades in and out while a flow is active.
fn pulse(content: AnyElement, key: &'static str, active: bool) -> AnyElement {
    if !active {
        return content;
    }
    div()
        .flex()
        .items_center()
        .child(content)
        .with_animation(
            key,
            Animation::new(Duration::from_millis(2000))
                .repeat()
                .with_easing(pulsating_between(0.4, 1.0)),
            |d, delta| d.opacity(delta),
        )
        .into_any_element()
}

/// The cell's tooltip, where the Tauri table has one.
fn tooltip(row: &GridRow, column: &PlacedColumn) -> Option<SharedString> {
    let r = &row.row;
    match &column.id {
        ColumnId::Header(_) => column.header.and_then(|i| row.extra.get(i).cloned().flatten()),
        ColumnId::Builtin(b) => match b {
            Builtin::State => Some(flow::state_name(r.1).into()),
            Builtin::Client => row.client.clone(),
            Builtin::Status => {
                r.21.zip(r.8)
                    .map(|(code, http)| format!("gRPC {} ({code}) · HTTP {http}", flow::grpc_name(code)).into())
            }
            Builtin::Proto => Some(r.16.as_str().into()),
            Builtin::Ssl => Some(flow::ssl_title(r.17)).filter(|t| !t.is_empty()).map(Into::into),
            Builtin::Notes => row.comment.clone(),
            _ => None,
        },
    }
}

/// One cell of `row` in `column` (the row's `ix`th).
pub fn cell(ix: usize, row: &GridRow, column: &PlacedColumn, cx: &CellContext) -> AnyElement {
    let t = cx.t;
    let base = div()
        .h_full()
        .flex()
        .items_center()
        .px(px(6.))
        .overflow_hidden()
        .when(column.grow, |d| d.flex_1().min_w(px(column.px)))
        .when(!column.grow, |d| d.flex_none().w(px(column.px)))
        .map(|d| match column.align {
            Align::Left => d,
            Align::Right => d.justify_end(),
            Align::Center => d.justify_center(),
        });
    let r = &row.row;
    let content: AnyElement = match &column.id {
        ColumnId::Header(_) => {
            let value = column.header.and_then(|i| row.extra.get(i).cloned().flatten());
            let color = if value.is_some() { cx.fg() } else { cx.dim() };
            div()
                .truncate()
                .text_color(color)
                .child(value.unwrap_or_else(|| "—".into()))
                .into_any_element()
        }
        ColumnId::Builtin(b) => match b {
            Builtin::Id => div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_color(cx.dim())
                .when(r.20, |d| {
                    d.child(icon(gpui_kit::assets::IconName::Pin, cx.tint(t.primary), 10.))
                })
                .child(row.number.clone())
                .into_any_element(),
            Builtin::State => {
                let mut color = state_dot_color(t, r.1).hsla();
                if r.1 == FlowState::Completed {
                    color.a *= 0.4;
                }
                if cx.look.selected {
                    color = t.selection_fg.hsla();
                }
                let dot = div().size(px(6.)).rounded_full().bg(color).into_any_element();
                pulse(dot, "pulse-state", r.1 == FlowState::Active)
            }
            Builtin::Url => url_cell(row, cx),
            Builtin::Client => div()
                .flex()
                .items_center()
                .gap(px(6.))
                .min_w_0()
                .font_family(cx.sans.clone())
                .text_size(px(12.))
                .text_color(if row.client.is_some() { cx.fg() } else { cx.dim() })
                .when_some(cx.icon.clone(), |d, src| d.child(img(src).flex_none().size(px(14.))))
                .child(div().truncate().child(row.client.clone().unwrap_or_else(|| "—".into())))
                .into_any_element(),
            Builtin::Method => {
                let (fg, bg) = method_colors(t, row.method_tone);
                pill(row.method.clone(), fg.hsla(), Some(bg.hsla()), None, cx)
            }
            Builtin::Status => {
                let (fg, bg) = status_colors(t, row.status_tone);
                let outline = bg.is_none().then(|| fg.hsla());
                let status = pill(row.status.clone(), fg.hsla(), bg.map(|b| b.hsla()), outline, cx);
                pulse(status, "pulse-status", row.status_tone == StatusTone::Active)
            }
            Builtin::Time => div().text_color(cx.dim()).child(row.time.clone()).into_any_element(),
            Builtin::Duration => div().text_color(cx.fg()).child(row.duration.clone()).into_any_element(),
            Builtin::Req => div().text_color(cx.dim()).child(row.req.clone()).into_any_element(),
            Builtin::Resp => div().text_color(cx.fg()).child(row.resp.clone()).into_any_element(),
            Builtin::Proto => div().text_color(cx.dim()).child(row.proto.clone()).into_any_element(),
            Builtin::Ssl => ssl_cell(r.17, cx),
            Builtin::Notes => div()
                .flex()
                .items_center()
                .gap(px(4.))
                .when_some(r.22, |d, label| {
                    d.child(
                        div()
                            .size(px(8.))
                            .rounded_full()
                            .bg(label_color(t, label).hsla())
                            .when(cx.look.selected, |d| d.border_1().border_color(t.selection_fg.hsla())),
                    )
                })
                .when(row.comment.is_some(), |d| {
                    d.child(icon(gpui_kit::assets::IconName::MessageSquareText, cx.dim(), 12.))
                })
                .when(r.15, |d| {
                    d.child(icon(gpui_kit::assets::IconName::PencilLine, cx.tint(t.rule), 12.))
                })
                .into_any_element(),
            // The row's "…" (its flow menu) arrives with the menu in G3.
            Builtin::Tools => div().into_any_element(),
        },
    };
    match tooltip(row, column) {
        Some(text) => base
            .id(("cell", ix))
            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx))
            .child(content)
            .into_any_element(),
        None => base.child(content).into_any_element(),
    }
}

fn ssl_cell(tls: TlsStatus, cx: &CellContext) -> AnyElement {
    use gpui_kit::assets::IconName;
    match tls {
        TlsStatus::None => div().into_any_element(),
        TlsStatus::Intercepted => icon(IconName::Lock, cx.dim(), 12.),
        TlsStatus::Passthrough => icon(IconName::LockOpen, cx.tint(cx.t.ssl_passthrough), 12.),
        TlsStatus::LikelyPinned => icon(IconName::ShieldAlert, cx.tint(cx.t.ssl_pinned), 12.),
    }
}

/// The coloured URL, after the Replay/AI/Repeat and GraphQL badges.
fn url_cell(row: &GridRow, cx: &CellContext) -> AnyElement {
    use gpui_kit::assets::IconName;
    let t = cx.t;
    let color = |part: UrlPart| -> Hsla {
        if cx.look.selected {
            return t.selection_fg.hsla();
        }
        match part {
            UrlPart::Scheme => t.url_scheme,
            UrlPart::Host => t.url_host,
            UrlPart::Port => t.url_port,
            UrlPart::Path => t.url_path,
            UrlPart::Query => t.url_query,
        }
        .hsla()
    };
    let runs = row.url_runs.iter().map(|(range, part)| {
        let weight = (*part == UrlPart::Host).then_some(FontWeight::MEDIUM);
        (
            range.clone(),
            HighlightStyle {
                color: Some(color(*part)),
                font_weight: weight,
                ..Default::default()
            },
        )
    });
    let badge = |label: &'static str, icon_name: IconName, fg: pm_theme::Rgba, bg: pm_theme::Rgba| {
        let (fg, bg) = if cx.look.selected {
            (t.selection_fg.hsla(), t.pill_on_selection.hsla())
        } else {
            (fg.hsla(), bg.hsla())
        };
        div()
            .flex_none()
            .h(px(16.))
            .px(px(4.))
            .flex()
            .items_center()
            .gap(px(2.))
            .rounded(px(3.))
            .bg(bg)
            .text_color(fg)
            .font_family(cx.sans.clone())
            .text_size(px(10.5))
            .font_weight(FontWeight::SEMIBOLD)
            .child(icon(icon_name, fg, 10.))
            .child(label)
    };
    let sent = row.row.19.map(|by| match by {
        SentBy::Agent => badge("AI", IconName::Bot, t.sent_agent, t.sent_agent_bg),
        SentBy::Repeat => badge("Repeat", IconName::Repeat, t.sent, t.sent_bg),
        SentBy::User => badge("Replay", IconName::Repeat2, t.sent, t.sent_bg),
    });
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .min_w_0()
        .overflow_hidden()
        .when_some(sent, |d, b| d.child(b))
        .when_some(row.graphql.clone(), |d, op| {
            // The URL truncates first; the badge keeps its width.
            d.child(
                div()
                    .flex_none()
                    .child(pill(op, t.graphql.hsla(), Some(t.graphql_bg.hsla()), None, cx)),
            )
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .child(StyledText::new(row.url.clone()).with_highlights(runs)),
        )
        .into_any_element()
}

/// The row: its cells, or a placeholder while its page loads.
pub fn row_element(
    index: usize,
    row: Option<&GridRow>,
    columns: &[PlacedColumn],
    cx: &CellContext,
    _window: &mut Window,
    _app: &mut App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let t = cx.t;
    let look = cx.look;
    let bg = if look.selected {
        Some(t.selection_bg.hsla())
    } else if look.odd {
        Some(t.pane_row_alt.hsla())
    } else {
        None
    };
    let id: SharedString = match row {
        Some(r) => r.element_id.clone(),
        None => format!("flow-row-pending-{index}").into(),
    };
    let base = div()
        .id(id)
        .role(Role::Row)
        .aria_row_index(index + 2)
        .aria_selected(look.selected)
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .text_size(px(11.5))
        .when_some(bg, |d, bg| d.bg(bg))
        .when(!look.selected, |d| d.hover(|s| s.bg(t.pane_row_hover.hsla())))
        .relative()
        // An inset outline that takes no room from the cells.
        .when(look.focused, |d| {
            d.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .border_1()
                    .border_color(t.selection_fg.hsla()),
            )
        });
    match row {
        Some(row) => base
            .aria_label(row.a11y_label.clone())
            .when(look.current, |d| d.aria_active_descendant())
            .children(columns.iter().enumerate().map(|(ix, c)| cell(ix, row, c, cx))),
        None => base.aria_label("Loading").child(
            div()
                .px(px(8.))
                .text_color(t.muted_foreground.opacity(0.4).hsla())
                .child("…"),
        ),
    }
}

trait Opacity {
    fn opacity(self, a: f32) -> Self;
}

impl Opacity for pm_theme::Rgba {
    fn opacity(mut self, a: f32) -> Self {
        self.a *= a;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::{ContentKind, HttpVersion, Scheme};

    pub(crate) fn flow_row(id: u32) -> FlowRow {
        FlowRow(
            id,
            FlowState::Completed,
            Scheme::Https,
            "GET".into(),
            "api.example.com".into(),
            8443,
            "/v1/users".into(),
            "page=2".into(),
            Some(404),
            ContentKind::Json,
            Some("Safari".into()),
            1_700_000_000_000.0,
            Some(1500),
            0,
            1536,
            true,
            HttpVersion::Http2,
            TlsStatus::Intercepted,
            Some("GetUser".into()),
            Some(SentBy::Agent),
            true,
            None,
            Some(FlowLabel::Red),
            Some("check".into()),
            vec![Some("req-1".into()), None],
        )
    }

    #[test]
    fn a_row_is_formatted_once_with_its_url_runs() {
        let row = GridRow::new(flow_row(6));
        assert_eq!(row.number.as_ref(), "7");
        assert_eq!(row.url.as_ref(), "https://api.example.com:8443/v1/users?page=2");
        let parts: Vec<(&str, UrlPart)> = row.url_runs.iter().map(|(r, p)| (&row.url[r.clone()], *p)).collect();
        assert_eq!(
            parts,
            [
                ("https://", UrlPart::Scheme),
                ("api.example.com", UrlPart::Host),
                (":8443", UrlPart::Port),
                ("/v1/users", UrlPart::Path),
                ("?page=2", UrlPart::Query),
            ]
        );
        assert_eq!(
            (row.status.as_ref(), row.status_tone, row.method_tone),
            ("404", StatusTone::S4xx, MethodTone::Get)
        );
        assert_eq!(
            (row.duration.as_ref(), row.resp.as_ref(), row.req.as_ref()),
            ("1.50 s", "1.5 KB", "—")
        );
        assert_eq!(row.proto.as_ref(), "h2");
        assert_eq!(row.extra, vec![Some("req-1".into()), None]);
        assert_eq!(row.element_id.as_ref(), "flow-row-6");
        assert_eq!(
            row.a11y_label.as_ref(),
            "7 GET https://api.example.com:8443/v1/users?page=2 404"
        );
    }
}
