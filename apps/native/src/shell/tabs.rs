//! Document tabs (`components/DocumentTabs.tsx`): the live capture, then
//! each opened session, HAR or Charles file. Shown only while a document is
//! open; the table (and G3's inspector) follow the active tab. A tab's
//! menu copies its file's name or path, saves or exports it (switching to
//! it first) and closes it.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, MouseButton, MouseDownEvent, Role, SharedString, div, px};
use pm_model::{DocumentInfo, DocumentKind, LIVE_STORE, StoreId};
use pm_theme::Tokens;

use crate::features::context_menu::Menu;
use crate::features::documents::{CloseDocument, ExportStore, SaveStore, ShowStore};
use crate::workspace::Workspace;

pub const TABS_HEIGHT: f32 = 28.;

/// A tab's menu (`doc` is `None` for the live capture).
pub fn tab_menu(store: StoreId, doc: Option<&DocumentInfo>) -> Menu {
    let mut menu = Menu::new();
    if let Some(doc) = doc {
        menu = menu
            .copy("Copy name", doc.name.clone(), "name")
            .copy("Copy path", doc.path.clone(), "path")
            .separator();
    }
    menu.item("Save session…", SaveStore { store })
        .item("Export HAR…", ExportStore { store })
        .when(store != LIVE_STORE, |m| {
            m.separator().item("Close", CloseDocument { store })
        })
}

/// The menu of tab `store` as the app would show it now.
pub fn current_tab_menu(store: StoreId, cx: &App) -> Option<Menu> {
    let ws = Workspace::global(cx)?;
    let ws = ws.read(cx);
    if store != LIVE_STORE && ws.document(store).is_none() {
        return None;
    }
    Some(tab_menu(store, ws.document(store)))
}

fn tab(
    store: StoreId,
    active: bool,
    label: SharedString,
    tip: Option<SharedString>,
    content: Vec<AnyElement>,
    close: Option<AnyElement>,
    t: &Tokens,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("tab-{store}")))
        .role(Role::Tab)
        .aria_label(label)
        .aria_selected(active)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .px(px(10.))
        .rounded_t(px(6.))
        .border_1()
        .border_b_0()
        .text_size(px(12.))
        .map(|d| {
            if active {
                d.bg(t.background.hsla())
                    .border_color(t.border.hsla())
                    .text_color(t.foreground.hsla())
            } else {
                d.border_color(gpui_kit::transparent_black())
                    .text_color(t.muted_foreground.hsla())
                    .hover(|s| s.text_color(t.foreground.hsla()))
            }
        })
        .children(content)
        .children(close)
        .when_some(tip, |d, tip| {
            d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        })
        .on_click(move |_, window, cx| window.dispatch_action(Box::new(ShowStore { store }), cx))
        .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
            if let Some(menu) = current_tab_menu(store, cx) {
                menu.show(e.position, window, cx);
            }
        })
        .test_support()
        .into_any_element()
}

/// The tab strip, or nothing while no document is open.
pub fn render(ws: &Workspace, t: &Tokens) -> Option<impl IntoElement + use<>> {
    if ws.documents.is_empty() {
        return None;
    }
    let mut tabs = vec![tab(
        LIVE_STORE,
        ws.is_live(),
        "Live".into(),
        None,
        vec![
            pm_widgets::icon(IconName::Radio, t.s_2xx.hsla(), 12.).into_any_element(),
            div().child("Live").into_any_element(),
        ],
        None,
        t,
    )];
    for doc in &ws.documents {
        let store = doc.store;
        let icon = if doc.kind == DocumentKind::Session {
            IconName::Archive
        } else {
            IconName::FileBraces
        };
        let close = div()
            .id(SharedString::from(format!("close-{store}")))
            .role(Role::Button)
            .aria_label(SharedString::from(format!("Close {}", doc.name)))
            .ml(px(-2.))
            .p(px(2.))
            .rounded(px(3.))
            .hover(|s| s.bg(t.accent.hsla()))
            .child(pm_widgets::icon(IconName::X, t.muted_foreground.hsla(), 11.))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(Box::new(CloseDocument { store }), cx);
            })
            .test_support()
            .into_any_element();
        tabs.push(tab(
            store,
            ws.store == store,
            doc.name.clone().into(),
            Some(doc.path.clone().into()),
            vec![
                pm_widgets::icon(icon, t.muted_foreground.hsla(), 12.).into_any_element(),
                div()
                    .max_w(px(192.))
                    .truncate()
                    .child(doc.name.clone())
                    .into_any_element(),
                div()
                    .text_size(px(10.))
                    .text_color(t.muted_foreground.hsla())
                    .child(pm_view::flow::format_count(u64::from(doc.flows)))
                    .into_any_element(),
            ],
            Some(close),
            t,
        ));
    }
    Some(
        div()
            .id("document-tabs")
            .role(Role::TabList)
            .aria_label("Documents")
            .flex()
            .flex_none()
            .items_end()
            .gap(px(2.))
            .h(px(TABS_HEIGHT))
            .px(px(8.))
            .overflow_x_scroll()
            .bg(t.pane_toolbar.hsla())
            .border_b_1()
            .border_color(t.border.hsla())
            .children(tabs),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_menus_copy_save_export_and_close() {
        let doc = DocumentInfo {
            store: 2,
            kind: DocumentKind::Charles,
            name: "capture.chlsj".into(),
            path: "/tmp/capture.chlsj".into(),
            flows: 40,
        };
        let menu = tab_menu(2, Some(&doc));
        assert_eq!(
            menu.labels(),
            [
                "Copy name",
                "Copy path",
                "—",
                "Save session…",
                "Export HAR…",
                "—",
                "Close"
            ]
        );
        assert_eq!(menu.copy_text("Copy name"), Some("capture.chlsj"));
        assert_eq!(tab_menu(LIVE_STORE, None).labels(), ["Save session…", "Export HAR…"]);
    }
}
