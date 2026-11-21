//! The rule form (`RuleForm`, `MatcherBuilder.tsx`, `ActionListEditor.tsx`,
//! `RuleTestPanel.tsx`): name, origin and group, what the rule matches,
//! its actions, and the dry run against the selected flow.
//!
//! The form edits a copy of the rule. Every input and picker is bound to a
//! key of `pm_view::rules::edit` (`a.0.pointer`, `m.host.kind`, …): typing
//! writes through [`RuleForm::set_text`], pickers through
//! [`RuleForm::choose`], and anything that can move fields around (a
//! picker, an action added, moved or removed) re-reads every input from
//! the rule. The engine compiles the rule 200 ms after each change (live
//! validation), as it would on save.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, FontWeight, MouseButton, MouseDownEvent, PathPromptOptions, Render,
    Role, ScrollHandle, SharedString, Subscription, Task, Window, div, px,
};
use pm_core::Engine;
use pm_model::{
    Action, BlockMode, MapLocalSource, Rule, RuleGroup, RuleOrigin, RuleTestResult, ScriptKind, StoreId, WsOp,
};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::rules::{self, edit};

use super::menus;
use super::window::origin_badge;
use crate::bridge::keys::{self, RulesCaches};
use crate::bridge::mutations;
use crate::features::context_menu::Menu;

/// How long after a change the engine compiles the rule.
pub const VALIDATE_AFTER: Duration = Duration::from_millis(200);
/// The dry run's bodies are shown up to this many characters (the copies
/// take all of them).
const SHOWN_BODY: usize = 4000;

pub enum FormEvent {
    /// Saved (created or replaced): the window selects it.
    Saved(Rule),
}

enum Field {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

impl Field {
    fn value(&self, cx: &App) -> String {
        match self {
            Field::Line(s) => s.read(cx).value().to_string(),
            Field::Area(s) => s.read(cx).value().to_string(),
        }
    }

    fn set(&self, value: String, window: &mut Window, cx: &mut App) {
        match self {
            Field::Line(s) => s.update(cx, |s, cx| s.set_value(value, window, cx)),
            Field::Area(s) => s.update(cx, |s, cx| s.set_value(value, window, cx)),
        }
    }
}

/// The dry run's state.
#[derive(Default)]
pub struct TestState {
    pub result: Option<RuleTestResult>,
    pub error: Option<String>,
    pub busy: bool,
}

pub struct RuleForm {
    /// The rule as opened (or as last saved / pushed).
    initial: Rule,
    rule: Rule,
    /// The group picker was used: the form claims the group from then on.
    group_picked: bool,
    groups: Vec<RuleGroup>,
    /// Inputs whose text doesn't parse (JSON, a number, status ranges).
    field_errors: BTreeMap<String, String>,
    invalid: Option<String>,
    save_error: Option<String>,
    saving: bool,
    pub test: TestState,
    fields: HashMap<String, (Field, Subscription)>,
    validate: Option<Task<()>>,
    validations: u64,
    engine: Option<Arc<Engine>>,
    caches: RulesCaches,
    /// The pickers' lists as last read (render keeps them fresh).
    scripts: Vec<keys::ScriptChoice>,
    mocks: Vec<pm_model::MockSource>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FormEvent> for RuleForm {}

impl RuleForm {
    pub fn new(initial: Rule, groups: Vec<RuleGroup>, engine: Option<Arc<Engine>>, cx: &mut Context<Self>) -> Self {
        let caches = keys::rules_caches(cx);
        let subscriptions = vec![
            cx.observe(&caches.scripts, |_, _, cx| cx.notify()),
            cx.observe(&caches.mock_sources, |_, _, cx| cx.notify()),
        ];
        let mut form = RuleForm {
            rule: initial.clone(),
            initial,
            group_picked: false,
            groups,
            field_errors: BTreeMap::new(),
            invalid: None,
            save_error: None,
            saving: false,
            test: TestState::default(),
            fields: HashMap::new(),
            validate: None,
            validations: 0,
            engine,
            caches,
            scripts: Vec::new(),
            mocks: Vec::new(),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        form.schedule_validation(cx);
        form
    }

    pub fn rule(&self) -> &Rule {
        &self.rule
    }

    pub fn initial(&self) -> &Rule {
        &self.initial
    }

    /// The saved rule changed (the book was pushed): on/off, order and the
    /// group belong to the list, so they follow it.
    pub fn set_initial(&mut self, rule: Rule, groups: Vec<RuleGroup>, cx: &mut Context<Self>) {
        if self.initial != rule || self.groups != groups {
            self.initial = rule;
            self.groups = groups;
            cx.notify();
        }
    }

    /// The rule as it would be saved: the edits, with on/off and order as
    /// listed, and the group as listed unless the picker changed it.
    pub fn live(&self) -> Rule {
        Rule {
            enabled: self.initial.enabled,
            order: self.initial.order,
            group: if self.group_picked {
                self.rule.group.clone()
            } else {
                self.initial.group.clone()
            },
            ..self.rule.clone()
        }
    }

    pub fn is_unsaved(&self) -> bool {
        self.rule.id.is_empty() || self.live() != self.initial
    }

    /// The engine's error for the rule, if it doesn't compile.
    pub fn invalid(&self) -> Option<&str> {
        self.invalid.as_deref()
    }

    /// What the footer shows: a failed save, else why the rule is invalid.
    pub fn error(&self) -> Option<&str> {
        self.save_error.as_deref().or(self.invalid.as_deref())
    }

    pub fn field_error(&self, key: &str) -> Option<&str> {
        self.field_errors.get(key).map(String::as_str)
    }

    /// Validations finished so far (tests wait on it).
    pub fn validations(&self) -> u64 {
        self.validations
    }

    pub fn can_save(&self) -> bool {
        !self.saving && self.invalid.is_none() && self.field_errors.is_empty() && self.is_unsaved()
    }

    // ---- Edits ----

    /// A field's text changed (typed, or the dev remote).
    pub fn set_text(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        match edit::set_text(&mut self.rule, key, value) {
            Ok(()) => {
                self.field_errors.remove(key);
            }
            Err(err) => {
                self.field_errors.insert(key.to_owned(), err);
            }
        }
        self.changed(cx);
    }

    /// Writes `value` into the field `key` as if typed: the input too, if
    /// it is shown.
    pub fn type_text(&mut self, key: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((field, _)) = self.fields.get(key) {
            field.set(value.to_owned(), window, cx);
        }
        self.set_text(key, value, cx);
    }

    /// A picker or toggle (`edit::choose`).
    pub fn choose(&mut self, key: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        if key == "group" {
            self.group_picked = true;
            self.rule.group = (value != NO_GROUP).then(|| value.to_owned());
            self.changed(cx);
            return;
        }
        if let Err(err) = edit::choose(&mut self.rule, key, value) {
            eprintln!("rule form: {err}");
            return;
        }
        self.sync_fields(window, cx);
        self.changed(cx);
    }

    pub fn add_action(&mut self, type_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if edit::add_action(&mut self.rule, type_id) {
            self.sync_fields(window, cx);
            self.changed(cx);
        }
    }

    /// A structural edit (`edit::add_row`, `move_action`, …): the inputs
    /// re-read their fields after it.
    pub fn edit_structure(&mut self, window: &mut Window, cx: &mut Context<Self>, f: impl FnOnce(&mut Rule) -> bool) {
        self.structural(f, window, cx);
    }

    fn structural(&mut self, f: impl FnOnce(&mut Rule) -> bool, window: &mut Window, cx: &mut Context<Self>) {
        if f(&mut self.rule) {
            self.sync_fields(window, cx);
            self.changed(cx);
        }
    }

    /// Every input re-reads its field (fields may have moved), and inputs
    /// whose field is gone are dropped. Unparsed text is reset with them.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rule = &self.rule;
        let gone: Vec<String> = self
            .fields
            .keys()
            .filter(|k| edit::text(rule, k).is_none())
            .cloned()
            .collect();
        for k in gone {
            self.fields.remove(&k);
        }
        for (key, (field, _)) in &self.fields {
            let Some(text) = edit::text(&self.rule, key) else {
                continue;
            };
            if field.value(cx) != text {
                field.set(text, window, cx);
            }
        }
        self.field_errors.clear();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.save_error = None;
        self.schedule_validation(cx);
        cx.notify();
    }

