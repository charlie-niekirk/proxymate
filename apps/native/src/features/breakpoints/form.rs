//! A paused request or response (`BreakpointForm`): its head (method and
//! URL, or the status), headers and body, editable before it goes on.
//! Execute sends it with the edits (only what changed), Abort closes the
//! connection, and Respond… answers the client with a response of its own.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, FontWeight, Role, Subscription, Window, div, px};
use pm_model::{BreakpointInfo, BreakpointResolution, MessageEdits, Phase};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::breakpoints::{self as logic, Answer, HttpDraft};
use pm_view::compose::language_for;
use pm_view::http::header_value;
use pm_view::kv::{self, KvOptions};
use pm_widgets::kv_editor::{self, KvChanged, KvEditor};

use super::window::phase_pill;
use crate::bridge::keys::{self, EditingCaches};
use crate::bridge::mutations;
use crate::bridge::resource::Load;
use crate::features::editing::{BodyEditor, error_line, heading, line_input, menus};

pub struct HttpForm {
    pub bp: BreakpointInfo,
    pub now_ms: f64,
    method: Entity<InputState>,
    url: Entity<InputState>,
    status: Entity<InputState>,
    headers: Entity<KvEditor>,
    body: Option<Entity<BodyEditor>>,
    /// The paused body as text (lossy), once it's here.
    original_body: Option<String>,
    body_error: Option<String>,
    responding: bool,
    answer_status: Entity<InputState>,
    answer_headers: Entity<KvEditor>,
    answer_body: Option<Entity<BodyEditor>>,
    error: Option<String>,
    busy: bool,
    caches: EditingCaches,
    _subscriptions: Vec<Subscription>,
}

fn kv_menu(req: kv_editor::MenuRequest, window: &mut Window, cx: &mut App) {
    menus::removed_row(&req.row).show(req.position, window, cx);
}

