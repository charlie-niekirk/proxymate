//! The Rules window's menus, built from state (G2's rule): a rule row's
//! (`RuleItems`), a group header's (`GroupItems`), the flow menu's presets,
//! the toolbar's network conditions, "Add action" and the form's pickers.

use gpui_kit::{App, SharedString};
use pm_model::{PresetKind, Rule, RuleGroup};
use pm_view::rules::{self, ThrottlePreset};

use super::{
    AddAction, DeleteGroup, DeleteRule, DuplicateRule, FormChoice, MoveGroup, MoveRule, MoveRuleToGroup, NewGroup,
    NewRuleInGroup, OpenPreset, RenameGroup, SetGroupEnabled, SetRuleEnabled, SetThrottle, ThrottleCustom,
};
use crate::features::chrome::OpenRule;
use crate::features::context_menu::Menu;
use crate::features::flow_menu::RowContext;

/// The flow menu's Map Local… and Breakpoint… (not for CONNECT tunnels or
/// WebSockets, which have no request to answer).
pub fn flow_presets(ctx: &RowContext, _: &App) -> Menu {
    let flow = ctx.flow();
    let sendable = !ctx.is_connect() && !ctx.is_websocket();
    let item = |menu: Menu, label: &str, preset| {
        let action = OpenPreset { flow, preset };
        if sendable {
            menu.item(label.to_owned(), action)
        } else {
            menu.disabled(label.to_owned(), action)
        }
    };
    let menu = item(Menu::new(), "Map Local…", PresetKind::MapLocal);
    item(menu, "Breakpoint…", PresetKind::Breakpoint)
}

fn id(s: &str) -> SharedString {
    s.to_owned().into()
}

/// A rule row's menu. `rules` are the listed rules (no built-ins).
pub fn rule_menu(rule: &Rule, rules: &[Rule], groups: &[RuleGroup]) -> Menu {
    let section = rules::section(rule, groups);
    let siblings = rules::members(rules, groups, section);
    let index = siblings.iter().position(|r| r.id == rule.id).unwrap_or(0);
    let last = siblings.len().saturating_sub(1);
    let rid = id(&rule.id);
    let mut move_to = Menu::new();
    for g in groups {
        let action = MoveRuleToGroup {
            id: rid.clone(),
            group: Some(id(&g.id)),
        };
        move_to = if rule.group.as_deref() == Some(g.id.as_str()) {
            move_to.disabled(g.name.clone(), action)
        } else {
            move_to.item(g.name.clone(), action)
        };
    }
    move_to = move_to.separator().item(
        "New group…",
        NewGroup {
            rule: Some(rid.clone()),
        },
    );
    let ungroup = MoveRuleToGroup {
        id: rid.clone(),
        group: None,
    };
    move_to = if rule.group.is_some() {
        move_to.item("No group", ungroup)
    } else {
        move_to.disabled("No group", ungroup)
    };
    let top = MoveRule {
        id: rid.clone(),
        top: true,
    };
    let bottom = MoveRule {
        id: rid.clone(),
        top: false,
    };
    let menu = Menu::new()
        .item("Open", OpenRule { id: rid.clone() })
        .item(
            if rule.enabled { "Disable" } else { "Enable" },
            SetRuleEnabled {
                id: rid.clone(),
                enabled: !rule.enabled,
            },
        )
        .item("Duplicate", DuplicateRule { id: rid.clone() })
        .separator();
    let menu = if index == 0 {
        menu.disabled("Move to top", top)
    } else {
        menu.item("Move to top", top)
    };
    let menu = if index == last {
        menu.disabled("Move to bottom", bottom)
    } else {
        menu.item("Move to bottom", bottom)
    };
    menu.submenu("Move to group", move_to)
        .separator()
        .copy("Copy name", rule.name.clone(), "rule name")
        .copy("Copy as JSON", rules::rule_json(rule), "rule as JSON")
        .separator()
        .item("Delete", DeleteRule { id: rid })
}

