//! Scripts end to end (M14): rewrite and mock scripts from rules through
//! the proxy, script errors on flows, editing and deleting scripts, view
//! scripts on live flows and reopened sessions, dry runs and `.js` files.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderMap, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{
    Action, BlockMode, BodyPart, BreakpointResolution, ContentKind, FlowDetail, FlowId, FlowState, FlowSummary,
    LIVE_STORE, MapLocalSource, Matcher, Pattern, Rule, RuleOrigin, Script, ScriptKind, ScriptPhase, ScriptTestSpec,
    ScriptViewKind, ViewConfig,
};
use pm_testkit::Upstream;
use pm_testkit::client::{self, eventually};
use pm_testkit::upstream::JSON_BODY;

const WAIT: Duration = Duration::from_secs(10);

struct Proxy {
    engine: Arc<Engine>,
    addr: SocketAddr,
}

impl Proxy {
    async fn start() -> Proxy {
        let dir = std::env::temp_dir()
            .join("proxymate-tests")
            .join(uuid::Uuid::new_v4().to_string());
        let engine = Engine::with_config(EngineConfig {
            data_dir: Some(dir),
            ..Default::default()
        });
        let addr = engine
            .start_proxy(ProxyConfig {
                ports: 0..=0,
                ..Default::default()
            })
            .await
            .expect("proxy starts");
        Proxy { engine, addr }
    }

    fn script(&self, kind: ScriptKind, source: &str) -> Script {
        self.engine
            .create_script(Script {
                id: String::new(),
                name: format!("{kind:?} script"),
                kind,
                enabled: true,
                source: source.into(),
                origin: RuleOrigin::User,
                view: (kind == ScriptKind::View).then(ViewConfig::default),
                updated_at_ms: 0.0,
            })
            .expect("script is valid")
    }

    fn rule(&self, name: &str, matcher: Matcher, actions: Vec<Action>) -> Rule {
        self.engine
            .create_rule(Rule {
                id: String::new(),
                name: name.into(),
                enabled: true,
                order: 0,
                origin: RuleOrigin::User,
                matcher,
                actions,
            })
            .expect("rule is valid")
    }

    async fn finished(&self, id: FlowId) -> (FlowSummary, FlowDetail) {
        let summary = eventually(WAIT, || {
            self.engine.flush();
            self.engine
                .get_flow(id)
                .filter(|f| !matches!(f.state, FlowState::Active | FlowState::Paused))
        })
        .await;
        let detail = self.engine.flow_detail(id).unwrap().expect("detail");
        (summary, detail)
    }
}

fn run(script: &Script) -> Action {
    Action::Script {
        script_id: script.id.clone(),
    }
}

fn on_path(path: &str) -> Matcher {
    Matcher {
        path: Some(Pattern::exact(path)),
        ..Matcher::default()
    }
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

async fn get(proxy: SocketAddr, url: &str) -> Reply {
    let response = client::send(proxy, Request::get(url).body(Full::default()).unwrap()).await;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.expect("body").to_bytes();
    Reply { status, headers, body }
}

fn json(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).expect("JSON")
}

#[tokio::test]
async fn a_rewrite_script_edits_a_gzipped_response_and_the_request() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start().await;
    let script = proxy.script(
        ScriptKind::Rewrite,
        "export function onRequest(context, url, request) { request.headers['x-scripted'] = context.rule.name; } \
         export function onResponse(context, url, request, response) { \
           if (request.path === '/gzip') { response.body.name = 'scripted'; delete response.body.nested; } \
           return response; }",
    );
    proxy.rule("rewrite", Matcher::default(), vec![run(&script)]);

    let reply = get(proxy.addr, &upstream.url("/gzip")).await;
    assert_eq!(reply.status, 200);
    assert!(
        reply.headers.get("content-encoding").is_none(),
        "re-sent identity-encoded"
    );
    let mut expected = json(JSON_BODY.as_bytes());
    expected["name"] = "scripted".into();
    expected.as_object_mut().unwrap().remove("nested");
    assert_eq!(json(&reply.body), expected);
    let (flow, detail) = proxy.finished(0).await;
    assert!(flow.edited);
    assert!(detail.rules.scripts.is_empty());
    assert_eq!(detail.rules.applied.len(), 2, "request and response phases");

    let reply = get(proxy.addr, &upstream.url("/inspect")).await;
    assert_eq!(json(&reply.body)["headers"]["x-scripted"], "rewrite");
}

