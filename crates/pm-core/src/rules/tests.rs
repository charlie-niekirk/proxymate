//! The rules matrix: compilation errors, every matcher condition, every
//! action, and ordering and precedence between rules.

use std::sync::Arc;

use http::HeaderMap;
use http::header::HeaderValue;
use pm_model::{
    Action, BlockMode, KvMatch, MapLocalSource, Matcher, Pattern, PatternKind, Phase, ResponseMatcher, Rule,
    RuleOrigin, Scheme, StatusRange, WsDirection, WsOp,
};
use serde_json::json;

use super::RuleError;
use super::compile::{CompiledRule, CompiledRuleSet};
use super::eval::{self, RequestDraft, ResponseDraft};
use crate::mocks::MockMap;
use crate::scripts::{ScriptMap, ScriptRun};
use futures_util::FutureExt;

fn rule(name: &str, matcher: Matcher, actions: Vec<Action>) -> Rule {
    Rule {
        id: name.into(),
        name: name.into(),
        enabled: true,
        order: 0,
        origin: RuleOrigin::User,
        matcher,
        actions,
    }
}

fn ordered(mut rules: Vec<Rule>) -> Vec<Rule> {
    for (i, r) in rules.iter_mut().enumerate() {
        r.order = i as i64;
    }
    rules
}

fn request(url: &str) -> RequestDraft {
    let uri: http::Uri = url.parse().unwrap();
    let scheme = if uri.scheme_str() == Some("https") {
        Scheme::Https
    } else {
        Scheme::Http
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        "host",
        HeaderValue::from_str(uri.authority().unwrap().as_str()).unwrap(),
    );
    headers.insert("accept", HeaderValue::from_static("application/json"));
    RequestDraft {
        method: "GET".into(),
        scheme,
        host: uri.host().unwrap().into(),
        port: uri.port_u16().unwrap_or(if scheme == Scheme::Https { 443 } else { 80 }),
        path: uri.path().into(),
        query: uri.query().unwrap_or_default().into(),
        headers,
        body: None,
    }
}

fn response(status: u16, content_type: &str, body: &str) -> ResponseDraft {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_str(content_type).unwrap());
    ResponseDraft {
        status,
        headers,
        body: Some(body.as_bytes().to_vec()),
    }
}

fn set(rules: Vec<Rule>) -> CompiledRuleSet {
    CompiledRuleSet::compile(&ordered(rules), &ScriptMap::new(), &MockMap::new()).expect("rules compile")
}

/// The names of the rules in `set` that match `req`.
fn matching(set: &CompiledRuleSet, req: &RequestDraft, app: Option<&str>) -> Vec<String> {
    set.matching(&req.info(app))
        .iter()
        .map(|r| r.applied.name.clone())
        .collect()
}

fn host(pattern: Pattern) -> Matcher {
    Matcher {
        host: Some(pattern),
        ..Matcher::default()
    }
}

fn header(draft: &HeaderMap, name: &str) -> Option<String> {
    draft.get(name).map(|v| v.to_str().unwrap().to_owned())
}

// ---- Compilation ----

fn compile_error(matcher: Matcher) -> String {
    match CompiledRule::compile(&rule("bad", matcher, Vec::new()), &ScriptMap::new(), &MockMap::new()) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected a compile error"),
    }
}

#[test]
fn invalid_regexes_are_rejected_with_the_field_and_reason() {
    let err = compile_error(host(Pattern::regex("api.(example")));
    assert!(err.starts_with("rule \"bad\", host: invalid regex"), "{err}");
    assert!(err.contains("unclosed group"), "{err}");
}

#[test]
fn oversized_regexes_are_rejected() {
    let err = compile_error(Matcher {
        path: Some(Pattern::regex(r"(\w{100}){100}")),
        ..Matcher::default()
    });
    assert!(err.contains("path: the regex is too complex"), "{err}");

    let err = compile_error(host(Pattern::regex("a".repeat(super::PATTERN_MAX_LEN + 1))));
    assert!(err.contains("characters long"), "{err}");
}

