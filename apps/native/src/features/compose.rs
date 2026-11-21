//! Compose and Edit & Replay (G8, `Compose/ComposeDialog.tsx`): a window
//! with the method and URL, the headers (a KvEditor, tinted against the
//! request it started from), and a body with a content-type picker (G4's
//! code editor, highlighted for the type). A binary body from Edit &
//! Replay is sent as recorded unless replaced with text. Send (⌘↩) records
//! the request as a new live flow, through the rules unless "Apply rules"
//! is off, and selects it. This replaces G2's `Compose` stub.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, Global, KeyBinding, Render,
    Role, SharedString, Subscription, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions, div, px, size,
};
use pm_model::RequestSpec;
use pm_theme::kit::TokensExt as _;
use pm_view::compose::{self as logic, CONTENT_TYPES, ComposeDraft, ContentTypeChoice, METHODS, language_for};
use pm_view::kv::KvOptions;
use pm_widgets::kv_editor::{KvChanged, KvEditor};

use crate::bridge::mutations;
use crate::features::chrome::Compose;
use crate::features::commands;
use crate::features::context_menu::Menu;
use crate::features::editing::{BodyEditor, BodyInit, error_line, line_input, menus};

pub const DEFAULT_SIZE: (f32, f32) = (920., 640.);
pub const MIN_SIZE: (f32, f32) = (640., 420.);
pub const KEY_CONTEXT: &str = "ComposeWindow";

/// The content-type picker's choice (`None`: no Content-Type).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SetComposeContentType {
    pub value: Option<SharedString>,
}

/// The method picker's choice.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SetComposeMethod {
    pub method: SharedString,
}

gpui_kit::actions!(proxymate, [SendCompose]);

pub fn register(cx: &mut App) {
    let primary = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
    let stroke: &'static str = Box::leak(format!("{primary}-enter").into_boxed_str());
    cx.bind_keys([KeyBinding::new(stroke, SendCompose, Some(KEY_CONTEXT))]);
    cx.on_action(|_: &Compose, cx| open(None, cx));
    commands::add("compose", commands::always, |_, cx| open(None, cx), cx);
}

/// Opens Compose (or brings it forward) with `spec` (Edit & Replay) or a
/// new request.
pub fn open(spec: Option<RequestSpec>, cx: &mut App) {
    cx.defer(move |cx| open_now(spec, cx));
}

struct Open {
    window: AnyWindowHandle,
    view: WeakEntity<ComposeWindow>,
}

impl Global for Open {}

fn open_now(spec: Option<RequestSpec>, cx: &mut App) {
    if let Some((window, view)) = ComposeWindow::global(cx) {
        let _ = window.update(cx, |_, window, cx| {
            window.activate_window();
            view.update(cx, |w, cx| w.start(spec, window, cx));
        });
        return;
    }
    let title: SharedString = if spec.is_some() { "Edit & Replay" } else { "Compose" }.into();
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(title),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)),
            cx,
        ))),
        window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        app_id: Some(pm_app::BUNDLE_ID.into()),
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| ComposeWindow::new(spec, window, cx));
        ComposeWindow::register(window.window_handle(), &view, cx);
        let focus = view.read(cx).url.focus_handle(cx);
        window.focus(&focus, cx);
        cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
    });
    match opened {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Err(err) => eprintln!("can't open Compose: {err}"),
    }
}