#[tokio::test]
async fn a_script_that_only_reads_keeps_the_wire_bytes() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start().await;
    let direct = get(proxy.addr, &upstream.url("/gzip")).await;
    let script = proxy.script(
        ScriptKind::Rewrite,
        "export function onResponse(c, url, req, res) { console.log(res.body.name, res.headers['content-encoding']); return res; }",
    );
    proxy.rule("reader", on_path("/gzip"), vec![run(&script)]);
    let reply = get(proxy.addr, &upstream.url("/gzip")).await;
    assert_eq!(reply.headers["content-encoding"], "gzip");
    assert_eq!(reply.body, direct.body, "byte-identical");
    let (flow, _) = proxy.finished(1).await;
    assert!(!flow.edited);
    let console = proxy.engine.script_console(None, 100);
    assert_eq!(console.last().unwrap().message, "proxymate gzip");
    assert_eq!(console.last().unwrap().flow_id, Some(1));
}

#[tokio::test]
async fn a_mock_never_dials_the_upstream() {
    let proxy = Proxy::start().await;
    let script = proxy.script(
        ScriptKind::Mock,
        "export function onMock(context, url, request) { \
           if (request.query.id === '7') return { status: 201, headers: { 'x-mock': '1' }, body: { id: 7n ** 20n } }; \
           return null; }",
    );
    proxy.rule("mock", Matcher::default(), vec![run(&script)]);
    let reply = get(proxy.addr, "http://nowhere.invalid/items?id=7").await;
    assert_eq!(reply.status, 201);
    assert_eq!(reply.headers["x-mock"], "1");
    assert_eq!(reply.headers["content-type"], "application/json");
    assert_eq!(reply.body, r#"{"id":79792266297612001}"#);
    let (flow, detail) = proxy.finished(0).await;
    assert_eq!((flow.state, flow.status), (FlowState::Completed, Some(201)));
    assert!(detail.rules.mocked);
    assert_eq!(detail.error, None);

    // `null` passes the request on (here to a host that doesn't exist).
    let reply = get(proxy.addr, "http://nowhere.invalid/items?id=8").await;
    assert_eq!(reply.status, 502);
}

#[tokio::test]
async fn a_throwing_script_is_reported_and_the_flow_passes_through() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start().await;
    let script = proxy.script(
        ScriptKind::Rewrite,
        "export function onResponse(c, url, req, res) { res.status = 500; throw new Error('boom'); }",
    );
    proxy.rule("broken", Matcher::default(), vec![run(&script)]);
    let reply = get(proxy.addr, &upstream.url("/json")).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, JSON_BODY);
    let (flow, detail) = proxy.finished(0).await;
    assert!(!flow.edited);
    let issue = &detail.rules.scripts[0];
    assert_eq!(
        (issue.phase, issue.script_id.as_str()),
        (ScriptPhase::Response, script.id.as_str())
    );
    assert!(issue.message.starts_with("Error: boom"), "{}", issue.message);
    let console = proxy.engine.script_console(None, 10);
    assert!(console.iter().any(|e| e.message.starts_with("Error: boom")));
}

#[tokio::test]
async fn a_slow_script_times_out_and_the_flow_passes_through() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start().await;
    let mut settings = proxy.engine.script_settings();
    settings.traffic_timeout_ms = 100;
    proxy.engine.set_script_settings(settings).unwrap();
    let script = proxy.script(ScriptKind::Rewrite, "export function onRequest() { for (;;) {} }");
    proxy.rule("spin", Matcher::default(), vec![run(&script)]);
    let reply = get(proxy.addr, &upstream.url("/json")).await;
    assert_eq!(reply.body, JSON_BODY);
    let (_, detail) = proxy.finished(0).await;
    assert!(detail.rules.scripts[0].message.contains("timed out"));
}

