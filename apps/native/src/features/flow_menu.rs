//! The flow row menu (`FlowContextMenu.tsx`): a right-click on a row (or
//! its "…" button) opens it for that flow. G3 builds every item that needs
//! no later dialog: Copy <column>, Copy URL, Copy as cURL, Copy ▸ (host,
//! path, headers, bodies, raw messages), Filter by ▸, Pin/Unpin, Export
//! HAR…, Add <host> to SSL bypass, Never record <host> and Delete.
//!
//! Later phases add theirs at a [`Slot`] with [`extend`] from their own
//! `register` (Copy as Code: G11 at `Copy`; Replay, Edit & Replay, Repeat:
//! G8/G11 at `Send`; Label, Compare, Comment: G11 at `Annotate`; Map Local,
//! Breakpoint: G7, Rewrite: G9, Script: G10 at `Rules`; Filter by <header>:
//! G11 at `Filter`), so this file isn't a merge hotspot. A right-click
//! inside a multi-selection is G11's bulk menu, which registers with
//! [`set_bulk`].

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::{App, Global};
use pm_model::{BodyPart, FlowRow, FlowSelection, FlowState, Scheme, StoreId};
use pm_view::columns::Column;
use pm_view::inspector::cell_text;

use crate::features::context_menu::Menu;
use crate::features::inspector::actions::{
    CopyFlowBody, CopyFlowCurl, CopyFlowHeaders, CopyFlowRaw, CopyFlowUrl, DeleteFlow, ExportFlowHar, FlowRef,
    NeverRecord, PinFlow,
};
use crate::features::inspector::menus::filter_by;

/// Where a later phase's items go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// After Copy as cURL (Copy as Code).
    Copy,
    /// After Copy ▸ (Filter by <header>).
    Filter,
    /// Replay, Edit & Replay, Repeat.
    Send,
    /// After Pin (Label, Compare, Comment).
    Annotate,
    /// Map Local, Breakpoint, Rewrite, Script.
    Rules,
}

/// What a row menu is for.
#[derive(Clone, Debug)]
pub struct RowContext {
    pub store: StoreId,
    pub row: FlowRow,
    /// The column right-clicked (none from the "…" button).
    pub column: Option<Column>,
    /// The live capture (not a document tab).
    pub live: bool,
}

impl RowContext {
    pub fn flow(&self) -> FlowRef {
        FlowRef {
            store: self.store,
            id: self.row.0,
        }
    }

    pub fn is_connect(&self) -> bool {
        &*self.row.3 == "CONNECT"
    }

    pub fn is_websocket(&self) -> bool {
        matches!(self.row.2, Scheme::Ws | Scheme::Wss)
    }
}

type Items = Rc<dyn Fn(&RowContext, &App) -> Menu>;
type Bulk = Rc<dyn Fn(StoreId, FlowSelection, usize, &App) -> Menu>;

#[derive(Default)]
struct Extensions {
    slots: HashMap<Slot, Vec<Items>>,
    bulk: Option<Bulk>,
}

impl Global for Extensions {}

/// Adds items at `slot` of every row menu.
pub fn extend(slot: Slot, items: impl Fn(&RowContext, &App) -> Menu + 'static, cx: &mut App) {
    cx.default_global::<Extensions>()
        .slots
        .entry(slot)
        .or_default()
        .push(Rc::new(items));
}

/// The menu of a right-click inside a multi-selection (G11).
pub fn set_bulk(menu: impl Fn(StoreId, FlowSelection, usize, &App) -> Menu + 'static, cx: &mut App) {
    cx.default_global::<Extensions>().bulk = Some(Rc::new(menu));
}

fn slot(menu: Menu, slot: Slot, ctx: &RowContext, cx: &App) -> Menu {
    let Some(ext) = cx.try_global::<Extensions>() else {
        return menu;
    };
    ext.slots
        .get(&slot)
        .into_iter()
        .flatten()
        .fold(menu, |menu, items| menu.append(items(ctx, cx)))
}

