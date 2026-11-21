//! The filter bar (`components/FilterBar.tsx`): protocol chips (one at a
//! time), content chips (several; All clears them) and the DSL search box
//! with autocomplete (`pm_view::dsl`): keys with their help, then values
//! (known hosts and `*.domain`s and apps from the sidebar snapshot, fixed
//! lists for the rest). ↓ opens the list, ↑/↓ pick, Tab or ↩ accept,
//! Escape closes it; Escape or ↩ with no list goes back to the flow table.
//!
//! Chips refilter at once; typing is debounced by the table (120 ms). While
//! the query has header or body terms the box shows the deep-search icon,
//! and the deep search's percentage while it runs.

use gpui_kit::Focusable as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FontWeight, MouseButton, Role, SharedString, Subscription, Toggled, Window, deferred, div, px,
};
use pm_model::{ContentGroup, ProtocolFilter, SidebarSnapshot};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::dsl::{self, DslContext, Suggestion};

use crate::bridge::stream::Latest;
use crate::features::flow_table::FlowTable;
use crate::workspace::Workspace;

pub const PROTOCOLS: [(ProtocolFilter, &str); 4] = [
    (ProtocolFilter::All, "All"),
    (ProtocolFilter::Http, "HTTP"),
    (ProtocolFilter::Https, "HTTPS"),
    (ProtocolFilter::WebSocket, "WebSocket"),
];

pub const CONTENT: [(ContentGroup, &str); 9] = [
    (ContentGroup::Json, "JSON"),
    (ContentGroup::Form, "Form"),
    (ContentGroup::Xml, "XML"),
    (ContentGroup::Js, "JS"),
    (ContentGroup::Css, "CSS"),
    (ContentGroup::GraphQl, "GraphQL"),
    (ContentGroup::Document, "Document"),
    (ContentGroup::Media, "Media"),
    (ContentGroup::Other, "Other"),
];

pub const PLACEHOLDER: &str = "Filter: text, host:*.example.com status:4xx body:token…";
pub const BAR_HEIGHT: f32 = 32.;

pub struct FilterBar {
    workspace: Entity<Workspace>,
    table: Entity<FlowTable>,
    snapshot: Entity<Latest<SidebarSnapshot>>,
    input: Entity<InputState>,
    /// The suggestion list is wanted (a term is being typed, or ↓).
    open: bool,
    active: usize,
    _subscriptions: Vec<Subscription>,
}