    fn schedule_validation(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            self.validations += 1;
            return;
        };
        let rule = self.rule.clone();
        self.validate = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(VALIDATE_AFTER).await;
            let answer = cx
                .background_spawn(async move { engine.validate_rule(&rule).map_err(|e| e.to_string()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.invalid = answer.err();
                this.validations += 1;
                cx.notify();
            });
        }));
    }

    // ---- Save, delete, test ----

    pub fn save(&mut self, cx: &mut Context<Self>) {
        if !self.can_save() {
            return;
        }
        self.saving = true;
        self.save_error = None;
        let rule = self.live();
        let this = cx.weak_entity();
        let failed = this.clone();
        mutations::save_rule(
            rule,
            move |saved, cx| {
                let _ = this.update(cx, |form, cx| {
                    form.saving = false;
                    form.rule = Rule {
                        id: saved.id.clone(),
                        ..form.rule.clone()
                    };
                    form.initial = saved.clone();
                    form.group_picked = false;
                    cx.emit(FormEvent::Saved(saved));
                    cx.notify();
                });
            },
            move |err, cx| {
                let _ = failed.update(cx, |form, cx| {
                    form.saving = false;
                    form.save_error = Some(err);
                    cx.notify();
                });
            },
            cx,
        );
        cx.notify();
    }

    /// "Test against selected flow": the engine's dry run (scripts make it
    /// async, so it runs on the bridge's runtime).
    pub fn run_test(&mut self, cx: &mut Context<Self>) {
        let Some((store, flow)) = selected_flow(cx) else { return };
        let Some(engine) = self.engine.clone() else { return };
        let Some(runtime) = cx
            .try_global::<crate::bridge::Bridge>()
            .map(|_| crate::bridge::Bridge::runtime(cx))
        else {
            return;
        };
        self.test.busy = true;
        let rule = self.rule.clone();
        let task = runtime.spawn(async move { engine.test_rule_in(store, &rule, flow, None).await });
        cx.spawn(async move |this, cx| {
            let answer = match task.await {
                Ok(answer) => answer.map_err(|e| e.to_string()),
                Err(err) => Err(err.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.test = match answer {
                    Ok(result) => TestState {
                        result: Some(result),
                        error: None,
                        busy: false,
                    },
                    Err(error) => TestState {
                        result: None,
                        error: Some(error),
                        busy: false,
                    },
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Shows a dry run's answer (tests and offscreen renders).
    pub fn set_test(&mut self, test: TestState, cx: &mut Context<Self>) {
        self.test = test;
        cx.notify();
    }

    /// A picker's menu, as the form would show it (the dev remote, tests).
    pub fn picker_menu(&self, key: &str, cx: &App) -> Option<Menu> {
        if key == "add-action" {
            return Some(menus::add_action_menu());
        }
        let options = self.options(key, cx)?;
        let current = self.current(key)?;
        Some(menus::choice_menu(key, &options, &current))
    }

    fn current(&self, key: &str) -> Option<String> {
        if key == "group" {
            let group = self.live().group;
            return Some(
                group
                    .filter(|g| self.groups.iter().any(|x| &x.id == g))
                    .unwrap_or_else(|| NO_GROUP.into()),
            );
        }
        let value = edit::chosen(&self.rule, key)?;
        // A picker over a list (scripts, recordings) shows "choose" for an
        // id that isn't in it.
        if (key.ends_with(".script") || key.ends_with(".serve"))
            && let Some(options) = self.options_cached(key)
            && !options.iter().any(|(v, _)| *v == value)
        {
            return Some(NONE.into());
        }
        Some(value)
    }

    /// The options of a dropdown picker.
    fn options(&self, key: &str, cx: &App) -> Option<Vec<(String, String)>> {
        let pairs = |list: &[(&str, &str)]| list.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect();
        if key == "group" {
            let mut out = vec![(NO_GROUP.to_owned(), "No group".to_owned())];
            out.extend(self.groups.iter().map(|g| (g.id.clone(), g.name.clone())));
            return Some(out);
        }
        let field = key.rsplit('.').next()?;
        Some(match field {
            "op" => pairs(&rules::WS_OPS),
            "mode" => pairs(&[("status", "A status"), ("reset", "A connection reset")]),
            "source" => pairs(&[
                ("inline", "Inline body"),
                ("file", "A file"),
                ("directory", "A directory"),
            ]),
            "direction" => pairs(&[
                ("both", "Both ways"),
                ("sent", "Sent by the client"),
                ("received", "From the server"),
            ]),
            "pick" => rules::SERVE_PICK.iter().map(|p| (p.1.into(), p.2.into())).collect(),
            "miss" => rules::SERVE_MISS.iter().map(|p| (p.1.into(), p.2.into())).collect(),
            "script" | "serve" => {
                let _ = cx;
                self.options_cached(key)?
            }
            _ => return None,
        })
    }

    /// The script and recording pickers' options, as cached.
    fn options_cached(&self, key: &str) -> Option<Vec<(String, String)>> {
        let chosen = edit::chosen(&self.rule, key).unwrap_or_default();
        if key.ends_with(".script") {
            let scripts = self.scripts_cached();
            let mut out = Vec::new();
            if !scripts.iter().any(|s| s.id == chosen) {
                out.push((
                    NONE.to_owned(),
                    if chosen.is_empty() {
                        "Choose a script…".to_owned()
                    } else {
                        "(missing script)".to_owned()
                    },
                ));
            }
            out.extend(scripts.iter().map(|s| {
                (
                    s.id.clone(),
                    format!(
                        "{} · {}{}",
                        s.name,
                        script_kind(s.kind),
                        if s.enabled { "" } else { " (off)" }
                    ),
                )
            }));
            return Some(out);
        }
        if key.ends_with(".serve") {
            let sources = self.mocks_cached();
            let mut out = Vec::new();
            if !sources.iter().any(|s| s.id == chosen) {
                out.push((
                    NONE.to_owned(),
                    if chosen.is_empty() {
                        "Choose a recording…".to_owned()
                    } else {
                        "(missing recording)".to_owned()
                    },
                ));
            }
            out.extend(sources.iter().map(|s| {
                (
                    s.id.clone(),
                    format!("{} · {} flow{}", s.name, s.flows, if s.flows == 1 { "" } else { "s" }),
                )
            }));
            return Some(out);
        }
        None
    }

    /// Rewrite and mock scripts: view scripts run on their own.
    fn scripts_cached(&self) -> Vec<keys::ScriptChoice> {
        self.scripts
            .iter()
            .filter(|s| s.kind != ScriptKind::View)
            .cloned()
            .collect()
    }

    fn mocks_cached(&self) -> Vec<pm_model::MockSource> {
        self.mocks.clone()
    }

    /// Sets the pickers' lists (tests and offscreen renders).
    pub fn set_lists(&mut self, scripts: Vec<keys::ScriptChoice>, mocks: Vec<pm_model::MockSource>) {
        self.scripts = scripts;
        self.mocks = mocks;
    }

    // ---- Inputs ----

    fn input(
        &mut self,
        key: &str,
        area: Option<usize>,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> &Field {
        if !self.fields.contains_key(key) {
            let text = edit::text(&self.rule, key).unwrap_or_default();
            let placeholder = placeholder.to_owned();
            let owned = key.to_owned();
            let entry = match area {
                Some(rows) => {
                    let state = cx.new(|cx| {
                        TextareaState::new(window, cx)
                            .rows(rows)
                            .placeholder(placeholder)
                            .default_value(text)
                    });
                    let sub = cx.subscribe(&state, move |this, state, e: &InputEvent, cx| {
                        if matches!(e, InputEvent::Change) {
                            let value = state.read(cx).value().to_string();
                            this.set_text(&owned, &value, cx);
                        }
                    });
                    (Field::Area(state), sub)
                }
                None => {
                    let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(text));
                    let sub = cx.subscribe(&state, move |this, state, e: &InputEvent, cx| {
                        if matches!(e, InputEvent::Change) {
                            let value = state.read(cx).value().to_string();
                            this.set_text(&owned, &value, cx);
                        }
                    });
                    (Field::Line(state), sub)
                }
            };
            self.fields.insert(key.to_owned(), entry);
        }
        &self.fields[key].0
    }

    /// A single-line input bound to `key`.
    fn line(
        &mut self,
        key: &str,
        placeholder: &str,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let font = if key == "name" {
            cx.theme().font_family.clone()
        } else {
            cx.theme().mono_font_family.clone()
        };
        let error = self.field_errors.contains_key(key);
        let Field::Line(state) = self.input(key, None, placeholder, window, cx) else {
            return div().into_any_element();
        };
        let state = state.clone();
        let t = *cx.tokens();
        div()
            .flex_1()
            .min_w(px(60.))
            .font_family(font)
            .text_size(px(12.))
            .when(error, |d| d.rounded(px(6.)).border_1().border_color(t.s_failed.hsla()))
            .child(
                Input::new(&state)
                    .id(SharedString::from(format!("field-{key}")))
                    .small()
                    .aria_label(SharedString::from(label.to_owned())),
            )
            .into_any_element()
    }

    /// A fixed-width single-line input (numbers).
    fn narrow(
        &mut self,
        key: &str,
        placeholder: &str,
        label: &str,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let input = self.line(key, placeholder, label, window, cx);
        div().flex_none().w(px(width)).flex().child(input).into_any_element()
    }

    fn area(&mut self, key: &str, rows: usize, label: &str, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mono = cx.theme().mono_font_family.clone();
        let Field::Area(state) = self.input(key, Some(rows), "", window, cx) else {
            return div().into_any_element();
        };
        let state = state.clone();
        div()
            .w_full()
            .font_family(mono)
            .text_size(px(12.))
            .child(Textarea::new(&state).aria_label(SharedString::from(label.to_owned())))
            .into_any_element()
    }

    /// Buttons for a small choice; the chosen one is selected.
    fn segmented(&self, key: &str, options: &[(&str, &str)], label: &str, cx: &mut Context<Self>) -> AnyElement {
        let current = edit::chosen(&self.rule, key).unwrap_or_default();
        div()
            .id(SharedString::from(format!("choice-{key}")))
            .role(Role::Group)
            .aria_label(SharedString::from(label.to_owned()))
            .flex()
            .flex_none()
            .gap(px(2.))
            .children(options.iter().map(|(value, text)| {
                let (k, v) = (key.to_owned(), (*value).to_owned());
                let on = current == *value;
                toggled(Button::new(SharedString::from(format!("choice-{key}-{value}"))), on)
                    .xsmall()
                    .selected(on)
                    .label(*text)
                    .accessibility_label(SharedString::from(format!("{label}: {text}")))
                    .on_click(cx.listener(move |this, _, window, cx| this.choose(&k, &v, window, cx)))
            }))
            .into_any_element()
    }

    /// A dropdown picker: its current label; a click shows the options as a
    /// native menu.
    fn picker(&self, key: &str, label: &str, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let options = self.options(key, cx).unwrap_or_default();
        let current = self.current(key).unwrap_or_default();
        let shown = options
            .iter()
            .find(|(v, _)| *v == current)
            .map_or_else(|| current.clone(), |(_, l)| l.clone());
        let k = key.to_owned();
        let t = *cx.tokens();
        Button::new(SharedString::from(format!("picker-{key}")))
            .outline()
            .xsmall()
            .accessibility_label(SharedString::from(format!("{label}: {shown}")))
            .child(
                div()
                    .w(px(width))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(4.))
                    .child(div().truncate().child(shown))
                    .child(pm_widgets::icon(IconName::ChevronDown, t.muted_foreground.hsla(), 12.)),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(menu) = this.picker_menu(&k, cx) {
                    menu.show(window.mouse_position(), window, cx);
                }
            }))
            .into_any_element()
    }

    fn checkbox(&self, key: &str, label: &str, cx: &mut Context<Self>) -> AnyElement {
        let on = edit::chosen(&self.rule, key).as_deref() == Some("true");
        let this = cx.weak_entity();
        let k = key.to_owned();
        Checkbox::new(SharedString::from(format!("check-{key}")))
            .checked(on)
            .label(label.to_owned())
            .small()
            .on_click(move |v, window, cx| {
                let value = if *v { "true" } else { "false" };
                let _ = this.update(cx, |f, cx| f.choose(&k, value, window, cx));
            })
            .into_any_element()
    }

    /// A pattern: kind, value and case-insensitivity.
    fn pattern(
        &mut self,
        key: &str,
        placeholder: &str,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kinds: Vec<(&str, &str)> = rules::PATTERN_KINDS.iter().map(|k| (k.1, k.2)).collect();
        let kind = self.segmented(&format!("{key}.kind"), &kinds, &format!("{label} match"), cx);
        let value = self.line(key, placeholder, label, window, cx);
        let ci_key = format!("{key}.ci");
        let ci = edit::chosen(&self.rule, &ci_key).as_deref() == Some("true");
        let t = *cx.tokens();
        let toggle = toggled(Button::new(SharedString::from(format!("toggle-{ci_key}"))), ci)
            .xsmall()
            .selected(ci)
            .tooltip("Case-insensitive")
            .accessibility_label(SharedString::from(format!(
                "{label}: case-insensitive {}",
                if ci { "on" } else { "off" }
            )))
            .child(pm_widgets::icon(
                IconName::CaseSensitive,
                if ci { t.primary_foreground } else { t.foreground }.hsla(),
                14.,
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.choose(&ci_key, if ci { "false" } else { "true" }, window, cx);
            }));
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(kind)
            .child(value)
            .child(toggle)
            .into_any_element()
    }

    fn icon_button(
        &self,
        id: String,
        icon: IconName,
        label: &str,
        disabled: bool,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.tokens();
        Button::new(SharedString::from(id))
            .ghost()
            .xsmall()
            .disabled(disabled)
            .tooltip(label.to_owned())
            .accessibility_label(label.to_owned())
            .child(pm_widgets::icon(icon, t.muted_foreground.hsla(), 13.))
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .into_any_element()
    }

    fn add_button(
        &self,
        id: &str,
        label: &str,
        t: &Tokens,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(SharedString::from(id.to_owned()))
            .ghost()
            .xsmall()
            .accessibility_label(label.to_owned())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(t.muted_foreground.hsla())
                    .child(pm_widgets::icon(IconName::Plus, t.muted_foreground.hsla(), 12.))
                    .child(label.to_owned()),
            )
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .into_any_element()
    }

    // ---- Sections ----

    fn matcher(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.tokens();
        let mut rows: Vec<AnyElement> = Vec::new();
        let methods = self.rule.matcher.methods.clone();
        let method_buttons = div()
            .id("matcher-methods")
            .role(Role::Group)
            .aria_label("Methods")
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.))
            .children(rules::METHODS.iter().map(|m| {
                let on = methods.iter().any(|x| x == m);
                let method = (*m).to_owned();
                toggled(Button::new(SharedString::from(format!("method-{m}"))), on)
                    .xsmall()
                    .selected(on)
                    .label(*m)
                    .accessibility_label(SharedString::from(format!("{m} {}", if on { "on" } else { "off" })))
                    .on_click(cx.listener(move |this, _, window, cx| this.choose("m.method", &method, window, cx)))
            }))
            .when(methods.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.muted_foreground.hsla())
                        .child("any method"),
                )
            });
        rows.push(field_row("Methods", vec![method_buttons.into_any_element()], &t));
        let scheme = self.segmented(
            "m.scheme",
            &[("any", "Any"), ("http", "http"), ("https", "https")],
            "Scheme",
            cx,
        );
        let port = self.narrow("m.port", "any port", "Port", 96., window, cx);
        rows.push(field_row("Scheme · port", vec![scheme, port], &t));
        let host = self.pattern("m.host", "any host, e.g. *.example.com", "Host", window, cx);
        rows.push(field_row("Host", vec![host], &t));
        let path = self.pattern("m.path", "any path, e.g. /api/*", "Path", window, cx);
        rows.push(field_row("Path", vec![path], &t));
        for (which, label, add) in [("query", "Query", "Query parameter"), ("headers", "Headers", "Header")] {
            let n = if which == "query" {
                self.rule.matcher.query.len()
            } else {
                self.rule.matcher.headers.len()
            };
            for i in 0..n {
                let base = format!("m.{which}.{i}");
                let name = self.narrow(
                    &format!("{base}.name"),
                    "name",
                    &format!("{label} name"),
                    128.,
                    window,
                    cx,
                );
                let value = self.pattern(
                    &format!("{base}.value"),
                    "present",
                    &format!("{label} value"),
                    window,
                    cx,
                );
                let not = self.checkbox(&format!("{base}.negate"), "not", cx);
                let w = which.to_owned();
                let remove = self.icon_button(
                    format!("remove-{base}"),
                    IconName::X,
                    "Remove",
                    false,
                    move |this, window, cx| this.structural(|r| edit::remove_row(r, &w, i), window, cx),
                    cx,
                );
                rows.push(field_row(
                    if i == 0 { label } else { "" },
                    vec![name, value, not, remove],
                    &t,
                ));
            }
            let w = which.to_owned();
            let button = self.add_button(
                &format!("add-{which}"),
                add,
                &t,
                move |this, window, cx| this.structural(|r| edit::add_row(r, &w), window, cx),
                cx,
            );
            rows.push(field_row(if n == 0 { label } else { "" }, vec![button], &t));
        }
        let app = self.pattern("m.app", "any app, e.g. Safari", "Client app", window, cx);
        rows.push(field_row("Client app", vec![app], &t));
        if self.rule.matcher.response.is_some() {
            let status = self.line("m.status", "any, e.g. 2xx, 404, 500-599", "Response status", window, cx);
            let remove = self.icon_button(
                "remove-response".into(),
                IconName::X,
                "Remove response matcher",
                false,
                |this, window, cx| this.choose("m.response", "false", window, cx),
                cx,
            );
            rows.push(field_row("Response status", vec![status, remove], &t));
            if let Some(err) = self.field_errors.get("m.status").cloned() {
                rows.push(field_row("", vec![error_text(err, &t)], &t));
            }
            let ctype = self.pattern("m.ctype", "any, e.g. application/json*", "Content-Type", window, cx);
            rows.push(field_row("Content-Type", vec![ctype], &t));
        } else {
            let button = self.add_button(
                "add-response",
                "Only for some responses",
                &t,
                |this, window, cx| this.choose("m.response", "true", window, cx),
                cx,
            );
            rows.push(field_row("Response", vec![button], &t));
        }
        div()
            .id("matcher-builder")
            .flex()
            .flex_col()
            .gap(px(6.))
            .children(rows)
            .into_any_element()
    }

    fn actions(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.tokens();
        let n = self.rule.actions.len();
        let mut cards: Vec<AnyElement> = Vec::new();
        if n == 0 {
            cards.push(
                div()
                    .px(px(4.))
                    .text_size(px(12.))
                    .text_color(t.muted_foreground.hsla())
                    .child("No actions yet: the rule matches but does nothing.")
                    .into_any_element(),
            );
        }
        for i in 0..n {
            let type_id = rules::action_type(&self.rule.actions[i]);
            let label = rules::action_label(type_id).to_owned();
            let up = self.icon_button(
                format!("action-{i}-up"),
                IconName::ArrowUp,
                "Move up",
                i == 0,
                move |this, window, cx| this.structural(|r| edit::move_action(r, i, false), window, cx),
                cx,
            );
            let down = self.icon_button(
                format!("action-{i}-down"),
                IconName::ArrowDown,
                "Move down",
                i + 1 == n,
                move |this, window, cx| this.structural(|r| edit::move_action(r, i, true), window, cx),
                cx,
            );
            let remove = self.icon_button(
                format!("action-{i}-remove"),
                IconName::X,
                "Remove action",
                false,
                move |this, window, cx| this.structural(|r| edit::remove_action(r, i), window, cx),
                cx,
            );
            let body = self.action_form(i, window, cx);
            cards.push(
                div()
                    .id(SharedString::from(format!("action-{i}")))
                    .role(Role::Group)
                    .aria_label(SharedString::from(format!("Action {}: {label}", i + 1)))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(t.border.hsla())
                    .child(
                        div()
                            .h(px(26.))
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .pl(px(8.))
                            .pr(px(4.))
                            .bg(t.muted.hsla().opacity(0.4))
                            .border_b_1()
                            .border_color(t.border.hsla())
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(label),
                            )
                            .child(up)
                            .child(down)
                            .child(remove),
                    )
                    .child(div().flex().flex_col().gap(px(6.)).p(px(8.)).children(body))
                    .into_any_element(),
            );
        }
        let add = Button::new("add-action")
            .outline()
            .xsmall()
            .accessibility_label("Add action")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(pm_widgets::icon(IconName::Plus, t.foreground.hsla(), 12.))
                    .child("Add action"),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some(menu) = this.picker_menu("add-action", cx) {
                    menu.show(window.mouse_position(), window, cx);
                }
            }));
        div()
            .id("action-list")
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(cards)
            .child(div().flex().child(add))
            .into_any_element()
    }

    fn phase(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        self.segmented(
            &format!("a.{i}.phase"),
            &[("request", "Request"), ("response", "Response")],
            "Phase",
            cx,
        )
    }

    /// One action's fields.
    fn action_form(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = *cx.tokens();
        let k = |f: &str| format!("a.{i}.{f}");
        let action = self.rule.actions[i].clone();
        let mut rows = Vec::new();
        match &action {
            Action::SetHeader { .. } | Action::AddHeader { .. } | Action::FormSet { .. } => {
                let what = if matches!(action, Action::FormSet { .. }) {
                    "Field"
                } else {
                    "Header"
                };
                let phase = self.phase(i, cx);
                rows.push(field_row("Phase", vec![phase], &t));
                let name = self.line(&k("name"), "name", &format!("{what} name"), window, cx);
                let value = self.line(&k("value"), "value", &format!("{what} value"), window, cx);
                rows.push(field_row(what, vec![name, value], &t));
            }
            Action::RemoveHeader { .. } | Action::FormRemove { .. } => {
                let what = if matches!(action, Action::FormRemove { .. }) {
                    "Field"
                } else {
                    "Header"
                };
                let phase = self.phase(i, cx);
                rows.push(field_row("Phase", vec![phase], &t));
                let name = self.line(&k("name"), "name", &format!("{what} name"), window, cx);
                rows.push(field_row(what, vec![name], &t));
            }
            Action::ReplaceHeaderValue { .. } | Action::ReplaceBody { .. } => {
                let phase = self.phase(i, cx);
                rows.push(field_row("Phase", vec![phase], &t));
                if matches!(action, Action::ReplaceHeaderValue { .. }) {
                    let name = self.line(&k("name"), "name", "Header name", window, cx);
                    rows.push(field_row("Header", vec![name], &t));
                }
                let find = self.pattern(&k("pattern"), "", "Find", window, cx);
                rows.push(field_row("Find", vec![find], &t));
                let with = self.line(&k("replacement"), "$1 for a regex group", "Replace with", window, cx);
                rows.push(field_row("Replace with", vec![with], &t));
            }
            Action::SetQuery { .. } => {
                let name = self.line(&k("name"), "name", "Parameter name", window, cx);
                let value = self.line(&k("value"), "value", "Parameter value", window, cx);
                rows.push(field_row("Parameter", vec![name, value], &t));
            }
            Action::RemoveQuery { .. } => {
                let name = self.line(&k("name"), "name", "Parameter name", window, cx);
                rows.push(field_row("Parameter", vec![name], &t));
            }
            Action::SetBody { .. } => {
                let phase = self.phase(i, cx);
                let ctype = self.line(&k("ctype"), "Content-Type (unchanged)", "Content-Type", window, cx);
                rows.push(field_row("Phase", vec![phase, ctype], &t));
                rows.push(self.area(&k("body"), 5, "Body", window, cx));
            }
            Action::JsonSet { .. } | Action::JsonRemove { .. } => {
                let phase = self.phase(i, cx);
                rows.push(field_row("Phase", vec![phase], &t));
                let placeholder = if matches!(action, Action::JsonSet { .. }) {
                    "/data/items/0/name"
                } else {
                    "/data/secret"
                };
                let pointer = self.line(&k("pointer"), placeholder, "JSON pointer", window, cx);
                rows.push(field_row("JSON pointer", vec![pointer], &t));
                if matches!(action, Action::JsonSet { .. }) {
                    rows.extend(self.json_value(&k("raw"), window, cx));
                }
            }
            Action::SetStatus { .. } => {
                let status = self.narrow(&k("status"), "200", "Status", 96., window, cx);
                rows.push(field_row("Status", vec![status], &t));
            }
            Action::MapLocal { source, .. } => {
                let from = self.picker(&k("source"), "Answer from", 110., cx);
                let status = self.narrow(&k("status"), "200", "Status", 64., window, cx);
                let inline = matches!(source, MapLocalSource::Inline { .. });
                let ctype = self.line(
                    &k("ctype"),
                    if inline {
                        "Content-Type"
                    } else {
                        "Content-Type (from the extension)"
                    },
                    "Content-Type",
                    window,
                    cx,
                );
                rows.push(field_row("Answer from", vec![from, status, ctype], &t));
                match source {
                    MapLocalSource::Inline { .. } => rows.push(self.area(&k("inline"), 8, "Inline body", window, cx)),
                    MapLocalSource::File { .. } => {
                        let path = self.line(&k("file"), "/Users/you/mock.json", "Map Local file", window, cx);
                        let choose = self.choose_path(i, false, cx);
                        rows.push(field_row("File", vec![path, choose], &t));
                    }
                    MapLocalSource::Directory { .. } => {
                        let path = self.line(&k("root"), "/Users/you/site", "Map Local directory", window, cx);
                        let choose = self.choose_path(i, true, cx);
                        rows.push(field_row("Root", vec![path, choose], &t));
                        rows.push(note(
                            "The request path is looked up under the root, then with leading segments dropped; a path ending in / serves index.html.",
                            &t,
                        ));
                    }
                }
            }
            Action::MapRemote { .. } => {
                let scheme = self.segmented(
                    &k("scheme"),
                    &[("same", "Same"), ("http", "http"), ("https", "https")],
                    "Scheme",
                    cx,
                );
                rows.push(field_row("Scheme", vec![scheme], &t));
                let host = self.line(&k("host"), "same host", "Host", window, cx);
                let port = self.narrow(&k("port"), "default", "Port", 96., window, cx);
                rows.push(field_row("Host · port", vec![host, port], &t));
                let path = self.line(&k("path"), "same path", "Path", window, cx);
                rows.push(field_row("Path", vec![path], &t));
                let keep = self.checkbox(&k("preserve_host"), "Keep the original Host header", cx);
                rows.push(field_row("", vec![keep], &t));
            }
            Action::Breakpoint { .. } => {
                let req = self.checkbox(&k("request"), "Request", cx);
                let resp = self.checkbox(&k("response"), "Response", cx);
                rows.push(field_row("Pause at", vec![req, resp], &t));
            }
            Action::Delay { .. } => {
                let phase = self.phase(i, cx);
                let ms = self.narrow(&k("ms"), "0", "Delay in milliseconds", 96., window, cx);
                rows.push(field_row("Delay", vec![phase, ms, muted("ms", &t)], &t));
            }
            Action::Throttle { .. } => {
                let down = self.narrow(&k("down"), "0", "Download kbps", 96., window, cx);
                let up = self.narrow(&k("up"), "0", "Upload kbps", 96., window, cx);
                rows.push(field_row(
                    "Down · up",
                    vec![down, up, muted("kbps (0: unlimited)", &t)],
                    &t,
                ));
                let latency = self.narrow(&k("latency"), "0", "Latency in milliseconds", 96., window, cx);
                rows.push(field_row("Latency", vec![latency, muted("ms", &t)], &t));
            }
            Action::Block { mode } => {
                let picker = self.picker(&k("mode"), "Block with", 140., cx);
                let mut row = vec![picker];
                if matches!(mode, BlockMode::Status { .. }) {
                    row.push(self.narrow(&k("status"), "403", "Block status", 72., window, cx));
                }
                rows.push(field_row("Block with", row, &t));
            }
            Action::Script { .. } => {
                let picker = self.picker(&k("script"), "Script", 240., cx);
                rows.push(field_row("Script", vec![picker], &t));
            }
            Action::WsMessage { op, .. } => {
                let dir = self.picker(&k("direction"), "Messages", 140., cx);
                rows.push(field_row("Messages", vec![dir], &t));
                let filter = self.pattern(&k("filter"), "any message", "Containing", window, cx);
                rows.push(field_row("Containing", vec![filter], &t));
                let picker = self.picker(&k("op"), "Do", 110., cx);
                let mut row = vec![picker];
                match op {
                    WsOp::Delay { .. } => {
                        row.push(self.narrow(&k("ms"), "0", "Delay in milliseconds", 96., window, cx));
                        row.push(muted("ms; later messages wait behind it", &t));
                    }
                    WsOp::Drop => row.push(muted("recorded, never forwarded", &t)),
                    WsOp::Breakpoint => row.push(muted("later messages wait behind it", &t)),
                    _ => {}
                }
                rows.push(field_row("Do", row, &t));
                match op {
                    WsOp::Replace { .. } => {
                        let find = self.pattern(&k("pattern"), "", "Find", window, cx);
                        rows.push(field_row("Find", vec![find], &t));
                        let with = self.line(&k("replacement"), "$1 for a regex group", "Replace with", window, cx);
                        rows.push(field_row("Replace with", vec![with], &t));
                    }
                    WsOp::JsonSet { .. } | WsOp::JsonRemove { .. } => {
                        let pointer = self.line(&k("pointer"), "/data/price", "JSON pointer", window, cx);
                        rows.push(field_row("JSON pointer", vec![pointer], &t));
                        if matches!(op, WsOp::JsonSet { .. }) {
                            rows.extend(self.json_value(&k("raw"), window, cx));
                        }
                    }
                    _ => {}
                }
            }
            Action::ServeFrom { on_miss, source, .. } => {
                let picker = self.picker(&k("serve"), "Recording", 240., cx);
                let library = Button::new(SharedString::from(format!("library-{i}")))
                    .ghost()
                    .xsmall()
                    .label("Library…")
                    .tooltip("The mock library, in Settings")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::features::chrome::OpenSettings { page: "mocking".into() }),
                            cx,
                        );
                    })
                    .into_any_element();
                rows.push(field_row("Recording", vec![picker, library], &t));
                if let Some(s) = self.mocks_cached().into_iter().find(|s| &s.id == source)
                    && !s.hosts.is_empty()
                {
                    rows.push(field_row(
                        "",
                        vec![muted(&format!("Recorded from {}", s.hosts.join(", ")), &t)],
                        &t,
                    ));
                }
                let q = self.checkbox(&k("query"), "Query (any order)", cx);
                let b = self.checkbox(&k("body"), "Request body", cx);
                rows.push(field_row("Also compare", vec![q, b], &t));
                let pick = self.picker(&k("pick"), "Several match", 140., cx);
                rows.push(field_row("Several match", vec![pick], &t));
                let miss = self.picker(&k("miss"), "None match", 140., cx);
                let what = if *on_miss == pm_model::ServeMiss::PassThrough {
                    "the request goes to the server"
                } else {
                    "without contacting the server"
                };
                rows.push(field_row("None match", vec![miss, muted(what, &t)], &t));
            }
            Action::Comment { .. } => {
                let text = self.line(&k("text"), "Config request", "Comment", window, cx);
                rows.push(field_row("Comment", vec![text], &t));
                rows.push(field_row(
                    "",
                    vec![muted(
                        "Added to each flow this rule matches (once its response matches, if the rule has a response condition). Search comment: to find them.",
                        &t,
                    )],
                    &t,
                ));
            }
        }
        rows
    }

    /// Exact JSON text, applied once it parses.
    fn json_value(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = *cx.tokens();
        let input = self.line(key, "\"text\", 42, true, null, {\"a\": 1}", "Value (JSON)", window, cx);
        let mut rows = vec![field_row("Value (JSON)", vec![input], &t)];
        if let Some(err) = self.field_errors.get(key).cloned() {
            rows.push(field_row("", vec![error_text(err, &t)], &t));
        }
        rows
    }

    /// "Choose…" for a Map Local file or directory.
    fn choose_path(&self, i: usize, folder: bool, cx: &mut Context<Self>) -> AnyElement {
        Button::new(SharedString::from(format!("choose-path-{i}")))
            .outline()
            .xsmall()
            .label("Choose…")
            .on_click(cx.listener(move |_, _, window, cx| {
                let picked = cx.prompt_for_paths(PathPromptOptions {
                    files: !folder,
                    directories: folder,
                    multiple: false,
                    prompt: Some(
                        if folder {
                            "Map Local directory"
                        } else {
                            "Map Local file"
                        }
                        .into(),
                    ),
                });
                let handle = window.window_handle();
                cx.spawn(async move |this, cx| {
                    let Ok(Ok(Some(paths))) = picked.await else { return };
                    let Some(path) = paths.into_iter().next() else { return };
                    let key = format!("a.{i}.{}", if folder { "root" } else { "file" });
                    let path: PathBuf = path;
                    let text = path.to_string_lossy().into_owned();
                    let _ = handle.update(cx, |_, window, cx| {
                        let _ = this.update(cx, |f, cx| f.type_text(&key, &text, window, cx));
                    });
                })
                .detach();
            }))
            .into_any_element()
    }

    fn dry_run(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.tokens();
        let flow = selected_flow(cx);
        let busy = self.test.busy;
        let button = Button::new("test-rule")
            .outline()
            .xsmall()
            .disabled(flow.is_none() || busy || self.engine.is_none())
            .loading(busy)
            .accessibility_label("Test against selected flow")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(pm_widgets::icon(IconName::FlaskConical, t.foreground.hsla(), 12.))
                    .child("Test against selected flow"),
            )
            .on_click(cx.listener(|this, _, _, cx| this.run_test(cx)));
        let mut col = div().id("rule-test").flex().flex_col().gap(px(8.)).child(
            div().flex().items_center().gap(px(8.)).child(button).child(muted(
                &match flow {
                    None => "Select a flow in the table first".to_owned(),
                    Some((_, id)) => format!("Flow #{}", id + 1),
                },
                &t,
            )),
        );
        if let Some(err) = &self.test.error {
            col = col.child(error_text(err.clone(), &t));
        }
        if let Some(result) = &self.test.result {
            col = col.child(test_result(result, &t));
        }
        col.into_any_element()
    }
}

