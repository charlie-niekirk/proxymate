//! Rules (G7): the Rules window (`RulesEditor.tsx`, `RuleList.tsx`), a
//! separate native window with the ordered rule list beside the selected
//! rule's form; the flow menu's Map Local… and Breakpoint… presets; and the
//! toolbar's Rules button, No Caching and Block Cookies toggles, network
//! conditions and breakpoints badge.
//!
//! The rule book is a push (`bridge::rule_book`); every write goes through
//! `bridge::mutations`. The list's and the form's menus and pickers are
//! [`menus`] built from state, whose entries dispatch the actions below;
//! their handlers find the open window ([`RulesWindow::global`]).

pub mod form;
pub mod menus;
pub mod tools;
pub mod window;

use gpui_kit::{App, AppContext as _, SharedString};
use pm_model::{PresetKind, Rule, RuleGroupPatch};
use pm_view::rules;

use crate::bridge::{self, mutations};
use crate::features::chrome::OpenRule;
use crate::features::clipboard::{self, report_error};
use crate::features::commands;
use crate::features::flow_menu::{self, Slot};
use crate::features::inspector::actions::FlowRef;
use crate::shell::toolbar;
use crate::workspace::Workspace;

pub use window::{RulesWindow, Target};

/// The Rules window (⌘K "Rules…", the toolbar's Rules button).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct OpenRules;

/// A preset's unsaved rule from a flow (the flow menu's Map Local… and
/// Breakpoint…).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct OpenPreset {
    pub flow: FlowRef,
    pub preset: PresetKind,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SetRuleEnabled {
    pub id: SharedString,
    pub enabled: bool,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct DuplicateRule {
    pub id: SharedString,
}

/// Move to top / bottom (of the rule's section).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct MoveRule {
    pub id: SharedString,
    pub top: bool,
}

/// Move to group ▸ a group, or "No group" (`None`).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct MoveRuleToGroup {
    pub id: SharedString,
    pub group: Option<SharedString>,
}

/// New group… (with the rule in it), or the list's New group (`None`).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct NewGroup {
    pub rule: Option<SharedString>,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct DeleteRule {
    pub id: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SetGroupEnabled {
    pub id: SharedString,
    pub enabled: bool,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct NewRuleInGroup {
    pub id: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct RenameGroup {
    pub id: SharedString,
}

/// Move up / down.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct MoveGroup {
    pub id: SharedString,
    pub down: bool,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct DeleteGroup {
    pub id: SharedString,
    pub delete_rules: bool,
}

/// A throttle preset by name, or off (`None`).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SetThrottle {
    pub preset: Option<SharedString>,
}

/// A picker of the open rule form: `key` (see `pm_view::rules::edit`) set
/// to `value`.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct FormChoice {
    pub key: SharedString,
    pub value: SharedString,
}

/// "Add action" ▸ a type.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct AddAction {
    pub type_id: SharedString,
}

gpui_kit::actions!(
    proxymate,
    [
        NewRule,
        RemoveAgentRules,
        OpenBreakpoints,
        ToggleNoCache,
        ToggleNoCookies,
        /// The throttle menu's "Custom" entry: it only shows the state.
        ThrottleCustom
    ]
);