#[test]
fn invalid_globs_and_actions_are_rejected() {
    assert!(compile_error(host(Pattern::glob("[a-"))).contains("invalid glob"));
    let bad = |action: Action| {
        CompiledRule::compile(
            &rule("x", Matcher::default(), vec![action]),
            &ScriptMap::new(),
            &MockMap::new(),
        )
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default()
    };
    assert!(bad(Action::SetStatus { status: 42 }).contains("action 1: 42 isn't a valid status"));
    assert!(
        bad(Action::SetHeader {
            phase: Phase::Request,
            name: "bad header".into(),
            value: "x".into()
        })
        .contains("isn't a valid header name")
    );
    assert!(
        bad(Action::JsonRemove {
            phase: Phase::Response,
            pointer: "name".into()
        })
        .contains("must start with /")
    );
    assert!(
        bad(Action::MapRemote {
            scheme: None,
            host: None,
            port: None,
            path: Some("no-slash".into()),
            preserve_host: false
        })
        .contains("must start with /")
    );
}

#[test]
fn disabled_rules_are_validated_but_never_match() {
    let mut broken = rule("broken", host(Pattern::regex("(")), Vec::new());
    broken.enabled = false;
    assert!(matches!(
        CompiledRuleSet::compile(&[broken], &ScriptMap::new(), &MockMap::new()),
        Err(RuleError::Invalid { .. })
    ));

    let mut off = rule("off", Matcher::default(), Vec::new());
    off.enabled = false;
    let rules = set(vec![off, rule("on", Matcher::default(), Vec::new())]);
    assert_eq!(matching(&rules, &request("http://a.test/"), None), ["on"]);
}

// ---- Matching ----

#[test]
fn host_patterns() {
    let rules = set(vec![
        rule("exact", host(Pattern::exact("api.example.com")), Vec::new()),
        rule("glob", host(Pattern::glob("*.example.com")), Vec::new()),
        rule("regex", host(Pattern::regex(r"^cdn\d+\.")), Vec::new()),
        rule(
            "case",
            host(Pattern {
                kind: PatternKind::Exact,
                value: "API.EXAMPLE.COM".into(),
                case_insensitive: true,
            }),
            Vec::new(),
        ),
        rule("any", Matcher::default(), Vec::new()),
    ]);
    assert_eq!(
        matching(&rules, &request("http://api.example.com/"), None),
        ["exact", "glob", "case", "any"]
    );
    // `*.example.com` also matches the apex domain.
    assert_eq!(matching(&rules, &request("http://example.com/"), None), ["glob", "any"]);
    assert_eq!(matching(&rules, &request("http://cdn42.test/"), None), ["regex", "any"]);
    assert_eq!(matching(&rules, &request("http://notexample.com/"), None), ["any"]);
    // Exact is case-sensitive unless asked.
    let strict = set(vec![rule(
        "strict",
        host(Pattern::exact("API.example.com")),
        Vec::new(),
    )]);
    assert!(matching(&strict, &request("http://api.example.com/"), None).is_empty());
}

#[test]
fn method_scheme_port_and_path() {
    let m = Matcher {
        methods: vec!["post".into(), "PUT".into()],
        scheme: Some(Scheme::Https),
        port: Some(8443),
        path: Some(Pattern::glob("/api/*/users")),
        ..Matcher::default()
    };
    let rules = set(vec![rule("r", m, Vec::new())]);
    let mut req = request("https://a.test:8443/api/v1/users");
    assert!(matching(&rules, &req, None).is_empty(), "GET isn't allowed");
    req.method = "POST".into();
    assert_eq!(matching(&rules, &req, None), ["r"]);
    // Globs cross `/`.
    req.path = "/api/v1/beta/users".into();
    assert_eq!(matching(&rules, &req, None), ["r"]);
    req.path = "/api/v1/users/1".into();
    assert!(matching(&rules, &req, None).is_empty());
    let mut other = request("http://a.test:8443/api/v1/users");
    other.method = "PUT".into();
    assert!(matching(&rules, &other, None).is_empty(), "wrong scheme");
    let mut other = request("https://a.test/api/v1/users");
    other.method = "PUT".into();
    assert!(matching(&rules, &other, None).is_empty(), "wrong port");
}

