//! `rules.test.ts`, plus the field editor.

use pm_model::{
    Action, KvMatch, Matcher, PatternKind, Phase, PresetKind, Rule, RuleGroup, RuleOrigin, ServeMiss, ServePick,
    StatusRange, WsDirection, WsOp,
};

use super::edit::{self, choose, chosen, set_text, text};
use super::*;

fn names(pairs: &[(&str, &str)]) -> Names {
    pairs.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
}

#[test]
fn builds_a_default_for_every_action_type() {
    assert_eq!(ACTION_TYPES.len(), 23);
    for t in ACTION_TYPES {
        let action = default_action(t.id).unwrap_or_else(|| panic!("{}", t.id));
        assert_eq!(action_type(&action), t.id);
        assert!(!action_summary(&action, &Names::new(), &Names::new()).is_empty());
        assert_eq!(action_label(t.id), t.label);
    }
    assert!(default_action("Nope").is_none());
    assert_eq!(
        action_groups(),
        [
            "Headers",
            "Query",
            "Body",
            "Status",
            "Routing",
            "Flow control",
            "Scripts",
            "WebSocket",
            "Annotate"
        ]
    );
}

#[test]
fn builds_and_summarises_every_websocket_message_op() {
    for (kind, _) in WS_OPS {
        let op = default_ws_op(kind).unwrap();
        assert_eq!(ws_op_kind(&op), kind);
        let a = Action::WsMessage {
            direction: Some(WsDirection::Received),
            filter: None,
            op,
        };
        assert!(action_summary(&a, &Names::new(), &Names::new()).starts_with("received "));
    }
    let drop = Action::WsMessage {
        direction: None,
        filter: Some(new_pattern(PatternKind::Exact, "ping")),
        op: WsOp::Drop,
    };
    assert_eq!(action_summary(&drop, &Names::new(), &Names::new()), "ws = ping drop");
}

#[test]
fn summarises_a_serve_from_with_its_recordings_name() {
    let mocks = names(&[("s1", "checkout")]);
    let serve = |pick, on_miss, source: &str| Action::ServeFrom {
        source: source.into(),
        key: pm_model::ServeKey::default(),
        on_miss,
        pick,
    };
    let none = Names::new();
    assert_eq!(
        action_summary(&serve(ServePick::First, ServeMiss::PassThrough, "s1"), &none, &mocks),
        "serve from checkout"
    );
    assert_eq!(
        action_summary(&serve(ServePick::RoundRobin, ServeMiss::NotFound, "s1"), &none, &mocks),
        "serve from checkout (round-robin), else 404"
    );
    assert_eq!(
        action_summary(&serve(ServePick::First, ServeMiss::BadGateway, "s1"), &none, &none),
        "serve from (missing recording), else 502"
    );
    let Some(Action::ServeFrom { key, on_miss, pick, .. }) = default_action("ServeFrom") else {
        panic!()
    };
    assert!(key.query && !key.body);
    assert_eq!((on_miss, pick), (ServeMiss::PassThrough, ServePick::First));
}

#[test]
fn summarises_a_comment_and_a_script() {
    assert_eq!(default_action("Comment"), Some(Action::Comment { text: String::new() }));
    let none = Names::new();
    assert_eq!(
        action_summary(
            &Action::Comment {
                text: " Config request ".into()
            },
            &none,
            &none
        ),
        "comment “Config request”"
    );
    let script = Action::Script { script_id: "x1".into() };
    assert_eq!(action_summary(&script, &names(&[("x1", "b64")]), &none), "script b64");
    assert_eq!(action_summary(&script, &none, &none), "script x1");
    assert_eq!(
        action_summary(&Action::Script { script_id: String::new() }, &none, &none),
        "script (none)"
    );
}