impl FilterBar {
    pub fn new(
        workspace: Entity<Workspace>,
        table: Entity<FlowTable>,
        snapshot: Entity<Latest<SidebarSnapshot>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> FilterBar {
        let text = workspace.read(cx).filter.text.clone();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(PLACEHOLDER).default_value(text));
        let weak = cx.weak_entity();
        let subscriptions = vec![
            cx.subscribe_in(&input, window, Self::on_input),
            cx.observe_in(&workspace, window, |this, ws, window, cx| {
                // The text changed elsewhere (a palette command, a menu).
                let text = ws.read(cx).filter.text.clone();
                if this.input.read(cx).value().as_ref() != text {
                    this.input.update(cx, |s, cx| s.set_value(text, window, cx));
                    this.open = false;
                }
                cx.notify();
            }),
            cx.observe(&table, |_, _, cx| cx.notify()),
            cx.observe(&snapshot, |this, _, cx| {
                if this.open {
                    cx.notify();
                }
            }),
            // The list's keys come before the input's own (↑/↓, Tab, ↩, Escape).
            cx.intercept_keystrokes(move |e, window, cx| {
                let Some(this) = weak.upgrade() else { return };
                if !this.read(cx).is_focused(window, cx) {
                    return;
                }
                let k = &e.keystroke;
                let m = &k.modifiers;
                if m.control || m.alt || m.platform || m.shift || m.function {
                    return;
                }
                let key = k.key.clone();
                if this.update(cx, |this, cx| this.on_key(&key, window, cx)) {
                    cx.stop_propagation();
                }
            }),
        ];
        FilterBar {
            workspace,
            table,
            snapshot,
            input,
            open: false,
            active: 0,
            _subscriptions: subscriptions,
        }
    }

    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.input.read(cx).focus_handle(cx).is_focused(window)
    }

    /// ⌘F: focuses the box and selects its text.
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.focus(window, cx));
        window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
    }

    /// Focuses the box with the caret at the end (after "Search bodies…").
    pub fn focus_at_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.workspace.read(cx).filter.text.clone();
        self.input.update(cx, |s, cx| {
            s.set_value(text.clone(), window, cx);
            s.set_cursor_position(end_position(&text), window, cx);
        });
    }

    pub fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Types `text` as if the user had (the dev remote; tests).
    pub fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| {
            s.set_value(text.to_owned(), window, cx);
            s.set_cursor_position(end_position(text), window, cx);
        });
        self.changed(cx);
    }

    fn on_input(&mut self, _: &Entity<InputState>, e: &InputEvent, _: &mut Window, cx: &mut Context<Self>) {
        match e {
            InputEvent::Change => self.changed(cx),
            InputEvent::Blur => {
                self.open = false;
                cx.notify();
            }
            _ => {}
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        let (text, caret) = {
            let s = self.input.read(cx);
            (s.value().to_string(), s.cursor())
        };
        self.active = 0;
        // Suggest while a term is being typed, not after a space.
        self.open = !dsl::term_at(&text, caret).1.is_empty();
        self.workspace.update(cx, |w, cx| w.set_text(text, cx));
        cx.notify();
    }

    /// The suggestions for the term at the caret, while the list is open.
    pub fn suggestions(&self, cx: &App) -> Vec<Suggestion> {
        if !self.open {
            return Vec::new();
        }
        let s = self.input.read(cx);
        let ctx = self
            .snapshot
            .read(cx)
            .value
            .as_ref()
            .map(DslContext::from_sidebar)
            .unwrap_or_default();
        dsl::suggest(&s.value(), s.cursor(), &ctx)
    }

    /// One of the list's keys; true if it was the list's.
    pub fn on_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let suggestions = self.suggestions(cx);
        if !suggestions.is_empty() {
            let n = suggestions.len();
            match key {
                "down" => self.active = (self.active + 1) % n,
                "up" => self.active = (self.active + n - 1) % n,
                "tab" | "enter" => {
                    let s = suggestions[self.active.min(n - 1)].clone();
                    self.choose(&s, window, cx);
                }
                "escape" => self.open = false,
                _ => return false,
            }
            cx.notify();
            return true;
        }
        match key {
            "down" => {
                self.open = true;
                cx.notify();
                true
            }
            "escape" | "enter" => {
                crate::shell::focus_table(window, cx);
                true
            }
            _ => false,
        }
    }

    fn choose(&mut self, s: &Suggestion, window: &mut Window, cx: &mut Context<Self>) {
        let (text, caret) = {
            let input = self.input.read(cx);
            dsl::accept(&input.value(), input.cursor(), s)
        };
        self.input.update(cx, |input, cx| {
            input.set_value(text.clone(), window, cx);
            input.set_cursor_position(end_position(&text[..caret]), window, cx);
        });
        self.active = 0;
        self.open = !s.complete;
        self.workspace.update(cx, |w, cx| w.set_text(text, cx));
        cx.notify();
    }

    fn chip<F: Fn(&mut Workspace, &mut Context<Workspace>) + 'static>(
        id: SharedString,
        label: &'static str,
        on: bool,
        t: &Tokens,
        click: F,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<F> {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .aria_toggled(if on { Toggled::True } else { Toggled::False })
            .h(px(20.))
            .px(px(8.))
            .flex()
            .items_center()
            .rounded(px(3.))
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .map(|d| {
                if on {
                    d.bg(t.primary_chip_bg.hsla()).text_color(t.primary_chip_fg.hsla())
                } else {
                    d.text_color(t.muted_foreground.hsla()).hover(|s| s.bg(t.muted.hsla()))
                }
            })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.workspace.update(cx, |w, cx| click(w, cx))))
            .test_support()
    }
}

/// The input's position for byte offset `text.len()` (one line; the
/// column counts UTF-16 units, as the input's positions do).
fn end_position(text: &str) -> gpui_kit::component::input::Position {
    gpui_kit::component::input::Position::new(0, text.encode_utf16().count() as u32)
}