#[tokio::test]
async fn scripts_run_in_rule_order() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start().await;
    // A script sees the edits of the actions before it.
    let copy = proxy.script(
        ScriptKind::Rewrite,
        "export function onRequest(c, url, r) { r.headers['x-copy'] = r.headers['x-set'] ?? 'missing'; }",
    );
    proxy.rule(
        "set then copy",
        on_path("/inspect"),
        vec![
            Action::SetHeader {
                phase: pm_model::Phase::Request,
                name: "x-set".into(),
                value: "from a rule".into(),
            },
            run(&copy),
        ],
    );
    let reply = get(proxy.addr, &upstream.url("/inspect")).await;
    assert_eq!(json(&reply.body)["headers"]["x-copy"], "from a rule");

    // The first answer wins, Map Local or mock.
    let mock = proxy.script(ScriptKind::Mock, "export function onMock() { return 'mocked'; }");
    proxy.rule(
        "local first",
        on_path("/a"),
        vec![
            Action::MapLocal {
                source: MapLocalSource::Inline { body: "local".into() },
                status: 200,
                headers: Vec::new(),
                content_type: None,
            },
            run(&mock),
        ],
    );
    proxy.rule("mock first", on_path("/b"), vec![run(&mock)]);
    proxy.rule(
        "then local",
        on_path("/b"),
        vec![Action::MapLocal {
            source: MapLocalSource::Inline { body: "local".into() },
            status: 200,
            headers: Vec::new(),
            content_type: None,
        }],
    );
    assert_eq!(get(proxy.addr, &upstream.url("/a")).await.body, "local");
    assert_eq!(get(proxy.addr, &upstream.url("/b")).await.body, "mocked");

    // A block beats scripts, whatever the order.
    proxy.rule("mock", on_path("/c"), vec![run(&mock)]);
    proxy.rule(
        "block",
        on_path("/c"),
        vec![Action::Block {
            mode: BlockMode::Status { status: 403 },
        }],
    );
    assert_eq!(get(proxy.addr, &upstream.url("/c")).await.status, 403);
}

#[tokio::test]
async fn a_flow_in_flight_keeps_the_script_it_started_with() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start().await;
    let mut script = proxy.script(
        ScriptKind::Rewrite,
        "export function onResponse(c, u, q, res) { res.headers['x-version'] = 'old'; }",
    );
    proxy.rule(
        "pause",
        Matcher::default(),
        vec![
            Action::Breakpoint {
                request: true,
                response: false,
            },
            run(&script),
        ],
    );
    let addr = proxy.addr;
    let url = upstream.url("/json");
    let reply = tokio::spawn(async move { get(addr, &url).await });
    let paused = eventually(WAIT, || proxy.engine.breakpoints().into_iter().next()).await;
    script.source = "export function onResponse(c, u, q, res) { res.headers['x-version'] = 'new'; }".into();
    proxy.engine.update_script(script).unwrap();
    proxy
        .engine
        .resolve_breakpoint(paused.id, BreakpointResolution::Continue { edits: None })
        .unwrap();
    let reply = reply.await.unwrap();
    assert_eq!(reply.headers["x-version"], "old");

    let addr = proxy.addr;
    let url = upstream.url("/json");
    let reply = tokio::spawn(async move { get(addr, &url).await });
    let paused = eventually(WAIT, || proxy.engine.breakpoints().into_iter().next()).await;
    proxy
        .engine
        .resolve_breakpoint(paused.id, BreakpointResolution::Continue { edits: None })
        .unwrap();
    assert_eq!(reply.await.unwrap().headers["x-version"], "new");
}

