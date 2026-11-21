//! The Rules window (G7) driven like a user would, against a real engine
//! (in memory: no data dir, no proxy): the list's menus and drag and drop
//! change the engine's rule book, the form edits, validates (the engine
//! compiles the rule 200 ms after a change) and saves, its pickers apply
//! through their menus, and the toolbar's tools flip the built-in rules.
//!
//! The engine pushes its rule book from its publisher thread, which a gpui
//! test doesn't wait for, so [`sync`] reads the book and sets it.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use pm_core::{Engine, EngineConfig};
use pm_model::{Action, Pattern, ProxyStatus, Rule, RuleOrigin};
use pm_view::rules::{self, edit};
use proxymate_native::bridge;
use proxymate_native::features::rules::form::RuleForm;
use proxymate_native::features::rules::tools::ToolState;
use proxymate_native::features::rules::window::{RulesWindow, Target};
use proxymate_native::features::rules::{self as feature};
use proxymate_native::workspace::Workspace;

fn engine() -> Arc<Engine> {
    Engine::with_config(EngineConfig::default())
}

fn rule(name: &str, path: &str) -> Rule {
    Rule {
        name: name.into(),
        matcher: pm_model::Matcher {
            path: Some(Pattern::exact(path)),
            ..Default::default()
        },
        actions: vec![Action::SetStatus { status: 418 }],
        ..rules::new_rule()
    }
}

struct Harness<'a> {
    engine: Arc<Engine>,
    view: Entity<RulesWindow>,
    cx: &'a mut VisualTestContext,
}

fn open<'a>(cx: &'a mut TestAppContext, target: Target) -> Harness<'a> {
    let engine = engine();
    cx.update(|cx| {
        gpui_kit::init(cx);
        proxymate_native::prefs::init(None, cx);
        proxymate_native::features::register(cx);
        let ws = cx.new(|cx| Workspace::new(Some(engine.clone()), ProxyStatus::default(), cx));
        Workspace::set_global(ws, cx);
    });
    let book = cx.update(|cx| {
        let book = bridge::rule_book(cx);
        book.update(cx, |b, _| b.value = Some(engine.rule_book()));
        book
    });
    let e = engine.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut w = RulesWindow::new(book, Some(e), window, cx);
        w.go(target, window, cx);
        w
    });
    let v = view.clone();
    cx.update(|window, cx| RulesWindow::register(window.window_handle(), &v, cx));
    cx.run_until_parked();
    Harness { engine, view, cx }
}

