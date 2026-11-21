//! The Rules window (`RulesEditor.tsx`, `RuleList.tsx`): a separate native
//! window with the ordered rule list (the ungrouped rules, then each group
//! as a collapsible section with its own on/off switch) beside the
//! selected rule's form. Rules drag within and between sections; groups
//! drag by their headers. The built-in rules (throttle, No Caching, Block
//! Cookies) belong to the toolbar and aren't listed.

use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, Bounds, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, Global,
    KeyDownEvent, MouseButton, MouseDownEvent, Render, Role, SharedString, Subscription, TitlebarOptions, WeakEntity,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use pm_core::Engine;
use pm_model::{Rule, RuleBook, RuleGroup, RuleGroupPatch, RuleOrigin, RuleSettings};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::rules::{self, Names};

use super::form::{FormEvent, RuleForm};
use super::{NewGroup, NewRule, RemoveAgentRules, SetGroupEnabled, SetRuleEnabled, menus, user_rules};
use crate::bridge::keys::{self, RulesCaches};
use crate::bridge::stream::Latest;
use crate::bridge::{self, mutations};
use crate::features::context_menu::Menu;

pub const DEFAULT_SIZE: (f32, f32) = (1180., 760.);
pub const MIN_SIZE: (f32, f32) = (820., 520.);
pub const LIST_WIDTH: f32 = 320.;
pub const DESCRIPTION: &str = "Every matching rule runs, top to bottom. Map Local, rewrites, breakpoints, Map Remote, \
throttling and blocking are all actions. Groups turn their rules on and off together.";
pub const EMPTY: &str = "Select a rule, or create one from a flow's context menu.";

/// What to show when the window opens or comes forward.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Whatever it showed.
    Keep,
    /// A saved rule, by id (rule badges, the row menu's Open).
    Rule(String),
    /// An unsaved rule (New rule, a preset).
    Draft(Box<Rule>),
}

/// A rule being dragged.
#[derive(Clone)]
pub struct RuleDrag {
    pub id: String,
    pub name: SharedString,
}

/// A group being dragged by its header.
#[derive(Clone)]
pub struct GroupDrag {
    pub id: String,
    pub name: SharedString,
}

struct Ghost(SharedString);

impl Render for Ghost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        div()
            .px(px(8.))
            .py(px(2.))
            .rounded(px(4.))
            .bg(t.popover.hsla())
            .border_1()
            .border_color(t.border.hsla())
            .text_size(px(12.))
            .text_color(t.popover_foreground.hsla())
            .child(self.0.clone())
    }
}

/// The open Rules window.
struct Open {
    window: AnyWindowHandle,
    view: WeakEntity<RulesWindow>,
}

impl Global for Open {}

pub struct RulesWindow {
    book: Entity<Latest<RuleBook>>,
    engine: Option<Arc<Engine>>,
    caches: RulesCaches,
    /// The saved rule shown, if any.
    selected: Option<String>,
    /// The form shows an unsaved rule.
    draft: bool,
    /// A rule asked for before the book had it (just created elsewhere).
    pending: Option<String>,
    form: Option<Entity<RuleForm>>,
    collapsed: HashSet<String>,
    renaming: Option<(String, Entity<InputState>, Subscription)>,
    timeout: Option<(Entity<InputState>, Subscription)>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    _form_events: Option<Subscription>,
}

/// Opens the Rules window, or brings it forward, on `target`.
pub fn open_or_focus(target: Target, cx: &mut App) {
    if let Some((window, view)) = RulesWindow::global(cx) {
        let _ = window.update(cx, |_, window, cx| {
            window.activate_window();
            view.update(cx, |w, cx| w.go(target, window, cx));
        });
        return;
    }
    let book = bridge::rule_book(cx);
    let engine = cx.try_global::<bridge::Bridge>().map(|_| bridge::Bridge::engine(cx));
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Rules".into()),
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
        let view = cx.new(|cx| {
            let mut w = RulesWindow::new(book, engine, window, cx);
            w.go(target, window, cx);
            w
        });
        cx.set_global(Open {
            window: window.window_handle(),
            view: view.downgrade(),
        });
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
    });
    match opened {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Err(err) => eprintln!("can't open the Rules window: {err}"),
    }
}

