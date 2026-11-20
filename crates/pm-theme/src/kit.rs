//! The tokens on gpui-kit (feature `kit`): [`apply`] loads gpui-kit's theme
//! for the mode, lays the matching tokens over its colours, sets the
//! compact density, and keeps the tokens themselves in the [`ActiveTokens`]
//! global for Proxymate's own colours (methods, statuses, URL parts, …),
//! which views read with [`TokensExt::tokens`]. Components never hard-code
//! a colour.

use gpui_kit::component::theme::{Theme, ThemeColor, ThemeMode};
use gpui_kit::{App, Global, Hsla, px};

use crate::{Mode, Tokens};

impl From<crate::Rgba> for Hsla {
    fn from(c: crate::Rgba) -> Self {
        gpui_kit::Rgba {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
        .into()
    }
}

impl crate::Rgba {
    pub fn hsla(self) -> Hsla {
        self.into()
    }
}

/// The tokens in force, as a gpui global.
pub struct ActiveTokens {
    pub mode: Mode,
    pub tokens: Tokens,
}

impl Global for ActiveTokens {}

pub trait TokensExt {
    /// The active theme's tokens (dark until [`apply`] first runs).
    fn tokens(&self) -> &Tokens;
    fn theme_mode(&self) -> Mode;
}

impl TokensExt for App {
    fn tokens(&self) -> &Tokens {
        self.try_global::<ActiveTokens>()
            .map_or(&DARK_ON_PLATFORM, |t| &t.tokens)
    }