impl HttpForm {
    pub fn new(bp: BreakpointInfo, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = HttpDraft::of(&bp);
        let input = |text: String, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(text))
        };
        let method = input(draft.method.clone(), window, cx);
        let url = input(draft.url.clone(), window, cx);
        let status = input(draft.status.clone(), window, cx);
        let headers = cx.new(|cx| {
            KvEditor::new(
                format!("bp-headers-{}", bp.id),
                draft.headers.clone(),
                draft.original_headers.clone(),
                KvOptions {
                    fold: true,
                    ..Default::default()
                },
                cx,
            )
            .title("Headers")
            .name_width(220.)
            .on_menu(kv_menu)
        });
        let answer = Answer::default();
        let answer_status = input(answer.status.clone(), window, cx);
        let answer_headers = cx.new(|cx| {
            KvEditor::new(
                format!("bp-answer-headers-{}", bp.id),
                answer.headers.clone(),
                Vec::new(),
                KvOptions {
                    fold: true,
                    ..Default::default()
                },
                cx,
            )
            .title("Headers")
            .name_width(220.)
            .on_menu(kv_menu)
        });
        let caches = keys::editing_caches(cx);
        let subscriptions = vec![
            cx.observe_in(&caches.breakpoint_body, window, |this, _, window, cx| {
                this.load_body(window, cx)
            }),
            cx.subscribe_in(&headers, window, |this, _, _: &KvChanged, window, cx| {
                this.follow_content_type(window, cx)
            }),
            cx.subscribe_in(&answer_headers, window, |this, _, _: &KvChanged, window, cx| {
                let language = language_for(header_value(&this.answer_headers.read(cx).headers(), "content-type"));
                if let Some(body) = this.answer_body.clone() {
                    body.update(cx, |b, cx| b.set_language(language, window, cx));
                }
            }),
            cx.subscribe(&method, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        let mut form = HttpForm {
            now_ms: crate::features::breakpoints::window::now_ms(),
            bp,
            method,
            url,
            status,
            headers,
            body: None,
            original_body: None,
            body_error: None,
            responding: false,
            answer_status,
            answer_headers,
            answer_body: None,
            error: None,
            busy: false,
            caches,
            _subscriptions: subscriptions,
        };
        form.load_body(window, cx);
        form
    }

    /// Reads the paused body (once): the editor waits for it.
    fn load_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.body.is_some() || self.body_error.is_some() {
            return;
        }
        let id = self.bp.id;
        let load = self.caches.breakpoint_body.update(cx, |c, cx| c.get(&id, cx));
        match load {
            Load::Ready(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let language = language_for(header_value(&self.headers.read(cx).headers(), "content-type"));
                let editable = self.bp.body.editable;
                let original = text.clone();
                let body = cx.new(|cx| {
                    BodyEditor::new(
                        format!("bp-body-{id}"),
                        "Body",
                        text,
                        Some(original),
                        language,
                        !editable,
                        window,
                        cx,
                    )
                });
                self.original_body = Some(String::from_utf8_lossy(&bytes).into_owned());
                self.body = Some(body);
                cx.notify();
            }
            Load::Failed(err) => {
                self.body_error = Some(err.to_string());
                cx.notify();
            }
            Load::Loading => {}
        }
    }

    fn follow_content_type(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let language = language_for(header_value(&self.headers.read(cx).headers(), "content-type"));
        if let Some(body) = self.body.clone() {
            body.update(cx, |b, cx| b.set_language(language, window, cx));
        }
        cx.notify();
    }

    pub fn headers(&self) -> &Entity<KvEditor> {
        &self.headers
    }

    pub fn body(&self) -> Option<&Entity<BodyEditor>> {
        self.body.as_ref()
    }

    pub fn answer_body(&self) -> Option<&Entity<BodyEditor>> {
        self.answer_body.as_ref()
    }

    pub fn answer_headers(&self) -> &Entity<KvEditor> {
        &self.answer_headers
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn is_responding(&self) -> bool {
        self.responding
    }

    /// Types into the head's inputs (the dev remote, tests).
    pub fn set_head(
        &mut self,
        method: Option<&str>,
        url: Option<&str>,
        status: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (state, value) in [(&self.method, method), (&self.url, url), (&self.status, status)] {
            if let Some(v) = value {
                state.update(cx, |s, cx| s.set_value(v.to_owned(), window, cx));
            }
        }
        cx.notify();
    }

    /// Types the Respond… form's status (the dev remote, tests).
    pub fn set_answer_status(&mut self, status: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.answer_status
            .update(cx, |s, cx| s.set_value(status.to_owned(), window, cx));
    }

    /// The form as a draft.
    pub fn draft(&self, cx: &App) -> HttpDraft {
        let headers = self.headers.read(cx);
        HttpDraft {
            method: self.method.read(cx).value().trim().to_uppercase(),
            url: self.url.read(cx).value().trim().to_owned(),
            status: self.status.read(cx).value().to_string(),
            headers: headers.rows().to_vec(),
            original_headers: headers.original().to_vec(),
            body: self.body.as_ref().map(|b| b.read(cx).text().to_owned()),
        }
    }

    /// The edits Execute sends, or why it can't.
    pub fn edits(&self, cx: &App) -> Result<MessageEdits, String> {
        self.draft(cx)
            .edits(&self.bp, self.original_body.as_deref().unwrap_or_default())
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

    /// Execute, with `extra` (the dev remote's) merged over the form's edits.
    pub fn execute(&mut self, extra: Option<MessageEdits>, cx: &mut Context<Self>) {
        match self.edits(cx) {
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

    pub fn abort(&mut self, cx: &mut Context<Self>) {
        self.resolve(BreakpointResolution::Abort, cx);
    }

    /// Respond…: shows the answer form.
    pub fn start_responding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.responding {
            return;
        }
        self.responding = true;
        let id = self.bp.id;
        self.answer_body = Some(cx.new(|cx| {
            BodyEditor::new(
                format!("bp-answer-body-{id}"),
                "Response body",
                String::new(),
                None,
                "json",
                false,
                window,
                cx,
            )
        }));
        cx.notify();
    }

    /// Send response: answers the client with the answer form.
    pub fn respond(&mut self, cx: &mut Context<Self>) {
        let answer = Answer {
            status: self.answer_status.read(cx).value().to_string(),
            headers: self.answer_headers.read(cx).rows().to_vec(),
            body: self
                .answer_body
                .as_ref()
                .map(|b| b.read(cx).text().to_owned())
                .unwrap_or_default(),
        };
        match answer.resolution() {
            Ok(resolution) => self.resolve(resolution, cx),
            Err(err) => {
                self.error = Some(err);
                cx.notify();
            }
        }
    }

    fn head(&self, t: &Tokens, cx: &App) -> AnyElement {
        if self.bp.phase == Phase::Request {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .w(px(96.))
                        .flex_none()
                        .flex()
                        .child(line_input(&self.method, "bp-method", "Method", cx)),
                )
                .child(line_input(&self.url, "bp-url", "URL", cx))
                .into_any_element()
        } else {
            let mono = gpui_kit::component::ActiveTheme::theme(cx).mono_font_family.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(mono)
                        .text_size(px(12.))
                        .text_color(t.muted_foreground.hsla())
                        .child(format!("{} {}", self.bp.request.method, self.bp.request.url)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(t.muted_foreground.hsla())
                                .child("Status"),
                        )
                        .child(
                            div()
                                .w(px(80.))
                                .flex_none()
                                .flex()
                                .child(line_input(&self.status, "bp-status", "Status", cx)),
                        ),
                )
                .into_any_element()
        }
    }
}