#[tokio::test]
async fn scripts_rules_use_cant_be_deleted() {
    let proxy = Proxy::start().await;
    let script = proxy.script(ScriptKind::Mock, "export function onMock() { return 'x'; }");
    let rule = proxy.rule("uses it", Matcher::default(), vec![run(&script)]);
    let err = proxy.engine.delete_script(&script.id).unwrap_err().to_string();
    assert!(err.contains("used by rule \"uses it\""), "{err}");
    proxy.engine.delete_rule(&rule.id).unwrap();
    proxy.engine.delete_script(&script.id).unwrap();
    assert!(proxy.engine.scripts().is_empty());

    // Rules can't name a script that doesn't exist, or a view script.
    let view = proxy.script(ScriptKind::View, "export function onView() { return []; }");
    let make = |id: &str| {
        proxy.engine.create_rule(Rule {
            id: String::new(),
            name: "bad".into(),
            enabled: true,
            order: 0,
            origin: RuleOrigin::User,
            matcher: Matcher::default(),
            actions: vec![Action::Script { script_id: id.into() }],
        })
    };
    assert!(make("missing").unwrap_err().to_string().contains("no script"));
    assert!(make(&view.id).unwrap_err().to_string().contains("view script"));

    // A script must export its kind's entry point, and parse.
    let bad = proxy.engine.create_script(Script {
        kind: ScriptKind::Mock,
        source: "export function onRequest() {}".into(),
        ..view.clone()
    });
    assert!(bad.unwrap_err().to_string().contains("must export onMock"));
    let bad = proxy.engine.create_script(Script {
        source: "export function onView( {".into(),
        ..view
    });
    assert!(bad.unwrap_err().to_string().contains("SyntaxError"));
}

const DECODE_PAYLOAD: &str = "export function onView(context, flow) { \
    const body = flow[context.part].body; \
    if (!body || !body.payload) return null; \
    return [{ title: 'Payload', body: Proxymate.base64DecodeJson(body.payload) }]; }";

