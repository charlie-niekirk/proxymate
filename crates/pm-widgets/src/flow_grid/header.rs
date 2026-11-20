//! The grid's column headers (`Header.tsx`): a sortable column's title is a
//! button (click sorts; dragging it onto another header reorders), its
//! right edge resizes (drag; double-click restores the default width;
//! ←/→ by 8 px, ⇧ 32, Home/End, ↩ when focused), and a right-click opens
//! the column menu (a native menu: show and hide columns, remove a header
//! column, Reset columns).

use std::collections::HashMap;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, ClickEvent, Context, DragMoveEvent, FocusHandle, FontWeight, KeyDownEvent, MouseButton,
    MouseDownEvent, Orientation, Pixels, Point, Render, Role, SharedString, Window, div, px, svg,
};
use pm_model::{BodyPart, SortDir, SortKey};
use pm_theme::kit::TokensExt as _;
use pm_view::columns::{Align, Builtin, ColumnId, HEADER_HEIGHT, MAX_COLUMN_WIDTH, PlacedColumn};
use pm_view::keys::{Key, ResizeColumn, TableKey, resize_key};

use super::{AddHeaderColumn, FlowGrid, FlowGridEvent, RemoveHeaderColumn, ResetColumns, ToggleColumn};

/// A column edge being dragged.
#[derive(Clone, Debug)]
pub struct ResizeDrag {
    id: ColumnId,
}

/// A header being dragged onto another.
#[derive(Clone, Debug)]
pub struct ColumnDrag {
    id: ColumnId,
    title: SharedString,
}

struct ResizeStart {
    id: ColumnId,
    x: Pixels,
    px: f32,
}

#[derive(Default)]
pub(super) struct HeaderState {
    resize: Option<ResizeStart>,
    /// Each resize handle's focus, so its keys reach it.
    pub(super) handles: HashMap<ColumnId, FocusHandle>,
}

/// What follows the pointer while a header is dragged.
struct ColumnGhost {
    title: SharedString,
}

impl Render for ColumnGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        div()
            .px(px(8.))
            .h(px(HEADER_HEIGHT))
            .flex()
            .items_center()
            .rounded(px(4.))
            .bg(t.popover.hsla())
            .border_1()
            .border_color(t.primary.hsla())
            .text_size(px(12.))
            .text_color(t.popover_foreground.hsla())
            .child(self.title.clone())
    }
}

/// A resize drag shows nothing but the column itself changing.
struct NoGhost;