impl Harness<'_> {
    /// Lets writes land, then pushes the engine's rule book as its
    /// publisher would.
    fn sync(&mut self) {
        self.cx.run_until_parked();
        let book = self.engine.rule_book();
        self.cx.update(|_, cx| {
            bridge::rule_book(cx).update(cx, |b, cx| {
                b.value = Some(book);
                cx.notify();
            });
        });
        self.cx.run_until_parked();
    }

    fn names(&self) -> Vec<String> {
        feature::user_rules(&self.engine.rules())
            .into_iter()
            .map(|r| r.name)
            .collect()
    }

    fn id(&self, name: &str) -> String {
        self.engine.rules().into_iter().find(|r| r.name == name).unwrap().id
    }

    fn run_menu(&mut self, key: &str, label: &str) {
        let view = self.view.clone();
        let ran = self.cx.update(|window, cx| {
            let menu = view.read(cx).menu_for(key, cx).expect("the menu");
            menu.run(label, window, cx)
        });
        assert!(ran, "{key}: {label}");
        self.sync();
    }

    fn form(&mut self) -> Entity<RuleForm> {
        self.cx
            .update(|_, cx| self.view.read(cx).form().cloned())
            .expect("a form")
    }

    /// Past the 200 ms validation delay.
    fn validate(&mut self) {
        self.cx.run_until_parked();
        self.cx.executor().advance_clock(Duration::from_millis(250));
        self.cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn the_rule_menu_reorders_groups_duplicates_and_deletes(cx: &mut TestAppContext) {
    let mut h = open(cx, Target::Keep);
    for (name, path) in [("A", "/a"), ("B", "/b"), ("C", "/c")] {
        h.engine.create_rule(rule(name, path)).unwrap();
    }
    h.sync();
    let (a, c) = (h.id("A"), h.id("C"));
    h.run_menu(&format!("rule:{c}"), "Move to top");
    assert_eq!(h.names(), ["C", "A", "B"]);
    h.run_menu(&format!("rule:{c}"), "Move to bottom");
    assert_eq!(h.names(), ["A", "B", "C"]);
    h.run_menu(&format!("rule:{a}"), "Disable");
    assert!(!h.engine.rule(&a).unwrap().enabled);
    h.run_menu(&format!("rule:{a}"), "Enable");
    assert!(h.engine.rule(&a).unwrap().enabled);

    // Copy name and Copy as JSON copy the whole value.
    h.run_menu(&format!("rule:{a}"), "Copy name");
    assert_eq!(h.cx.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some("A"));

    // New group… makes a group with the rule in it, then renames inline.
    h.run_menu(&format!("rule:{a}"), "Move to group › New group…");
    h.sync();
    let group = h.engine.rule_groups().pop().expect("a group");
    assert_eq!(h.engine.rule(&a).unwrap().group.as_deref(), Some(group.id.as_str()));
    let renaming = h.cx.update(|_, cx| h.view.read(cx).renaming().map(str::to_owned));
    assert_eq!(
        renaming.as_deref(),
        Some(group.id.as_str()),
        "the new group's name is a field"
    );
    let view = h.view.clone();
    let gid = group.id.clone();
    h.cx.update(|window, cx| {
        view.update(cx, |w, cx| {
            w.type_group_name("Ticket 42", window, cx);
            w.finish_rename(&gid, true, window, cx);
        });
    });
    h.sync();
    assert_eq!(h.engine.rule_groups()[0].name, "Ticket 42");
    h.run_menu(&format!("group:{}", group.id), "Turn group off");
    assert!(!h.engine.rule_groups()[0].enabled);
    assert!(!h.engine.rule(&a).unwrap().is_active(&h.engine.rule_groups()));
    h.run_menu(&format!("rule:{a}"), "Move to group › No group");
    assert_eq!(h.engine.rule(&a).unwrap().group, None);
    assert_eq!(h.names(), ["B", "C", "A"], "to the end of the ungrouped rules");
    h.run_menu(&format!("group:{}", group.id), "Delete group, keep its rules");
    assert!(h.engine.rule_groups().is_empty());

    // Duplicate saves "<name> copy" and opens it; Delete removes it.
    h.run_menu(&format!("rule:{c}"), "Duplicate");
    h.sync();
    assert_eq!(h.names(), ["B", "C", "A", "C copy"]);
    let copy = h.id("C copy");
    let selected = h.cx.update(|_, cx| h.view.read(cx).selected().map(str::to_owned));
    assert_eq!(selected.as_deref(), Some(copy.as_str()));
    h.run_menu(&format!("rule:{copy}"), "Delete");
    h.sync();
    assert_eq!(h.names(), ["B", "C", "A"]);
    assert!(
        h.cx.update(|_, cx| h.view.read(cx).form().is_none()),
        "the deleted rule's form closes"
    );
}

#[gpui_kit::test]
fn dragging_a_rule_reorders_and_moves_it_between_sections(cx: &mut TestAppContext) {
    let mut h = open(cx, Target::Keep);
    for (name, path) in [("A", "/a"), ("B", "/b"), ("C", "/c")] {
        h.engine.create_rule(rule(name, path)).unwrap();
    }
    let group = h.engine.create_rule_group("Ticket", true, RuleOrigin::User).unwrap();
    h.sync();
    let (a, c) = (h.id("A"), h.id("C"));
    h.cx.update(|window, cx| window.drag_to(format!("rule-row-{c}"), format!("rule-row-{a}"), cx));
    h.sync();
    assert_eq!(h.names(), ["C", "A", "B"], "C takes A's place");
    h.cx.update(|window, cx| {
        window.drag_to(format!("rule-row-{a}"), format!("group-header-{}", group.id), cx);
    });
    h.sync();
    assert_eq!(h.engine.rule(&a).unwrap().group.as_deref(), Some(group.id.as_str()));
    h.cx.update(|window, cx| window.drag_to(format!("rule-row-{a}"), "rule-section-ungrouped", cx));
    h.sync();
    assert_eq!(h.engine.rule(&a).unwrap().group, None, "back to the ungrouped rules");
}

#[gpui_kit::test]
fn the_form_validates_saves_and_its_pickers_apply(cx: &mut TestAppContext) {
    let mut h = open(cx, Target::Draft(Box::new(rules::new_rule())));
    let form = h.form();
    let view = h.view.clone();
    h.cx.update(|window, cx| {
        form.update(cx, |f, cx| {
            f.type_text("name", "Mock users", window, cx);
            f.choose("m.method", "GET", window, cx);
            f.type_text("m.path", "/v1/(users", window, cx);
            f.choose("m.path.kind", "regex", window, cx);
        });
    });
    h.validate();
    let invalid = h.cx.update(|_, cx| form.read(cx).invalid().map(str::to_owned));
    assert!(invalid.is_some(), "an unclosed group doesn't compile");
    assert!(!h.cx.update(|_, cx| form.read(cx).can_save()));
    h.cx.update(|window, cx| form.update(cx, |f, cx| f.type_text("m.path", "^/v1/users", window, cx)));
    // "Add action" ▸ Map Local, through the action its menu dispatches.
    let add =
        h.cx.update(|_, cx| form.read(cx).picker_menu("add-action", cx).unwrap());
    h.cx.update(|window, cx| assert!(add.run("Routing › Map Local", window, cx)));
    h.cx.run_until_parked();
    // A picker's menu: Answer from ▸ A file, then its path.
    let source =
        h.cx.update(|_, cx| form.read(cx).picker_menu("a.0.source", cx).unwrap());
    h.cx.update(|window, cx| assert!(source.run("A file", window, cx)));
    h.cx.run_until_parked();
    h.cx.update(|window, cx| form.update(cx, |f, cx| f.type_text("a.0.file", "/tmp/users.json", window, cx)));
    // The field is on screen, bound to its key.
    assert!(h.cx.update(|window, _| window.try_find("field-a.0.file").is_some()));
    h.validate();
    let rule = h.cx.update(|_, cx| form.read(cx).rule().clone());
    // The glob's default case-insensitivity stays when the kind changes.
    assert_eq!(rules::matcher_summary(&rule.matcher), "GET * /^/v1/users/i");
    assert_eq!(
        rules::actions_summary(&rule, &Default::default(), &Default::default()),
        "Map Local /tmp/users.json"
    );
    assert!(h.cx.update(|_, cx| form.read(cx).invalid().is_none()));

    // JSON that doesn't parse blocks Save until it does.
    let add =
        h.cx.update(|_, cx| form.read(cx).picker_menu("add-action", cx).unwrap());
    h.cx.update(|window, cx| assert!(add.run("Body › JSON set", window, cx)));
    h.cx.run_until_parked();
    h.cx.update(|window, cx| form.update(cx, |f, cx| f.type_text("a.1.raw", "{oops", window, cx)));
    h.validate();
    assert!(h.cx.update(|_, cx| form.read(cx).field_error("a.1.raw").is_some()));
    assert!(!h.cx.update(|_, cx| form.read(cx).can_save()));
    h.cx.update(|window, cx| form.update(cx, |f, cx| f.type_text("a.1.raw", "1.50", window, cx)));
    h.validate();
    assert!(h.cx.update(|_, cx| form.read(cx).can_save()));

    h.cx.update(|_, cx| form.update(cx, |f, cx| f.save(cx)));
    h.sync();
    let saved = h
        .engine
        .rules()
        .into_iter()
        .find(|r| r.name == "Mock users")
        .expect("saved");
    assert_eq!(
        saved.actions[1],
        Action::JsonSet {
            phase: pm_model::Phase::Response,
            pointer: "/".into(),
            value: serde_json::Value::Null,
            raw: Some("1.50".into())
        }
    );
    let (selected, draft) = h.cx.update(|_, cx| {
        let v = view.read(cx);
        (v.selected().map(str::to_owned), v.is_draft())
    });
    assert_eq!(selected.as_deref(), Some(saved.id.as_str()));
    assert!(!draft);
    assert!(!h.cx.update(|_, cx| form.read(cx).is_unsaved()));

    // An action moved down keeps its fields with it.
    h.cx.update(|window, cx| {
        form.update(cx, |f, cx| {
            f.edit_structure(window, cx, |r| edit::move_action(r, 0, true))
        })
    });
    h.cx.run_until_parked();
    let text = h.cx.update(|_, cx| edit::text(form.read(cx).rule(), "a.1.file"));
    assert_eq!(text.as_deref(), Some("/tmp/users.json"));
    assert!(h.cx.update(|_, cx| form.read(cx).is_unsaved()));
}

#[gpui_kit::test]
fn the_toolbar_tools_flip_the_builtin_rules(cx: &mut TestAppContext) {
    let mut h = open(cx, Target::Keep);
    let state = |h: &mut Harness| {
        let book = h.engine.rule_book();
        ToolState::new(Some(&book), None)
    };
    assert_eq!(state(&mut h), ToolState::default());
    h.cx.dispatch_action(feature::ToggleNoCache);
    h.cx.dispatch_action(feature::ToggleNoCookies);
    h.sync();
    h.cx.dispatch_action(feature::SetThrottle {
        preset: Some("3G".into()),
    });
    h.sync();
    let s = state(&mut h);
    assert!(s.no_cache && s.no_cookies);
    assert_eq!(s.throttle.map(|p| p.name).as_deref(), Some("3G"));
    h.cx.dispatch_action(feature::ToggleNoCache);
    h.cx.dispatch_action(feature::SetThrottle { preset: None });
    h.sync();
    let s = state(&mut h);
    assert!(!s.no_cache && s.no_cookies && s.throttle.is_none());
    // The built-in rules aren't listed.
    assert!(h.names().is_empty());
    assert_eq!(
        ToolState {
            paused: 2,
            ..ToolState::default()
        }
        .breakpoints_tip(),
        "2 paused at a breakpoint"
    );
    assert_eq!(ToolState::default().breakpoints_tip(), "Breakpoints · none paused");
}