pub struct ComposeWindow {
    /// Edit & Replay (it started from a recorded request).
    edit_replay: bool,
    method: Entity<InputState>,
    url: Entity<InputState>,
    headers: Entity<KvEditor>,
    body: Entity<BodyEditor>,
    /// A binary body from Edit & Replay, sent as recorded (base64).
    binary: Option<String>,
    apply_rules: bool,
    error: Option<String>,
    sending: bool,
    /// The last flow sent (the dev remote reads it).
    pub sent: Option<pm_model::FlowId>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl ComposeWindow {
    pub fn new(spec: Option<RequestSpec>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = spec.as_ref().map_or_else(ComposeDraft::default, ComposeDraft::of);
        let method = cx.new(|cx| InputState::new(window, cx).default_value(draft.method.clone()));
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://api.example.com/path?query")
                .default_value(draft.url.clone())
        });
        let (headers, body, subscriptions) = Self::parts(&draft, window, cx);
        ComposeWindow {
            edit_replay: spec.is_some(),
            method,
            url,
            headers,
            body,
            binary: draft.body_base64.then(|| draft.body.clone()),
            apply_rules: true,
            error: None,
            sending: false,
            sent: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    fn parts(
        draft: &ComposeDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<KvEditor>, Entity<BodyEditor>, Vec<Subscription>) {
        let rows = draft.headers.clone();
        let headers = cx.new(|cx| {
            KvEditor::new(
                "compose-headers",
                rows.clone(),
                rows,
                KvOptions {
                    fold: true,
                    ..Default::default()
                },
                cx,
            )
            .title("Headers")
            .name_width(150.)
            .on_menu(|req, window, cx| menus::removed_row(&req.row).show(req.position, window, cx))
        });
        let language = language_for(draft.content_type());
        let text = if draft.body_base64 {
            String::new()
        } else {
            draft.body.clone()
        };
        let init = BodyInit {
            text,
            original: None,
            language,
            read_only: false,
        };
        let body = cx.new(|cx| BodyEditor::new("compose-body", "Body", init, window, cx));
        let subscriptions = vec![cx.subscribe_in(&headers, window, |this, _, _: &KvChanged, window, cx| {
            let language = language_for(this.draft(cx).content_type());
            this.body.update(cx, |b, cx| b.set_language(language, window, cx));
            cx.notify();
        })];
        (headers, body, subscriptions)
    }

    /// A new draft in the open window (Compose again, or another Edit &
    /// Replay).
    pub fn start(&mut self, spec: Option<RequestSpec>, window: &mut Window, cx: &mut Context<Self>) {
        window.set_window_title(if spec.is_some() { "Edit & Replay" } else { "Compose" });
        *self = ComposeWindow::new(spec, window, cx);
        cx.notify();
    }

    pub fn register(window: AnyWindowHandle, view: &Entity<ComposeWindow>, cx: &mut App) {
        cx.set_global(Open {
            window,
            view: view.downgrade(),
        });
    }

    pub fn global(cx: &App) -> Option<(AnyWindowHandle, Entity<ComposeWindow>)> {
        let open = cx.try_global::<Open>()?;
        Some((open.window, open.view.upgrade()?))
    }

    pub fn is_edit_replay(&self) -> bool {
        self.edit_replay
    }

    pub fn headers(&self) -> &Entity<KvEditor> {
        &self.headers
    }

    pub fn body(&self) -> &Entity<BodyEditor> {
        &self.body
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn is_binary(&self) -> bool {
        self.binary.is_some()
    }

    pub fn apply_rules(&self) -> bool {
        self.apply_rules
    }

    /// What the form holds.
    pub fn draft(&self, cx: &App) -> ComposeDraft {
        ComposeDraft {
            method: self.method.read(cx).value().to_string(),
            url: self.url.read(cx).value().to_string(),
            headers: self.headers.read(cx).rows().to_vec(),
            body: match &self.binary {
                Some(b64) => b64.clone(),
                None => self.body.read(cx).text().to_owned(),
            },
            body_base64: self.binary.is_some(),
        }
    }

    /// Fills the form (the dev remote): any of method, URL, headers, body.
    pub fn fill(&mut self, spec: &RequestSpec, window: &mut Window, cx: &mut Context<Self>) {
        let draft = ComposeDraft::of(spec);
        self.method
            .update(cx, |s, cx| s.set_value(draft.method.clone(), window, cx));
        self.url.update(cx, |s, cx| s.set_value(draft.url.clone(), window, cx));
        let original = self.headers.read(cx).original().to_vec();
        self.headers
            .update(cx, |h, cx| h.set_rows(draft.headers.clone(), original, cx));
        self.binary = draft.body_base64.then(|| draft.body.clone());
        if !draft.body_base64 {
            let language = language_for(draft.content_type());
            self.body.update(cx, |b, cx| {
                b.set_language(language, window, cx);
                b.set_text(draft.body.clone(), window, cx);
            });
        }
        cx.notify();
    }

    pub fn set_method(&mut self, method: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.method
            .update(cx, |s, cx| s.set_value(method.to_owned(), window, cx));
        cx.notify();
    }

    pub fn set_content_type(&mut self, value: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let mut draft = self.draft(cx);
        draft.set_content_type(value);
        let original = self.headers.read(cx).original().to_vec();
        self.headers
            .update(cx, |h, cx| h.set_rows(draft.headers.clone(), original, cx));
        let language = language_for(draft.content_type());
        self.body.update(cx, |b, cx| b.set_language(language, window, cx));
        cx.notify();
    }

    pub fn set_apply_rules(&mut self, on: bool, cx: &mut Context<Self>) {
        self.apply_rules = on;
        cx.notify();
    }

    /// "Replace with text": the binary body goes, an empty text body comes.
    pub fn replace_binary(&mut self, cx: &mut Context<Self>) {
        self.binary = None;
        cx.notify();
    }

    /// Send (⌘↩): records the request as a new live flow and selects it;
    /// the window closes. A bad method, URL or header stays here as an error.
    pub fn send(&mut self, cx: &mut Context<Self>) {
        if self.sending {
            return;
        }
        self.error = None;
        self.sending = true;
        let spec = self.draft(cx).spec();
        let this = cx.weak_entity();
        mutations::send_request(
            spec,
            self.apply_rules,
            move |answer, cx| {
                let _ = this.update(cx, |w, cx| {
                    w.sending = false;
                    match &answer {
                        Ok(id) => w.sent = Some(*id),
                        Err(err) => w.error = Some(err.clone()),
                    }
                    cx.notify();
                });
                if let Ok(id) = answer {
                    crate::features::replay::show_live_flow(id, cx);
                    cx.defer(close);
                }
            },
            cx,
        );
        cx.notify();
    }

    /// The content-type picker's menu.
    pub fn content_type_menu(&self, cx: &App) -> Menu {
        let choice = self.draft(cx).content_type_choice();
        let mut menu = CONTENT_TYPES.iter().fold(Menu::new(), |menu, (label, value)| {
            menu.check(
                *label,
                choice == ContentTypeChoice::Listed(label),
                SetComposeContentType {
                    value: value.map(SharedString::from),
                },
            )
        });
        if let ContentTypeChoice::Custom(custom) = choice {
            menu = menu.separator().check(
                custom.clone(),
                true,
                SetComposeContentType {
                    value: Some(custom.into()),
                },
            );
        }
        menu
    }

    /// The method picker's menu.
    pub fn method_menu(&self, cx: &App) -> Menu {
        let current = self.method.read(cx).value().trim().to_uppercase();
        METHODS.iter().fold(Menu::new(), |menu, m| {
            menu.check(*m, current == *m, SetComposeMethod { method: (*m).into() })
        })
    }
}

/// Closes Compose, if it is open.
pub fn close(cx: &mut App) {
    if let Some((window, _)) = ComposeWindow::global(cx) {
        let _ = window.update(cx, |_, window, _| window.remove_window());
        cx.remove_global::<Open>();
    }
}

impl Focusable for ComposeWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ComposeWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let draft = self.draft(cx);
        let type_label: SharedString = match draft.content_type_choice() {
            ContentTypeChoice::Listed(label) => label.into(),
            ContentTypeChoice::Custom(c) => c.into(),
        };
        let title = if self.edit_replay { "Edit & Replay" } else { "Compose" };
        let body = match &self.binary {
            Some(_) => div()
                .id("compose-binary")
                .role(Role::Group)
                .aria_label("Binary body")
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(t.muted_foreground.hsla())
                .child(pm_widgets::icon(
                    gpui_kit::assets::IconName::Binary,
                    t.muted_foreground.hsla(),
                    20.,
                ))
                .child(format!(
                    "Binary body · {} · sent as recorded",
                    pm_view::flow::format_size(draft.binary_size() as u64)
                ))
                .child(
                    Button::new("compose-replace-binary")
                        .outline()
                        .xsmall()
                        .label("Replace with text")
                        .on_click(cx.listener(|this, _, _, cx| this.replace_binary(cx))),
                )
                .into_any_element(),
            None => div().size_full().p(px(6.)).child(self.body.clone()).into_any_element(),
        };
        let this = cx.weak_entity();
        div()
            .id("compose")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background.hsla())
            .text_color(t.foreground.hsla())
            .on_action(cx.listener(|this, _: &SendCompose, _, cx| this.send(cx)))
            .on_action(cx.listener(|this, a: &SetComposeContentType, window, cx| {
                this.set_content_type(a.value.as_deref(), window, cx)
            }))
            .on_action(cx.listener(|this, a: &SetComposeMethod, window, cx| this.set_method(&a.method, window, cx)))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px(px(16.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(t.border.hsla())
                    .child(
                        div()
                            .id("compose-title")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .role(Role::Heading)
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(pm_widgets::icon(
                                gpui_kit::assets::IconName::Send,
                                t.foreground.hsla(),
                                16.,
                            ))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.muted_foreground.hsla())
                            .child(logic::DESCRIPTION),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(12.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(t.border.hsla())
                    .child(div().w(px(96.)).flex_none().flex().child(line_input(
                        &self.method,
                        "compose-method",
                        "Method",
                        cx,
                    )))
                    .child(
                        Button::new("compose-methods")
                            .ghost()
                            .xsmall()
                            .icon(gpui_kit::assets::IconName::ChevronDown)
                            .tooltip("Methods")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.method_menu(cx).show(window.mouse_position(), window, cx)
                            })),
                    )
                    .child(line_input(&self.url, "compose-url", "URL", cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .id("compose-headers-pane")
                            .w(gpui_kit::relative(0.46))
                            .flex_none()
                            .overflow_y_scroll()
                            .p(px(12.))
                            .border_r_1()
                            .border_color(t.border.hsla())
                            .child(self.headers.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex_none()
                                    .h(px(32.))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .px(px(8.))
                                    .border_b_1()
                                    .border_color(t.border.hsla())
                                    .text_size(px(12.))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(t.muted_foreground.hsla())
                                            .child("Body"),
                                    )
                                    .child(
                                        Button::new("compose-type")
                                            .outline()
                                            .xsmall()
                                            .accessibility_label(SharedString::from(format!(
                                                "Content type: {type_label}"
                                            )))
                                            .child(
                                                div()
                                                    .min_w(px(96.))
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .gap(px(4.))
                                                    .child(type_label.clone())
                                                    .child(pm_widgets::icon(
                                                        gpui_kit::assets::IconName::ChevronDown,
                                                        t.muted_foreground.hsla(),
                                                        12.,
                                                    )),
                                            )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.content_type_menu(cx).show(window.mouse_position(), window, cx)
                                            })),
                                    ),
                            )
                            .child(div().flex_1().min_h_0().child(body)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(12.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(t.border.hsla())
                    .child(
                        Checkbox::new("compose-rules")
                            .checked(self.apply_rules)
                            .label("Apply rules")
                            .on_click(move |on, _, cx| {
                                let on = *on;
                                let _ = this.update(cx, |w, cx| w.set_apply_rules(on, cx));
                            }),
                    )
                    .child(error_line("compose-error", self.error.as_deref(), &t))
                    .child(div().text_size(px(11.)).text_color(t.muted_foreground.hsla()).child(
                        if cfg!(target_os = "macos") {
                            "⌘↩"
                        } else {
                            "Ctrl+↩"
                        },
                    ))
                    .child(
                        Button::new("compose-send")
                            .primary()
                            .small()
                            .icon(gpui_kit::assets::IconName::Send)
                            .label("Send")
                            .disabled(self.sending)
                            .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                    ),
            )
            .test_support()
    }
}