impl Render for HttpForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let bp = self.bp.clone();
        let busy = self.busy;
        let body = match (&self.body, &self.body_error) {
            (Some(body), _) => body.clone().into_any_element(),
            (None, Some(err)) => div()
                .text_size(px(12.))
                .text_color(t.s_failed.hsla())
                .child(err.clone())
                .into_any_element(),
            (None, None) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(t.muted_foreground.hsla())
                .child("Loading…")
                .into_any_element(),
        };
        let answer = self.responding.then(|| {
            div()
                .id("bp-respond-form")
                .role(Role::Group)
                .aria_label("Answer the client with")
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(8.))
                .rounded(px(6.))
                .border_1()
                .border_color(t.border.hsla())
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Answer the client with"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(t.muted_foreground.hsla())
                                .child("Status"),
                        )
                        .child(
                            div()
                                .w(px(80.))
                                .flex_none()
                                .flex()
                                .child(line_input(&self.answer_status, "bp-answer-status", "Answer status", cx)),
                        ),
                )
                .child(self.answer_headers.clone())
                .children(
                    self.answer_body
                        .clone()
                        .map(|b| div().h(px(140.)).child(b).into_any_element()),
                )
        });
        div()
            .id("breakpoint-form")
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
                            .role(Role::Timer)
                            .text_color(t.muted_foreground.hsla())
                            .child(format!(
                                "continues in {}",
                                logic::countdown(bp.deadline_ms, self.now_ms)
                            )),
                    ),
            )
            .child(
                div()
                    .id("bp-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .p(px(12.))
                            .child(self.head(&t, cx))
                            .child(self.headers.clone())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(heading(logic::body_heading(bp.body.size), &t))
                                    .children(bp.body.note.clone().map(|n| {
                                        div()
                                            .text_size(px(12.))
                                            .text_color(t.muted_foreground.hsla())
                                            .child(n)
                                    }))
                                    .child(div().h(px(260.)).child(body)),
                            )
                            .children(answer),
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
                        Button::new("bp-abort")
                            .ghost()
                            .small()
                            .icon(gpui_kit::assets::IconName::CircleX)
                            .label("Abort")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.abort(cx))),
                    )
                    .child(if self.responding {
                        Button::new("bp-respond-send")
                            .outline()
                            .small()
                            .icon(gpui_kit::assets::IconName::Reply)
                            .label("Send response")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.respond(cx)))
                    } else {
                        Button::new("bp-respond")
                            .outline()
                            .small()
                            .icon(gpui_kit::assets::IconName::Reply)
                            .label("Respond…")
                            .on_click(cx.listener(|this, _, window, cx| this.start_responding(window, cx)))
                    })
                    .child(
                        Button::new("bp-execute")
                            .primary()
                            .small()
                            .icon(gpui_kit::assets::IconName::CirclePlay)
                            .label("Execute")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.execute(None, cx))),
                    ),
            )
            .test_support()
    }
}

/// The rows of `headers` the form would send (for the dev remote).
pub fn header_rows(form: &HttpForm, cx: &App) -> Vec<(String, String)> {
    kv::to_headers(form.headers.read(cx).rows())
        .into_iter()
        .map(|h| (h.name, h.value))
        .collect()
}
