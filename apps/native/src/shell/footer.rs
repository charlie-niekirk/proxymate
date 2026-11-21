//! The status bar (`components/Footer.tsx`): Clear, Filter, the Auto
//! Select toggle, the row and selection counts, the deep search's progress,
//! the background save's status (with its menu), "Not recording bodies",
//! "N not recorded", the last filter's timing and the throughput, from the
//! storage stream (`subscribe_storage`).

use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FontWeight, MouseButton, MouseDownEvent, Role, SharedString, Subscription, Toggled, Window,
    div, px,
};
use pm_model::StorageStatus;
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;

use crate::bridge::stream::Latest;
use crate::features::capture;
use crate::features::chrome::OpenSettings;
use crate::features::commands::MAC;
use crate::features::context_menu::Menu;
use crate::features::documents::DismissSave;
use crate::features::flow_table::FlowTable;
use crate::shell::{FOOTER_HEIGHT, thousands};
use crate::workspace::{SaveJob, Workspace};

/// How often the throughput is sampled.
const RATE_EVERY: Duration = Duration::from_secs(1);

/// "Searching bodies · 120 of 400 (30%) · 7 found", then "Searched …".
pub fn deep_text(d: &pm_model::DeepProgress) -> String {
    let pct = if d.total > 0 {
        u64::from(d.checked) * 100 / u64::from(d.total)
    } else {
        100
    };
    if d.running {
        format!(
            "Searching bodies · {} of {} ({pct}%) · {} found",
            thousands(d.checked.into()),
            thousands(d.total.into()),
            thousands(d.matched.into())
        )
    } else {
        format!(
            "Searched {} flows · {} found",
            thousands(d.total.into()),
            thousands(d.matched.into())
        )
    }
}