#[test]
fn query_and_header_conditions_with_negation() {
    let m = Matcher {
        query: vec![
            KvMatch {
                name: "page".into(),
                value: Some(Pattern::regex(r"^\d+$")),
                negate: false,
            },
            KvMatch {
                name: "debug".into(),
                value: None,
                negate: true,
            },
        ],
        headers: vec![KvMatch {
            name: "Accept".into(),
            value: Some(Pattern::glob("*json*")),
            negate: false,
        }],
        ..Matcher::default()
    };
    let rules = set(vec![rule("r", m, Vec::new())]);
    assert_eq!(matching(&rules, &request("http://a.test/?page=2"), None), ["r"]);
    assert!(matching(&rules, &request("http://a.test/?page=two"), None).is_empty());
    assert!(matching(&rules, &request("http://a.test/?page=2&debug"), None).is_empty());
    assert!(matching(&rules, &request("http://a.test/"), None).is_empty());
    let mut req = request("http://a.test/?page=2");
    req.headers.insert("accept", HeaderValue::from_static("text/html"));
    assert!(matching(&rules, &req, None).is_empty());

    // Percent-encoded names and values are decoded first.
    let m = Matcher {
        query: vec![KvMatch {
            name: "q x".into(),
            value: Some(Pattern::exact("a&b")),
            negate: false,
        }],
        ..Matcher::default()
    };
    let rules = set(vec![rule("r", m, Vec::new())]);
    assert_eq!(matching(&rules, &request("http://a.test/?q%20x=a%26b"), None), ["r"]);
}

#[test]
fn client_app_condition() {
    let m = Matcher {
        client_app: Some(Pattern::glob("safari*")),
        ..Matcher::default()
    };
    let rules = set(vec![rule("r", m, Vec::new())]);
    assert!(rules.needs_client_app());
    let req = request("http://a.test/");
    assert_eq!(matching(&rules, &req, Some("Safari")), ["r"]);
    assert!(matching(&rules, &req, Some("curl")).is_empty());
    assert!(matching(&rules, &req, None).is_empty());
}

#[test]
fn mismatch_explains_itself() {
    let r = CompiledRule::compile(
        &rule("r", host(Pattern::exact("a.test")), Vec::new()),
        &ScriptMap::new(),
        &MockMap::new(),
    )
    .unwrap();
    let req = request("http://b.test/");
    assert_eq!(
        r.mismatch(&req.info(None), true).as_deref(),
        Some("host b.test doesn't match")
    );
}

#[test]
fn rules_match_in_order_regardless_of_the_host_prefilter() {
    let rules = set(vec![
        rule("first", Matcher::default(), Vec::new()),
        rule("second", host(Pattern::exact("a.test")), Vec::new()),
        rule("third", Matcher::default(), Vec::new()),
        rule("fourth", host(Pattern::glob("*.test")), Vec::new()),
    ]);
    assert_eq!(
        matching(&rules, &request("http://a.test/"), None),
        ["first", "second", "third", "fourth"]
    );
}

// ---- Request phase ----

fn run_request(rules: Vec<Rule>, draft: &mut RequestDraft) -> eval::RequestEffects {
    let set = set(rules);
    let matched = set.matching(&draft.info(None));
    eval::request_phase(&matched, draft, ScriptRun::none())
        .now_or_never()
        .expect("a rule set without scripts never awaits")
}

fn any(actions: Vec<Action>) -> Rule {
    rule("any", Matcher::default(), actions)
}

#[test]
fn header_actions() {
    let mut req = request("http://a.test/");
    req.headers.append("x-multi", HeaderValue::from_static("one"));
    req.headers.append("x-multi", HeaderValue::from_static("two"));
    req.headers
        .insert("authorization", HeaderValue::from_static("Bearer secret-123"));
    let fx = run_request(
        vec![any(vec![
            Action::SetHeader {
                phase: Phase::Request,
                name: "X-Multi".into(),
                value: "only".into(),
            },
            Action::RemoveHeader {
                phase: Phase::Request,
                name: "accept".into(),
            },
            Action::ReplaceHeaderValue {
                phase: Phase::Request,
                name: "authorization".into(),
                pattern: Pattern::regex(r"secret-(\d+)"),
                replacement: "token-$1".into(),
            },
            // Response-phase actions don't touch the request.
            Action::SetHeader {
                phase: Phase::Response,
                name: "x-response".into(),
                value: "1".into(),
            },
        ])],
        &mut req,
    );
    assert!(fx.head_changed);
    assert_eq!(req.headers.get_all("x-multi").iter().count(), 1);
    assert_eq!(header(&req.headers, "x-multi").as_deref(), Some("only"));
    assert_eq!(header(&req.headers, "accept"), None);
    assert_eq!(
        header(&req.headers, "authorization").as_deref(),
        Some("Bearer token-123")
    );
    assert_eq!(header(&req.headers, "x-response"), None);
    assert_eq!(fx.applied.len(), 1);
    assert_eq!(fx.applied[0].phase, Phase::Request);
}