/// A group header's menu; `held` is how many rules it has.
pub fn group_menu(group: &RuleGroup, groups: &[RuleGroup], held: usize) -> Menu {
    let gid = id(&group.id);
    let index = groups.iter().position(|g| g.id == group.id).unwrap_or(0);
    let up = MoveGroup {
        id: gid.clone(),
        down: false,
    };
    let down = MoveGroup {
        id: gid.clone(),
        down: true,
    };
    let menu = Menu::new()
        .item(
            if group.enabled {
                "Turn group off"
            } else {
                "Turn group on"
            },
            SetGroupEnabled {
                id: gid.clone(),
                enabled: !group.enabled,
            },
        )
        .item("New rule in group", NewRuleInGroup { id: gid.clone() })
        .item("Rename…", RenameGroup { id: gid.clone() })
        .separator();
    let menu = if index == 0 {
        menu.disabled("Move up", up)
    } else {
        menu.item("Move up", up)
    };
    let menu = if index + 1 >= groups.len() {
        menu.disabled("Move down", down)
    } else {
        menu.item("Move down", down)
    };
    let delete_all = DeleteGroup {
        id: gid.clone(),
        delete_rules: true,
    };
    let delete_label = format!(
        "Delete group and its {}",
        if held == 1 {
            "rule".to_owned()
        } else {
            format!("{held} rules")
        }
    );
    let menu = menu
        .separator()
        .copy("Copy name", group.name.clone(), "group name")
        .separator()
        .item(
            "Delete group, keep its rules",
            DeleteGroup {
                id: gid,
                delete_rules: false,
            },
        );
    if held == 0 {
        menu.disabled(delete_label, delete_all)
    } else {
        menu.item(delete_label, delete_all)
    }
}

/// The toolbar's network conditions: "Throttle all traffic", Off and the
/// presets with their speeds, the active one checked ("Custom" when the
/// rule was edited elsewhere).
pub fn throttle_menu(active: Option<&ThrottlePreset>) -> Menu {
    let mut menu = Menu::new().disabled("Throttle all traffic", ThrottleCustom).check(
        "Off — Full speed",
        active.is_none(),
        SetThrottle { preset: None },
    );
    for (name, ..) in rules::THROTTLE_PRESETS {
        let Some(p) = rules::throttle_preset(name) else {
            continue;
        };
        menu = menu.check(
            format!("{name} — {}", rules::throttle_detail(&p)),
            active.is_some_and(|a| a.name == name),
            SetThrottle {
                preset: Some(name.into()),
            },
        );
    }
    if active.is_some_and(|a| a.name == "Custom") {
        menu = menu.check("Custom — Edited in the Rules window", true, ThrottleCustom);
    }
    menu
}

/// "Add action": each group's actions under its name.
pub fn add_action_menu() -> Menu {
    let mut menu = Menu::new();
    for group in rules::action_groups() {
        let mut sub = Menu::new();
        for a in rules::ACTION_TYPES.iter().filter(|a| a.group == group) {
            sub = sub.item(a.label, AddAction { type_id: a.id.into() });
        }
        menu = menu.submenu(group, sub);
    }
    menu
}

