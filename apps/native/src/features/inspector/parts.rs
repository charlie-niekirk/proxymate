//! The inspector's bars above the messages (`DetailPane.tsx`,
//! `RuleBadges.tsx`): the URL bar with its buttons and menu, the notes bar,
//! the OpenAPI operation line, the rule badges, the pinned banner, and the
//! failure banner with its fixes.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::SelectableText;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, FontWeight, HighlightStyle, MouseButton, MouseDownEvent, Role, SharedString, div, px,
};
use pm_model::{FlowDetail, FlowRules, FlowSummary, OperationMatch, RuleOrigin, SentBy, TlsStatus};
use pm_theme::Tokens;
use pm_view::errors::{FailureHelp, FixAction, explain_failure};
use pm_view::flow::{self, MethodTone};
use pm_widgets::flow_grid::row::{UrlPart, label_color, method_colors, status_colors};

use super::Inspector;
use super::actions::FlowRef;
use super::menus;
use crate::bridge::keys::{FlowKey, Inspected};
use crate::bridge::mutations;
use crate::features::chrome::{OpenCertWizard, OpenComment, OpenRule, OpenSettings, RewriteFlow};
use crate::workspace::Workspace;

/// Why the flow failed and what fixes it, unless the pinned banner says it.
pub fn failure_help(flow: &FlowSummary, detail: Option<&FlowDetail>) -> Option<FailureHelp> {
    if flow.tls == TlsStatus::LikelyPinned {
        return None;
    }
    explain_failure(flow, detail)
}

fn pill(label: impl Into<SharedString>, fg: pm_theme::Rgba, bg: Option<pm_theme::Rgba>) -> impl IntoElement {
    div()
        .flex_none()
        .h(px(18.))
        .px(px(6.))
        .flex()
        .items_center()
        .rounded(px(3.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg.hsla())
        .map(|d| match bg {
            Some(bg) => d.bg(bg.hsla()),
            None => d.border_1().border_color(fg.hsla()),
        })
        .child(label.into())
}

/// A small outlined tag (`HTTP/2`, `OpenAPI`).
fn tag(label: impl Into<SharedString>, t: &Tokens) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(4.))
        .rounded(px(3.))
        .border_1()
        .border_color(t.border.hsla())
        .text_size(px(10.5))
        .text_color(t.muted_foreground.hsla())
        .child(label.into())
}

/// The coloured URL as selectable text: blue host, green path (the bar's
/// tone).
fn url_text(flow: &FlowSummary, t: &Tokens, mono: SharedString) -> impl IntoElement {
    let parts = flow::url_parts(flow.scheme, &flow.host, flow.port, &flow.path, &flow.query);
    let mut url = String::new();
    let mut runs = Vec::new();
    for (text, part) in [
        (&parts.scheme, UrlPart::Scheme),
        (&parts.host, UrlPart::Host),
        (&parts.port, UrlPart::Port),
        (&parts.path, UrlPart::Path),
        (&parts.query, UrlPart::Query),
    ] {
        if text.is_empty() {
            continue;
        }
        let color = match part {
            UrlPart::Scheme => t.url_scheme,
            UrlPart::Host => t.url_bar_host,
            UrlPart::Port => t.url_port,
            UrlPart::Path => t.url_bar_path,
            UrlPart::Query => t.url_query,
        };
        runs.push((
            url.len()..url.len() + text.len(),
            HighlightStyle {
                color: Some(color.hsla()),
                font_weight: (part == UrlPart::Host).then_some(FontWeight::MEDIUM),
                ..Default::default()
            },
        ));
        url.push_str(text);
    }
    div()
        .min_w_0()
        .flex_1()
        .truncate()
        .font_family(mono)
        .text_size(px(12.))
        .child(SelectableText::new("url-bar-text", url).highlights(runs))
}