impl Render for NoGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl FlowGrid {
    pub(super) fn render_header(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = *cx.tokens();
        let sort = self.prefs.sort;
        let columns = self.placed.clone();
        let mut cells: Vec<AnyElement> = Vec::with_capacity(columns.len());
        for (ix, col) in columns.iter().enumerate() {
            cells.push(self.header_cell(ix, col, sort, window, cx));
        }
        div()
            .id("flow-table-header")
            .role(Role::Row)
            .aria_row_index(1)
            .flex_none()
            .flex()
            .flex_row()
            .h(px(HEADER_HEIGHT))
            .w_full()
            .bg(t.pane_header.hsla())
            .border_b_1()
            .border_color(t.border.hsla())
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .font_family(cx.theme().font_family.clone())
            .text_color(t.muted_foreground.hsla())
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<ResizeDrag>, _, cx| this.drag_resize(e, cx)))
            .children(cells)
    }

    fn header_cell(
        &mut self,
        ix: usize,
        col: &PlacedColumn,
        sort: pm_model::SortSpec,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.tokens();
        let id = col.id.clone();
        let sorted = col.sorted_by(&sort) && (col.sort_key != Some(SortKey::Id) || sort.dir == SortDir::Desc);
        let label: SharedString = col.label.clone().into();
        let title: SharedString = col.title.clone().into();
        let align = col.align;
        let primary = t.primary.hsla();

        let mut title_area = div()
            .id(SharedString::from(format!("sort-{id}")))
            .flex_1()
            .min_w_0()
            .h_full()
            .px(px(6.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .map(|d| match align {
                Align::Left => d,
                Align::Right => d.justify_end(),
                Align::Center => d.justify_center(),
            })
            .child(div().truncate().child(title.clone()))
            .when(sorted, |d| {
                let arrow = if sort.dir == SortDir::Asc {
                    gpui_kit::assets::IconName::ArrowUp
                } else {
                    gpui_kit::assets::IconName::ArrowDown
                };
                d.child(svg().path(arrow.path()).flex_none().size(px(12.)).text_color(t.muted_foreground.hsla()))
            })
            .on_drag(
                ColumnDrag {
                    id: id.clone(),
                    title: if title.is_empty() { label.clone() } else { title.clone() },
                },
                |drag, _, _, cx| cx.new(|_| ColumnGhost { title: drag.title.clone() }),
            );
        if col.sort_key.is_some() {
            let target = id.clone();
            title_area = title_area
                .role(Role::Button)
                .aria_label(SharedString::from(format!("Sort by {label}")))
                .when(sorted, |d| {
                    d.aria_description(if sort.dir == SortDir::Asc { "ascending" } else { "descending" })
                })
                .hover(|s| s.text_color(t.foreground.hsla()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.sort_by(&target, cx)));
        }

        let handle = self.resize_handle(ix, col, cx);
        let drop_target = id.clone();
        let menu_target = id.clone();
        div()
            .id(SharedString::from(format!("column-{id}")))
            .role(Role::ColumnHeader)
            .aria_label(label)
            .aria_column_index(ix + 1)
            .h_full()
            .flex()
            .flex_row()
            .relative()
            .when(col.grow, |d| d.flex_1().min_w(px(col.px)))
            .when(!col.grow, |d| d.flex_none().w(px(col.px)))
            .border_r_1()
            .border_color(t.border.hsla())
            .drag_over::<ColumnDrag>(move |s, _, _, _| s.border_l_2().border_color(primary))
            .on_drop(cx.listener(move |this, drag: &ColumnDrag, _, cx| {
                if drag.id != drop_target {
                    this.move_column(&drag.id, Some(&drop_target), cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    this.open_column_menu(e.position, Some(menu_target.clone()), window, cx);
                }),
            )
            .child(title_area)
            .child(handle)
            .into_any_element()
    }

    fn resize_handle(&mut self, ix: usize, col: &PlacedColumn, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.tokens();
        let id = col.id.clone();
        let focus = self
            .header
            .handles
            .entry(id.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let limits = ResizeColumn {
            px: col.px,
            min_width: col.min_width,
            width: col.width,
        };
        let (down_id, click_id, key_id) = (id.clone(), id.clone(), id.clone());
        let down_focus = focus.clone();
        div()
            .id(SharedString::from(format!("resize-{id}")))
            .role(Role::Splitter)
            .aria_label(SharedString::from(format!("Resize {}", col.label)))
            .aria_orientation(Orientation::Vertical)
            .aria_numeric_value(col.px as f64)
            .aria_min_numeric_value(col.min_width as f64)
            .aria_max_numeric_value(MAX_COLUMN_WIDTH as f64)
            .aria_column_index(ix + 1)
            .track_focus(&focus)
            .flex_none()
            .w(px(5.))
            .h_full()
            .cursor_col_resize()
            .hover(|s| s.bg(t.primary.hsla().opacity(0.4)))
            .focus(|s| s.bg(t.primary.hsla().opacity(0.6)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    window.focus(&down_focus, cx);
                    this.header.resize = Some(ResizeStart {
                        id: down_id.clone(),
                        x: e.position.x,
                        px: limits.px,
                    });
                }),
            )
            .on_drag(ResizeDrag { id: id.clone() }, |_, _, _, cx| cx.new(|_| NoGhost))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() >= 2 {
                    this.set_column_width(&click_id, limits.width, cx);
                }
            }))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                let ks = &e.keystroke;
                let key = TableKey {
                    key: Some(Key::from_name(&ks.key)),
                    shift: ks.modifiers.shift,
                    ..Default::default()
                };
                if ks.modifiers.platform || ks.modifiers.control || ks.modifiers.alt {
                    return;
                }
                if let Some(width) = resize_key(key, limits, MAX_COLUMN_WIDTH) {
                    cx.stop_propagation();
                    this.set_column_width(&key_id, width, cx);
                }
            }))
            .into_any_element()
    }

    fn drag_resize(&mut self, e: &DragMoveEvent<ResizeDrag>, cx: &mut Context<Self>) {
        let Some(start) = &self.header.resize else { return };
        if e.drag(cx).id != start.id {
            return;
        }
        let Some(col) = self.placed.iter().find(|c| c.id == start.id) else {
            return;
        };
        let width = (start.px + f32::from(e.event.position.x - start.x)).clamp(col.min_width, MAX_COLUMN_WIDTH);
        if (width - col.px).abs() < 0.5 {
            return;
        }
        // Applied as it moves; saved once, when the drag ends.
        let id = start.id.clone();
        self.prefs.set_width(&id, width);
        self.placed = std::rc::Rc::new(self.prefs.place());
        cx.notify();
    }

    /// The edge was let go: the new width is the layout's.
    pub(super) fn end_resize(&mut self, cx: &mut Context<Self>) {
        if self.header.resize.take().is_some() {
            cx.emit(FlowGridEvent::PrefsChanged);
        }
    }

    /// The column menu, at `position`, for the header `pressed`.
    pub fn open_column_menu(
        &mut self,
        position: Point<Pixels>,
        pressed: Option<ColumnId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The menu's actions go to the focused element: the grid.
        window.focus(&self.focus, cx);
        let mut menu = NativeMenu::new();
        for b in Builtin::ALL {
            let c = b.column();
            if c.required {
                continue;
            }
            let hidden = self.prefs.is_hidden(&c.id);
            menu = menu.menu_with_check(c.label, !hidden, Box::new(ToggleColumn { id: c.id }));
        }
        for (_, id) in self.prefs.header_ids() {
            let ColumnId::Header(h) = &id else { continue };
            let part = match h.part {
                BodyPart::Request => "request",
                BodyPart::Response => "response",
            };
            let hidden = self.prefs.is_hidden(&id);
            menu = menu.menu_with_check(format!("{} ({part})", h.name), !hidden, Box::new(ToggleColumn { id }));
        }
        menu = menu.separator().menu_with_disabled(
            "Add header column…",
            // The dialog comes with G11; the dev remote's `headerColumns` sets them meanwhile.
            true,
            Box::new(AddHeaderColumn),
        );
        if let Some(id @ ColumnId::Header(h)) = &pressed {
            menu = menu.menu(
                format!("Remove “{}” column", h.name),
                Box::new(RemoveHeaderColumn { id: id.clone() }),
            );
        }
        menu.menu("Reset columns", Box::new(ResetColumns)).show(position, window, cx);
    }
}