/// "N of M rows", "1 / N of M rows" with a flow selected, "K selected · …".
pub fn row_count(selected: usize, len: usize, total: usize) -> (Option<String>, String) {
    let lead = match selected {
        0 => None,
        1 => Some("1 / ".to_owned()),
        n => Some(format!("{} selected · ", thousands(n as u64))),
    };
    (
        lead,
        format!("{} of {} rows", thousands(len as u64), thousands(total as u64)),
    )
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The save status's text.
pub fn save_text(job: &SaveJob) -> String {
    let p = &job.progress;
    if let Some(err) = &p.error {
        format!("{} failed: {err}", job.label)
    } else if p.finished {
        format!("Saved {} flows to {}", thousands(p.done.into()), file_name(&p.path))
    } else {
        let pct = if p.total > 0 {
            u64::from(p.done) * 100 / u64::from(p.total)
        } else {
            0
        };
        format!("{} · {pct}%", job.label)
    }
}

/// The save status's menu: the error, or where it went; Dismiss.
pub fn save_menu(job: &SaveJob) -> Option<Menu> {
    let p = &job.progress;
    let menu = if let Some(err) = &p.error {
        Menu::new().copy("Copy error", err.clone(), "error")
    } else if p.finished {
        Menu::new().copy("Copy path", p.path.clone(), "path").copy(
            "Copy file name",
            file_name(&p.path).to_owned(),
            "file name",
        )
    } else {
        return None;
    };
    Some(menu.separator().item("Dismiss", DismissSave))
}

/// The save status's menu as the app would show it now.
pub fn current_save_menu(cx: &App) -> Option<Menu> {
    let ws = Workspace::global(cx)?;
    save_menu(ws.read(cx).save.as_ref()?)
}

fn rate_text(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1.0 {
        "0 B/s".into()
    } else {
        format!("{}/s", pm_view::flow::format_size(bytes_per_sec.round() as u64))
    }
}

pub struct Footer {
    workspace: Entity<Workspace>,
    table: Entity<FlowTable>,
    storage: Entity<Latest<StorageStatus>>,
    /// Bytes per second up and down, once two samples are in.
    rate: Option<(f64, f64)>,
    last: Option<(Instant, u64, u64)>,
    _subscriptions: Vec<Subscription>,
}

impl Footer {
    pub fn new(
        workspace: Entity<Workspace>,
        table: Entity<FlowTable>,
        storage: Entity<Latest<StorageStatus>>,
        cx: &mut Context<Self>,
    ) -> Footer {
        let subscriptions = vec![
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            cx.observe(&table, |_, _, cx| cx.notify()),
            cx.observe(&storage, |_, _, cx| cx.notify()),
        ];
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(RATE_EVERY).await;
                if this.update(cx, |this, cx| this.sample(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Footer {
            workspace,
            table,
            storage,
            rate: None,
            last: None,
            _subscriptions: subscriptions,
        }
    }

    fn sample(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.storage.read(cx).value.as_ref() else {
            return;
        };
        let now = Instant::now();
        let (sent, received) = (s.sent_bytes, s.received_bytes);
        if let Some((at, prev_sent, prev_received)) = self.last {
            let secs = now.duration_since(at).as_secs_f64().max(0.001);
            // A clear or a new engine counts from zero: no negative rates.
            let rate = (
                sent.saturating_sub(prev_sent) as f64 / secs,
                received.saturating_sub(prev_received) as f64 / secs,
            );
            if self.rate != Some(rate) {
                self.rate = Some(rate);
                cx.notify();
            }
        }
        self.last = Some((now, sent, received));
    }

    fn status(id: &'static str, t: &Tokens) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .mr(px(12.))
            .text_color(t.muted_foreground.hsla())
    }
}

impl Render for Footer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let ws = self.workspace.read(cx);
        let live = ws.is_live();
        let save = ws.save.clone();
        let table = self.table.read(cx);
        let total = table.total();
        let deep = table.deep();
        let timing = table.filter_timing();
        let grid = table.grid().read(cx);
        let (len, selected, auto_select) = (grid.len(), grid.selection_count(), grid.auto_select());
        let storage = self.storage.read(cx).value.clone();
        let shortcut = |key: &str| pm_view::commands::shortcut_label(key, false, false, MAC);

        let clear = Button::new("footer-clear")
            .ghost()
            .xsmall()
            .disabled(!live)
            .label("Clear")
            .icon(IconName::Trash)
            .tooltip(SharedString::from(format!(
                "Clear all flows · {}",
                shortcut("backspace")
            )))
            .on_click(|_, _, cx| capture::clear(cx));
        let filter = Button::new("footer-filter")
            .ghost()
            .xsmall()
            .label("Filter")
            .icon(IconName::Funnel)
            .tooltip(SharedString::from(format!("Search flows · {}", shortcut("f"))))
            .on_click(|_, window, cx| crate::shell::focus_search(window, cx));
        let auto = div()
            .id("auto-select")
            .role(Role::Switch)
            .aria_label("Auto Select")
            .aria_toggled(if auto_select { Toggled::True } else { Toggled::False })
            .flex()
            .items_center()
            .gap(px(4.))
            .h(px(20.))
            .px(px(6.))
            .rounded(px(4.))
            .map(|d| {
                if auto_select {
                    d.bg(t.accent.hsla()).text_color(t.accent_foreground.hsla())
                } else {
                    d.hover(|s| s.bg(t.muted.hsla()))
                }
            })
            .child(pm_widgets::icon(
                IconName::ArrowDownToLine,
                if auto_select {
                    t.accent_foreground.hsla()
                } else {
                    t.muted_foreground.hsla()
                },
                12.,
            ))
            .child("Auto Select")
            .tooltip(|window, cx| Tooltip::new("Select and inspect new traffic as it arrives").build(window, cx))
            .on_click(|_, _, cx| capture::toggle_auto_select(cx))
            .test_support();

        let (lead, rows) = row_count(selected, len, total);
        let count = div()
            .id("row-count")
            .role(Role::Status)
            .flex_1()
            .flex()
            .justify_center()
            .children(lead.map(|lead| {
                div()
                    .when(selected > 1, |d| {
                        d.text_color(t.foreground.hsla()).font_weight(FontWeight::MEDIUM)
                    })
                    .child(lead)
            }))
            .child(rows);

        let deep = deep.map(|d| {
            Self::status("deep-status", &t)
                .role(Role::Status)
                .child(pm_widgets::icon(
                    if d.running {
                        IconName::LoaderCircle
                    } else {
                        IconName::ScanSearch
                    },
                    t.muted_foreground.hsla(),
                    12.,
                ))
                .child(deep_text(&d))
        });

        let save = save.map(|job| {
            let p = &job.progress;
            let (icon, color) = if p.error.is_some() {
                (IconName::CircleAlert, t.s_failed)
            } else if p.finished {
                (IconName::CircleCheck, t.s_2xx)
            } else {
                (IconName::LoaderCircle, t.muted_foreground)
            };
            let text = save_text(&job);
            let failed = p.error.is_some();
            let tip: SharedString = p.error.clone().unwrap_or_else(|| p.path.clone()).into();
            Self::status("save-status", &t)
                .role(Role::Status)
                .aria_label(SharedString::from(text.clone()))
                .when(failed, |d| d.text_color(t.s_failed.hsla()))
                .child(pm_widgets::icon(icon, color.hsla(), 12.))
                .child(div().max_w(px(320.)).truncate().child(text))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .when(failed, |d| {
                    d.on_click(|_, window, cx| window.dispatch_action(Box::new(DismissSave), cx))
                })
                .on_mouse_down(MouseButton::Right, |e: &MouseDownEvent, window, cx| {
                    if let Some(menu) = current_save_menu(cx) {
                        menu.show(e.position, window, cx);
                    }
                })
        });

        let warning = storage
            .as_ref()
            .and_then(|s| s.error.clone())
            .filter(|_| live)
            .map(|err| {
                let tip: SharedString =
                    format!("Proxymate {err}. Traffic is still forwarded. Click for Storage settings.").into();
                Self::status("storage-error", &t)
                    .role(Role::Button)
                    .aria_label("Not recording bodies")
                    .text_color(t.s_failed.hsla())
                    .child(pm_widgets::icon(IconName::HardDriveDownload, t.s_failed.hsla(), 12.))
                    .child("Not recording bodies")
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(OpenSettings { page: "storage".into() }), cx)
                    })
            });

        let unrecorded = storage.as_ref().map_or(0, |s| s.unrecorded);
        let unrecorded = (live && unrecorded > 0).then(|| {
            let text = format!("{} not recorded", thousands(unrecorded));
            Self::status("unrecorded-count", &t)
                .role(Role::Button)
                .aria_label(SharedString::from(text.clone()))
                .hover(|s| s.text_color(t.foreground.hsla()))
                .child(pm_widgets::icon(IconName::EyeOff, t.muted_foreground.hsla(), 12.))
                .child(text)
                .tooltip(|window, cx| {
                    Tooltip::new(
                        "Flows to hosts on the Never record list (or off the Only record list) since launch. \
                         They were still proxied. Click for Storage settings.",
                    )
                    .build(window, cx)
                })
                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings { page: "storage".into() }), cx))
        });

        let timing = timing.map(|f| {
            div()
                .id("filter-timing")
                .flex_none()
                .child(format!(
                    "filter {:.1} ms · {:.0} ms end to end",
                    f.engine_ms, f.end_to_end_ms
                ))
                .tooltip(|window, cx| Tooltip::new("Engine refilter · filter change to painted rows").build(window, cx))
        });

        let throughput: SharedString = match (&storage, self.rate) {
            (Some(_), Some((up, down))) => format!("↑ {} ↓ {}", rate_text(up), rate_text(down)).into(),
            (Some(_), None) => "↑ 0 B/s ↓ 0 B/s".into(),
            (None, _) => "↑ — ↓ —".into(),
        };

        div()
            .id("footer")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(FOOTER_HEIGHT))
            .px(px(8.))
            .bg(t.pane_toolbar.hsla())
            .border_t_1()
            .border_color(t.border.hsla())
            .text_size(px(12.))
            .text_color(t.muted_foreground.hsla())
            .child(clear)
            .child(filter)
            .child(auto)
            .child(count)
            .children(deep)
            .children(save)
            .children(warning)
            .children(unrecorded)
            .children(timing)
            .child(
                div()
                    .id("throughput")
                    .ml(px(12.))
                    .flex_none()
                    .font_family("Menlo")
                    .child(throughput)
                    .tooltip(|window, cx| Tooltip::new("Body bytes through the proxy per second").build(window, cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use pm_model::{DeepProgress, SaveProgress};

    use super::*;

    #[test]
    fn counts_read_like_the_tauri_footer() {
        assert_eq!(row_count(0, 12, 1234), (None, "12 of 1,234 rows".into()));
        assert_eq!(row_count(1, 12, 12).0.as_deref(), Some("1 / "));
        assert_eq!(row_count(2500, 3000, 3000).0.as_deref(), Some("2,500 selected · "));
        let d = DeepProgress {
            checked: 120,
            total: 400,
            matched: 7,
            running: true,
        };
        assert_eq!(deep_text(&d), "Searching bodies · 120 of 400 (30%) · 7 found");
        assert_eq!(
            deep_text(&DeepProgress { running: false, ..d }),
            "Searched 400 flows · 7 found"
        );
        assert_eq!(rate_text(0.4), "0 B/s");
    }

    #[test]
    fn save_status_and_its_menu() {
        let job = |p: SaveProgress| SaveJob::new("Saving session", p);
        let running = job(SaveProgress {
            done: 25,
            total: 100,
            ..Default::default()
        });
        assert_eq!(save_text(&running), "Saving session · 25%");
        assert!(save_menu(&running).is_none());
        let saved = job(SaveProgress {
            done: 1200,
            total: 1200,
            finished: true,
            error: None,
            path: "/Users/me/Desktop/Login.proxymate".into(),
        });
        assert_eq!(save_text(&saved), "Saved 1,200 flows to Login.proxymate");
        let menu = save_menu(&saved).unwrap();
        assert_eq!(menu.labels(), ["Copy path", "Copy file name", "—", "Dismiss"]);
        assert_eq!(menu.copy_text("Copy file name"), Some("Login.proxymate"));
        assert_eq!(menu.copy_text("Copy path"), Some("/Users/me/Desktop/Login.proxymate"));
        let failed = job(SaveProgress {
            finished: true,
            error: Some("disk full".into()),
            ..Default::default()
        });
        assert_eq!(save_text(&failed), "Saving session failed: disk full");
        assert_eq!(save_menu(&failed).unwrap().copy_text("Copy error"), Some("disk full"));
    }
}