/// A sent-by badge (Replay, AI, Repeat) or the GraphQL operation.
fn badge(label: impl Into<SharedString>, icon: Option<IconName>, fg: pm_theme::Rgba, bg: pm_theme::Rgba) -> AnyElement {
    div()
        .flex_none()
        .h(px(18.))
        .px(px(5.))
        .flex()
        .items_center()
        .gap(px(3.))
        .rounded(px(3.))
        .bg(bg.hsla())
        .text_color(fg.hsla())
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .when_some(icon, |d, icon| d.child(pm_widgets::icon(icon, fg.hsla(), 11.)))
        .child(label.into())
        .into_any_element()
}

/// "Add to SSL bypass": stop decrypting the host from its next connection.
pub fn bypass_button(id: &'static str, host: &str, compact: bool, bypassed: bool) -> Button {
    let label: SharedString = if bypassed {
        "In SSL bypass".into()
    } else if compact {
        "Bypass SSL".into()
    } else {
        format!("Add {host} to SSL bypass").into()
    };
    let tip: SharedString = if bypassed {
        format!("{host} is no longer decrypted").into()
    } else {
        format!("Stop decrypting {host}: new connections are tunnelled as is").into()
    };
    let host = host.to_owned();
    Button::new(id)
        .map(|b| if compact { b.ghost() } else { b.outline() })
        .xsmall()
        .icon(IconName::ShieldOff)
        .label(label)
        .disabled(bypassed)
        .tooltip(tip)
        .on_click(move |_, _, cx| mutations::add_ssl_bypass(host.clone(), cx))
}

fn is_bypassed(host: &str, cx: &gpui_kit::App) -> bool {
    Workspace::global(cx).is_some_and(|w| w.read(cx).ssl.bypass.iter().any(|h| h.eq_ignore_ascii_case(host)))
}

fn cert_wizard_button(id: &'static str) -> Button {
    Button::new(id)
        .outline()
        .xsmall()
        .label("Certificate wizard…")
        .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenCertWizard), cx))
}