#[tokio::test]
async fn view_scripts_decode_live_and_saved_flows() {
    let proxy = Proxy::start().await;
    let view = proxy
        .engine
        .create_script(Script {
            id: String::new(),
            name: "Decode payload".into(),
            kind: ScriptKind::View,
            enabled: true,
            source: DECODE_PAYLOAD.into(),
            origin: RuleOrigin::User,
            view: Some(ViewConfig {
                matcher: Matcher {
                    host: Some(Pattern::glob("*.example.test")),
                    ..Matcher::default()
                },
                parts: vec![BodyPart::Response],
                kinds: vec![ContentKind::Json],
            }),
            updated_at_ms: 0.0,
        })
        .unwrap();

    // A session saved before scripts existed.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/session-v1.proxymate");
    let doc = proxy.engine.open_document(&fixture).unwrap();
    let results = proxy
        .engine
        .run_view_scripts(doc.store, 0, BodyPart::Response)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(
        (result.script_id.as_str(), result.error.as_deref()),
        (view.id.as_str(), None)
    );
    assert_eq!(result.views[0].kind, ScriptViewKind::Json);
    assert_eq!(
        result.views[0].text.as_deref().unwrap(),
        "{\n  \"user\": \"ada\",\n  \"id\": 12345678901234567890\n}"
    );
    // Finished flows are cached: the same result, duration and all.
    let again = proxy
        .engine
        .run_view_scripts(doc.store, 0, BodyPart::Response)
        .await
        .unwrap();
    assert_eq!(&again, &results);
    // The request part, and a text body, don't match.
    assert!(
        proxy
            .engine
            .run_view_scripts(doc.store, 0, BodyPart::Request)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        proxy
            .engine
            .run_view_scripts(doc.store, 1, BodyPart::Response)
            .await
            .unwrap()
            .is_empty()
    );

    // A dry run of unsaved source against the saved flow.
    let test = proxy
        .engine
        .test_script(
            doc.store,
            0,
            ScriptTestSpec {
                script_id: None,
                source: Some(DECODE_PAYLOAD.replace("'Payload'", "'Draft'")),
                kind: Some(ScriptKind::View),
                part: Some(BodyPart::Response),
            },
        )
        .await
        .unwrap();
    assert_eq!((test.error, test.views[0].title.as_str()), (None, "Draft"));

    // A script that throws shows its error instead of views.
    let mut broken = view.clone();
    broken.source = "export function onView() { throw new Error('nope'); }".into();
    proxy.engine.update_script(broken).unwrap();
    let results = proxy
        .engine
        .run_view_scripts(doc.store, 0, BodyPart::Response)
        .await
        .unwrap();
    assert!(results[0].error.as_deref().unwrap().starts_with("Error: nope"));
    assert!(results[0].views.is_empty());
    assert!(
        proxy
            .engine
            .run_view_scripts(LIVE_STORE, 99, BodyPart::Response)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn dry_runs_show_before_and_after() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start().await;
    get(proxy.addr, &upstream.url("/json")).await;
    proxy.finished(0).await;
    let spec = |source: &str, kind| ScriptTestSpec {
        script_id: None,
        source: Some(source.into()),
        kind: Some(kind),
        part: None,
    };
    let result = proxy
        .engine
        .test_script(
            LIVE_STORE,
            0,
            spec(
                "export function onRequest(c, u, r) { r.headers['x-a'] = '1'; console.log('ran'); } \
                 export function onResponse(c, u, q, r) { r.body.version = 2; }",
                ScriptKind::Rewrite,
            ),
        )
        .await
        .unwrap();
    assert_eq!(result.error, None);
    assert!(!result.request_before.unwrap().headers.iter().any(|h| h.name == "x-a"));
    assert!(result.request.unwrap().headers.iter().any(|h| h.name == "x-a"));
    assert_eq!(json(result.response_body_before.unwrap().as_bytes())["version"], 1);
    assert_eq!(json(result.response_body.unwrap().as_bytes())["version"], 2);
    assert_eq!(result.console[0].message, "ran");

    let result = proxy
        .engine
        .test_script(
            LIVE_STORE,
            0,
            spec("export function onMock() { return { status: 503 }; }", ScriptKind::Mock),
        )
        .await
        .unwrap();
    assert_eq!(result.response.unwrap().status, 503);

    let result = proxy
        .engine
        .test_script(
            LIVE_STORE,
            0,
            spec("export function onRequest() { throw 1; }", ScriptKind::Rewrite),
        )
        .await
        .unwrap();
    assert!(result.error.is_some());
}

#[tokio::test]
async fn scripts_export_and_import_as_js() {
    let proxy = Proxy::start().await;
    let script = proxy.script(ScriptKind::View, DECODE_PAYLOAD);
    let file = proxy.engine.export_script(&script.id).unwrap();
    assert!(file.starts_with("/* proxymate: {\"name\":\"View script\",\"kind\":\"view\""));
    let imported = proxy
        .engine
        .import_script(&file, Some("ignored.js"), RuleOrigin::User)
        .unwrap();
    assert_ne!(imported.id, script.id);
    assert_eq!(
        (imported.name.as_str(), imported.kind, imported.source.as_str()),
        ("View script", ScriptKind::View, DECODE_PAYLOAD)
    );
    let bare = proxy
        .engine
        .import_script(
            "export function onMock() { return 'x'; }\n",
            Some("fake-login.js"),
            RuleOrigin::User,
        )
        .unwrap();
    assert_eq!((bare.name.as_str(), bare.kind), ("fake-login", ScriptKind::Mock));
    assert_eq!(proxy.engine.scripts().len(), 3);
}

#[tokio::test]
async fn scripts_persist_in_the_app_database() {
    let dir: PathBuf = std::env::temp_dir()
        .join("proxymate-tests")
        .join(uuid::Uuid::new_v4().to_string());
    let config = || EngineConfig {
        data_dir: Some(dir.clone()),
        ..Default::default()
    };
    let engine = Engine::with_config(config());
    let script = engine
        .create_script(Script {
            id: String::new(),
            name: "Mock".into(),
            kind: ScriptKind::Mock,
            enabled: true,
            source: "export function onMock() { return 'x'; }".into(),
            origin: RuleOrigin::User,
            view: None,
            updated_at_ms: 0.0,
        })
        .unwrap();
    engine
        .create_rule(Rule {
            id: String::new(),
            name: "mock".into(),
            enabled: true,
            order: 0,
            origin: RuleOrigin::User,
            matcher: Matcher::default(),
            actions: vec![Action::Script {
                script_id: script.id.clone(),
            }],
        })
        .unwrap();
    drop(engine);
    let engine = Engine::with_config(config());
    assert_eq!(engine.scripts(), vec![script]);
    assert!(engine.rules().iter().any(|r| r.name == "mock" && r.enabled));
}