/// A toggle button: filled while on, outlined while off.
fn toggled(button: Button, on: bool) -> Button {
    if on { button.primary() } else { button.outline() }
}

/// The picker value for "no group".
pub const NO_GROUP: &str = "none";
/// The picker value shown for a script or recording that isn't chosen (or
/// is missing).
const NONE: &str = "__none__";

fn script_kind(kind: ScriptKind) -> &'static str {
    match kind {
        ScriptKind::Rewrite => "rewrite",
        ScriptKind::Mock => "mock",
        ScriptKind::View => "view",
    }
}

/// The flow the dry run tests: the table's focused flow, in the tab shown.
pub fn selected_flow(cx: &App) -> Option<(StoreId, pm_model::FlowId)> {
    let table = crate::shell::parts(cx)?.table.upgrade()?;
    let id = table.read(cx).grid().read(cx).selected().focused?;
    let store = crate::workspace::Workspace::global(cx).map_or(pm_model::LIVE_STORE, |w| w.read(cx).store);
    Some((store, id))
}

/// A labelled row: the label right-aligned in a 96 px column.
pub fn field_row(label: &str, children: Vec<AnyElement>, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .w(px(96.))
                .flex_none()
                .text_right()
                .text_size(px(12.))
                .text_color(t.muted_foreground.hsla())
                .child(label.to_owned()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .children(children),
        )
        .into_any_element()
}

