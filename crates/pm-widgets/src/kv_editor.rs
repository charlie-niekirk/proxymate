//! KvEditor (`Rewrite/KvEditor.tsx`): an editable name/value table for
//! headers, query parameters and form fields, over [`pm_view::kv`]. Rows
//! are tinted where they differ from the original (added, changed),
//! removed rows stay listed, struck through, with "Put back" (a button and
//! their menu), and the framing headers can be kept read-only. G8's
//! Breakpoint Editor and Compose edit headers with it; G9's Rewrite dialog
//! adds its locks.
//!
//! The owner reads the rows after each [`KvChanged`].

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, MouseButton,
    MouseDownEvent, Pixels, Point, Role, SharedString, Subscription, Window, div, px,
};
use pm_theme::kit::TokensExt as _;
use pm_view::kv::{self, KvOptions, KvRow, KvStatus};

/// "Put back" from a removed row's menu.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = kv_editor, no_json)]
pub struct PutBack {
    pub id: u64,
}

/// A right-click on a removed row: the owner shows Copy name, Copy value
/// and Put back (dispatching [`PutBack`]).
pub struct MenuRequest {
    pub row: KvRow,
    pub position: Point<Pixels>,
}

pub type OnMenu = Rc<dyn Fn(MenuRequest, &mut Window, &mut App)>;

/// The rows changed (typed, added, removed or put back).
pub struct KvChanged;

pub struct KvEditor {
    id: SharedString,
    title: Option<SharedString>,
    add_label: SharedString,
    rows: Vec<KvRow>,
    original: Vec<KvRow>,
    opts: KvOptions,
    name_width: f32,
    inputs: HashMap<u64, (Entity<InputState>, Entity<InputState>)>,
    focus: FocusHandle,
    on_menu: Option<OnMenu>,
    _subscriptions: HashMap<u64, [Subscription; 2]>,
}

impl EventEmitter<KvChanged> for KvEditor {}

impl KvEditor {
    /// `rows` edited, compared with `original` (often the same rows).
    pub fn new(
        id: impl Into<SharedString>,
        rows: Vec<KvRow>,
        original: Vec<KvRow>,
        opts: KvOptions,
        cx: &mut Context<Self>,
    ) -> Self {
        KvEditor {
            id: id.into(),
            title: None,
            add_label: "Header".into(),
            rows,
            original,
            opts,
            name_width: 200.,
            inputs: HashMap::new(),
            focus: cx.focus_handle(),
            on_menu: None,
            _subscriptions: HashMap::new(),
        }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn add_label(mut self, label: impl Into<SharedString>) -> Self {
        self.add_label = label.into();
        self
    }

    pub fn name_width(mut self, width: f32) -> Self {
        self.name_width = width;
        self
    }

    pub fn on_menu(mut self, f: impl Fn(MenuRequest, &mut Window, &mut App) + 'static) -> Self {
        self.on_menu = Some(Rc::new(f));
        self
    }

    pub fn rows(&self) -> &[KvRow] {
        &self.rows
    }

    pub fn original(&self) -> &[KvRow] {
        &self.original
    }

    /// The rows as headers (nameless ones dropped, names trimmed).
    pub fn headers(&self) -> Vec<pm_model::Header> {
        kv::to_headers(&self.rows)
    }

    pub fn view(&self) -> kv::KvView {
        kv::view(&self.rows, &self.original, self.opts)
    }

    /// New rows and original (the inputs are made again).
    pub fn set_rows(&mut self, rows: Vec<KvRow>, original: Vec<KvRow>, cx: &mut Context<Self>) {
        self.rows = rows;
        self.original = original;
        self.inputs.clear();
        self._subscriptions.clear();
        cx.notify();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(KvChanged);
        cx.notify();
    }

    pub fn add_row(&mut self, cx: &mut Context<Self>) {
        self.rows.push(KvRow::new("", ""));
        self.changed(cx);
    }

    /// Adds a row with this name and value (the dev remote, tests).
    pub fn push(&mut self, name: &str, value: &str, cx: &mut Context<Self>) {
        self.rows.push(KvRow::new(name, value));
        self.changed(cx);
    }

    pub fn remove(&mut self, id: u64, cx: &mut Context<Self>) {
        kv::remove(&mut self.rows, id);
        self.inputs.remove(&id);
        self._subscriptions.remove(&id);
        self.changed(cx);
    }

    pub fn put_back(&mut self, id: u64, cx: &mut Context<Self>) {
        self.rows = kv::put_back(&self.rows, &self.original, id);
        self.changed(cx);
    }

    /// Sets a row's name and value (the dev remote, tests), as typing would.
    pub fn set(&mut self, id: u64, name: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        kv::set_name(&mut self.rows, id, name);
        kv::set_value(&mut self.rows, id, value);
        if let Some((n, v)) = self.inputs.get(&id).cloned() {
            n.update(cx, |s, cx| s.set_value(name.to_owned(), window, cx));
            v.update(cx, |s, cx| s.set_value(value.to_owned(), window, cx));
        }
        self.changed(cx);
    }

    fn ensure_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for row in self.rows.clone() {
            if self.inputs.contains_key(&row.id) {
                continue;
            }
            let make = |text: &str, placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
                let text = text.to_owned();
                cx.new(|cx| {
                    let mut s = InputState::new(window, cx).placeholder(placeholder);
                    s.set_value(text, window, cx);
                    s
                })
            };
            let name = make(&row.name, "name", window, cx);
            let value = make(&row.value, "value", window, cx);
            let id = row.id;
            let on_name = cx.subscribe(&name, move |this, s, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    let text = s.read(cx).value().to_string();
                    if this.rows.iter().any(|r| r.id == id && r.name != text) {
                        kv::set_name(&mut this.rows, id, &text);
                        this.changed(cx);
                    }
                }
            });
            let on_value = cx.subscribe(&value, move |this, s, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    let text = s.read(cx).value().to_string();
                    if this.rows.iter().any(|r| r.id == id && r.value != text) {
                        kv::set_value(&mut this.rows, id, &text);
                        this.changed(cx);
                    }
                }
            });
            self.inputs.insert(id, (name, value));
            self._subscriptions.insert(id, [on_name, on_value]);
        }
    }

    fn open_menu(&mut self, row: KvRow, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(menu) = self.on_menu.clone() else { return };
        let request = MenuRequest {
            row,
            position: e.position,
        };
        cx.defer_in(window, move |_, window, cx| menu(request, window, cx));
    }
}