impl Inspector {
    /// Method, URL (sent and GraphQL badges), status, HTTP version,
    /// "duration · size", Rewrite… and Bypass SSL; right-click for its menu.
    pub(super) fn url_bar(
        &self,
        key: FlowKey,
        insp: &Arc<Inspected>,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let flow = &insp.flow;
        let detail = insp.detail.as_ref();
        let (mfg, mbg) = method_colors(t, MethodTone::of(&flow.method));
        let tone = flow::status_tone(flow.status, flow.state, flow.grpc_status);
        let (sfg, sbg) = status_colors(t, tone);
        let status = flow::status_label(flow.status, flow.state, flow.grpc_status);
        let sent = flow.sent_by.map(|by| match by {
            SentBy::Agent => badge("AI", Some(IconName::Bot), t.sent_agent, t.sent_agent_bg),
            SentBy::Repeat => badge("Repeat", Some(IconName::Repeat), t.sent, t.sent_bg),
            SentBy::User => badge("Replay", Some(IconName::Repeat2), t.sent, t.sent_bg),
        });
        let graphql = flow
            .graphql_op
            .as_deref()
            .map(|op| badge(op.to_owned(), None, t.graphql, t.graphql_bg));
        let stream = pm_view::inspector::stream_kind(flow, detail);
        let rewrite =
            detail.is_some() && &*flow.method != "CONNECT" && stream != Some(pm_view::inspector::StreamKind::WebSocket);
        let bypass = matches!(flow.tls, TlsStatus::Intercepted | TlsStatus::LikelyPinned);
        let flow_ref = FlowRef {
            store: key.store,
            id: key.id,
        };
        let a11y: SharedString = format!(
            "{} {} {status}",
            flow.method,
            pm_view::flow::flow_url(flow.scheme, &flow.host, flow.port, &flow.path, &flow.query)
        )
        .into();
        let insp_menu = insp.clone();
        div()
            .id("url-bar")
            .role(Role::Group)
            .aria_label(a11y)
            .aria_description("Right-click to copy the URL, its parts or the request as cURL")
            .flex_none()
            .h(px(32.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(t.border.hsla())
            .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                let menu = menus::url_bar(key.store, &insp_menu.flow, insp_menu.detail.as_ref());
                menu.show(e.position, window, cx);
            })
            .test_support()
            .child(pill(flow.method.to_string(), mfg, Some(mbg)))
            .children(sent)
            .children(graphql)
            .child(url_text(flow, t, {
                let app: &gpui_kit::App = cx;
                gpui_kit::component::ActiveTheme::theme(app).mono_font_family.clone()
            }))
            .child(pill(status, sfg, sbg))
            .child(tag(flow.http_version.as_str(), t))
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(t.muted_foreground.hsla())
                    .child(format!(
                        "{} · {}",
                        flow::format_duration(flow.duration_ms),
                        flow::format_size(flow.resp_size)
                    )),
            )
            .when(rewrite, |d| {
                d.child(
                    Button::new("inspector-rewrite")
                        .ghost()
                        .xsmall()
                        .icon(IconName::PencilLine)
                        .label("Rewrite…")
                        .tooltip("Edit this flow and save the changes as a rule")
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(RewriteFlow {
                                    store: flow_ref.store,
                                    id: flow_ref.id,
                                }),
                                cx,
                            )
                        }),
                )
            })
            .when(bypass, |d| {
                d.child(bypass_button(
                    "bypass-ssl",
                    &flow.host,
                    true,
                    is_bypassed(&flow.host, cx),
                ))
            })
    }

    /// The label chip and comment; a click opens the comment dialog (G11).
    pub(super) fn notes_bar(&self, key: FlowKey, flow: &FlowSummary, t: &Tokens) -> Option<AnyElement> {
        if flow.label.is_none() && flow.comment.is_none() {
            return None;
        }
        let label = flow.label.map(|l| {
            let name = format!("{l:?}");
            let bg = match l {
                pm_model::FlowLabel::Red => t.label_red_bg,
                pm_model::FlowLabel::Orange => t.label_orange_bg,
                pm_model::FlowLabel::Yellow => t.label_yellow_bg,
                pm_model::FlowLabel::Green => t.label_green_bg,
                pm_model::FlowLabel::Blue => t.label_blue_bg,
                pm_model::FlowLabel::Purple => t.label_purple_bg,
            };
            div()
                .flex_none()
                .px(px(6.))
                .rounded(px(3.))
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .bg(bg.hsla())
                .text_color(label_color(t, l).hsla())
                .child(name)
        });
        let comment = flow.comment.clone();
        Some(
            div()
                .id("flow-notes")
                .role(Role::Button)
                .aria_label("Edit the comment")
                .flex_none()
                .h(px(24.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .border_b_1()
                .border_color(t.border.hsla())
                .text_size(px(12.))
                .cursor_pointer()
                .hover(|s| s.bg(t.accent.hsla()))
                .children(label)
                .when_some(comment, |d, c| {
                    d.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(t.muted_foreground.hsla())
                            .child(SelectableText::new("flow-comment", c.to_string())),
                    )
                })
                .on_click(move |_, window, cx| {
                    window.dispatch_action(
                        Box::new(OpenComment {
                            store: key.store,
                            id: key.id,
                        }),
                        cx,
                    )
                })
                .test_support()
                .into_any_element(),
        )
    }

    /// The pinned banner: the client rejected our certificate.
    pub(super) fn pinned_banner(&self, flow: &FlowSummary, t: &Tokens, cx: &Context<Self>) -> Option<AnyElement> {
        if flow.tls != TlsStatus::LikelyPinned {
            return None;
        }
        Some(
            div()
                .id("pinned-banner")
                .role(Role::Alert)
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(6.))
                .border_b_1()
                .border_color(t.border.hsla())
                .bg(t.ssl_pinned_bg.hsla())
                .text_size(px(12.))
                .child(pm_widgets::icon(IconName::ShieldAlert, t.ssl_pinned.hsla(), 14.))
                .child(div().flex_1().min_w_0().child(
                    "The client rejected Proxymate's certificate: it pins its certificates, or doesn't trust the Proxymate CA yet.",
                ))
                .child(cert_wizard_button("pinned-cert-wizard"))
                .child(bypass_button(
                    "bypass-ssl-banner",
                    &flow.host,
                    false,
                    is_bypassed(&flow.host, cx),
                ))
                .test_support()
                .into_any_element(),
        )
    }

    /// Why the flow failed, in a line, with the fixes that apply;
    /// right-click copies the error.
    pub(super) fn failure_banner(
        &self,
        flow: &FlowSummary,
        help: FailureHelp,
        t: &Tokens,
        cx: &Context<Self>,
    ) -> AnyElement {
        let host = flow.host.to_string();
        let insecure = Workspace::global(cx).is_some_and(|w| w.read(cx).is_insecure(&host));
        let bypassed = is_bypassed(&host, cx);
        let menu_help = help.clone();
        let fixes = help.actions.iter().map(|action| -> AnyElement {
            let settings = |id: &'static str, label: &'static str, page: &'static str| {
                Button::new(id)
                    .outline()
                    .xsmall()
                    .label(label)
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(OpenSettings { page: page.into() }), cx)
                    })
                    .into_any_element()
            };
            match action {
                FixAction::CertWizard => cert_wizard_button("fix-cert-wizard").into_any_element(),
                FixAction::Bypass => bypass_button("fix-bypass", &host, true, bypassed).into_any_element(),
                FixAction::InsecureUpstream => {
                    let host = host.clone();
                    Button::new("fix-insecure-upstream")
                        .outline()
                        .xsmall()
                        .label(if insecure {
                            "Verify upstream again"
                        } else {
                            "Allow insecure upstream"
                        })
                        .on_click(move |_, _, cx| mutations::set_insecure_upstream(host.clone(), !insecure, cx))
                        .into_any_element()
                }
                FixAction::Timeouts => settings("fix-timeouts", "Timeouts…", "advanced"),
                FixAction::NetworkSettings => settings("fix-network", "Network settings…", "network"),
                FixAction::TlsSettings => settings("fix-tls", "SSL Proxying…", "ssl"),
            }
        });
        div()
            .id("failure-banner")
            .role(Role::Alert)
            .aria_label(SharedString::from(help.title.clone()))
            .flex_none()
            .flex()
            .items_start()
            .gap(px(8.))
            .px(px(8.))
            .py(px(6.))
            .border_b_1()
            .border_color(t.border.hsla())
            .bg(t.s_5xx_bg.hsla())
            .text_size(px(12.))
            .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                menus::failure(&menu_help).show(e.position, window, cx);
            })
            .child(
                div()
                    .pt(px(1.))
                    .child(pm_widgets::icon(IconName::CircleAlert, t.s_failed.hsla(), 14.)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.s_failed.hsla())
                            .child(SelectableText::new("failure-title", help.title.clone())),
                    )
                    .child(
                        div()
                            .text_color(t.muted_foreground.hsla())
                            .child(SelectableText::new("failure-detail", help.detail.clone())),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(4.))
                    .children(fixes),
            )
            .test_support()
            .into_any_element()
    }
}