#[test]
fn query_actions() {
    let mut req = request("http://a.test/?a=1&b=2&a=3&keep=%20x");
    run_request(
        vec![any(vec![
            Action::SetQuery {
                name: "a".into(),
                value: "new value".into(),
            },
            Action::RemoveQuery { name: "b".into() },
            Action::SetQuery {
                name: "added".into(),
                value: "yes".into(),
            },
        ])],
        &mut req,
    );
    assert_eq!(req.query, "a=new%20value&keep=%20x&added=yes");
    assert_eq!(req.url(), "http://a.test/?a=new%20value&keep=%20x&added=yes");
}

#[test]
fn body_actions_need_a_buffered_body() {
    let actions = vec![Action::ReplaceBody {
        phase: Phase::Request,
        pattern: Pattern::exact("world"),
        replacement: "there".into(),
    }];
    let set = set(vec![any(actions.clone())]);
    let matched = set.matching(&request("http://a.test/").info(None));
    assert!(eval::needs_body(&matched, Phase::Request));
    assert!(!eval::needs_body(&matched, Phase::Response));

    let mut req = request("http://a.test/");
    let fx = run_request(vec![any(actions.clone())], &mut req);
    assert_eq!(fx.body_skipped.len(), 1, "no body was buffered");

    let mut req = request("http://a.test/");
    req.body = Some(b"hello world, world".to_vec());
    let fx = run_request(vec![any(actions)], &mut req);
    assert!(fx.body_changed);
    assert_eq!(req.body.as_deref(), Some(&b"hello there, there"[..]));
}

#[test]
fn set_body_replaces_and_sets_the_content_type() {
    let mut req = request("http://a.test/");
    run_request(
        vec![any(vec![Action::SetBody {
            phase: Phase::Request,
            body: "<a/>".into(),
            content_type: Some("application/xml".into()),
        }])],
        &mut req,
    );
    assert_eq!(req.body.as_deref(), Some(&b"<a/>"[..]));
    assert_eq!(header(&req.headers, "content-type").as_deref(), Some("application/xml"));
}

#[test]
fn json_set_and_remove() {
    let mut req = request("http://a.test/");
    req.body = Some(br#"{"user":{"name":"a","tags":["x"]},"drop":1,"list":[1,2,3]}"#.to_vec());
    let fx = run_request(
        vec![any(vec![
            Action::JsonSet {
                phase: Phase::Request,
                pointer: "/user/name".into(),
                value: json!("b"),
                raw: None,
            },
            Action::JsonSet {
                phase: Phase::Request,
                pointer: "/user/address/city".into(),
                value: json!("Paris"),
                raw: None,
            },
            Action::JsonSet {
                phase: Phase::Request,
                pointer: "/user/tags/-".into(),
                value: json!("y"),
                raw: None,
            },
            Action::JsonSet {
                phase: Phase::Request,
                pointer: "/list/0".into(),
                value: json!(10),
                raw: None,
            },
            Action::JsonRemove {
                phase: Phase::Request,
                pointer: "/drop".into(),
            },
            Action::JsonRemove {
                phase: Phase::Request,
                pointer: "/list/2".into(),
            },
            Action::JsonSet {
                phase: Phase::Request,
                pointer: "/a~1b".into(),
                value: json!(true),
                raw: None,
            },
        ])],
        &mut req,
    );
    assert!(fx.body_changed && fx.body_skipped.is_empty(), "{:?}", fx.body_skipped);
    let doc: serde_json::Value = serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
    assert_eq!(
        doc,
        json!({"user":{"name":"b","tags":["x","y"],"address":{"city":"Paris"}},"list":[10,2],"a/b":true})
    );

    let mut req = request("http://a.test/");
    req.body = Some(b"not json".to_vec());
    let fx = run_request(
        vec![any(vec![Action::JsonSet {
            phase: Phase::Request,
            pointer: "/x".into(),
            value: json!(1),
            raw: None,
        }])],
        &mut req,
    );
    assert!(fx.body_skipped[0].contains("isn't JSON"), "{:?}", fx.body_skipped);
    assert_eq!(req.body.as_deref(), Some(&b"not json"[..]));
}

#[test]
fn rewrites_apply_in_rule_order_and_all_matching_rules_run() {
    let set_header = |value: &str| Action::SetHeader {
        phase: Phase::Request,
        name: "x-order".into(),
        value: value.into(),
    };
    let mut req = request("http://a.test/");
    let fx = run_request(
        vec![
            rule("one", Matcher::default(), vec![set_header("1")]),
            rule("two", host(Pattern::exact("a.test")), vec![set_header("2")]),
            rule("other host", host(Pattern::exact("b.test")), vec![set_header("x")]),
        ],
        &mut req,
    );
    assert_eq!(header(&req.headers, "x-order").as_deref(), Some("2"));
    let names: Vec<_> = fx.applied.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["one", "two"]);
}