#[test]
fn labels_patterns_and_summarises_matchers() {
    assert_eq!(pattern_label(None), "any");
    let mut re = new_pattern(PatternKind::Regex, "^/a");
    re.case_insensitive = true;
    assert_eq!(pattern_label(Some(&re)), "/^/a/i");
    assert_eq!(pattern_label(Some(&new_pattern(PatternKind::Exact, "x"))), "= x");
    let m = Matcher {
        methods: vec!["GET".into(), "POST".into()],
        host: Some(new_pattern(PatternKind::Glob, "*.example.com")),
        path: Some(new_pattern(PatternKind::Exact, "/v1/users")),
        headers: vec![KvMatch {
            name: "x-a".into(),
            value: None,
            negate: false,
        }],
        ..Matcher::default()
    };
    assert_eq!(matcher_summary(&m), "GET|POST *.example.com (Aa) /v1/users +1");
    assert_eq!(matcher_summary(&Matcher::default()), "*");
    let mut rule = new_rule();
    assert_eq!(actions_summary(&rule, &Names::new(), &Names::new()), "no actions");
    rule.actions = vec![
        Action::SetStatus { status: 404 },
        Action::Block {
            mode: pm_model::BlockMode::Reset,
        },
    ];
    assert_eq!(
        actions_summary(&rule, &Names::new(), &Names::new()),
        "status 404 · block (reset)"
    );
}

#[test]
fn labels_origins_with_the_ai_badge_for_agents() {
    assert_eq!(origin_label(&RuleOrigin::User), None);
    assert_eq!(
        origin_label(&RuleOrigin::Preset {
            preset: PresetKind::MapLocal
        })
        .as_deref(),
        Some("Map Local")
    );
    let agent = RuleOrigin::Agent {
        client: "claude-code".into(),
        session: None,
        created_at_ms: 0.,
    };
    assert_eq!(origin_label(&agent).as_deref(), Some("AI · claude-code"));
    assert_eq!(
        origin_tip(&agent).as_deref(),
        Some("Created by the MCP client claude-code")
    );
}

#[test]
fn parses_and_formats_status_ranges() {
    let r = |from, to| StatusRange { from, to };
    assert_eq!(
        parse_status_ranges("2xx, 404, 500-599"),
        Some(vec![r(200, 299), r(404, 404), r(500, 599)])
    );
    assert_eq!(parse_status_ranges(""), Some(vec![]));
    assert_eq!(parse_status_ranges("abc"), None);
    assert_eq!(parse_status_ranges("500-400"), None);
    assert_eq!(parse_status_ranges("40"), None);
    assert_eq!(
        format_status_ranges(&[r(200, 299), r(404, 404), r(500, 503)]),
        "2xx, 404, 500-503"
    );
}

#[test]
fn switches_the_global_throttle_between_presets_and_off() {
    let mut rule = Rule {
        id: THROTTLE_RULE_ID.into(),
        enabled: false,
        actions: vec![default_action("Throttle").unwrap()],
        ..new_rule()
    };
    assert!(is_builtin(&rule.id));
    assert_eq!(active_throttle(Some(&rule)), None);
    let edge = throttle_preset("Edge").unwrap();
    let patch = throttle_patch(Some(&edge));
    rule.enabled = patch.enabled.unwrap();
    rule.actions = patch.actions.unwrap();
    assert_eq!(active_throttle(Some(&rule)).unwrap().name, "Edge");
    let off = throttle_patch(None);
    assert_eq!(off.enabled, Some(false));
    assert!(off.actions.is_none(), "off keeps the speeds");
    rule.actions = vec![Action::Throttle {
        download_kbps: 1,
        upload_kbps: 2,
        latency_ms: 3,
    }];
    assert_eq!(active_throttle(Some(&rule)).unwrap().name, "Custom");
    assert_eq!(
        throttle_detail(&throttle_preset("LTE").unwrap()),
        "12 Mbps ↓ · 6 Mbps ↑ · 40 ms"
    );
    assert_eq!(kbps(1600), "1.6 Mbps");
}