/// The imported OpenAPI operation the flow's method and path match (M20).
pub fn operation_line(op: &OperationMatch, t: &Tokens) -> AnyElement {
    let tip: SharedString = format!("{} {} in {}", op.method, op.path, op.spec_name).into();
    div()
        .id("flow-operation")
        .role(Role::Note)
        .aria_label(tip.clone())
        .flex_none()
        .h(px(24.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(t.border.hsla())
        .text_size(px(12.))
        .child(tag("OpenAPI", t))
        .when_some(op.operation_id.clone(), |d, id| {
            d.child(div().flex_none().font_weight(FontWeight::MEDIUM).child(id))
        })
        .when_some(op.summary.clone(), |d, s| {
            d.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(t.muted_foreground.hsla())
                    .child(s),
            )
        })
        .child(
            div()
                .ml_auto()
                .flex_none()
                .text_size(px(11.))
                .text_color(t.muted_foreground.hsla())
                .child(format!("{} · {}", op.path, op.spec_name)),
        )
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
        .into_any_element()
}

/// What rules did to the flow: a badge per applied rule (click opens it,
/// right-click for its menu), and why a body was left alone, a timed-out
/// breakpoint or a mocked response. Script failures are G10's.
pub fn rule_badges(rules: &FlowRules, t: &Tokens) -> Option<AnyElement> {
    let notes: Vec<String> = [
        rules
            .mocked
            .then(|| "Mocked: the response didn't come from the upstream.".to_owned()),
        rules
            .breakpoint_timeout
            .then(|| "A breakpoint timed out; the flow continued unmodified.".to_owned()),
        rules.rewrite_skipped.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if rules.applied.is_empty() && notes.is_empty() {
        return None;
    }
    let badges = rules.applied.iter().enumerate().map(|(i, r)| {
        let client = match &r.origin {
            RuleOrigin::Agent { client, .. } => Some(client.clone()),
            _ => None,
        };
        let (fg, bg) = if client.is_some() {
            (t.rule_agent, t.rule_agent_bg)
        } else {
            (t.rule, t.rule_bg)
        };
        let phase = menus::phase_short(r.phase);
        let tip: SharedString = format!(
            "{} changed the {}{}",
            r.name,
            match r.phase {
                pm_model::Phase::Request => "request",
                pm_model::Phase::Response => "response",
            },
            client
                .as_ref()
                .map(|c| format!(" · created by {c}"))
                .unwrap_or_default()
        )
        .into();
        let rule = r.clone();
        let id: SharedString = r.rule_id.to_string().into();
        div()
            .id(("rule-badge", i))
            .role(Role::Button)
            .aria_label(tip.clone())
            .h(px(20.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(3.))
            .bg(bg.hsla())
            .text_color(fg.hsla())
            .cursor_pointer()
            .when_some(client, |d, c| {
                d.child(pm_widgets::icon(IconName::Bot, fg.hsla(), 12.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("AI · {c}")))
            })
            .child(r.name.clone())
            .child(div().opacity(0.6).child(phase))
            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
            .on_click(move |_, window, cx| window.dispatch_action(Box::new(OpenRule { id: id.clone() }), cx))
            .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                menus::rule_badge(&rule).show(e.position, window, cx);
            })
            .test_support()
    });
    Some(
        div()
            .id("rule-badges")
            .role(Role::Group)
            .aria_label("Rules applied to this flow")
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .py(px(4.))
            .border_b_1()
            .border_color(t.border.hsla())
            .text_size(px(12.))
            .child(pm_widgets::icon(IconName::PencilLine, t.rule.hsla(), 12.))
            .child(div().mr(px(4.)).text_color(t.muted_foreground.hsla()).child("Rules"))
            .children(badges)
            .children(notes.into_iter().map(|n| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(t.muted_foreground.hsla())
                    .child(pm_widgets::icon(IconName::TriangleAlert, t.s_4xx.hsla(), 12.))
                    .child(n)
            }))
            .into_any_element(),
    )
}

/// Under a passthrough flow's Response tabs: it wasn't decrypted.
pub fn tunnel_note(flow: &FlowSummary, t: &Tokens) -> AnyElement {
    div()
        .id("tunnel-note")
        .flex_none()
        .px(px(8.))
        .py(px(6.))
        .border_b_1()
        .border_color(t.border.hsla())
        .text_size(px(12.))
        .text_color(t.muted_foreground.hsla())
        .child(format!(
            "Not decrypted: {} is in the SSL exclude or bypass list. {} sent, {} received through the tunnel.",
            flow.host,
            flow::format_size(flow.req_size),
            flow::format_size(flow.resp_size)
        ))
        .into_any_element()
}