#[test]
fn block_stops_the_flow_before_any_rewrite() {
    let mut req = request("http://a.test/");
    let fx = run_request(
        vec![
            any(vec![Action::SetHeader {
                phase: Phase::Request,
                name: "x-before".into(),
                value: "1".into(),
            }]),
            rule(
                "blocker",
                Matcher::default(),
                vec![Action::Block {
                    mode: BlockMode::Status { status: 451 },
                }],
            ),
        ],
        &mut req,
    );
    assert_eq!(fx.block, Some(BlockMode::Status { status: 451 }));
    assert_eq!(header(&req.headers, "x-before"), None);
    assert_eq!(fx.applied.len(), 1);
    assert_eq!(fx.applied[0].name, "blocker");
}

fn map_remote(host: &str) -> Action {
    Action::MapRemote {
        scheme: None,
        host: Some(host.into()),
        port: None,
        path: None,
        preserve_host: false,
    }
}

fn map_local(body: &str) -> Action {
    Action::MapLocal {
        source: MapLocalSource::Inline { body: body.into() },
        status: 200,
        headers: Vec::new(),
        content_type: None,
    }
}

#[test]
fn last_map_remote_wins() {
    let mut req = request("https://a.test:8443/x");
    let fx = run_request(
        vec![
            rule("first", Matcher::default(), vec![map_remote("b.test")]),
            rule("second", Matcher::default(), vec![map_remote("c.test")]),
        ],
        &mut req,
    );
    let remap = fx.map_remote.unwrap();
    assert_eq!(
        (remap.host.as_str(), remap.port, remap.scheme),
        ("c.test", 443, Scheme::Https)
    );

    // Without a host or scheme change the port stays.
    let mut req = request("http://a.test:8080/x");
    let fx = run_request(
        vec![any(vec![Action::MapRemote {
            scheme: None,
            host: None,
            port: None,
            path: Some("/y".into()),
            preserve_host: true,
        }])],
        &mut req,
    );
    let remap = fx.map_remote.unwrap();
    assert_eq!((remap.host.as_str(), remap.port), ("a.test", 8080));
    assert_eq!(remap.path.as_deref(), Some("/y"));
}

#[test]
fn first_map_local_wins_and_beats_map_remote() {
    let mut req = request("http://a.test/");
    let fx = run_request(
        vec![
            rule("remote", Matcher::default(), vec![map_remote("b.test")]),
            rule("local one", Matcher::default(), vec![map_local("one")]),
            rule("local two", Matcher::default(), vec![map_local("two")]),
        ],
        &mut req,
    );
    assert!(fx.map_remote.is_none());
    match fx.map_local.unwrap().source {
        MapLocalSource::Inline { body } => assert_eq!(body, "one"),
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.applied.len(), 3);
}