/// The bulk menu for `count` selected flows, once G11 registers it.
pub fn bulk_menu(store: StoreId, selection: FlowSelection, count: usize, cx: &App) -> Option<Menu> {
    let bulk = cx.try_global::<Extensions>()?.bulk.clone()?;
    Some(bulk(store, selection, count, cx))
}

/// The row menu of one flow.
pub fn row_menu(ctx: &RowContext, cx: &App) -> Menu {
    let row = &ctx.row;
    let flow = ctx.flow();
    let connect = ctx.is_connect();
    let https = matches!(row.2, Scheme::Https | Scheme::Wss);
    let host = row.4.to_string();
    let cell = ctx.column.as_ref().and_then(|c| cell_text(c, row));
    let mut menu = Menu::new();
    if let Some((label, text)) = cell {
        menu = menu.copy(format!("Copy {label}  {}", shorten(&text)), text, label.to_lowercase());
    }
    menu = menu
        .item("Copy URL", CopyFlowUrl { flow })
        .map_disabled("Copy as cURL", connect, CopyFlowCurl { flow });
    menu = slot(menu, Slot::Copy, ctx, cx);
    let copy = Menu::new()
        .copy("Host", host.clone(), "host")
        .copy("Path", row.6.clone(), "path")
        .separator()
        .item(
            "Request headers",
            CopyFlowHeaders {
                flow,
                part: BodyPart::Request,
            },
        )
        .item(
            "Response headers",
            CopyFlowHeaders {
                flow,
                part: BodyPart::Response,
            },
        )
        .map_disabled(
            "Request body",
            connect,
            CopyFlowBody {
                flow,
                part: BodyPart::Request,
            },
        )
        .map_disabled(
            "Response body",
            connect,
            CopyFlowBody {
                flow,
                part: BodyPart::Response,
            },
        )
        .separator()
        .map_disabled(
            "Raw request",
            connect,
            CopyFlowRaw {
                flow,
                part: BodyPart::Request,
            },
        )
        .map_disabled(
            "Raw response",
            connect,
            CopyFlowRaw {
                flow,
                part: BodyPart::Response,
            },
        );
    menu = menu.submenu("Copy", copy);
    menu = slot(menu, Slot::Filter, ctx, cx);
    menu = menu.submenu("Filter by", filter_by(&row.4, &row.3, row.8, row.10.as_deref()));
    menu = slot(menu.separator(), Slot::Send, ctx, cx).separator();
    let pinned = row.20;
    menu = menu.item(if pinned { "Unpin" } else { "Pin" }, PinFlow { flow, pinned: !pinned });
    menu = slot(menu, Slot::Annotate, ctx, cx);
    menu = slot(menu.separator(), Slot::Rules, ctx, cx).separator();
    menu.item("Export HAR…", ExportFlowHar { flow })
        .map_disabled(
            format!("Add {host} to SSL bypass"),
            !https,
            crate::features::chrome::AddSslBypass {
                host: host.clone().into(),
            },
        )
        .item(format!("Never record {host}"), NeverRecord { host: host.into() })
        .separator()
        .map_disabled(
            "Delete",
            matches!(row.1, FlowState::Active | FlowState::Paused),
            DeleteFlow { flow },
        )
}

/// A cell's value as the menu label shows it (the item copies all of it).
fn shorten(text: &str) -> String {
    const MAX: usize = 40;
    match text.char_indices().nth(MAX) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_owned(),
    }
}

trait MenuExt {
    fn map_disabled(
        self,
        label: impl Into<gpui_kit::SharedString>,
        disabled: bool,
        action: impl gpui_kit::Action,
    ) -> Menu;
}

impl MenuExt for Menu {
    fn map_disabled(
        self,
        label: impl Into<gpui_kit::SharedString>,
        disabled: bool,
        action: impl gpui_kit::Action,
    ) -> Menu {
        if disabled {
            self.disabled(label, action)
        } else {
            self.item(label, action)
        }
    }
}
