//! A held WebSocket message (`MessageForm`, M19): its payload as text, a
//! JSON tree or hex. Send goes on with any edits (hex edits as bytes), Drop
//! never forwards it. Later messages in its direction wait behind it.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputEvent;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Entity, FontWeight, Role, SharedString, Subscription, Window, div, px};
use pm_model::{BreakpointInfo, BreakpointResolution, MessageEdits};
use pm_theme::kit::TokensExt as _;
use pm_view::breakpoints::{self as logic, MessageDraft, MessageMode};
use pm_view::json::editor::{JsonEditorModel, is_pretty};
use pm_widgets::code_editor::CodeEditor;
use pm_widgets::json_editor::{self, JsonEditor};

use super::window::phase_pill;
use crate::bridge::keys::{self, EditingCaches};
use crate::bridge::mutations;
use crate::bridge::resource::Load;
use crate::features::editing::{error_line, menus};

/// The widget showing the payload in the current mode.
enum Shown {
    Code(Entity<CodeEditor>),
    Tree(Entity<JsonEditor>),
}

pub struct MessageForm {
    pub bp: BreakpointInfo,
    pub now_ms: f64,
    draft: Option<MessageDraft>,
    shown: Option<Shown>,
    load_error: Option<String>,
    error: Option<String>,
    busy: bool,
    caches: EditingCaches,
    _shown_events: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl MessageForm {
    pub fn new(bp: BreakpointInfo, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let caches = keys::editing_caches(cx);
        let subscriptions = vec![cx.observe_in(&caches.breakpoint_body, window, |this, _, window, cx| {
            this.load(window, cx)
        })];
        let mut form = MessageForm {
            now_ms: crate::features::breakpoints::window::now_ms(),
            bp,
            draft: None,
            shown: None,
            load_error: None,
            error: None,
            busy: false,
            caches,
            _shown_events: None,
            _subscriptions: subscriptions,
        };
        form.load(window, cx);
        form
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.is_some() || self.load_error.is_some() {
            return;
        }
        let Some(held) = self.bp.message.clone() else { return };
        let id = self.bp.id;
        match self.caches.breakpoint_body.update(cx, |c, cx| c.get(&id, cx)) {
            Load::Ready(bytes) => {
                self.draft = Some(MessageDraft::new(bytes.to_vec(), held.opcode));
                self.show(window, cx);
            }
            Load::Failed(err) => {
                self.load_error = Some(err.to_string());
                cx.notify();
            }
            Load::Loading => {}
        }
    }

    pub fn draft(&self) -> Option<&MessageDraft> {
        self.draft.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn tree(&self) -> Option<&Entity<JsonEditor>> {
        match &self.shown {
            Some(Shown::Tree(tree)) => Some(tree),
            _ => None,
        }
    }

    pub fn code(&self) -> Option<&Entity<CodeEditor>> {
        match &self.shown {
            Some(Shown::Code(code)) => Some(code),
            _ => None,
        }
    }

    /// Builds the widget for the draft's mode, from its current text or hex.
    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &self.draft else { return };
        let editable = self.bp.body.editable;
        let id = self.bp.id;
        match draft.mode {
            MessageMode::Tree => {
                let original = draft.original_text.clone();
                let pretty = original.as_deref().is_some_and(is_pretty);
                let Ok(model) = JsonEditorModel::for_text(&draft.current_text(), original.as_deref()) else {
                    return;
                };
                let tree = cx.new(|cx| {
                    JsonEditor::new(format!("bp-message-json-{id}"), "Message as JSON", model, window, cx)
                        .on_menu(|req, window, cx| menus::json_row(&req.spec).show(req.position, window, cx))
                });
                self._shown_events = Some(cx.subscribe(&tree, move |this, tree, _: &json_editor::Changed, cx| {
                    let text = tree.read(cx).model().text(pretty);
                    if let Some(d) = &mut this.draft {
                        d.text = Some(text);
                    }
                }));
                self.shown = Some(Shown::Tree(tree));
            }
            mode => {
                let hex = mode == MessageMode::Hex;
                let text = if hex { draft.current_hex() } else { draft.current_text() };
                let language = if !hex && draft.original_text.as_deref().is_some_and(logic::is_json) {
                    "json"
                } else {
                    "text"
                };
                let (eid, label) = if hex {
                    (format!("bp-message-hex-{id}"), "Message as hex")
                } else {
                    (format!("bp-message-text-{id}"), "Message as text")
                };
                let code = cx.new(|cx| {
                    let mut editor = CodeEditor::new(eid, label, language, !editable, window, cx);
                    editor.set_wrap(hex, window, cx);
                    editor.set_text(text.into(), window, cx);
                    editor
                });
                let state = code.read(cx).state().clone();
                self._shown_events = Some(cx.subscribe(&state, move |this, state, e: &InputEvent, cx| {
                    if !matches!(e, InputEvent::Change) {
                        return;
                    }
                    let value = state.read(cx).value().to_string();
                    if let Some(d) = &mut this.draft {
                        if hex {
                            if d.hex.as_deref() != Some(value.as_str()) && value != d.current_hex() {
                                d.hex = Some(value);
                            }
                        } else if value != d.current_text() {
                            d.text = Some(value);
                        }
                    }
                }));
                self.shown = Some(Shown::Code(code));
            }
        }
        cx.notify();
    }

    /// Text / Tree / Hex: carries the edits over, or says why it can't.
    pub fn switch_to(&mut self, mode: MessageMode, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &mut self.draft else { return };
        self.error = None;
        match draft.switch_to(mode) {
            Ok(()) => self.show(window, cx),
            Err(err) => {
                self.error = Some(err);
                cx.notify();
            }
        }
    }

    /// Types into the shown editor (the dev remote, tests): text or hex.
    pub fn set_payload(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Shown::Code(code)) = &self.shown {
            code.update(cx, |c, cx| c.set_text(text.to_owned().into(), window, cx));
        }
        if let Some(d) = &mut self.draft {
            match d.mode {
                MessageMode::Hex => d.hex = Some(text.to_owned()),
                _ => d.text = Some(text.to_owned()),
            }
        }
        if matches!(self.draft.as_ref().map(|d| d.mode), Some(MessageMode::Tree)) {
            self.show(window, cx);
        }
        cx.notify();
    }