#[test]
fn breakpoint_delay_and_throttle() {
    let mut req = request("http://a.test/");
    let fx = run_request(
        vec![
            rule(
                "bp",
                Matcher::default(),
                vec![Action::Breakpoint {
                    request: true,
                    response: false,
                }],
            ),
            any(vec![
                Action::Delay {
                    phase: Phase::Request,
                    ms: 100,
                },
                Action::Throttle {
                    download_kbps: 800,
                    upload_kbps: 80,
                    latency_ms: 50,
                },
            ]),
            any(vec![
                Action::Delay {
                    phase: Phase::Request,
                    ms: 50,
                },
                Action::Throttle {
                    download_kbps: 1600,
                    upload_kbps: 0,
                    latency_ms: 0,
                },
                Action::Delay {
                    phase: Phase::Response,
                    ms: 1000,
                },
            ]),
        ],
        &mut req,
    );
    assert_eq!(fx.breakpoint.unwrap().applied.name, "bp");
    assert_eq!(fx.delay_ms, 150, "request delays add up; response delays wait");
    let throttle = fx.throttle.unwrap();
    assert_eq!(
        (throttle.down_bps, throttle.up_bps),
        (200_000, 0),
        "the last throttle wins"
    );

    // A response-only breakpoint doesn't pause the request.
    let mut req = request("http://a.test/");
    let fx = run_request(
        vec![any(vec![Action::Breakpoint {
            request: false,
            response: true,
        }])],
        &mut req,
    );
    assert!(fx.breakpoint.is_none());
}

// ---- Response phase ----

fn run_response(rules: Vec<Rule>, req: &RequestDraft, draft: &mut ResponseDraft) -> eval::ResponseEffects {
    let set = set(rules);
    let matched: Vec<Arc<CompiledRule>> = set.matching(&req.info(None));
    let gated = eval::response_rules(&matched, draft.status, &draft.headers);
    eval::response_phase(&gated, draft, ScriptRun::none(), None)
        .now_or_never()
        .expect("a rule set without scripts never awaits")
}