#[test]
fn moves_list_items() {
    let mut v = vec!["a", "b", "c"];
    move_item(&mut v, 0, 2);
    assert_eq!(v, ["b", "c", "a"]);
    let mut v = vec!["a", "b", "c"];
    move_item(&mut v, 2, 0);
    assert_eq!(v, ["c", "a", "b"]);
}

fn rule(id: &str, group: Option<&str>, enabled: bool) -> Rule {
    Rule {
        id: id.into(),
        name: id.into(),
        enabled,
        group: group.map(Into::into),
        ..new_rule()
    }
}

fn group(id: &str) -> RuleGroup {
    RuleGroup {
        id: id.into(),
        name: id.into(),
        enabled: true,
        order: 0,
        origin: RuleOrigin::User,
    }
}

#[test]
fn the_list_orders_sections_and_drops() {
    let groups = [group("g1"), group("g2")];
    let rules = [
        rule("a", None, true),
        rule("b", Some("gone"), true),
        rule("c", Some("g1"), true),
        rule("d", Some("g1"), false),
    ];
    let ids = |v: Vec<&Rule>| v.into_iter().map(|r| r.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(members(&rules, &groups, None)), ["a", "b"], "an unknown group is no group");
    let g1 = members(&rules, &groups, Some("g1"));
    assert_eq!(ids(g1.clone()), ["c", "d"]);
    assert_eq!(group_count(&g1), "1/2");
    assert_eq!(group_count(&members(&rules, &groups, Some("g2"))), "0");
    assert_eq!(order_after_drop(&rules, "d", "a").unwrap(), ["d", "a", "b", "c"]);
    assert_eq!(order_after_drop(&rules, "a", "c").unwrap(), ["b", "c", "a", "d"]);
    assert_eq!(order_after_drop(&rules, "a", "a"), None);
    assert_eq!(order_to_edge(&rules, "c", true), ["c", "a", "b", "d"]);
    assert_eq!(order_to_edge(&rules, "a", false), ["b", "c", "d", "a"]);
    assert_eq!(group_order_after_move(&groups, "g1", true).unwrap(), ["g2", "g1"]);
    assert_eq!(group_order_after_move(&groups, "g1", false), None);
    assert_eq!(group_order_after_drop(&groups, "g2", "g1").unwrap(), ["g2", "g1"]);
    let copy = duplicate(&Rule {
        origin: RuleOrigin::Preset {
            preset: PresetKind::MapLocal,
        },
        ..rules[0].clone()
    });
    assert_eq!((copy.id.as_str(), copy.name.as_str()), ("", "a copy"));
    assert_eq!(copy.origin, RuleOrigin::User);
    assert!(rule_json(&rules[0]).contains("\"id\": \"a\""));
}

