//! The sidebar (`components/Sidebar.tsx`) over the window's vibrancy:
//! Favorites (a placeholder), Pinned, Saved (the live capture and opened
//! documents), All, Apps with icons and counts, and the Domains tree, each
//! section collapsible, with a "Filter apps and domains" box (⌘L) at the
//! bottom. Apps and Domains come from the engine's sidebar snapshot
//! (`subscribe_sidebar`, at most every 250 ms), counted over the tab shown
//! (`set_sidebar_store`, which the flow table sets with its view).
//!
//! Each list has one menu builder keyed by its row ([`menu_for`]), shared
//! by right-clicks, tests and the dev remote.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, FontWeight, Image, ImageFormat, ImageSource, MouseButton, MouseDownEvent, Role,
    SharedString, Subscription, Window, div, img, px,
};
use pm_core::Engine;
use pm_model::{AppCount, DocumentInfo, DocumentKind, LIVE_STORE, SidebarSnapshot, StorageStatus};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;

use crate::bridge::stream::Latest;
use crate::features::chrome::{AddSslBypass, SetAppSslExcluded, SetInsecureUpstream, ShowApp, ShowHost, ToggleText};
use crate::features::context_menu::Menu;
use crate::features::documents::{CloseDocument, ShowStore};
use crate::menus::Open;
use crate::shell::{TOOLBAR_HEIGHT, chrome, thousands};
use crate::workspace::Workspace;

/// A saved document's menu: copy its name or path, close it.
pub fn document_menu(doc: &DocumentInfo) -> Menu {
    Menu::new()
        .copy("Copy name", doc.name.clone(), "name")
        .copy("Copy path", doc.path.clone(), "path")
        .separator()
        .item(format!("Close {}", doc.name), CloseDocument { store: doc.store })
}

/// The key "Don't decrypt" uses: the bundle id, else the name.
fn app_key(app: &AppCount) -> &str {
    app.bundle_id.as_deref().unwrap_or(&app.name)
}

/// An app row's menu.
pub fn app_menu(app: &AppCount, active: bool, excluded: bool) -> Menu {
    let show = if active {
        Menu::new().item("Show all traffic", ShowApp { app: None })
    } else {
        Menu::new().item(
            format!("Show only {}", app.name),
            ShowApp {
                app: Some(app.name.clone().into()),
            },
        )
    };
    show.separator()
        .copy("Copy name", app.name.clone(), "app name")
        .when(app.bundle_id.is_some(), |m| {
            m.copy("Copy bundle id", app.bundle_id.clone().unwrap_or_default(), "bundle id")
        })
        .separator()
        .check(
            format!("Don't decrypt HTTPS from {}", app.name),
            excluded,
            SetAppSslExcluded {
                app: app_key(app).to_owned().into(),
                excluded: !excluded,
            },
        )
}

/// A domain or host row's menu. `scope` is what the row filters by
/// (`*.example.com` for a domain with hosts under it).
pub fn host_menu(host: &str, scope: &str, shown: bool, insecure: bool) -> Menu {
    let show = if shown {
        Menu::new().item("Show all traffic", ShowHost { host: None })
    } else {
        Menu::new().item(
            format!("Show only {scope}"),
            ShowHost {
                host: Some(scope.to_owned().into()),
            },
        )
    };
    show.separator()
        .copy(format!("Copy {host}"), host.to_owned(), "host")
        .when(scope != host, |m| {
            m.copy(format!("Copy {scope}"), scope.to_owned(), "domain pattern")
        })
        .separator()
        .item(
            format!("Add {host} to SSL bypass"),
            AddSslBypass {
                host: host.to_owned().into(),
            },
        )
        .check(
            format!("Accept {host}'s certificate unverified"),
            insecure,
            SetInsecureUpstream {
                host: host.to_owned().into(),
                allow: !insecure,
            },
        )
}