impl RulesWindow {
    pub fn new(
        book: Entity<Latest<RuleBook>>,
        engine: Option<Arc<Engine>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let caches = keys::rules_caches(cx);
        let subscriptions = vec![
            cx.observe_in(&book, window, |this, _, window, cx| this.book_changed(window, cx)),
            cx.observe(&caches.settings, |_, _, cx| cx.notify()),
            cx.observe(&caches.scripts, |_, _, cx| cx.notify()),
            cx.observe(&caches.mock_sources, |_, _, cx| cx.notify()),
        ];
        RulesWindow {
            book,
            engine,
            caches,
            selected: None,
            draft: false,
            pending: None,
            form: None,
            collapsed: HashSet::new(),
            renaming: None,
            timeout: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
            _form_events: None,
        }
    }

    /// The open window and its view.
    pub fn global(cx: &App) -> Option<(AnyWindowHandle, Entity<RulesWindow>)> {
        let open = cx.try_global::<Open>()?;
        Some((open.window, open.view.upgrade()?))
    }

    /// Runs `f` on the open window's view, in its window.
    pub fn update_global(
        cx: &mut App,
        f: impl FnOnce(&mut RulesWindow, &mut Window, &mut Context<RulesWindow>) + 'static,
    ) {
        let Some((window, view)) = RulesWindow::global(cx) else {
            return;
        };
        let _ = window.update(cx, |_, window, cx| view.update(cx, |w, cx| f(w, window, cx)));
    }

    /// Runs `f` on the open form, in its window.
    pub fn update_form(cx: &mut App, f: impl FnOnce(&mut RuleForm, &mut Window, &mut Context<RuleForm>) + 'static) {
        let Some((window, view)) = RulesWindow::global(cx) else {
            return;
        };
        let Some(form) = view.read(cx).form.clone() else { return };
        let _ = window.update(cx, |_, window, cx| form.update(cx, |f2, cx| f(f2, window, cx)));
    }

    pub fn form(&self) -> Option<&Entity<RuleForm>> {
        self.form.as_ref()
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn is_draft(&self) -> bool {
        self.draft
    }

    pub fn book(&self, cx: &App) -> RuleBook {
        self.book.read(cx).value.clone().unwrap_or_default()
    }

    /// The rules listed (no built-ins) and the groups.
    pub fn listed(&self, cx: &App) -> (Vec<Rule>, Vec<RuleGroup>) {
        let book = self.book(cx);
        (user_rules(&book.rules), book.groups)
    }

    pub fn go(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Target::Keep => {}
            Target::Rule(id) => self.select(Some(id), window, cx),
            Target::Draft(rule) => {
                self.selected = None;
                self.pending = None;
                self.draft = true;
                self.set_form(Some(*rule), cx);
            }
        }
        cx.notify();
    }

    /// Shows a saved rule (or nothing).
    pub fn select(&mut self, id: Option<String>, _: &mut Window, cx: &mut Context<Self>) {
        self.draft = false;
        let rule = id
            .as_ref()
            .and_then(|id| self.book(cx).rules.into_iter().find(|r| &r.id == id));
        self.pending = match (&id, &rule) {
            (Some(id), None) => Some(id.clone()),
            _ => None,
        };
        self.selected = id;
        self.set_form(rule, cx);
        cx.notify();
    }

    fn set_form(&mut self, rule: Option<Rule>, cx: &mut Context<Self>) {
        let Some(rule) = rule else {
            self.form = None;
            self._form_events = None;
            return;
        };
        let groups = self.book(cx).groups;
        let engine = self.engine.clone();
        let form = cx.new(|cx| RuleForm::new(rule, groups, engine, cx));
        self._form_events = Some(cx.subscribe(&form, |this, _, e: &FormEvent, cx| match e {
            FormEvent::Saved(rule) => {
                this.draft = false;
                this.pending = None;
                this.selected = Some(rule.id.clone());
                cx.notify();
            }
        }));
        self.form = Some(form);
    }

    /// The rule `id` was deleted: the form closes if it showed it.
    pub fn forget(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(id) {
            self.select(None, window, cx);
        }
    }