fn muted(text: &str, t: &Tokens) -> AnyElement {
    div()
        .text_size(px(11.))
        .text_color(t.muted_foreground.hsla())
        .child(text.to_owned())
        .into_any_element()
}

fn note(text: &str, t: &Tokens) -> AnyElement {
    div()
        .pl(px(104.))
        .text_size(px(11.))
        .text_color(t.muted_foreground.hsla())
        .child(text.to_owned())
        .into_any_element()
}

fn error_text(text: String, t: &Tokens) -> AnyElement {
    div()
        .text_size(px(11.))
        .text_color(t.s_failed.hsla())
        .child(text)
        .into_any_element()
}

fn section(title: &str, body: AnyElement, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .id(SharedString::from(format!("section-{title}")))
                .role(Role::Heading)
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted_foreground.hsla())
                .child(title.to_uppercase()),
        )
        .child(body)
        .into_any_element()
}

/// A message of the dry run, as text: start line, headers, body.
pub fn message_text(start: &str, headers: &[pm_model::Header], body: Option<&str>) -> (String, String) {
    let rows = pm_view::http::header_rows(headers);
    let block = pm_view::copy_formats::headers_block(&rows);
    let full = match body {
        Some(b) if !b.is_empty() => format!("{start}\n{block}\n\n{b}"),
        _ => format!("{start}\n{block}"),
    };
    (full, block)
}