/// The menu of the sidebar row `key`: `doc:<store>`, `app:<name>`,
/// `domain:<domain>` or `host:<host>`.
pub fn menu_for(key: &str, cx: &App) -> Option<Menu> {
    let ws = Workspace::global(cx)?;
    let ws = ws.read(cx);
    let (kind, value) = key.split_once(':')?;
    match kind {
        "doc" => {
            let store = value.parse().ok()?;
            ws.document(store).map(document_menu)
        }
        "app" => {
            let snapshot = sidebar_snapshot(cx)?;
            let app = snapshot.apps.iter().find(|a| a.name == value)?;
            let active = ws.filter.client.as_deref() == Some(value);
            Some(app_menu(app, active, ws.is_app_excluded(app_key(app))))
        }
        "domain" => {
            let snapshot = sidebar_snapshot(cx)?;
            let node = snapshot.domains.iter().find(|d| d.domain == value)?;
            let scope = domain_scope(&node.domain, !node.hosts.is_empty());
            let shown = ws.filter.host.as_deref() == Some(scope.as_str());
            Some(host_menu(&node.domain, &scope, shown, ws.is_insecure(&node.domain)))
        }
        "host" => {
            let shown = ws.filter.host.as_deref() == Some(value);
            Some(host_menu(value, value, shown, ws.is_insecure(value)))
        }
        _ => None,
    }
}

/// A domain row filters to the domain and every host under it.
fn domain_scope(domain: &str, has_hosts: bool) -> String {
    if has_hosts {
        format!("*.{domain}")
    } else {
        domain.to_owned()
    }
}

fn sidebar_snapshot(cx: &App) -> Option<SidebarSnapshot> {
    let sidebar = crate::shell::parts(cx)?.sidebar.upgrade()?;
    sidebar.read(cx).snapshot.read(cx).value.clone()
}

/// An app's PNG icon by name (blocking: run off the foreground thread).
type IconSource = Arc<dyn Fn(&str) -> Option<Arc<Vec<u8>>> + Send + Sync>;

const SECTIONS: [&str; 6] = ["Favorites", "Pinned", "Saved", "All", "Apps", "Domains"];

pub struct Sidebar {
    workspace: Entity<Workspace>,
    snapshot: Entity<Latest<SidebarSnapshot>>,
    storage: Entity<Latest<StorageStatus>>,
    filter: Entity<InputState>,
    collapsed: HashSet<&'static str>,
    /// Domains opened by their chevron.
    expanded: HashSet<String>,
    icon_source: Option<IconSource>,
    icons: HashMap<String, Option<ImageSource>>,
    _subscriptions: Vec<Subscription>,
}