    fn book_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let book = self.book(cx);
        if let Some(id) = self.pending.clone()
            && book.rules.iter().any(|r| r.id == id)
        {
            self.select(Some(id), window, cx);
            return;
        }
        if let Some(id) = &self.selected
            && !self.draft
        {
            match book.rules.iter().find(|r| &r.id == id) {
                Some(rule) => {
                    if let Some(form) = &self.form {
                        let (rule, groups) = (rule.clone(), book.groups.clone());
                        form.update(cx, |f, cx| f.set_initial(rule, groups, cx));
                    }
                }
                None => {
                    self.selected = None;
                    self.form = None;
                }
            }
        } else if let Some(form) = &self.form {
            let groups = book.groups.clone();
            form.update(cx, |f, cx| {
                let initial = f.initial().clone();
                f.set_initial(initial, groups, cx);
            });
        }
        cx.notify();
    }

    // ---- Groups ----

    /// Shows the group's name as a field: ↩ or leaving saves, Escape cancels.
    pub fn start_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group) = self.book(cx).groups.into_iter().find(|g| g.id == id) else {
            return;
        };
        self.collapsed.remove(id);
        let state = cx.new(|cx| InputState::new(window, cx).default_value(group.name.clone()));
        let gid = id.to_owned();
        let sub = cx.subscribe_in(&state, window, move |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.finish_rename(&gid, true, window, cx),
            _ => {}
        });
        state.update(cx, |s, cx| s.focus(window, cx));
        self.renaming = Some((id.to_owned(), state, sub));
        cx.notify();
    }

    pub fn renaming(&self) -> Option<&str> {
        self.renaming.as_ref().map(|(id, ..)| id.as_str())
    }

    /// Types a group's new name (the dev remote, tests).
    pub fn type_group_name(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, state, _)) = &self.renaming {
            state.update(cx, |s, cx| s.set_value(name.to_owned(), window, cx));
        }
    }

    pub fn finish_rename(&mut self, id: &str, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.as_ref().map(|r| r.0.as_str()) != Some(id) {
            return;
        }
        let Some((_, state, _)) = self.renaming.take() else {
            return;
        };
        let name = state.read(cx).value().trim().to_owned();
        let old = self.book(cx).groups.into_iter().find(|g| g.id == id).map(|g| g.name);
        if save && !name.is_empty() && old.as_deref() != Some(name.as_str()) {
            mutations::patch_rule_group(
                id.to_owned(),
                RuleGroupPatch {
                    name: Some(name),
                    enabled: None,
                },
                cx,
            );
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn toggle_collapsed(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(id) {
            self.collapsed.insert(id.to_owned());
        }
        cx.notify();
    }

    // ---- Drag and drop ----

    /// A rule dropped on another rule takes its place (moving into that
    /// rule's group first).
    fn drop_on_rule(&mut self, drag: &RuleDrag, target: &str, cx: &mut Context<Self>) {
        let (rules, groups) = self.listed(cx);
        let Some(order) = rules::order_after_drop(&rules, &drag.id, target) else {
            return;
        };
        let Some(moving) = rules.iter().find(|r| r.id == drag.id).cloned() else {
            return;
        };
        let target_group = rules
            .iter()
            .find(|r| r.id == target)
            .and_then(|r| rules::section(r, &groups).map(str::to_owned));
        let moving = Rule {
            group: rules::section(&moving, &groups).map(str::to_owned),
            ..moving
        };
        mutations::drop_rule(moving, target_group, order, cx);
    }

    /// A rule dropped on a section's header moves to its end.
    fn drop_rule_on_section(&mut self, drag: &RuleDrag, group: Option<&str>, cx: &mut Context<Self>) {
        let (rules, groups) = self.listed(cx);
        let Some(moving) = rules.iter().find(|r| r.id == drag.id) else {
            return;
        };
        if rules::section(moving, &groups) != group {
            mutations::move_rules_to_group(vec![drag.id.clone()], group.map(str::to_owned), cx);
        }
    }

    fn drop_group(&mut self, drag: &GroupDrag, target: &str, cx: &mut Context<Self>) {
        let groups = self.book(cx).groups;
        if let Some(ids) = rules::group_order_after_drop(&groups, &drag.id, target) {
            mutations::reorder_rule_groups(ids, cx);
        }
    }

    // ---- Menus ----

    /// A row's menu: `rule:<id>` or `group:<id>` (the dev remote, tests).
    pub fn menu_for(&self, key: &str, cx: &App) -> Option<Menu> {
        let (rules, groups) = self.listed(cx);
        match key.split_once(':')? {
            ("rule", id) => {
                let rule = rules.iter().find(|r| r.id == id)?;
                Some(menus::rule_menu(rule, &rules, &groups))
            }
            ("group", id) => {
                let group = groups.iter().find(|g| g.id == id)?;
                let held = rules::members(&rules, &groups, Some(id)).len();
                Some(menus::group_menu(group, &groups, held))
            }
            _ => None,
        }
    }

    /// The breakpoint timeout as typed: saved once it is a number.
    pub fn set_timeout_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Ok(secs) = text
            .trim()
            .parse::<u32>()
            .or_else(|e| if text.trim().is_empty() { Ok(0) } else { Err(e) })
            && self
                .caches
                .settings
                .read(cx)
                .peek(&())
                .map(|s| s.breakpoint_timeout_secs)
                != Some(secs)
        {
            mutations::set_rule_settings(
                RuleSettings {
                    breakpoint_timeout_secs: secs,
                },
                cx,
            );
        }
    }

    // ---- Rendering ----

    fn names(&self, cx: &App) -> (Names, Names) {
        let scripts = self
            .caches
            .scripts
            .read(cx)
            .peek(&())
            .map(|s| s.iter().map(|s| (s.id.clone(), s.name.clone())).collect())
            .unwrap_or_default();
        let mocks = self
            .caches
            .mock_sources
            .read(cx)
            .peek(&())
            .map(|s| s.iter().map(|s| (s.id.clone(), s.name.clone())).collect())
            .unwrap_or_default();
        (scripts, mocks)
    }

    fn rule_row(
        &self,
        rule: &Rule,
        grouped: bool,
        group_off: bool,
        names: &(Names, Names),
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected.as_deref() == Some(rule.id.as_str()) && !self.draft;
        let off = !rule.enabled || group_off;
        let name = if rule.name.is_empty() {
            "Untitled".to_owned()
        } else {
            rule.name.clone()
        };
        let matcher = rules::matcher_summary(&rule.matcher);
        let actions = rules::actions_summary(rule, &names.0, &names.1);
        let text = if selected {
            t.selection_fg
        } else if off {
            t.muted_foreground
        } else {
            t.foreground
        };
        let sub = if selected || off {
            text.hsla()
        } else {
            t.foreground.hsla().opacity(0.7)
        };
        let id = rule.id.clone();
        let (open_id, drop_id, menu_id) = (id.clone(), id.clone(), id.clone());
        let check_tip = if group_off {
            "Its group is off: the rule runs once the group is back on"
        } else if rule.enabled {
            "Enabled: click to turn it off"
        } else {
            "Disabled: click to turn it on"
        };
        let enabled = rule.enabled;
        let check_id: SharedString = id.clone().into();
        let checkbox = Checkbox::new(SharedString::from(format!("rule-enabled-{id}")))
            .checked(enabled)
            .small()
            .tooltip(check_tip)
            .accessibility_label(SharedString::from(format!("{name} enabled")))
            .on_click(move |on, window, cx| {
                window.dispatch_action(
                    Box::new(SetRuleEnabled {
                        id: check_id.clone(),
                        enabled: *on,
                    }),
                    cx,
                );
            });
        let label: SharedString = format!("{name}{}: {matcher}; {actions}", if off { " (off)" } else { "" }).into();
        div()
            .id(SharedString::from(format!("rule-row-{id}")))
            .role(Role::ListBoxOption)
            .aria_label(label)
            .aria_selected(selected)
            .flex()
            .items_start()
            .gap(px(4.))
            .px(px(4.))
            .py(px(4.))
            .when(grouped, |d| d.pl(px(16.)))
            .rounded(px(4.))
            .text_size(px(12.))
            .when(selected, |d| d.bg(t.selection_bg.hsla()))
            .when(!selected, |d| d.hover(|s| s.bg(t.accent.hsla())))
            .on_drag(
                RuleDrag {
                    id: id.clone(),
                    name: name.clone().into(),
                },
                |d, _, _, cx| cx.new(|_| Ghost(d.name.clone())),
            )
            .drag_over::<RuleDrag>({
                let primary = t.primary.hsla();
                move |s, _, _, _| s.border_t_2().border_color(primary)
            })
            .on_drop(cx.listener(move |this, d: &RuleDrag, _, cx| this.drop_on_rule(d, &drop_id, cx)))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select(Some(open_id.clone()), window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    if let Some(menu) = this.menu_for(&format!("rule:{menu_id}"), cx) {
                        menu.show(e.position, window, cx);
                    }
                }),
            )
            .child(div().pt(px(2.)).child(pm_widgets::icon(
                IconName::GripVertical,
                t.muted_foreground.hsla().opacity(0.6),
                12.,
            )))
            .child(div().pt(px(1.)).child(checkbox))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .text_color(text.hsla())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(div().truncate().font_weight(FontWeight::MEDIUM).child(name))
                            .children(origin_badge(&rule.origin, t)),
                    )
                    .child(
                        div()
                            .truncate()
                            .font_family("Menlo")
                            .text_size(px(10.5))
                            .text_color(sub)
                            .child(matcher),
                    )
                    .child(div().truncate().text_size(px(11.)).text_color(sub).child(actions)),
            )
            .test_support()
            .into_any_element()
    }

    fn group_header(&self, group: &RuleGroup, members: &[&Rule], t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let open = !self.collapsed.contains(&group.id);
        let id = group.id.clone();
        let count = rules::group_count(members);
        let on = members.iter().filter(|r| r.enabled).count();
        let count_tip = format!(
            "{on} of {} rule{} enabled",
            members.len(),
            if members.len() == 1 { "" } else { "s" }
        );
        let (toggle_id, drop_id, drop_rule_id, menu_id, rename_id) =
            (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
        let gid: SharedString = id.clone().into();
        let enabled = group.enabled;
        let chevron = Button::new(SharedString::from(format!("group-open-{id}")))
            .ghost()
            .xsmall()
            .accessibility_label(SharedString::from(format!(
                "{} {}",
                if open { "Collapse" } else { "Expand" },
                group.name
            )))
            .child(pm_widgets::icon(
                if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                },
                t.muted_foreground.hsla(),
                13.,
            ))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_collapsed(&toggle_id, cx)));
        let checkbox = Checkbox::new(SharedString::from(format!("group-enabled-{id}")))
            .checked(enabled)
            .small()
            .tooltip(if enabled {
                "On: its enabled rules run. Click to turn the whole group off"
            } else {
                "Off: none of its rules run. Click to turn the group on"
            })
            .accessibility_label(SharedString::from(format!("{} group enabled", group.name)))
            .on_click(move |on, window, cx| {
                window.dispatch_action(
                    Box::new(SetGroupEnabled {
                        id: gid.clone(),
                        enabled: *on,
                    }),
                    cx,
                );
            });
        let name: AnyElement = match &self.renaming {
            Some((rid, state, _)) if *rid == group.id => {
                let cancel = rid.clone();
                div()
                    .flex_1()
                    .min_w_0()
                    .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                        if e.keystroke.key == "escape" {
                            cx.stop_propagation();
                            this.finish_rename(&cancel, false, window, cx);
                        }
                    }))
                    .child(
                        Input::new(state)
                            .id("group-name-input")
                            .xsmall()
                            .aria_label("Group name"),
                    )
                    .into_any_element()
            }
            _ => div()
                .id(SharedString::from(format!("group-name-{id}")))
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .when(!enabled, |d| d.text_color(t.muted_foreground.hsla()))
                .child(group.name.clone())
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    if e.click_count() >= 2 {
                        this.start_rename(&rename_id, window, cx);
                    }
                }))
                .into_any_element(),
        };
        div()
            .id(SharedString::from(format!("group-header-{id}")))
            .role(Role::Group)
            .aria_label(SharedString::from(format!("Group {}, {count_tip}", group.name)))
            .aria_expanded(open)
            .mt(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(4.))
            .py(px(4.))
            .rounded(px(4.))
            .text_size(px(12.))
            .bg(t.muted.hsla().opacity(0.6))
            .on_drag(
                GroupDrag {
                    id: id.clone(),
                    name: group.name.clone().into(),
                },
                |d, _, _, cx| cx.new(|_| Ghost(d.name.clone())),
            )
            .drag_over::<GroupDrag>({
                let primary = t.primary.hsla();
                move |s, _, _, _| s.border_t_2().border_color(primary)
            })
            .drag_over::<RuleDrag>({
                let primary = t.primary.hsla();
                move |s, _, _, _| s.border_1().border_color(primary)
            })
            .on_drop(cx.listener(move |this, d: &GroupDrag, _, cx| this.drop_group(d, &drop_id, cx)))
            .on_drop(cx.listener(move |this, d: &RuleDrag, _, cx| {
                this.drop_rule_on_section(d, Some(&drop_rule_id), cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    if let Some(menu) = this.menu_for(&format!("group:{menu_id}"), cx) {
                        menu.show(e.position, window, cx);
                    }
                }),
            )
            .child(chevron)
            .child(checkbox)
            .child(name)
            .children(origin_badge(&group.origin, t))
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(t.muted_foreground.hsla())
                    .child(count),
            )
            .into_any_element()
    }

    fn list(&self, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let (rules, groups) = self.listed(cx);
        let names = self.names(cx);
        let mut items: Vec<AnyElement> = Vec::new();
        if self.draft {
            items.push(
                div()
                    .mb(px(2.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .bg(t.selection_bg.hsla())
                    .text_color(t.selection_fg.hsla())
                    .text_size(px(12.))
                    .italic()
                    .child("Unsaved rule")
                    .into_any_element(),
            );
        }
        if rules.is_empty() && groups.is_empty() && !self.draft {
            items.push(
                div()
                    .p(px(8.))
                    .text_size(px(12.))
                    .text_color(t.muted_foreground.hsla())
                    .child("No rules yet.")
                    .into_any_element(),
            );
        }
        let ungrouped = rules::members(&rules, &groups, None);
        if !groups.is_empty() && (!ungrouped.is_empty() || cx.has_active_drag()) {
            items.push(
                div()
                    .id("rule-section-ungrouped")
                    .px(px(8.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .rounded(px(4.))
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.muted_foreground.hsla())
                    .drag_over::<RuleDrag>({
                        let primary = t.primary.hsla();
                        move |s, _, _, _| s.border_1().border_color(primary)
                    })
                    .on_drop(cx.listener(|this, d: &RuleDrag, _, cx| this.drop_rule_on_section(d, None, cx)))
                    .child("UNGROUPED")
                    .into_any_element(),
            );
        }
        for rule in &ungrouped {
            items.push(self.rule_row(rule, false, false, &names, t, cx));
        }
        for group in &groups {
            let members = rules::members(&rules, &groups, Some(&group.id));
            items.push(self.group_header(group, &members, t, cx));
            if self.collapsed.contains(&group.id) {
                continue;
            }
            for rule in &members {
                items.push(self.rule_row(rule, true, !group.enabled, &names, t, cx));
            }
            if members.is_empty() {
                items.push(
                    div()
                        .pl(px(32.))
                        .pr(px(8.))
                        .py(px(4.))
                        .text_size(px(11.))
                        .italic()
                        .text_color(t.muted_foreground.hsla())
                        .child("No rules: drag rules here, or use a rule's Move to Group menu.")
                        .into_any_element(),
                );
            }
        }
        let agent_rules = rules.iter().filter(|r| super::form::is_agent(r)).count();
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .p(px(6.))
            .border_b_1()
            .border_color(t.border.hsla())
            .child(
                Button::new("new-rule")
                    .outline()
                    .xsmall()
                    .accessibility_label("New rule")
                    .child(icon_label(IconName::Plus, "New rule", t))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(NewRule), cx)),
            )
            .child(
                Button::new("new-rule-group")
                    .ghost()
                    .xsmall()
                    .accessibility_label("New group")
                    .tooltip("A named group of rules that turn on and off together")
                    .child(icon_label(IconName::FolderPlus, "New group", t))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(NewGroup { rule: None }), cx)),
            )
            .when(agent_rules > 0, |d| {
                d.child(div().flex_1()).child(
                    Button::new("remove-agent-rules")
                        .ghost()
                        .xsmall()
                        .accessibility_label("Remove agent rules")
                        .tooltip("Delete every rule an AI agent created")
                        .child(icon_label(IconName::Bot, "Remove agent rules", t))
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(RemoveAgentRules), cx)),
                )
            });
        div()
            .id("rule-list-pane")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(LIST_WIDTH))
            .h_full()
            .bg(t.muted.hsla().opacity(0.2))
            .border_r_1()
            .border_color(t.border.hsla())
            .child(toolbar)
            .child(
                div()
                    .id("rule-list")
                    .role(Role::ListBox)
                    .aria_label("Rules")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .children(items),
            )
            .children(self.timeout_row(t))
            .into_any_element()
    }

    fn timeout_row(&self, t: &Tokens) -> Option<AnyElement> {
        let (state, _) = self.timeout.as_ref()?;
        Some(
            div()
                .id("breakpoint-timeout")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .p(px(8.))
                .border_t_1()
                .border_color(t.border.hsla())
                .text_size(px(12.))
                .text_color(t.muted_foreground.hsla())
                .child("Breakpoint timeout")
                .child(
                    div().w(px(64.)).child(
                        Input::new(state)
                            .id("breakpoint-timeout-input")
                            .xsmall()
                            .aria_label("Breakpoint timeout in seconds"),
                    ),
                )
                .child("s")
                .child(div().text_size(px(11.)).child("(0 waits forever)"))
                .into_any_element(),
        )
    }

    /// The timeout field, once the settings are in.
    fn ensure_timeout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.caches.settings.update(cx, |c, cx| c.get(&(), cx));
        let Some(settings) = settings.ready().cloned() else {
            return;
        };
        let text = settings.breakpoint_timeout_secs.to_string();
        match &self.timeout {
            Some((state, _)) => {
                // Follows a change made elsewhere unless it is being typed.
                let focused = state.read(cx).focus_handle(cx).is_focused(window);
                let shown = state.read(cx).value().trim().parse::<u32>().ok();
                if !focused && shown != Some(settings.breakpoint_timeout_secs) {
                    state.update(cx, |s, cx| s.set_value(text, window, cx));
                }
            }
            None => {
                let state = cx.new(|cx| InputState::new(window, cx).default_value(text));
                let sub = cx.subscribe(&state, |this, state, e: &InputEvent, cx| {
                    if matches!(e, InputEvent::Change) {
                        let text = state.read(cx).value().to_string();
                        this.set_timeout_text(&text, cx);
                    }
                });
                self.timeout = Some((state, sub));
            }
        }
    }
}

