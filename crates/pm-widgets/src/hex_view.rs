//! HexView (`BodyViewer/HexView.tsx`): offset, 16 bytes in hex and their
//! ASCII per row, a `uniform_list`, so any size scrolls. The ASCII column
//! is selectable text; a right-click names the row and the byte under the
//! pointer so the owner's menu can copy them ([`MenuRequest`]).

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::{SelectableText, TestSupportExt as _, TextSelectionHandle};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, ElementId, FocusHandle, Focusable, MouseButton, MouseDownEvent, Pixels, Point, Role, SharedString,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use pm_theme::kit::TokensExt as _;
use pm_view::copy_formats::{HEX_WIDTH, offset_digits};

const ROW_HEIGHT: f32 = 18.;
const FONT_SIZE: f32 = 11.5;

pub struct MenuRequest {
    /// The row under the pointer.
    pub row: Option<usize>,
    /// The byte under the pointer (its offset).
    pub offset: Option<usize>,
    pub selection: Option<String>,
    pub position: Point<Pixels>,
}

pub type OnMenu = Rc<dyn Fn(MenuRequest, &mut Window, &mut App)>;

pub struct HexView {
    id: SharedString,
    bytes: Arc<Vec<u8>>,
    scroll: UniformListScrollHandle,
    selection: TextSelectionHandle,
    focus: FocusHandle,
    on_menu: Option<OnMenu>,
}

impl HexView {
    pub fn new(id: impl Into<SharedString>, bytes: Arc<Vec<u8>>, cx: &mut Context<Self>) -> Self {
        HexView {
            id: id.into(),
            bytes,
            scroll: UniformListScrollHandle::new(),
            selection: TextSelectionHandle::new(String::new(), cx),
            focus: cx.focus_handle(),
            on_menu: None,
        }
    }

    pub fn on_menu(mut self, f: impl Fn(MenuRequest, &mut Window, &mut App) + 'static) -> Self {
        self.on_menu = Some(Rc::new(f));
        self
    }

    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.bytes
    }

    pub fn row_count(&self) -> usize {
        self.bytes.len().div_ceil(HEX_WIDTH)
    }

    fn menu(&self, row: Option<usize>, offset: Option<usize>, e: &MouseDownEvent, window: &mut Window, cx: &mut App) {
        let Some(menu) = self.on_menu.clone() else { return };
        let text = gpui_kit::base::TextSelection::selected_text(window, cx);
        let request = MenuRequest {
            row,
            offset,
            selection: (!text.is_empty()).then_some(text),
            position: e.position,
        };
        window.defer(cx, move |window, cx| menu(request, window, cx));
    }

    fn render_rows(&mut self, range: Range<usize>, _window: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let t = *cx.tokens();
        let digits = offset_digits(self.bytes.len());
        range
            .map(|row| {
                let start = row * HEX_WIDTH;
                let slice = &self.bytes[start..(start + HEX_WIDTH).min(self.bytes.len())];
                let ascii: String = slice
                    .iter()
                    .map(|b| if (0x20..0x7f).contains(b) { *b as char } else { '·' })
                    .collect();
                let cells = (0..HEX_WIDTH).map(|i| {
                    let b = slice.get(i).copied();
                    let text: SharedString = b.map_or_else(|| "  ".into(), |b| format!("{b:02x}").into());
                    let offset = start + i;
                    div()
                        .id(("hex-byte", offset))
                        .when(i == 8, |d| d.ml(px(7.)))
                        .mr(px(7.))
                        .when(b == Some(0), |d| d.text_color(t.muted_foreground.hsla().opacity(0.6)))
                        .when(b.is_some(), |d| {
                            d.on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.menu(Some(row), Some(offset), e, window, cx);
                                }),
                            )
                        })
                        .child(text)
                });
                div()
                    .id(("hex-row", row))
                    .role(Role::Row)
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .whitespace_nowrap()
                    .hover(|s| s.bg(t.code_row_hover.hsla()))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.menu(Some(row), None, e, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .w(px((digits as f32 + 2.) * 7.))
                            .flex_none()
                            .text_color(t.syntax_comment.hsla())
                            .child(format!("{start:0digits$x}")),
                    )
                    .child(div().flex().flex_none().children(cells))
                    .child(
                        div().ml(px(7.)).text_color(t.syntax_string.hsla()).child(
                            SelectableText::with_handle(
                                ElementId::Name(format!("{}-ascii-{row}", self.id).into()),
                                self.selection.clone(),
                                ascii,
                            )
                            .document_order(row as u64),
                        ),
                    )
                    .into_any_element()
            })
            .collect()
    }
}

trait Opacity {
    fn opacity(self, a: f32) -> Self;
}

impl Opacity for gpui_kit::Hsla {
    fn opacity(mut self, a: f32) -> Self {
        self.a *= a;
        self
    }
}

impl Focusable for HexView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for HexView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        let digits = offset_digits(self.bytes.len());
        let header_hex: String = (0..HEX_WIDTH)
            .map(|i| format!("{}{i:02x}", if i == 8 { "  " } else { " " }))
            .collect();
        let rows = self.row_count();
        let size = self.bytes.len() as u64;
        div()
            .id(ElementId::Name(self.id.clone()))
            .track_focus(&self.focus)
            .role(Role::Table)
            .aria_label("Hex dump")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background.hsla())
            .text_color(t.foreground.hsla())
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &MouseDownEvent, window, cx| this.menu(None, None, e, window, cx)),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(t.border.hsla())
                    .text_size(px(11.))
                    .text_color(t.muted_foreground.hsla())
                    .child(SharedString::from(format!(
                        "{} bytes ({})",
                        pm_view::body::grouped(size),
                        pm_view::flow::format_size(size)
                    ))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .font_family(mono)
                    .text_size(px(FONT_SIZE))
                    .child(
                        div()
                            .flex_none()
                            .h(px(ROW_HEIGHT))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .whitespace_nowrap()
                            .bg(t.pane_header.hsla())
                            .text_color(t.muted_foreground.hsla())
                            .child(div().w(px((digits as f32 + 2.) * 7.)).flex_none().child("Offset"))
                            .child(div().flex_none().child(SharedString::from(header_hex.trim_start().to_owned())))
                            .child(div().ml(px(14.)).child("ASCII")),
                    )
                    .child(
                        uniform_list(ElementId::Name(format!("{}-rows", self.id).into()), rows, cx.processor(Self::render_rows))
                            .track_scroll(&self.scroll)
                            .flex_1(),
                    ),
            )
            .test_support()
    }
}