/// The dry run's menus: Copy request/response, Copy headers, Copy body;
/// Copy notes.
pub fn test_menu(result: &RuleTestResult, part: &str) -> Menu {
    match part {
        "notes" => Menu::new().copy("Copy notes", result.notes.join("\n"), "notes"),
        "request" => {
            let start = format!("{} {}", result.request.method, result.request.url);
            let (full, headers) = message_text(&start, &result.request.headers, result.request_body.as_deref());
            Menu::new()
                .copy("Copy request", full, "request")
                .copy("Copy headers", headers, "headers")
                .when(result.request_body.is_some(), |m| {
                    m.copy("Copy body", result.request_body.clone().unwrap_or_default(), "body")
                })
        }
        _ => match &result.response {
            Some(r) => {
                let start = format!("{} {}", r.status, r.reason);
                let (full, headers) = message_text(&start, &r.headers, result.response_body.as_deref());
                Menu::new()
                    .copy("Copy response", full, "response")
                    .copy("Copy headers", headers, "headers")
                    .when(result.response_body.is_some(), |m| {
                        m.copy("Copy body", result.response_body.clone().unwrap_or_default(), "body")
                    })
            }
            None => Menu::new(),
        },
    }
}

fn test_result(result: &RuleTestResult, t: &Tokens) -> AnyElement {
    let (icon, colour, text) = if result.matched {
        (IconName::CircleCheck, t.s_2xx, "The rule matches this flow".to_owned())
    } else {
        (
            IconName::CircleX,
            t.s_failed,
            format!(
                "No match: {}",
                result.mismatch.as_deref().unwrap_or("the matcher doesn't match")
            ),
        )
    };
    let mut col = div()
        .id("rule-test-result")
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_size(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .font_weight(FontWeight::MEDIUM)
                .child(pm_widgets::icon(icon, colour.hsla(), 14.))
                .child(text)
                .when(result.matched && result.response_matched == Some(false), |d| {
                    d.child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(t.muted_foreground.hsla())
                            .child("· its response matcher doesn't match"),
                    )
                }),
        );
    if !result.notes.is_empty() {
        let r = result.clone();
        col = col.child(
            div()
                .id("rule-test-notes")
                .flex()
                .flex_col()
                .pl(px(16.))
                .text_color(t.muted_foreground.hsla())
                .children(result.notes.iter().map(|n| div().child(format!("• {n}"))))
                .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                    test_menu(&r, "notes").show(e.position, window, cx);
                }),
        );
    }
    if result.matched {
        let request = {
            let start = format!("{} {}", result.request.method, result.request.url);
            message_block(
                "request",
                "Request",
                &start,
                &result.request.headers,
                result.request_body.as_deref(),
                result,
                t,
            )
        };
        let response = match &result.response {
            Some(r) => message_block(
                "response",
                "Response",
                &format!("{} {}", r.status, r.reason),
                &r.headers,
                result.response_body.as_deref(),
                result,
                t,
            ),
            None => div()
                .flex_1()
                .rounded(px(6.))
                .border_1()
                .border_color(t.border.hsla())
                .p(px(8.))
                .text_color(t.muted_foreground.hsla())
                .child("No response")
                .into_any_element(),
        };
        col = col.child(div().flex().gap(px(8.)).child(request).child(response));
    }
    col.into_any_element()
}

