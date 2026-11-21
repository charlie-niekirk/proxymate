//! The toolbar's rule tools: the Rules button (`RulesButton`), the No
//! Caching and Block Cookies toggles (`NoiseToggles`), network conditions
//! (`ThrottleMenu`) and the breakpoints badge (`BreakpointBadge`). Each
//! observes the rule book or the breakpoint queue it shows.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyView, App, Context, Entity, Render, SharedString, Subscription, Window, div, px};
use pm_model::{BreakpointInfo, RuleBook};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::rules;

use super::{OpenBreakpoints, OpenRules, ToggleNoCache, ToggleNoCookies, menus};
use crate::bridge::{self, stream::Latest};

pub const RULES_TIP: &str = "Rules: Map Local, rewrites, breakpoints, Map Remote, block";

/// The Rules button: opens the Rules window.
pub fn rules_button(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| RulesButton).into()
}

pub struct RulesButton;

impl Render for RulesButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        Button::new("rules-button")
            .ghost()
            .small()
            .accessibility_label("Rules")
            .tooltip(RULES_TIP)
            .child(pm_widgets::icon(IconName::ListChecks, t.foreground.hsla(), 15.))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenRules), cx))
    }
}

/// The noise toggles, network conditions and the breakpoints badge.
pub fn rule_tools(_: &mut Window, cx: &mut App) -> AnyView {
    let book = bridge::rule_book(cx);
    let breakpoints = bridge::breakpoints(cx);
    cx.new(|cx| RuleTools::new(book, breakpoints, cx)).into()
}

/// What the tools show, from the rule book and the queue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolState {
    pub no_cache: bool,
    pub no_cookies: bool,
    pub throttle: Option<rules::ThrottlePreset>,
    pub paused: usize,
}

impl ToolState {
    pub fn new(book: Option<&RuleBook>, queue: Option<&[BreakpointInfo]>) -> ToolState {
        let rule = |id: &str| book.and_then(|b| b.rules.iter().find(|r| r.id == id));
        ToolState {
            no_cache: rule(rules::NO_CACHE_RULE_ID).is_some_and(|r| r.enabled),
            no_cookies: rule(rules::NO_COOKIES_RULE_ID).is_some_and(|r| r.enabled),
            throttle: rules::active_throttle(rule(rules::THROTTLE_RULE_ID)),
            paused: queue.map_or(0, <[BreakpointInfo]>::len),
        }
    }

    pub fn breakpoints_tip(&self) -> String {
        if self.paused > 0 {
            format!("{} paused at a breakpoint", self.paused)
        } else {
            "Breakpoints · none paused".into()
        }
    }
}

pub struct RuleTools {
    book: Entity<Latest<RuleBook>>,
    breakpoints: Entity<Latest<Vec<BreakpointInfo>>>,
    _subscriptions: [Subscription; 2],
}

impl RuleTools {
    pub fn new(
        book: Entity<Latest<RuleBook>>,
        breakpoints: Entity<Latest<Vec<BreakpointInfo>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = [
            cx.observe(&book, |_, _, cx| cx.notify()),
            cx.observe(&breakpoints, |_, _, cx| cx.notify()),
        ];
        RuleTools {
            book,
            breakpoints,
            _subscriptions: subscriptions,
        }
    }

    pub fn state(&self, cx: &App) -> ToolState {
        ToolState::new(
            self.book.read(cx).value.as_ref(),
            self.breakpoints.read(cx).value.as_deref(),
        )
    }
}

/// A toggle: the icon alone when off; the icon and its label in the
/// warning colour when on.
fn toggle(
    id: &'static str,
    on: bool,
    label: &'static str,
    tip: &'static str,
    icon: IconName,
    action: impl gpui_kit::Action + Clone,
    t: &Tokens,
) -> Button {
    let colour = if on { t.s_4xx } else { t.muted_foreground };
    Button::new(id)
        .ghost()
        .small()
        .accessibility_label(if on {
            format!("{label}: on")
        } else {
            format!("{label}: off")
        })
        .tooltip(tip)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(pm_widgets::icon(icon, colour.hsla(), 15.))
                .when(on, |d| {
                    d.child(div().text_size(px(12.)).text_color(colour.hsla()).child(label))
                }),
        )
        .on_click(move |_, window, cx| window.dispatch_action(Box::new(action.clone()), cx))
}

impl Render for RuleTools {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let s = self.state(cx);
        let no_cache = toggle(
            "no-cache-toggle",
            s.no_cache,
            "No Caching",
            if s.no_cache {
                "No Caching is on: validators stripped, responses not stored · click to turn off"
            } else {
                "No Caching: strip If-None-Match, ETag and friends so every response is fresh"
            },
            IconName::ArchiveX,
            ToggleNoCache,
            &t,
        );
        let no_cookies = toggle(
            "no-cookies-toggle",
            s.no_cookies,
            "Block Cookies",
            if s.no_cookies {
                "Block Cookies is on: Cookie and Set-Cookie removed · click to turn off"
            } else {
                "Block Cookies: remove Cookie and Set-Cookie headers"
            },
            IconName::Cookie,
            ToggleNoCookies,
            &t,
        );
        let throttle_colour = if s.throttle.is_some() {
            t.s_4xx
        } else {
            t.muted_foreground
        };
        let active = s.throttle.clone();
        let throttle = Button::new("throttle-menu")
            .ghost()
            .small()
            .accessibility_label(SharedString::from(match &s.throttle {
                Some(p) => format!("Network conditions: {}", p.name),
                None => "Network conditions".into(),
            }))
            .tooltip("Network conditions")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(pm_widgets::icon(IconName::Gauge, throttle_colour.hsla(), 15.))
                    .when_some(s.throttle.as_ref(), |d, p| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(throttle_colour.hsla())
                                .child(p.name.clone()),
                        )
                    }),
            )
            .on_click(move |_, window, cx| {
                menus::throttle_menu(active.as_ref()).show(window.mouse_position(), window, cx);
            });
        let paused = s.paused;
        let bp_colour = if paused > 0 { t.s_paused } else { t.muted_foreground };
        let breakpoints = Button::new("breakpoint-badge")
            .ghost()
            .small()
            .accessibility_label(SharedString::from(s.breakpoints_tip()))
            .tooltip(SharedString::from(s.breakpoints_tip()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(pm_widgets::icon(IconName::OctagonPause, bp_colour.hsla(), 15.))
                    .when(paused > 0, |d| {
                        d.child(
                            div()
                                .px(px(6.))
                                .rounded_full()
                                .bg(t.s_paused_bg.hsla())
                                .text_color(t.s_paused.hsla())
                                .text_size(px(11.))
                                .font_family("Menlo")
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(paused.to_string()),
                        )
                    }),
            )
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenBreakpoints), cx));
        div()
            .id("rule-tools")
            .flex()
            .items_center()
            .gap(px(2.))
            .child(no_cache)
            .child(no_cookies)
            .child(throttle)
            .child(breakpoints)
    }
}