    fn resolve(&mut self, resolution: BreakpointResolution, cx: &mut Context<Self>) {
        self.error = None;
        self.busy = true;
        let this = cx.weak_entity();
        mutations::resolve_breakpoint(
            self.bp.id,
            resolution,
            move |answer, cx| {
                let _ = this.update(cx, |f, cx| {
                    f.busy = false;
                    f.error = answer.err();
                    cx.notify();
                });
            },
            cx,
        );
        cx.notify();
    }

    /// The edits Send sends, or why it can't.
    pub fn edits(&self) -> Result<MessageEdits, String> {
        match &self.draft {
            Some(d) => d.edits(self.bp.body.editable),
            None => Ok(MessageEdits::default()),
        }
    }

    /// Send, with `extra` (the dev remote's) merged over the form's edits.
    pub fn send(&mut self, extra: Option<MessageEdits>, cx: &mut Context<Self>) {
        match self.edits() {
            Ok(edits) => {
                let edits = match extra {
                    Some(extra) => logic::merge_edits(edits, extra),
                    None => edits,
                };
                self.resolve(logic::execute(edits), cx);
            }
            Err(err) => {
                self.error = Some(err);
                cx.notify();
            }
        }
    }

    pub fn drop_message(&mut self, cx: &mut Context<Self>) {
        self.resolve(BreakpointResolution::Abort, cx);
    }
}

impl Render for MessageForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let bp = self.bp.clone();
        let Some(held) = bp.message.clone() else {
            return div().into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        let modes = self.draft.as_ref().map(MessageDraft::modes).unwrap_or_default();
        let current = self.draft.as_ref().map(|d| d.mode);
        let busy = self.busy;
        let tabs: Vec<AnyElement> = modes
            .into_iter()
            .map(|m| {
                let on = current == Some(m);
                let b = Button::new(SharedString::from(format!("bp-message-{}", m.id())))
                    .xsmall()
                    .label(m.label())
                    .selected(on)
                    .accessibility_label(SharedString::from(format!("Edit as {}", m.label())))
                    .on_click(cx.listener(move |this, _, window, cx| this.switch_to(m, window, cx)));
                if on { b.primary() } else { b.ghost() }.into_any_element()
            })
            .collect();
        let body: AnyElement = match (&self.shown, &self.load_error) {
            (Some(Shown::Code(c)), _) => c.clone().into_any_element(),
            (Some(Shown::Tree(tree)), _) => tree.clone().into_any_element(),
            (None, Some(err)) => div()
                .p(px(8.))
                .text_size(px(12.))
                .text_color(t.s_failed.hsla())
                .child(err.clone())
                .into_any_element(),
            (None, None) => div()
                .p(px(8.))
                .text_size(px(12.))
                .text_color(t.muted_foreground.hsla())
                .child("Loading…")
                .into_any_element(),
        };
        div()
            .id("message-form")
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(t.border.hsla())
                    .text_size(px(12.))
                    .child(phase_pill(bp.phase, &t))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(bp.rule_name.clone()),
                    )
                    .child(
                        div()
                            .id("bp-countdown")
                            .ml_auto()
                            .flex_none()
                            .text_color(t.muted_foreground.hsla())
                            .child(format!(
                                "continues in {}",
                                logic::countdown(bp.deadline_ms, self.now_ms)
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .text_size(px(12.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(mono)
                            .text_color(t.muted_foreground.hsla())
                            .child(bp.request.url.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.muted_foreground.hsla())
                                    .child(logic::message_heading(held.direction, held.opcode, bp.body.size)),
                            )
                            .child(div().text_color(t.muted_foreground.hsla()).child(logic::MESSAGE_WAITS))
                            .child(
                                div()
                                    .id("bp-message-mode")
                                    .role(Role::Group)
                                    .aria_label("Edit the message as")
                                    .ml_auto()
                                    .flex()
                                    .gap(px(4.))
                                    .children(tabs),
                            ),
                    )
                    .children(
                        bp.body
                            .note
                            .clone()
                            .map(|n| div().text_color(t.muted_foreground.hsla()).child(n)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(t.border.hsla())
                            .overflow_hidden()
                            .child(body),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(t.border.hsla())
                    .child(error_line("bp-error", self.error.as_deref(), &t))
                    .child(
                        Button::new("bp-drop")
                            .ghost()
                            .small()
                            .icon(gpui_kit::assets::IconName::Trash)
                            .label("Drop")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.drop_message(cx))),
                    )
                    .child(
                        Button::new("bp-send")
                            .primary()
                            .small()
                            .icon(gpui_kit::assets::IconName::CirclePlay)
                            .label("Send")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.send(None, cx))),
                    ),
            )
            .test_support()
            .into_any_element()
    }
}