fn message_block(
    part: &'static str,
    title: &str,
    start: &str,
    headers: &[pm_model::Header],
    body: Option<&str>,
    result: &RuleTestResult,
    t: &Tokens,
) -> AnyElement {
    let mut text = format!("{start}\n");
    text.push_str(
        &headers
            .iter()
            .map(|h| format!("{}: {}", pm_view::http::display_header_name(&h.name), h.value))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    if let Some(b) = body.filter(|b| !b.is_empty()) {
        text.push_str("\n\n");
        match b.char_indices().nth(SHOWN_BODY) {
            Some((cut, _)) => {
                text.push_str(&b[..cut]);
                text.push('…');
            }
            None => text.push_str(b),
        }
    }
    let r = result.clone();
    div()
        .id(SharedString::from(format!("rule-test-{part}")))
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .rounded(px(6.))
        .border_1()
        .border_color(t.border.hsla())
        .child(
            div()
                .px(px(8.))
                .py(px(2.))
                .bg(t.muted.hsla().opacity(0.4))
                .border_b_1()
                .border_color(t.border.hsla())
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_owned()),
        )
        .child(
            div()
                .id(SharedString::from(format!("rule-test-{part}-text")))
                .max_h(px(224.))
                .overflow_y_scroll()
                .px(px(8.))
                .py(px(4.))
                .font_family("Menlo")
                .text_size(px(11.))
                .child(gpui_kit::base::SelectableText::new(
                    SharedString::from(format!("rule-test-{part}-selectable")),
                    text,
                )),
        )
        .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
            let selected = crate::features::context_menu::selected_text(window, cx);
            Menu::new()
                .selection(selected)
                .append(test_menu(&r, part))
                .show(e.position, window, cx);
        })
        .into_any_element()
}

