//! The toolbar's MCP indicator (`Settings/McpIndicator.tsx`): a robot whose
//! colour says whether the server is off, failing, serving or has agents
//! connected (with their count); its tooltip names them. A click opens
//! Settings › MCP (G12's window).

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyView, App, Context, Entity, FontWeight, SharedString, Subscription, Window, div, px};
use pm_model::McpStatus;
use pm_theme::kit::TokensExt as _;
use pm_view::integration::{self as view, McpTone};

use crate::bridge::{self, stream::Latest};
use crate::features::chrome::OpenSettings;

pub struct McpIndicator {
    status: Entity<Latest<McpStatus>>,
    _subscription: Subscription,
}

pub fn mcp_indicator(_: &mut Window, cx: &mut App) -> AnyView {
    let status = bridge::mcp(cx);
    cx.new(|cx| McpIndicator {
        _subscription: cx.observe(&status, |_, _, cx| cx.notify()),
        status,
    })
    .into()
}

impl Render for McpIndicator {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let status = self.status.read(cx).value.clone();
        let tone = view::mcp_tone(status.as_ref());
        let count = status.as_ref().map_or(0, |s| s.clients.len());
        let (colour, opacity) = match tone {
            McpTone::Off => (t.muted_foreground, 0.6),
            McpTone::Error => (t.s_5xx, 1.0),
            McpTone::Serving => (t.muted_foreground, 1.0),
            McpTone::Clients => (t.rule_agent, 1.0),
        };
        Button::new("mcp-indicator")
            .ghost()
            .small()
            .accessibility_label(SharedString::from(view::mcp_label(status.as_ref())))
            .tooltip(SharedString::from(view::mcp_tip(status.as_ref())))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .opacity(opacity)
                    .child(pm_widgets::icon(IconName::Bot, colour.hsla(), 15.))
                    .when(count > 0, |d| {
                        d.child(
                            div()
                                .id("mcp-client-count")
                                .px(px(6.))
                                .rounded_full()
                                .bg(t.rule_agent_bg.hsla())
                                .text_color(t.rule_agent.hsla())
                                .text_size(px(11.))
                                .font_family("Menlo")
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(count.to_string())
                                .test_support(),
                        )
                    }),
            )
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings { page: "mcp".into() }), cx))
    }
}