/// A picker's menu: each option checked if it is `current`.
pub fn choice_menu(key: &str, options: &[(String, String)], current: &str) -> Menu {
    options.iter().fold(Menu::new(), |menu, (value, label)| {
        menu.check(
            label.clone(),
            value == current,
            FormChoice {
                key: key.to_owned().into(),
                value: value.clone().into(),
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use pm_model::RuleOrigin;

    use super::*;

    fn rule(id: &str, group: Option<&str>) -> Rule {
        Rule {
            id: id.into(),
            name: format!("Rule {id}"),
            group: group.map(Into::into),
            ..rules::new_rule()
        }
    }

    fn group(id: &str) -> RuleGroup {
        RuleGroup {
            id: id.into(),
            name: format!("Group {id}"),
            enabled: true,
            order: 0,
            origin: RuleOrigin::User,
        }
    }

    #[test]
    fn a_rule_menu_moves_within_its_section_and_copies() {
        let groups = [group("g1")];
        let rules = [rule("a", None), rule("b", None), rule("c", Some("g1"))];
        let menu = rule_menu(&rules[0], &rules, &groups);
        assert_eq!(
            menu.labels(),
            [
                "Open",
                "Disable",
                "Duplicate",
                "—",
                "Move to top",
                "Move to bottom",
                "Move to group",
                "—",
                "Copy name",
                "Copy as JSON",
                "—",
                "Delete"
            ]
        );
        assert!(!menu.is_enabled("Move to top"));
        assert!(menu.is_enabled("Move to bottom"));
        assert!(menu.is_enabled("Move to group › Group g1"));
        assert!(menu.is_enabled("Move to group › New group…"));
        assert!(!menu.is_enabled("Move to group › No group"));
        assert_eq!(menu.copy_text("Copy name"), Some("Rule a"));
        assert!(menu.copy_text("Copy as JSON").unwrap().contains("\"name\": \"Rule a\""));
        let grouped = rule_menu(&rules[2], &rules, &groups);
        assert!(!grouped.is_enabled("Move to top"), "first of its group");
        assert!(!grouped.is_enabled("Move to bottom"), "last of its group");
        assert!(!grouped.is_enabled("Move to group › Group g1"));
        assert!(grouped.is_enabled("Move to group › No group"));
        let off = Rule {
            enabled: false,
            ..rules[1].clone()
        };
        assert_eq!(rule_menu(&off, &rules, &groups).labels()[1], "Enable");
    }

    #[test]
    fn a_group_menu_counts_its_rules() {
        let groups = [group("g1"), group("g2")];
        let menu = group_menu(&groups[0], &groups, 3);
        assert_eq!(
            menu.labels(),
            [
                "Turn group off",
                "New rule in group",
                "Rename…",
                "—",
                "Move up",
                "Move down",
                "—",
                "Copy name",
                "—",
                "Delete group, keep its rules",
                "Delete group and its 3 rules"
            ]
        );
        assert!(!menu.is_enabled("Move up"));
        assert!(menu.is_enabled("Move down"));
        let empty = group_menu(&groups[1], &groups, 0);
        assert!(!empty.is_enabled("Move down"));
        assert!(!empty.is_enabled("Delete group and its 0 rules"));
        assert_eq!(
            group_menu(&groups[1], &groups, 1).labels()[10],
            "Delete group and its rule"
        );
    }

    #[test]
    fn the_throttle_menu_checks_the_active_preset() {
        let menu = throttle_menu(None);
        assert_eq!(menu.labels().len(), 7);
        assert_eq!(menu.labels()[2], "LTE — 12 Mbps ↓ · 6 Mbps ↑ · 40 ms");
        assert!(!menu.is_enabled("Throttle all traffic"), "a heading");
        let edge = rules::throttle_preset("Edge").unwrap();
        let on = throttle_menu(Some(&edge));
        let checked: Vec<&str> = on
            .entries()
            .iter()
            .filter_map(|e| match e {
                crate::features::context_menu::Entry::Check {
                    label, checked: true, ..
                } => Some(label.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(checked, ["Edge — 240 kbps ↓ · 200 kbps ↑ · 400 ms"]);
        let custom = ThrottlePreset {
            name: "Custom".into(),
            download_kbps: 1,
            upload_kbps: 1,
            latency_ms: 1,
        };
        assert_eq!(
            throttle_menu(Some(&custom)).labels().last().unwrap(),
            "Custom — Edited in the Rules window"
        );
    }

    #[test]
    fn add_action_lists_all_23_by_group() {
        let menu = add_action_menu();
        assert_eq!(menu.labels().len(), 9);
        let mut n = 0;
        for e in menu.entries() {
            if let crate::features::context_menu::Entry::Submenu { menu, .. } = e {
                n += menu.labels().len();
            }
        }
        assert_eq!(n, 23);
        assert!(menu.is_enabled("Routing › Serve from recording"));
        assert!(menu.is_enabled("Annotate › Comment on flow"));
    }
}
