//! KvTable: a name/value table (`Inspector/KvTable.tsx`): headers, query
//! parameters, TLS and certificate rows. Names take a third of the width in
//! the `--syntax-key` colour, values wrap anywhere; both are selectable
//! text. A right-click names the row it hit (`None` between rows), so the
//! owner builds one menu for the table, keyed by row, as the Tauri UI's
//! `data-menu-key` does.

use std::rc::Rc;

use gpui_kit::base::{SelectableText, TestSupportExt as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ElementId, FontWeight, MouseButton, MouseDownEvent, Pixels, Point, Role, SharedString, Window, div, px,
};
use pm_theme::kit::TokensExt as _;

/// What a right-click hit: a row index, or the table outside its rows.
pub type OnMenu = Rc<dyn Fn(Option<usize>, Point<Pixels>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct KvTable {
    id: SharedString,
    label: SharedString,
    rows: Rc<Vec<(SharedString, SharedString)>>,
    on_menu: Option<OnMenu>,
}

impl KvTable {
    /// `id` names the table (`request-headers`); its rows are `<id>-<n>`.
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        rows: Rc<Vec<(SharedString, SharedString)>>,
    ) -> Self {
        KvTable {
            id: id.into(),
            label: label.into(),
            rows,
            on_menu: None,
        }
    }

    pub fn on_menu(mut self, f: impl Fn(Option<usize>, Point<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.on_menu = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for KvTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = *cx.tokens();
        let mono = gpui_kit::component::ActiveTheme::theme(cx).mono_font_family.clone();
        let id = self.id.clone();
        let menu = self.on_menu.clone();
        div()
            .id(ElementId::Name(self.id.clone()))
            .role(Role::Table)
            .aria_label(self.label.clone())
            .w_full()
            .flex()
            .flex_col()
            .font_family(mono)
            .text_size(px(12.))
            .when_some(menu.clone(), |d, menu| {
                d.on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                    menu(None, e.position, window, cx)
                })
            })
            .children(self.rows.iter().enumerate().map(|(i, (name, value))| {
                let row_id: SharedString = format!("{id}-{i}").into();
                let menu = menu.clone();
                div()
                    .id(ElementId::Name(row_id.clone()))
                    .role(Role::Row)
                    .aria_label(SharedString::from(format!("{name}: {value}")))
                    .w_full()
                    .flex()
                    .flex_row()
                    .border_b_1()
                    .border_color(t.border.hsla().opacity(0.5))
                    .when_some(menu, |d, menu| {
                        d.on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            menu(Some(i), e.position, window, cx)
                        })
                    })
                    .child(
                        div()
                            .id(ElementId::Name(format!("{row_id}-n").into()))
                            .role(Role::Cell)
                            .w(gpui_kit::relative(1. / 3.))
                            .flex_none()
                            .px(px(8.))
                            .py(px(4.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.syntax_key.hsla())
                            .child(SelectableText::new(
                                ElementId::Name(format!("{row_id}-name").into()),
                                name.clone(),
                            )),
                    )
                    .child(
                        div()
                            .id(ElementId::Name(format!("{row_id}-v").into()))
                            .role(Role::Cell)
                            .flex_1()
                            .min_w_0()
                            .px(px(8.))
                            .py(px(4.))
                            .text_color(t.foreground.hsla())
                            .child(SelectableText::new(
                                ElementId::Name(format!("{row_id}-value").into()),
                                value.clone(),
                            )),
                    )
            }))
            .test_support()
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