impl Sidebar {
    pub fn new(
        workspace: Entity<Workspace>,
        snapshot: Entity<Latest<SidebarSnapshot>>,
        storage: Entity<Latest<StorageStatus>>,
        engine: Option<Arc<Engine>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Sidebar {
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter apps and domains")
                .clean_on_escape()
        });
        let subscriptions = vec![
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            cx.observe(&snapshot, |_, _, cx| cx.notify()),
            cx.observe(&storage, |_, _, cx| cx.notify()),
            cx.subscribe(&filter, |_, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Sidebar {
            workspace,
            snapshot,
            storage,
            filter,
            collapsed: HashSet::new(),
            expanded: HashSet::new(),
            icon_source: engine.map(|e| -> IconSource { Arc::new(move |name: &str| e.app_icon(name)) }),
            icons: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    /// ⌘L: focuses the filter box and selects its text.
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |s, cx| s.focus(window, cx));
        window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
    }

    pub fn filter_text(&self, cx: &App) -> String {
        self.filter.read(cx).value().to_string()
    }

    pub fn set_filter_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx));
        cx.notify();
    }

    fn toggle_section(&mut self, title: &'static str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(title) {
            self.collapsed.insert(title);
        }
        cx.notify();
    }

    fn app_icon(&mut self, name: &str, cx: &mut Context<Self>) -> Option<ImageSource> {
        if let Some(icon) = self.icons.get(name) {
            return icon.clone();
        }
        let source = self.icon_source.clone()?;
        self.icons.insert(name.to_owned(), None);
        let key = name.to_owned();
        let bytes = cx.background_spawn(async move { source(&key) });
        let name = name.to_owned();
        cx.spawn(async move |this, cx| {
            let Some(bytes) = bytes.await else { return };
            let image = ImageSource::from(Arc::new(Image::from_bytes(ImageFormat::Png, bytes.to_vec())));
            let _ = this.update(cx, |this, cx| {
                this.icons.insert(name, Some(image));
                cx.notify();
            });
        })
        .detach();
        None
    }

    fn section(
        &self,
        title: &'static str,
        children: Vec<AnyElement>,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let open = !self.collapsed.contains(title);
        div()
            .flex()
            .flex_col()
            .pb(px(8.))
            .child(
                div()
                    .id(SharedString::from(format!("section-{title}")))
                    .role(Role::Button)
                    .aria_label(title)
                    .aria_expanded(open)
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(8.))
                    .py(px(4.))
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.muted_foreground.hsla())
                    .hover(|s| s.text_color(t.foreground.hsla()))
                    .child(title.to_uppercase())
                    .child(pm_widgets::icon(
                        if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        t.muted_foreground.hsla(),
                        11.,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(title, cx))),
            )
            .when(open, |d| d.children(children))
    }
}

/// A row: icon, label (cut with an ellipsis; the tooltip has it whole), count.
#[derive(Clone)]
struct Row {
    id: SharedString,
    icon: Option<AnyIcon>,
    label: SharedString,
    count: Option<u32>,
    active: bool,
    muted: bool,
    indent: bool,
    /// The row's menu key (see [`menu_for`]).
    menu: Option<SharedString>,
}

#[derive(Clone)]
enum AnyIcon {
    Glyph(IconName),
    Image(ImageSource),
    /// An app with no icon: its initial.
    Initial(char),
}

type Click = Rc<dyn Fn(&mut Window, &mut App)>;

fn row(r: Row, on_click: Option<Click>, badge: Option<AnyElement>, t: &Tokens) -> AnyElement {
    let tip: SharedString = match r.count {
        Some(n) => format!("{} · {} flows", r.label, thousands(n as u64)).into(),
        None => r.label.clone(),
    };
    let icon: Option<AnyElement> = r.icon.map(|i| match i {
        AnyIcon::Glyph(name) => {
            pm_widgets::icon(name, t.sidebar_foreground.hsla().opacity(0.7), 14.).into_any_element()
        }
        AnyIcon::Image(source) => img(source).size(px(14.)).flex_none().into_any_element(),
        AnyIcon::Initial(c) => div()
            .size(px(14.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .bg(t.muted.hsla())
            .text_size(px(9.))
            .text_color(t.muted_foreground.hsla())
            .child(c.to_string())
            .into_any_element(),
    });
    let icon = icon.map(|icon| {
        div()
            .relative()
            .flex_none()
            .child(icon)
            .children(badge.map(|b| div().absolute().right(px(-4.)).bottom(px(-4.)).child(b)))
    });
    let menu = r.menu.clone();
    let mut d = div()
        .id(r.id.clone())
        .role(if on_click.is_some() { Role::Button } else { Role::Label })
        .aria_label(r.label.clone())
        .aria_selected(r.active)
        .h(px(24.))
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .when(r.indent, |d| d.pl(px(28.)))
        .rounded(px(5.))
        .text_size(px(13.))
        .text_color(if r.muted {
            t.muted_foreground.hsla()
        } else {
            t.sidebar_foreground.hsla()
        })
        .when(r.muted, |d| d.italic())
        .when(r.active, |d| {
            d.bg(t.sidebar_accent.hsla())
                .text_color(t.sidebar_accent_foreground.hsla())
                .font_weight(FontWeight::MEDIUM)
        })
        .when(on_click.is_some() && !r.active, |d| {
            d.hover(|s| s.bg(t.sidebar_accent.hsla()))
        })
        .children(icon)
        .child(div().flex_1().min_w_0().truncate().child(r.label.clone()))
        .children(r.count.map(|n| {
            div()
                .flex_none()
                .text_size(px(10.))
                .text_color(t.muted_foreground.hsla())
                .child(pm_view::flow::format_count(n as u64))
        }))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx));
    if let Some(click) = on_click {
        d = d.on_click(move |_, window, cx| click(window, cx));
    }
    if let Some(key) = menu {
        d = d.on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
            if let Some(menu) = menu_for(&key, cx) {
                menu.show(e.position, window, cx);
            }
        });
    }
    d.test_support().into_any_element()
}