impl Render for FilterBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let filter = self.workspace.read(cx).filter.clone();
        let text = self.input.read(cx).value().to_string();
        let deep_query = dsl::has_deep_terms(&text);
        let deep = self.table.read(cx).deep();
        let suggestions = self.suggestions(cx);
        let active = self.active.min(suggestions.len().saturating_sub(1));

        let protocols: Vec<_> = PROTOCOLS
            .iter()
            .map(|(p, label)| {
                let p = *p;
                Self::chip(
                    format!("protocol-{label}").into(),
                    label,
                    filter.protocol == p,
                    &t,
                    move |w, cx| w.set_protocol(p, cx),
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let mut content = vec![
            Self::chip(
                "content-All".into(),
                "All",
                filter.content.is_empty(),
                &t,
                |w, cx| w.clear_content(cx),
                cx,
            )
            .into_any_element(),
        ];
        content.extend(CONTENT.iter().map(|(g, label)| {
            let g = *g;
            Self::chip(
                format!("content-{label}").into(),
                label,
                filter.content.contains(&g),
                &t,
                move |w, cx| w.toggle_content(g, cx),
                cx,
            )
            .into_any_element()
        }));

        let icon = if deep_query {
            pm_widgets::icon(IconName::ScanSearch, t.primary_chip_fg.hsla(), 12.)
        } else {
            pm_widgets::icon(IconName::Search, t.muted_foreground.hsla(), 12.)
        };
        let badge = deep.filter(|d| deep_query && d.running).map(|d| {
            let pct = if d.total > 0 {
                (u64::from(d.checked) * 100 / u64::from(d.total)) as u32
            } else {
                0
            };
            div()
                .id("deep-badge")
                .role(Role::Status)
                .aria_label(SharedString::from(format!("Deep search {pct}%")))
                .text_size(px(10.))
                .text_color(t.muted_foreground.hsla())
                .child(format!("{pct}%"))
                .tooltip(|window, cx| {
                    Tooltip::new("Searching headers and bodies: matches appear as they are found").build(window, cx)
                })
        });
        let clear = (!text.is_empty()).then(|| {
            div()
                .id("clear-search")
                .role(Role::Button)
                .aria_label("Clear search")
                .child(pm_widgets::icon(IconName::X, t.muted_foreground.hsla(), 12.))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.input.update(cx, |s, cx| s.set_value("", window, cx));
                    this.open = false;
                    this.workspace.update(cx, |w, cx| w.set_text("", cx));
                }))
        });
        let list = (!suggestions.is_empty()).then(|| {
            let options = suggestions.iter().enumerate().map(|(i, s)| {
                let chosen = s.clone();
                div()
                    .id(("flow-search-option", i))
                    .role(Role::ListBoxOption)
                    .aria_label(SharedString::from(s.label.clone()))
                    .aria_selected(i == active)
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(3.))
                    .text_size(px(12.))
                    .when(i == active, |d| {
                        d.bg(t.accent.hsla()).text_color(t.accent_foreground.hsla())
                    })
                    .child(div().flex_none().font_family("Menlo").child(s.label.clone()))
                    .children(
                        s.detail
                            .map(|d| div().truncate().text_color(t.muted_foreground.hsla()).child(d)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            window.prevent_default();
                            this.choose(&chosen, window, cx);
                        }),
                    )
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.active != i {
                            this.active = i;
                            cx.notify();
                        }
                    }))
            });
            deferred(
                div()
                    .id("flow-search-suggestions")
                    .role(Role::ListBox)
                    .aria_label("Filter suggestions")
                    .absolute()
                    .top(px(26.))
                    .right_0()
                    .w(px(420.))
                    .p(px(4.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(t.border.hsla())
                    .bg(t.popover.hsla())
                    .text_color(t.popover_foreground.hsla())
                    .shadow_md()
                    .children(options),
            )
            .with_priority(1)
        });

        div()
            .id("filter-bar")
            .role(Role::Toolbar)
            .aria_label("Filters")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .h(px(BAR_HEIGHT))
            .px(px(8.))
            .bg(t.pane_toolbar.hsla())
            .border_b_1()
            .border_color(t.border.hsla())
            .children(protocols)
            .child(div().mx(px(4.)).w(px(1.)).h(px(16.)).bg(t.border.hsla()))
            .children(content)
            .child(
                div()
                    .ml_auto()
                    .w(px(320.))
                    .relative()
                    .child(
                        Input::new(&self.input)
                            .id("flow-search")
                            .aria_label("Search flows")
                            .h(px(24.))
                            .prefix(icon)
                            .suffix(div().flex().items_center().gap(px(6.)).children(badge).children(clear)),
                    )
                    .children(list),
            )
    }
}