impl Focusable for KvEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for KvEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_inputs(window, cx);
        let t = *cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        let view = self.view();
        let rows: Vec<(KvRow, kv::RowView)> = self.rows.iter().cloned().zip(view.rows).collect();
        let name_width = px(self.name_width);
        let editor_id = self.id.clone();
        div()
            .id(ElementId::Name(self.id.clone()))
            .track_focus(&self.focus)
            .role(Role::Table)
            .aria_label(self.title.clone().unwrap_or_else(|| "Headers".into()))
            .w_full()
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(12.))
            .on_action(cx.listener(|this, a: &PutBack, _, cx| this.put_back(a.id, cx)))
            .children(self.title.clone().map(|title| {
                div()
                    .mb(px(2.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.muted_foreground.hsla())
                    .child(title)
            }))
            .children(rows.into_iter().enumerate().map(|(i, (row, v))| {
                let (name, value) = self.inputs.get(&row.id).cloned().expect("inputs made above");
                let id = row.id;
                let bg = match v.status {
                    KvStatus::Added => Some(t.diff_add_bg),
                    KvStatus::Changed => Some(t.diff_change_bg),
                    KvStatus::Same => None,
                };
                let label = format!(
                    "{} {}{}{}",
                    row.name,
                    row.value,
                    if v.status == KvStatus::Same {
                        String::new()
                    } else {
                        format!(", {}", v.status.id())
                    },
                    v.note.map(|n| format!(", {n}")).unwrap_or_default()
                );
                let action = if v.removable {
                    Button::new(SharedString::from(format!("{editor_id}-remove-{i}")))
                        .ghost()
                        .xsmall()
                        .icon(gpui_kit::assets::IconName::X)
                        .tooltip("Remove")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.remove(id, cx)))
                } else {
                    Button::new(SharedString::from(format!("{editor_id}-locked-{i}")))
                        .ghost()
                        .xsmall()
                        .icon(gpui_kit::assets::IconName::Lock)
                        .tooltip(v.note.unwrap_or(kv::FRAMING_NOTE))
                };
                div()
                    .id(SharedString::from(format!("{editor_id}-row-{i}")))
                    .role(Role::Row)
                    .aria_label(SharedString::from(label))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(2.))
                    .rounded(px(3.))
                    .when_some(bg, |d, bg| d.bg(bg.hsla()))
                    .child(
                        div().w(name_width).flex_none().font_family(mono.clone()).child(
                            Input::new(&name)
                                .xsmall()
                                .disabled(v.name_locked)
                                .aria_label(SharedString::from(format!("Name {}", i + 1))),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().font_family(mono.clone()).child(
                            Input::new(&value)
                                .xsmall()
                                .disabled(v.value_locked)
                                .aria_label(SharedString::from(format!("Value {}", i + 1))),
                        ),
                    )
                    .child(action)
            }))
            .children(view.removed.into_iter().enumerate().map(|(i, row)| {
                let id = row.id;
                let for_menu = row.clone();
                div()
                    .id(SharedString::from(format!("{editor_id}-removed-{i}")))
                    .role(Role::Row)
                    .aria_label(SharedString::from(format!("{} {}, removed", row.name, row.value)))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(2.))
                    .h(px(24.))
                    .rounded(px(3.))
                    .bg(t.diff_del_bg.hsla())
                    .font_family(mono.clone())
                    .text_color(t.diff_del.hsla())
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_menu(for_menu.clone(), e, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .w(name_width)
                            .flex_none()
                            .px(px(8.))
                            .truncate()
                            .line_through()
                            .child(row.name),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px(px(8.))
                            .truncate()
                            .line_through()
                            .child(row.value),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{editor_id}-put-back-{i}")))
                            .ghost()
                            .xsmall()
                            .icon(gpui_kit::assets::IconName::RotateCcw)
                            .tooltip("Put back")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.put_back(id, cx))),
                    )
            }))
            .child(
                div().flex().child(
                    Button::new(SharedString::from(format!("{editor_id}-add")))
                        .ghost()
                        .xsmall()
                        .icon(gpui_kit::assets::IconName::Plus)
                        .label(self.add_label.clone())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_row(cx))),
                ),
            )
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::*;

    fn setup(cx: &mut TestAppContext) -> (Entity<KvEditor>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            pm_theme::kit::apply(pm_theme::Mode::Light, cx);
        });
        let original = vec![KvRow::new("accept", "*/*"), KvRow::new("content-length", "2")];
        let opts = KvOptions {
            fold: true,
            framing_locked: true,
            ..Default::default()
        };
        let (editor, cx) = cx.add_window_view(move |_, cx| KvEditor::new("kv", original.clone(), original, opts, cx));
        cx.run_until_parked();
        (editor, cx)
    }

    fn click(cx: &mut VisualTestContext, id: &'static str) {
        cx.update(|window, cx| window.click(id, cx));
        cx.run_until_parked();
    }

    fn exists(cx: &mut VisualTestContext, id: &'static str) -> bool {
        cx.update(|window, _| window.try_find(id).is_some())
    }

    #[gpui_kit::test]
    fn removes_puts_back_and_tints(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx);
        let first = editor.read_with(cx, |e, _| e.rows()[0].id);
        // The framing header has a lock in place of its remove button.
        assert!(exists(cx, "kv-remove-0"));
        assert!(!exists(cx, "kv-remove-1"));
        assert!(exists(cx, "kv-locked-1"));
        click(cx, "kv-remove-0");
        let view = editor.read_with(cx, |e, _| e.view());
        assert_eq!(view.removed.len(), 1);
        click(cx, "kv-put-back-0");
        assert_eq!(editor.read_with(cx, |e, _| e.rows()[0].id), first, "back in its place");
        click(cx, "kv-add");
        let id = editor.read_with(cx, |e, _| e.rows()[2].id);
        editor.update_in(cx, |e, window, cx| e.set(id, "x-new", "1", window, cx));
        let view = editor.read_with(cx, |e, _| e.view());
        assert_eq!(view.rows[2].status, KvStatus::Added);
        assert_eq!(editor.read_with(cx, |e, _| e.headers().len()), 3);
    }
}