fn dispatch(action: impl gpui_kit::Action + Clone) -> Option<Click> {
    Some(Rc::new(move |window: &mut Window, cx: &mut App| {
        window.dispatch_action(Box::new(action.clone()), cx)
    }))
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let ws = self.workspace.read(cx);
        let (store, live, filter) = (ws.store, ws.is_live(), ws.filter.clone());
        let documents = ws.documents.clone();
        let excluded: HashSet<String> = ws.ssl.exclude_apps.iter().map(|a| a.to_lowercase()).collect();
        let snapshot = self.snapshot.read(cx).value.clone().unwrap_or_default();
        let pinned = self.storage.read(cx).value.as_ref().map_or(0, |s| s.pinned);
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        let matches = |s: &str| s.to_lowercase().contains(&needle);

        let plain = |id: &str, icon: IconName, label: &str| Row {
            id: id.to_owned().into(),
            icon: Some(AnyIcon::Glyph(icon)),
            label: label.to_owned().into(),
            count: None,
            active: false,
            muted: true,
            indent: false,
            menu: None,
        };

        let favorites = vec![row(
            plain("sidebar-favorites", IconName::Star, "No favorites yet"),
            None,
            None,
            &t,
        )];

        let showing_pinned = filter.text.trim() == "is:pinned";
        let pinned_rows = if pinned == 0 || !live {
            vec![row(
                plain("sidebar-pinned", IconName::Pin, "No pinned flows"),
                None,
                None,
                &t,
            )]
        } else {
            vec![row(
                Row {
                    count: Some(pinned),
                    active: showing_pinned,
                    muted: false,
                    ..plain("sidebar-pinned", IconName::Pin, "Pinned flows")
                },
                dispatch(ToggleText {
                    text: "is:pinned".into(),
                }),
                None,
                &t,
            )]
        };

        let mut saved = vec![row(
            Row {
                active: live,
                muted: false,
                ..plain("saved-live", IconName::Radio, "Live capture")
            },
            dispatch(ShowStore { store: LIVE_STORE }),
            None,
            &t,
        )];
        for doc in &documents {
            let icon = if doc.kind == DocumentKind::Session {
                IconName::Archive
            } else {
                IconName::FileBraces
            };
            saved.push(row(
                Row {
                    count: Some(doc.flows),
                    active: store == doc.store,
                    muted: false,
                    menu: Some(format!("doc:{}", doc.store).into()),
                    ..plain(&format!("saved-{}", doc.store), icon, &doc.name)
                },
                dispatch(ShowStore { store: doc.store }),
                None,
                &t,
            ));
        }
        saved.push(row(
            plain("saved-open", IconName::FolderOpen, "Open session or HAR…"),
            dispatch(Open),
            None,
            &t,
        ));

        let all = vec![row(
            Row {
                count: Some(snapshot.total),
                active: filter.host.is_none() && filter.client.is_none(),
                muted: false,
                ..plain("sidebar-all", IconName::Layers, "All traffic")
            },
            dispatch(ShowApp { app: None }),
            None,
            &t,
        )];

        let apps: Vec<&AppCount> = snapshot.apps.iter().filter(|a| matches(&a.name)).collect();
        let mut app_rows = Vec::new();
        if apps.is_empty() {
            let label = if needle.is_empty() {
                "No apps yet"
            } else {
                "No matching apps"
            };
            app_rows.push(row(plain("sidebar-no-apps", IconName::Layers, label), None, None, &t));
        }
        for app in apps {
            let active = filter.client.as_deref() == Some(app.name.as_str());
            let icon = match self.app_icon(&app.name, cx) {
                Some(source) => AnyIcon::Image(source),
                None => AnyIcon::Initial(app.name.chars().next().unwrap_or('?').to_ascii_uppercase()),
            };
            let shield = excluded
                .contains(&app_key(app).to_lowercase())
                .then(|| pm_widgets::icon(IconName::ShieldOff, t.muted_foreground.hsla(), 10.).into_any_element());
            app_rows.push(row(
                Row {
                    id: format!("sidebar-app-{}", app.name).into(),
                    icon: Some(icon),
                    label: app.name.clone().into(),
                    count: Some(app.count),
                    active,
                    muted: false,
                    indent: false,
                    menu: Some(format!("app:{}", app.name).into()),
                },
                dispatch(ShowApp {
                    app: (!active).then(|| app.name.clone().into()),
                }),
                shield,
                &t,
            ));
        }

        let domains: Vec<_> = snapshot
            .domains
            .iter()
            .filter(|d| matches(&d.domain) || d.hosts.iter().any(|h| matches(&h.host)))
            .collect();
        let mut domain_rows = Vec::new();
        if domains.is_empty() {
            let label = if needle.is_empty() {
                "No domains yet"
            } else {
                "No matching domains"
            };
            domain_rows.push(row(plain("sidebar-no-domains", IconName::Globe, label), None, None, &t));
        }
        for node in domains {
            let expandable = !node.hosts.is_empty();
            // A filter opens every match.
            let expanded = expandable && (self.expanded.contains(&node.domain) || !needle.is_empty());
            let scope = domain_scope(&node.domain, expandable);
            let active = filter.host.as_deref() == Some(scope.as_str());
            let domain = node.domain.clone();
            let chevron = div()
                .id(SharedString::from(format!("expand-{}", node.domain)))
                .role(Role::Button)
                .aria_label(if expanded { "Collapse" } else { "Expand" })
                .aria_expanded(expanded)
                .flex_none()
                .p(px(2.))
                .ml(px(-4.))
                .rounded(px(3.))
                .when(!expandable, |d| d.invisible())
                .child(pm_widgets::icon(
                    if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    },
                    t.muted_foreground.hsla(),
                    11.,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.expanded.remove(&domain) {
                        this.expanded.insert(domain.clone());
                    }
                    cx.notify();
                }));
            let main = row(
                Row {
                    id: format!("sidebar-domain-{}", node.domain).into(),
                    icon: Some(AnyIcon::Glyph(IconName::Globe)),
                    label: node.domain.clone().into(),
                    count: Some(node.count),
                    active,
                    muted: false,
                    indent: false,
                    menu: Some(format!("domain:{}", node.domain).into()),
                },
                dispatch(ShowHost {
                    host: (!active).then(|| scope.clone().into()),
                }),
                None,
                &t,
            );
            domain_rows.push(
                div()
                    .flex()
                    .items_center()
                    .pl(px(4.))
                    .child(chevron)
                    .child(div().flex_1().min_w_0().child(main))
                    .into_any_element(),
            );
            if expanded {
                for h in &node.hosts {
                    let active = filter.host.as_deref() == Some(h.host.as_str());
                    domain_rows.push(row(
                        Row {
                            id: format!("sidebar-host-{}", h.host).into(),
                            icon: None,
                            label: h.host.clone().into(),
                            count: Some(h.count),
                            active,
                            muted: false,
                            indent: true,
                            menu: Some(format!("host:{}", h.host).into()),
                        },
                        dispatch(ShowHost {
                            host: (!active).then(|| h.host.clone().into()),
                        }),
                        None,
                        &t,
                    ));
                }
            }
        }

        let mut lists = vec![favorites, pinned_rows, saved, all, app_rows, domain_rows].into_iter();
        let sections: Vec<AnyElement> = SECTIONS
            .iter()
            .map(|title| {
                let rows = lists.next().unwrap_or_default();
                self.section(title, rows, &t, cx).into_any_element()
            })
            .collect();

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(chrome::drag("sidebar-drag").h(px(TOOLBAR_HEIGHT)).flex_none())
            .child(
                div()
                    .id("sidebar-list")
                    .role(Role::Navigation)
                    .aria_label("Sidebar")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(8.))
                    .children(sections),
            )
            .child(
                div().flex_none().p(px(8.)).child(
                    Input::new(&self.filter)
                        .id("sidebar-filter")
                        .aria_label("Filter apps and domains")
                        .prefix(pm_widgets::icon(IconName::Search, t.muted_foreground.hsla(), 12.))
                        .cleanable(true),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(bundle: Option<&str>) -> AppCount {
        AppCount {
            name: "Google Chrome".into(),
            bundle_id: bundle.map(Into::into),
            has_icon: true,
            remote: false,
            count: 12,
        }
    }

    #[test]
    fn app_menus_copy_the_name_and_bundle_id() {
        let menu = app_menu(&app(Some("com.google.Chrome")), false, false);
        assert_eq!(
            menu.labels(),
            [
                "Show only Google Chrome",
                "—",
                "Copy name",
                "Copy bundle id",
                "—",
                "Don't decrypt HTTPS from Google Chrome"
            ]
        );
        assert_eq!(menu.copy_text("Copy name"), Some("Google Chrome"));
        assert_eq!(menu.copy_text("Copy bundle id"), Some("com.google.Chrome"));
        let shown = app_menu(&app(None), true, true);
        assert_eq!(shown.labels()[0], "Show all traffic");
        assert!(shown.copy_text("Copy bundle id").is_none());
        let Some(crate::features::context_menu::Entry::Check { checked, .. }) =
            shown.find("Don't decrypt HTTPS from Google Chrome")
        else {
            panic!("a check item");
        };
        assert!(*checked);
    }

    #[test]
    fn host_menus_copy_the_host_and_the_domain_pattern() {
        let domain = host_menu("example.com", "*.example.com", false, false);
        assert_eq!(
            domain.labels(),
            [
                "Show only *.example.com",
                "—",
                "Copy example.com",
                "Copy *.example.com",
                "—",
                "Add example.com to SSL bypass",
                "Accept example.com's certificate unverified"
            ]
        );
        assert_eq!(domain.copy_text("Copy *.example.com"), Some("*.example.com"));
        let host = host_menu("api.example.com", "api.example.com", true, true);
        assert_eq!(host.labels()[0], "Show all traffic");
        assert_eq!(host.copy_text("Copy api.example.com"), Some("api.example.com"));
        assert_eq!(host.labels().len(), 6, "no pattern for a host");
    }

    #[test]
    fn document_menus_copy_and_close() {
        let doc = DocumentInfo {
            store: 3,
            kind: DocumentKind::Har,
            name: "trip.har".into(),
            path: "/Users/me/trip.har".into(),
            flows: 9,
        };
        let menu = document_menu(&doc);
        assert_eq!(menu.labels(), ["Copy name", "Copy path", "—", "Close trip.har"]);
        assert_eq!(menu.copy_text("Copy path"), Some("/Users/me/trip.har"));
    }
}