pub fn register(cx: &mut App) {
    cx.on_action(|_: &OpenRules, cx| open(Target::Keep, cx));
    cx.on_action(|a: &OpenRule, cx| open(Target::Rule(a.id.to_string()), cx));
    cx.on_action(|_: &NewRule, cx| open(Target::Draft(Box::new(rules::new_rule())), cx));
    cx.on_action(|a: &OpenPreset, cx| open_preset(a.flow, a.preset, cx));
    cx.on_action(|a: &SetRuleEnabled, cx| mutations::set_rule_enabled(a.id.to_string(), a.enabled, cx));
    cx.on_action(|a: &DuplicateRule, cx| {
        let Some(rule) = find_rule(&a.id, cx) else { return };
        mutations::save_rule(
            rules::duplicate(&rule),
            |copy, cx| open(Target::Rule(copy.id), cx),
            |err, cx| report_error("Couldn't duplicate the rule", err, cx),
            cx,
        );
    });
    cx.on_action(|a: &MoveRule, cx| {
        let Some(book) = book(cx) else { return };
        let user: Vec<Rule> = user_rules(&book.rules);
        mutations::reorder_rules(rules::order_to_edge(&user, &a.id, a.top), cx);
    });
    cx.on_action(|a: &MoveRuleToGroup, cx| {
        mutations::move_rules_to_group(vec![a.id.to_string()], a.group.as_ref().map(ToString::to_string), cx);
    });
    cx.on_action(|a: &NewGroup, cx| {
        let ids: Vec<String> = a.rule.iter().map(ToString::to_string).collect();
        mutations::create_rule_group(
            ids,
            |id, cx| {
                RulesWindow::update_global(cx, move |w, window, cx| w.start_rename(&id, window, cx));
            },
            cx,
        );
    });
    cx.on_action(|a: &DeleteRule, cx| {
        let id = a.id.to_string();
        mutations::delete_rule(
            id.clone(),
            move |cx| RulesWindow::update_global(cx, move |w, window, cx| w.forget(&id, window, cx)),
            cx,
        );
    });
    cx.on_action(|a: &SetGroupEnabled, cx| {
        mutations::patch_rule_group(
            a.id.to_string(),
            RuleGroupPatch {
                name: None,
                enabled: Some(a.enabled),
            },
            cx,
        );
    });
    cx.on_action(|a: &NewRuleInGroup, cx| {
        let rule = Rule {
            group: Some(a.id.to_string()),
            ..rules::new_rule()
        };
        open(Target::Draft(Box::new(rule)), cx);
    });
    cx.on_action(|a: &RenameGroup, cx| {
        let id = a.id.to_string();
        RulesWindow::update_global(cx, move |w, window, cx| w.start_rename(&id, window, cx));
    });
    cx.on_action(|a: &MoveGroup, cx| {
        let Some(book) = book(cx) else { return };
        if let Some(ids) = rules::group_order_after_move(&book.groups, &a.id, a.down) {
            mutations::reorder_rule_groups(ids, cx);
        }
    });
    cx.on_action(|a: &DeleteGroup, cx| mutations::delete_rule_group(a.id.to_string(), a.delete_rules, cx));
    cx.on_action(|_: &RemoveAgentRules, cx| mutations::delete_agent_rules(cx));
    cx.on_action(|a: &SetThrottle, cx| {
        let preset = a.preset.as_deref().and_then(rules::throttle_preset);
        mutations::set_throttle(preset, cx);
    });
    cx.on_action(|_: &ThrottleCustom, _| {});
    cx.on_action(|_: &ToggleNoCache, cx| toggle_builtin(rules::NO_CACHE_RULE_ID, cx));
    cx.on_action(|_: &ToggleNoCookies, cx| toggle_builtin(rules::NO_COOKIES_RULE_ID, cx));
    cx.on_action(|_: &OpenBreakpoints, cx| {
        clipboard::info("Breakpoints", Some("The Breakpoint Editor arrives in G8.".into()), cx);
    });
    cx.on_action(|a: &FormChoice, cx| {
        let (key, value) = (a.key.to_string(), a.value.to_string());
        RulesWindow::update_form(cx, move |f, window, cx| f.choose(&key, &value, window, cx));
    });
    cx.on_action(|a: &AddAction, cx| {
        let type_id = a.type_id.to_string();
        RulesWindow::update_form(cx, move |f, window, cx| f.add_action(&type_id, window, cx));
    });

    commands::add("rules", commands::always, |_, cx| open(Target::Keep, cx), cx);
    commands::add(
        "no-cache",
        commands::always,
        |_, cx| toggle_builtin(rules::NO_CACHE_RULE_ID, cx),
        cx,
    );
    commands::add(
        "no-cookies",
        commands::always,
        |_, cx| toggle_builtin(rules::NO_COOKIES_RULE_ID, cx),
        cx,
    );
    commands::add(
        "breakpoints",
        commands::always,
        |window, cx| window.dispatch_action(Box::new(OpenBreakpoints), cx),
        cx,
    );

    flow_menu::extend(Slot::Rules, menus::flow_presets, cx);
    toolbar::add_tool(toolbar::order::RULES, tools::rules_button, cx);
    toolbar::add_tool(toolbar::order::NOISE, tools::rule_tools, cx);
}

/// `PROXYMATE_DEV_OPEN=rules` opens the Rules window at launch (dev only).
pub fn open_from_env(cx: &mut App) {
    if crate::dev_env::value("PROXYMATE_DEV_OPEN").as_deref() == Some("rules") {
        open(Target::Keep, cx);
    }
}

/// The rule book as last pushed.
pub fn book(cx: &mut App) -> Option<pm_model::RuleBook> {
    bridge::rule_book(cx).read(cx).value.clone()
}

/// The rules the window lists: every rule but the built-in ones.
pub fn user_rules(rules: &[Rule]) -> Vec<Rule> {
    rules.iter().filter(|r| !rules::is_builtin(&r.id)).cloned().collect()
}

fn find_rule(id: &str, cx: &mut App) -> Option<Rule> {
    book(cx)?.rules.into_iter().find(|r| r.id == id)
}

/// No Caching / Block Cookies: flips the built-in rule.
pub fn toggle_builtin(id: &'static str, cx: &mut App) {
    let on = find_rule(id, cx).is_some_and(|r| r.enabled);
    mutations::set_rule_enabled(id.into(), !on, cx);
}

/// Opens the Rules window (or brings it forward) on `target`.
pub fn open(target: Target, cx: &mut App) {
    // Deferred: the action that asked is still being dispatched by the
    // window it came from.
    cx.defer(move |cx| window::open_or_focus(target, cx));
}

/// "Map Local…", "Breakpoint…": the preset's rule, drafted from the flow by
/// the engine, opened unsaved; the flow is selected for the dry run.
fn open_preset(flow: FlowRef, preset: PresetKind, cx: &mut App) {
    let Some(engine) = Workspace::global(cx).and_then(|ws| ws.read(cx).engine().cloned()) else {
        return;
    };
    let task = cx.background_spawn(async move { engine.draft_rule_in(flow.store, flow.id, preset) });
    cx.spawn(async move |cx| {
        let draft = task.await;
        cx.update(|cx| match draft {
            Ok(rule) => {
                if let Some(table) = crate::shell::parts(cx).and_then(|p| p.table.upgrade())
                    && Workspace::global(cx).is_some_and(|ws| ws.read(cx).store == flow.store)
                {
                    let grid = table.read(cx).grid().clone();
                    grid.update(cx, |g, cx| g.select(Some(flow.id), cx));
                }
                open(Target::Draft(Box::new(rule)), cx);
            }
            Err(err) => report_error("Couldn't draft the rule", err, cx),
        });
    })
    .detach();
}