#[test]
fn the_editor_reads_and_writes_matcher_fields() {
    let mut r = new_rule();
    set_text(&mut r, "name", "Mock users").unwrap();
    set_text(&mut r, "m.host", "api.example.com").unwrap();
    assert_eq!(r.matcher.host.as_ref().unwrap().kind, PatternKind::Glob);
    choose(&mut r, "m.host.kind", "exact").unwrap();
    choose(&mut r, "m.host.ci", "false").unwrap();
    assert_eq!(
        r.matcher.host,
        Some(pm_model::Pattern::exact("api.example.com")),
        "kind and case switch on the value kept"
    );
    assert_eq!(chosen(&r, "m.host.kind").as_deref(), Some("exact"));
    assert_eq!(chosen(&r, "m.path.kind").as_deref(), Some("glob"), "an empty pattern shows a glob");
    set_text(&mut r, "m.host", "").unwrap();
    assert_eq!(r.matcher.host, None, "empty is any host");
    set_text(&mut r, "m.port", "8443").unwrap();
    assert!(set_text(&mut r, "m.port", "99999").is_err());
    assert_eq!(r.matcher.port, Some(8443));
    assert_eq!(text(&r, "m.port").as_deref(), Some("8443"));
    set_text(&mut r, "m.port", "").unwrap();
    assert_eq!(r.matcher.port, None);
    choose(&mut r, "m.method", "get").unwrap();
    choose(&mut r, "m.method", "POST").unwrap();
    choose(&mut r, "m.method", "GET").unwrap();
    assert_eq!(r.matcher.methods, ["POST"]);
    choose(&mut r, "m.scheme", "https").unwrap();
    assert_eq!(chosen(&r, "m.scheme").as_deref(), Some("https"));
    assert!(choose(&mut r, "m.scheme", "ftp").is_err());

    assert!(edit::add_row(&mut r, "headers"));
    set_text(&mut r, "m.headers.0.name", "x-env").unwrap();
    set_text(&mut r, "m.headers.0.value", "staging").unwrap();
    choose(&mut r, "m.headers.0.negate", "true").unwrap();
    assert_eq!(
        r.matcher.headers,
        [KvMatch {
            name: "x-env".into(),
            value: Some(new_pattern(PatternKind::Glob, "staging")),
            negate: true
        }]
    );
    assert_eq!(text(&r, "m.headers.0.value").as_deref(), Some("staging"));
    assert!(edit::remove_row(&mut r, "headers", 0));
    assert!(text(&r, "m.headers.0.name").is_none());

    assert!(set_text(&mut r, "m.status", "2xx").is_err(), "no response matcher yet");
    choose(&mut r, "m.response", "true").unwrap();
    set_text(&mut r, "m.status", "2xx, 404").unwrap();
    assert!(set_text(&mut r, "m.status", "2x").is_err());
    set_text(&mut r, "m.ctype", "application/json*").unwrap();
    assert_eq!(text(&r, "m.status").as_deref(), Some("2xx, 404"));
    assert_eq!(text(&r, "m.ctype").as_deref(), Some("application/json*"));
    choose(&mut r, "m.response", "false").unwrap();
    assert_eq!(r.matcher.response, None);
    assert!(set_text(&mut r, "m.nope", "x").is_err());
}