#[test]
fn response_actions() {
    let req = request("http://a.test/");
    let mut resp = response(200, "application/json", r#"{"ok":true}"#);
    let fx = run_response(
        vec![any(vec![
            Action::SetStatus { status: 503 },
            Action::SetHeader {
                phase: Phase::Response,
                name: "x-mocked".into(),
                value: "1".into(),
            },
            Action::JsonSet {
                phase: Phase::Response,
                pointer: "/ok".into(),
                value: json!(false),
                raw: None,
            },
            Action::Breakpoint {
                request: false,
                response: true,
            },
            // Request-phase actions don't touch the response.
            Action::SetHeader {
                phase: Phase::Request,
                name: "x-request".into(),
                value: "1".into(),
            },
        ])],
        &req,
        &mut resp,
    );
    assert_eq!(resp.status, 503);
    assert_eq!(header(&resp.headers, "x-mocked").as_deref(), Some("1"));
    assert_eq!(header(&resp.headers, "x-request"), None);
    assert_eq!(resp.body.as_deref(), Some(&br#"{"ok":false}"#[..]));
    assert!(fx.breakpoint.is_some());
    assert_eq!(fx.applied[0].phase, Phase::Response);
}

#[test]
fn the_response_matcher_gates_only_the_response_phase() {
    let gated = |status: Vec<StatusRange>, content_type: Option<Pattern>| {
        rule(
            "gated",
            Matcher {
                response: Some(ResponseMatcher { status, content_type }),
                ..Matcher::default()
            },
            vec![
                Action::SetHeader {
                    phase: Phase::Request,
                    name: "x-req".into(),
                    value: "1".into(),
                },
                Action::SetStatus { status: 299 },
            ],
        )
    };
    let errors = || vec![StatusRange { from: 400, to: 599 }];
    let req = request("http://a.test/");

    // The request phase runs whatever the response will be.
    let mut draft = req.clone();
    run_request(vec![gated(errors(), None)], &mut draft);
    assert_eq!(header(&draft.headers, "x-req").as_deref(), Some("1"));

    let mut ok = response(200, "application/json", "{}");
    run_response(vec![gated(errors(), None)], &req, &mut ok);
    assert_eq!(ok.status, 200);
    let mut failed = response(502, "application/json", "{}");
    run_response(vec![gated(errors(), None)], &req, &mut failed);
    assert_eq!(failed.status, 299);

    let mut html = response(500, "text/html; charset=utf-8", "");
    run_response(
        vec![gated(errors(), Some(Pattern::glob("application/json*")))],
        &req,
        &mut html,
    );
    assert_eq!(html.status, 500);
    let mut json_error = response(500, "application/json; charset=utf-8", "");
    run_response(
        vec![gated(errors(), Some(Pattern::glob("application/json*")))],
        &req,
        &mut json_error,
    );
    assert_eq!(json_error.status, 299);
}

#[test]
fn reframing_drops_the_encoding() {
    let mut headers = HeaderMap::new();
    headers.insert("content-encoding", HeaderValue::from_static("gzip"));
    headers.insert("transfer-encoding", HeaderValue::from_static("chunked"));
    eval::reframe(&mut headers, 12);
    assert_eq!(header(&headers, "content-length").as_deref(), Some("12"));
    assert_eq!(header(&headers, "content-encoding"), None);
    assert_eq!(header(&headers, "transfer-encoding"), None);
}

#[test]
fn new_actions_compile_and_evaluate() {
    let bad = |action: Action| {
        CompiledRule::compile(&any(vec![action]), &ScriptMap::new(), &MockMap::new())
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default()
    };
    assert!(
        bad(Action::JsonSet {
            phase: Phase::Response,
            pointer: "/a".into(),
            value: json!(null),
            raw: Some("{oops".into()),
        })
        .contains("raw value isn't JSON")
    );
    assert!(
        bad(Action::FormSet {
            phase: Phase::Request,
            name: String::new(),
            value: "x".into(),
        })
        .contains("form field name is empty")
    );
    assert!(
        bad(Action::AddHeader {
            phase: Phase::Request,
            name: "bad header".into(),
            value: "x".into(),
        })
        .contains("isn't a valid header name")
    );

    let mut req = request("http://a.test/");
    req.body = Some(br#"{"id": 1, "n": 1.50}"#.to_vec());
    let fx = run_request(
        vec![any(vec![
            Action::JsonSet {
                phase: Phase::Request,
                pointer: "/id".into(),
                value: json!(0),
                raw: Some("12345678901234567890".into()),
            },
            Action::AddHeader {
                phase: Phase::Request,
                name: "accept".into(),
                value: "text/plain".into(),
            },
        ])],
        &mut req,
    );
    assert!(fx.body_changed && fx.head_changed);
    assert_eq!(
        req.body.as_deref(),
        Some(&br#"{"id": 12345678901234567890, "n": 1.50}"#[..])
    );
    let accept: Vec<_> = req.headers.get_all("accept").iter().collect();
    assert_eq!(accept, ["application/json", "text/plain"]);

    let mut req = request("http://a.test/");
    req.body = Some(b"a=1&b=2".to_vec());
    let fx = run_request(
        vec![any(vec![
            Action::FormSet {
                phase: Phase::Request,
                name: "c".into(),
                value: "x y".into(),
            },
            Action::FormRemove {
                phase: Phase::Request,
                name: "a".into(),
            },
        ])],
        &mut req,
    );
    assert!(fx.body_changed);
    assert_eq!(req.body.as_deref(), Some(&b"b=2&c=x%20y"[..]));
}

// ---- WebSocket messages (M19) ----

fn ws(direction: Option<WsDirection>, filter: Option<Pattern>, op: WsOp) -> Action {
    Action::WsMessage { direction, filter, op }
}

fn run_message(rules: Vec<Rule>, draft: &mut eval::WsDraft) -> eval::WsEffects {
    let set = set(rules);
    let matched: Vec<_> = set
        .matching(&request("http://a.test/socket").info(None))
        .into_iter()
        .filter(|r| r.acts_on_messages())
        .collect();
    eval::ws_message_phase(&matched, draft, ScriptRun::none())
        .now_or_never()
        .expect("message rules without scripts never await")
}

fn message(direction: WsDirection, payload: &str) -> eval::WsDraft {
    eval::WsDraft {
        direction,
        text: true,
        payload: payload.as_bytes().to_vec(),
    }
}

#[test]
fn message_rewrites_run_in_order_on_their_direction() {
    let rules = vec![
        any(vec![
            ws(
                Some(WsDirection::Received),
                None,
                WsOp::JsonSet {
                    pointer: "/price".into(),
                    value: json!(0),
                    raw: Some("12345678901234567890".into()),
                },
            ),
            ws(
                None,
                None,
                WsOp::Replace {
                    pattern: Pattern::exact("tick"),
                    replacement: "tock".into(),
                },
            ),
        ]),
        rule(
            "later",
            Matcher::default(),
            vec![ws(
                None,
                None,
                WsOp::JsonRemove {
                    pointer: "/debug".into(),
                },
            )],
        ),
    ];
    let mut received = message(WsDirection::Received, r#"{"type":"tick", "price": 1.50, "debug":true}"#);
    let fx = run_message(rules.clone(), &mut received);
    assert!(fx.changed && !fx.drop);
    assert_eq!(received.payload, br#"{"type":"tock", "price": 12345678901234567890}"#);
    assert_eq!(
        fx.applied.iter().map(|a| a.rule_id.as_str()).collect::<Vec<_>>(),
        ["any", "later"]
    );
    assert_eq!(fx.applied[0].phase, Phase::Response);

    // The JSON set is for received messages only.
    let mut sent = message(WsDirection::Sent, r#"{"type":"tick","price":2}"#);
    let fx = run_message(rules, &mut sent);
    assert_eq!(sent.payload, br#"{"type":"tock","price":2}"#);
    assert_eq!(fx.applied[0].phase, Phase::Request);
}

#[test]
fn a_drop_ends_the_run_and_filters_pick_messages() {
    let rules = vec![
        any(vec![
            ws(None, Some(Pattern::regex(r#""type":\s*"ping""#)), WsOp::Drop),
            ws(None, None, WsOp::Delay { ms: 30 }),
        ]),
        rule("after", Matcher::default(), vec![ws(None, None, WsOp::Breakpoint)]),
    ];
    let mut ping = message(WsDirection::Sent, r#"{"type": "ping"}"#);
    let fx = run_message(rules.clone(), &mut ping);
    assert!(fx.drop);
    assert_eq!((fx.delay_ms, fx.breakpoint.is_some()), (0, false));

    let mut other = message(WsDirection::Sent, r#"{"type":"data"}"#);
    let fx = run_message(rules, &mut other);
    assert!(!fx.drop && !fx.changed);
    assert_eq!(fx.delay_ms, 30);
    assert_eq!(fx.breakpoint.unwrap().applied.rule_id, "after");
}

#[test]
fn json_ops_skip_binary_and_non_json_messages() {
    let rules = vec![any(vec![ws(None, None, WsOp::JsonRemove { pointer: "/a".into() })])];
    let mut binary = eval::WsDraft {
        direction: WsDirection::Received,
        text: false,
        payload: br#"{"a":1}"#.to_vec(),
    };
    let fx = run_message(rules.clone(), &mut binary);
    assert!(!fx.changed);
    assert_eq!(fx.skipped.len(), 1, "{:?}", fx.skipped);
    let mut text = message(WsDirection::Received, "not json");
    let fx = run_message(rules, &mut text);
    assert!(!fx.changed && fx.skipped.len() == 1);
}

#[test]
fn message_actions_compile_their_patterns() {
    let bad = rule(
        "bad",
        Matcher::default(),
        vec![ws(None, Some(Pattern::regex("(")), WsOp::Drop)],
    );
    let err = CompiledRuleSet::compile(&[bad], &ScriptMap::new(), &MockMap::new())
        .err()
        .unwrap();
    assert!(err.to_string().contains("message filter"), "{err}");
    let bad = rule(
        "bad",
        Matcher::default(),
        vec![ws(
            None,
            None,
            WsOp::JsonSet {
                pointer: "a".into(),
                value: json!(1),
                raw: None,
            },
        )],
    );
    assert!(CompiledRuleSet::compile(&[bad], &ScriptMap::new(), &MockMap::new()).is_err());
    // A rule with only message actions leaves plain requests alone.
    let only = set(vec![any(vec![ws(None, None, WsOp::Drop)])]);
    let matched = only.matching(&request("http://a.test/").info(None));
    assert!(matched[0].acts_on_messages() && !matched[0].acts_on_http());
}