impl Render for RuleForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        // Keep the pickers' lists fresh (and fetched).
        let scripts = self.caches.scripts.update(cx, |c, cx| c.get(&(), cx));
        let mocks = self.caches.mock_sources.update(cx, |c, cx| c.get(&(), cx));
        if let Some(s) = scripts.ready() {
            self.scripts = s.clone();
        }
        if let Some(m) = mocks.ready() {
            self.mocks = m.clone();
        }

        let live = self.live();
        let name = self.line("name", "Rule name", "Rule name", window, cx);
        let mut header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .border_b_1()
            .border_color(t.border.hsla())
            .child(div().w(px(360.)).flex().text_size(px(13.)).child(name));
        if !self.initial.enabled {
            header = header.child(muted("Disabled", &t));
        }
        if let Some(badge) = origin_badge(&self.rule.origin, &t) {
            header = header.child(badge);
        }
        if !self.groups.is_empty() {
            header = header.child(self.picker("group", "Group", 140., cx));
        }
        if live
            .group
            .as_ref()
            .is_some_and(|g| self.groups.iter().any(|x| &x.id == g && !x.enabled))
        {
            header = header.child(muted("Group off", &t));
        }
        if self.is_unsaved() {
            header = header.child(muted("Unsaved", &t));
        }

        let matcher = self.matcher(window, cx);
        let actions = self.actions(window, cx);
        let dry = self.dry_run(cx);
        let body = div()
            .id("rule-form-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .p(px(12.))
                    .child(section("Match", matcher, &t))
                    .child(section("Actions", actions, &t))
                    .child(section("Dry run", dry, &t)),
            );

        let error = self.error().map(str::to_owned);
        let saved = !self.rule.id.is_empty();
        let id: SharedString = self.rule.id.clone().into();
        let footer = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(t.border.hsla())
            .child(
                div()
                    .id("rule-error")
                    .role(Role::Status)
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(t.s_failed.hsla())
                    .children(error),
            )
            .when(saved, |d| {
                d.child(
                    Button::new("delete-rule")
                        .ghost()
                        .small()
                        .accessibility_label("Delete rule")
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(pm_widgets::icon(IconName::Trash, t.foreground.hsla(), 13.))
                                .child("Delete"),
                        )
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(Box::new(super::DeleteRule { id: id.clone() }), cx);
                        }),
                )
            })
            .child(
                Button::new("save-rule")
                    .primary()
                    .small()
                    .disabled(!self.can_save())
                    .accessibility_label(if saved { "Save" } else { "Save rule" })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(pm_widgets::icon(IconName::Save, t.primary_foreground.hsla(), 13.))
                            .child(if saved { "Save" } else { "Save rule" }),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            );

        div()
            .id("rule-form")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background.hsla())
            .child(header)
            .child(body)
            .child(footer)
    }
}

/// Whether a rule came from an agent (the list's "Remove agent rules").
pub fn is_agent(rule: &Rule) -> bool {
    matches!(rule.origin, RuleOrigin::Agent { .. })
}