#[test]
fn the_editor_reads_and_writes_every_actions_fields() {
    let mut r = new_rule();
    for t in ACTION_TYPES {
        assert!(edit::add_action(&mut r, t.id));
    }
    let at = |t: &str| ACTION_TYPES.iter().position(|a| a.id == t).unwrap();
    let key = |t: &str, f: &str| format!("a.{}.{f}", at(t));
    // Every text field round-trips.
    let fields = [
        ("SetHeader", "name", "X-A"),
        ("SetHeader", "value", "1"),
        ("AddHeader", "value", "2"),
        ("RemoveHeader", "name", "cookie"),
        ("ReplaceHeaderValue", "pattern", "a(.)"),
        ("ReplaceHeaderValue", "replacement", "$1"),
        ("SetQuery", "name", "q"),
        ("RemoveQuery", "name", "debug"),
        ("SetBody", "body", "{}"),
        ("SetBody", "ctype", "application/json"),
        ("ReplaceBody", "pattern", "old"),
        ("JsonSet", "pointer", "/a"),
        ("JsonSet", "raw", "12345678901234567890"),
        ("JsonRemove", "pointer", "/secret"),
        ("FormSet", "value", "v"),
        ("FormRemove", "name", "f"),
        ("SetStatus", "status", "418"),
        ("MapLocal", "inline", "{\"ok\":true}"),
        ("MapLocal", "status", "201"),
        ("MapLocal", "ctype", "text/plain"),
        ("MapRemote", "host", "staging.example.com"),
        ("MapRemote", "port", "8080"),
        ("MapRemote", "path", "/v2"),
        ("Delay", "ms", "250"),
        ("Throttle", "down", "100"),
        ("Throttle", "up", "50"),
        ("Throttle", "latency", "20"),
        ("Block", "status", "451"),
        ("WsMessage", "pattern", "ping"),
        ("WsMessage", "replacement", "pong"),
        ("WsMessage", "filter", "type"),
        ("Comment", "text", "Config request"),
    ];
    for (t, f, v) in fields {
        let k = key(t, f);
        set_text(&mut r, &k, v).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert_eq!(text(&r, &k).as_deref(), Some(v), "{k}");
    }
    assert_eq!(
        r.actions[at("JsonSet")],
        Action::JsonSet {
            phase: Phase::Response,
            pointer: "/a".into(),
            value: serde_json::Value::Null,
            raw: Some("12345678901234567890".into()),
        },
        "the exact text is kept"
    );
    assert!(set_text(&mut r, &key("JsonSet", "raw"), "{oops").unwrap_err().starts_with("Not JSON yet"));
    assert!(set_text(&mut r, &key("SetStatus", "status"), "abc").is_err());
    assert!(set_text(&mut r, &key("SetStatus", "nope"), "1").is_err());
    set_text(&mut r, &key("MapRemote", "host"), "").unwrap();
    assert_eq!(text(&r, &key("MapRemote", "host")).as_deref(), Some(""));

    // Pickers and toggles.
    let picks = [
        ("SetHeader", "phase", "request"),
        ("MapLocal", "source", "file"),
        ("MapRemote", "scheme", "https"),
        ("MapRemote", "preserve_host", "true"),
        ("Breakpoint", "response", "true"),
        ("Breakpoint", "request", "false"),
        ("Block", "mode", "reset"),
        ("Script", "script", "s1"),
        ("WsMessage", "direction", "sent"),
        ("ServeFrom", "serve", "m1"),
        ("ServeFrom", "body", "true"),
        ("ServeFrom", "query", "false"),
        ("ServeFrom", "pick", "round_robin"),
        ("ServeFrom", "miss", "bad_gateway"),
    ];
    for (t, f, v) in picks {
        let k = key(t, f);
        choose(&mut r, &k, v).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert_eq!(chosen(&r, &k).as_deref(), Some(v), "{k}");
    }
    assert!(choose(&mut r, &key("SetStatus", "phase"), "request").is_err());
    set_text(&mut r, &key("MapLocal", "file"), "/tmp/a.json").unwrap();
    assert_eq!(
        r.actions[at("MapLocal")],
        Action::MapLocal {
            source: pm_model::MapLocalSource::File {
                path: "/tmp/a.json".into()
            },
            status: 201,
            headers: vec![],
            content_type: Some("text/plain".into()),
        }
    );
    choose(&mut r, &key("WsMessage", "op"), "json_set").unwrap();
    set_text(&mut r, &key("WsMessage", "raw"), "{\"a\": 1.50}").unwrap();
    assert_eq!(text(&r, &key("WsMessage", "raw")).as_deref(), Some("{\"a\": 1.50}"));
    choose(&mut r, &key("WsMessage", "op"), "delay").unwrap();
    set_text(&mut r, &key("WsMessage", "ms"), "75").unwrap();
    assert!(matches!(
        r.actions[at("WsMessage")],
        Action::WsMessage {
            op: WsOp::Delay { ms: 75 },
            ..
        }
    ));
    choose(&mut r, &key("ReplaceBody", "pattern.kind"), "regex").unwrap();
    choose(&mut r, &key("ReplaceBody", "pattern.ci"), "true").unwrap();
    assert_eq!(chosen(&r, &key("ReplaceBody", "pattern.kind")).as_deref(), Some("regex"));

    // Structure.
    let n = r.actions.len();
    assert!(edit::move_action(&mut r, 0, true));
    assert_eq!(action_type(&r.actions[1]), "SetHeader");
    assert!(!edit::move_action(&mut r, 0, false));
    assert!(!edit::move_action(&mut r, n - 1, true));
    assert!(edit::remove_action(&mut r, 0));
    assert_eq!(r.actions.len(), n - 1);
    assert!(!edit::add_action(&mut r, "Nope"));
}