fn icon_label(icon: IconName, label: &'static str, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(pm_widgets::icon(icon, t.foreground.hsla(), 12.))
        .child(label)
        .into_any_element()
}

/// Where a rule came from: nothing for the user's own, the preset's name,
/// or "AI · <client>" in the agent colours.
pub fn origin_badge(origin: &RuleOrigin, t: &Tokens) -> Option<AnyElement> {
    let label = rules::origin_label(origin)?;
    let agent = matches!(origin, RuleOrigin::Agent { .. });
    let (fg, bg) = if agent {
        (t.rule_agent, t.rule_agent_bg)
    } else {
        (t.muted_foreground, t.muted)
    };
    let tip: SharedString = rules::origin_tip(origin).unwrap_or_default().into();
    Some(
        div()
            .id(SharedString::from(format!("origin-{label}")))
            .flex_none()
            .h(px(16.))
            .px(px(4.))
            .flex()
            .items_center()
            .gap(px(2.))
            .rounded(px(3.))
            .bg(bg.hsla())
            .text_color(fg.hsla())
            .text_size(px(10.))
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .role(Role::Note)
            .aria_label(tip)
            .when(agent, |d| d.child(pm_widgets::icon(IconName::Bot, fg.hsla(), 10.)))
            .child(label)
            .into_any_element(),
    )
}

impl Focusable for RulesWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RulesWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        self.ensure_timeout(window, cx);
        // The pickers' lists and the list's names (fetched on first use).
        self.caches.scripts.update(cx, |c, cx| {
            c.get(&(), cx);
        });
        self.caches.mock_sources.update(cx, |c, cx| {
            c.get(&(), cx);
        });
        let header = div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(2.))
            .px(px(16.))
            .py(px(10.))
            .border_b_1()
            .border_color(t.border.hsla())
            .child(
                div()
                    .id("rules-title")
                    .role(Role::Heading)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(pm_widgets::icon(IconName::ListChecks, t.foreground.hsla(), 15.))
                    .child("Rules"),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.muted_foreground.hsla())
                    .child(DESCRIPTION),
            );
        let content: AnyElement = match &self.form {
            Some(form) => form.clone().into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(t.muted_foreground.hsla())
                .child(EMPTY)
                .into_any_element(),
        };
        div()
            .id("rules-window")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background.hsla())
            .text_color(t.foreground.hsla())
            .text_size(px(13.))
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.list(&t, cx))
                    .child(div().flex_1().min_w_0().h_full().child(content)),
            )
    }
}