    fn theme_mode(&self) -> Mode {
        self.try_global::<ActiveTokens>().map_or(Mode::Dark, |t| t.mode)
    }
}

const DARK_ON_PLATFORM: Tokens = if cfg!(target_os = "macos") {
    crate::DARK.macos()
} else {
    crate::DARK
};

/// Text sizes of the compact density (12–13 px text, 24–28 px controls).
pub const FONT_SIZE: f32 = 13.0;
pub const MONO_FONT_SIZE: f32 = 12.0;

/// Switches every window to `mode`: gpui-kit's components and our tokens.
pub fn apply(mode: Mode, cx: &mut App) {
    let tokens = mode.tokens();
    cx.set_global(ActiveTokens { mode, tokens });
    let kit_mode = if mode.is_dark() {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(kit_mode, None, cx);
    Theme::update(cx, |theme| {
        map_colors(&mut theme.colors, &tokens, mode);
        theme.font_size = px(FONT_SIZE);
        theme.mono_font_size = px(MONO_FONT_SIZE);
        theme.radius = px(crate::RADIUS_PX);
        theme.radius_lg = px(crate::RADIUS_PX + 2.0);
        theme.shadow = false;
    });
    cx.refresh_windows();
}

/// Lays the tokens over gpui-kit's colours. Its semantic slots take the
/// shadcn tokens (gpui-kit's own vocabulary is shadcn's); lists and tables
/// take the flow table's panes and the macOS selection blue.
pub fn map_colors(c: &mut ThemeColor, t: &Tokens, mode: Mode) {
    let h = |x: crate::Rgba| -> Hsla { x.into() };
    // A step towards the foreground for hover, two for pressed.
    let step = |x: crate::Rgba, n: f32| -> Hsla {
        let mut hsla = h(x);
        let delta = 0.04 * n;
        hsla.l = if mode.is_dark() {
            (hsla.l + delta).min(1.0)
        } else {
            (hsla.l - delta).max(0.0)
        };
        hsla
    };
    c.background = h(t.background);
    c.foreground = h(t.foreground);
    c.border = h(t.border);
    c.input = h(t.input);
    c.ring = h(t.ring);
    c.caret = h(t.primary);
    c.selection = h(t.code_selection);
    c.overlay = h(t.overlay);
    c.window_border = h(t.border);
    c.drag_border = h(t.primary);
    c.drop_target = h(t.primary_chip_bg);

    c.accent = h(t.accent);
    c.accent_foreground = h(t.accent_foreground);
    c.muted = h(t.muted);
    c.muted_foreground = h(t.muted_foreground);
    c.popover = h(t.popover);
    c.popover_foreground = h(t.popover_foreground);
    c.group_box = h(t.card);
    c.group_box_foreground = h(t.card_foreground);
    c.accordion = h(t.card);
    c.description_list_label = h(t.muted);
    c.description_list_label_foreground = h(t.muted_foreground);

    c.primary = h(t.primary);
    c.primary_foreground = h(t.primary_foreground);
    c.primary_hover = step(t.primary, 1.0);
    c.primary_active = step(t.primary, 2.0);
    c.secondary = h(t.secondary);
    c.secondary_foreground = h(t.secondary_foreground);
    c.secondary_hover = h(t.accent);
    c.secondary_active = step(t.accent, 1.0);
    c.danger = h(t.destructive_solid);
    c.danger_foreground = h(t.destructive_foreground);
    c.danger_hover = step(t.destructive_solid, 1.0);
    c.danger_active = step(t.destructive_solid, 2.0);
    c.success = h(t.s_2xx);
    c.success_foreground = h(t.primary_foreground);
    c.success_hover = step(t.s_2xx, 1.0);
    c.success_active = step(t.s_2xx, 2.0);
    c.warning = h(t.s_4xx);
    c.warning_foreground = h(t.background);
    c.warning_hover = step(t.s_4xx, 1.0);
    c.warning_active = step(t.s_4xx, 2.0);
    c.info = h(t.m_get);
    c.info_foreground = h(t.background);
    c.info_hover = step(t.m_get, 1.0);
    c.info_active = step(t.m_get, 2.0);
    c.link = h(t.url_bar_host);
    c.link_hover = step(t.url_bar_host, 1.0);
    c.link_active = step(t.url_bar_host, 2.0);

    c.button = h(t.secondary);
    c.button_foreground = h(t.secondary_foreground);
    c.button_hover = h(t.accent);
    c.button_active = step(t.accent, 1.0);
    c.button_primary = c.primary;
    c.button_primary_foreground = c.primary_foreground;
    c.button_primary_hover = c.primary_hover;
    c.button_primary_active = c.primary_active;
    c.button_secondary = c.secondary;
    c.button_secondary_foreground = c.secondary_foreground;
    c.button_secondary_hover = c.secondary_hover;
    c.button_secondary_active = c.secondary_active;
    c.button_danger = c.danger;
    c.button_danger_foreground = c.danger_foreground;
    c.button_danger_hover = c.danger_hover;
    c.button_danger_active = c.danger_active;

    c.sidebar = h(t.sidebar);
    c.sidebar_foreground = h(t.sidebar_foreground);
    c.sidebar_accent = h(t.sidebar_accent);
    c.sidebar_accent_foreground = h(t.sidebar_accent_foreground);
    c.sidebar_border = h(t.sidebar_border);
    c.sidebar_primary = h(t.sidebar_primary);
    c.sidebar_primary_foreground = h(t.sidebar_primary_foreground);

    c.list = h(t.pane_table);
    c.list_even = h(t.pane_row_alt);
    c.list_hover = h(t.pane_row_hover);
    c.list_active = h(t.selection_bg);
    c.list_active_border = h(t.selection_bg);
    c.list_head = h(t.pane_header);
    c.list_highlight_foreground = h(t.primary_chip_fg);
    c.table = h(t.pane_table);
    c.table_even = h(t.pane_row_alt);
    c.table_hover = h(t.pane_row_hover);
    c.table_active = h(t.selection_bg);
    c.table_active_border = h(t.selection_bg);
    c.table_head = h(t.pane_header);
    c.table_head_foreground = h(t.muted_foreground);
    c.table_foot = h(t.pane_header);
    c.table_foot_foreground = h(t.muted_foreground);
    c.table_row_border = h(t.border);

    c.title_bar = h(t.pane_toolbar);
    c.title_bar_border = h(t.border);
    c.status_bar = h(t.pane_toolbar);
    c.status_bar_border = h(t.border);
    c.tab_bar = h(t.pane_header);
    c.tab_bar_segmented = h(t.muted);
    c.tab = h(t.pane_header);
    c.tab_foreground = h(t.muted_foreground);
    c.tab_active = h(t.background);
    c.tab_active_foreground = h(t.foreground);

    c.scrollbar = Hsla::transparent_black();
    c.scrollbar_thumb = h(t.muted_foreground).opacity(0.45);
    c.scrollbar_thumb_hover = h(t.muted_foreground).opacity(0.7);
    c.progress_bar = h(t.primary);
    c.slider_bar = h(t.primary);
    c.slider_thumb = h(t.primary_foreground);
    c.switch = h(t.input);
    c.switch_thumb = h(t.primary_foreground);
    c.skeleton = h(t.muted);

    c.chart_1 = h(t.chart_1);
    c.chart_2 = h(t.chart_2);
    c.chart_3 = h(t.chart_3);
    c.chart_4 = h(t.chart_4);
    c.chart_5 = h(t.chart_5);

    c.red = h(t.label_red);
    c.red_light = h(t.label_red_bg);
    c.green = h(t.label_green);
    c.green_light = h(t.label_green_bg);
    c.blue = h(t.label_blue);
    c.blue_light = h(t.label_blue_bg);
    c.yellow = h(t.label_yellow);
    c.yellow_light = h(t.label_yellow_bg);
    c.magenta = h(t.label_purple);
    c.magenta_light = h(t.label_purple_bg);
    c.cyan = h(t.m_head);
    c.cyan_light = h(t.m_head_bg);
}
