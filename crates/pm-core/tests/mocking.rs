//! Offline mocking (M20) end to end: record against the testkit upstream,
//! save, stop the upstream and serve the recording (`ServeFrom`); Charles
//! sessions as documents; the Petstore spec's operations and mocks.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderMap, Request};
use http_body_util::{BodyExt, Full};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{
    Action, BodyPart, DocumentKind, FilterSpec, FlowSelection, FlowState, LIVE_STORE, Phase, PresetKind, Rule,
    RuleOrigin, ServeKey, ServeMiss, ServePick, SortSpec,
};
use pm_testkit::Upstream;
use pm_testkit::client::{self, eventually};
use pm_testkit::upstream::JSON_BODY;

const WAIT: Duration = Duration::from_secs(20);

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir()
        .join("proxymate-tests")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

async fn proxy(dir: &Path) -> (Arc<Engine>, SocketAddr) {
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(dir.to_owned()),
        ..Default::default()
    });
    let addr = engine
        .start_proxy(ProxyConfig {
            ports: 0..=0,
            ..Default::default()
        })
        .await
        .expect("proxy starts");
    (engine, addr)
}

/// What the client got: status, headers and the body as it came over the
/// wire (hyper doesn't decode it).
struct Got {
    status: u16,
    headers: HeaderMap,
    body: Bytes,
}

async fn send(proxy: SocketAddr, method: &str, url: &str, body: &str) -> Got {
    let req = Request::builder()
        .method(method)
        .uri(url)
        .body(Full::new(Bytes::from(body.to_owned())))
        .unwrap();
    let response = client::send(proxy, req).await;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.expect("body").to_bytes();
    Got { status, headers, body }
}

fn header<'a>(got: &'a Got, name: &str) -> Option<&'a str> {
    got.headers.get(name).and_then(|v| v.to_str().ok())
}

async fn settled(engine: &Engine, flows: u32) {
    eventually(WAIT, || {
        engine.flush();
        let done = engine
            .find_flows("", None, None, 1000, false)
            .ok()?
            .flows
            .iter()
            .filter(|f| !matches!(f.state, FlowState::Active | FlowState::Paused))
            .count();
        (done as u32 >= flows).then_some(())
    })
    .await;
}

fn save(engine: &Arc<Engine>, store: u32, path: &Path) {
    let (tx, rx) = mpsc::channel();
    engine
        .save_session(
            store,
            &FlowSelection::All,
            path.to_owned(),
            Box::new(move |p| {
                if p.finished {
                    let _ = tx.send(p);
                }
            }),
        )
        .unwrap();
    let done = rx.recv_timeout(Duration::from_secs(60)).unwrap();
    assert_eq!(done.error, None);
}

fn serve_rule(engine: &Engine, rule: &Rule, key: ServeKey, on_miss: ServeMiss, pick: ServePick) -> Rule {
    let mut rule = rule.clone();
    let Action::ServeFrom { source, .. } = &rule.actions[0] else {
        panic!("not a ServeFrom rule")
    };
    rule.actions[0] = Action::ServeFrom {
        source: source.clone(),
        key,
        on_miss,
        pick,
    };
    engine.update_rule(rule).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recording_answers_byte_identically_after_the_upstream_stops() {
    let dir = temp_dir();
    let session = dir.join("recorded.proxymate");
    let upstream = Upstream::start().await;
    let host = upstream.addr();

    // Record.
    let (recorder, addr) = proxy(&dir.join("recorder")).await;
    let originals = vec![
        send(addr, "GET", &upstream.url("/json"), "").await,
        send(addr, "GET", &upstream.url("/gzip"), "").await,
        send(addr, "GET", &upstream.url("/status/418?b=2&a=1"), "").await,
        send(addr, "POST", &upstream.url("/echo"), "one").await,
        send(addr, "POST", &upstream.url("/echo"), "two").await,
        send(addr, "POST", &upstream.url("/echo"), "three").await,
    ];
    assert_eq!(&originals[0].body[..], JSON_BODY.as_bytes());
    assert_eq!(header(&originals[1], "content-encoding"), Some("gzip"));
    settled(&recorder, 6).await;
    save(&recorder, LIVE_STORE, &session);
    drop(recorder);
    drop(upstream);

    // Serve, with nothing listening upstream.
    let (engine, addr) = proxy(&dir.join("server")).await;
    let source = engine.create_mock_source_from_file(&session, None).unwrap();
    assert_eq!(source.name, "recorded");
    assert_eq!(source.flows, 6);
    assert_eq!(source.hosts, vec![host.ip().to_string()]);
    assert!(
        source.size > 0
            && dir
                .join("server/mocks")
                .join(format!("{}.proxymate", source.id))
                .is_file()
    );
    let draft = engine.draft_serve_from_rule(&source.id).unwrap();
    assert_eq!(
        draft.origin,
        RuleOrigin::Preset {
            preset: PresetKind::ServeFrom
        }
    );
    let rule = engine.create_rule(draft).unwrap();

    for (i, (method, path, body)) in [
        ("GET", "/json", ""),
        ("GET", "/gzip", ""),
        ("GET", "/status/418?a=1&b=2", ""),
    ]
    .into_iter()
    .enumerate()
    {
        let got = send(addr, method, &format!("http://{host}{path}"), body).await;
        let want = &originals[i];
        assert_eq!(got.status, want.status, "{path}");
        assert_eq!(got.body, want.body, "{path}: the recorded bytes");
        for name in ["content-type", "content-encoding", "content-length"] {
            assert_eq!(header(&got, name), header(want, name), "{path}: {name}");
        }
    }
    engine.flush();
    let served = engine.find_flows("path:/gzip", None, None, 10, true).unwrap().flows;
    let detail = engine.flow_detail(served[0].id).unwrap().unwrap();
    assert!(detail.rules.mocked, "served flows are marked mocked");
    assert_eq!(detail.rules.applied[0].rule_id, rule.id);
    assert_eq!(detail.response_body.encoding.as_deref(), Some("gzip"));

    // The first recorded answer by default, then round-robin and last.
    let echo = format!("http://{host}/echo");
    assert_eq!(&send(addr, "POST", &echo, "x").await.body[..], b"one");
    serve_rule(
        &engine,
        &rule,
        ServeKey::default(),
        ServeMiss::PassThrough,
        ServePick::RoundRobin,
    );
    let mut turns = Vec::new();
    for _ in 0..4 {
        turns.push(send(addr, "POST", &echo, "x").await.body);
    }
    assert_eq!(
        turns,
        ["one", "two", "three", "one"].map(|s| Bytes::from_static(s.as_bytes()))
    );
    serve_rule(
        &engine,
        &rule,
        ServeKey::default(),
        ServeMiss::PassThrough,
        ServePick::Last,
    );
    assert_eq!(&send(addr, "POST", &echo, "x").await.body[..], b"three");

    // With the body in the key, each body has its own answer.
    let body_key = ServeKey {
        query: true,
        body: true,
    };
    serve_rule(&engine, &rule, body_key, ServeMiss::NotFound, ServePick::First);
    assert_eq!(&send(addr, "POST", &echo, "two").await.body[..], b"two");
    let miss = send(addr, "POST", &echo, "four").await;
    assert_eq!(miss.status, 404);
    assert!(String::from_utf8_lossy(&miss.body).contains("nothing recorded in \"recorded\""));

    // Misses: 404, 502, or on to the (stopped) upstream.
    let other_query = format!("http://{host}/status/418?a=1");
    assert_eq!(send(addr, "GET", &other_query, "").await.status, 404);
    serve_rule(
        &engine,
        &rule,
        ServeKey::default(),
        ServeMiss::BadGateway,
        ServePick::First,
    );
    let bad = send(addr, "GET", &other_query, "").await;
    assert_eq!(bad.status, 502);
    assert!(String::from_utf8_lossy(&bad.body).contains("Proxymate: nothing recorded"));
    let rule = serve_rule(
        &engine,
        &rule,
        ServeKey::default(),
        ServeMiss::PassThrough,
        ServePick::First,
    );
    let passed = send(addr, "GET", &other_query, "").await;
    assert_eq!(passed.status, 502, "nothing listens upstream");
    assert!(!String::from_utf8_lossy(&passed.body).contains("nothing recorded"));
    // Without the query in the key, it matches.
    serve_rule(
        &engine,
        &rule,
        ServeKey {
            query: false,
            body: false,
        },
        ServeMiss::PassThrough,
        ServePick::First,
    );
    assert_eq!(send(addr, "GET", &other_query, "").await.status, 418);

    // Response-phase actions still run on a recorded answer.
    let mut with_header = engine.rule(&rule.id).unwrap();
    with_header.actions.push(Action::SetHeader {
        phase: Phase::Response,
        name: "x-served".into(),
        value: "yes".into(),
    });
    engine.update_rule(with_header).unwrap();
    let got = send(addr, "GET", &format!("http://{host}/json"), "").await;
    assert_eq!(header(&got, "x-served"), Some("yes"));
    assert_eq!(&got.body[..], JSON_BODY.as_bytes());

    // The first answer wins: the recording, then Map Local once it's first.
    let local = engine
        .create_rule(Rule {
            id: String::new(),
            name: "local".into(),
            enabled: true,
            order: 0,
            origin: RuleOrigin::User,
            matcher: Default::default(),
            actions: vec![Action::MapLocal {
                source: pm_model::MapLocalSource::Inline { body: "local".into() },
                status: 200,
                headers: Vec::new(),
                content_type: None,
            }],
        })
        .unwrap();
    let json = format!("http://{host}/json");
    assert_eq!(&send(addr, "GET", &json, "").await.body[..], JSON_BODY.as_bytes());
    engine.reorder_rules(&[local.id.clone(), rule.id.clone()]).unwrap();
    assert_eq!(&send(addr, "GET", &json, "").await.body[..], b"local");
    engine.delete_rule(&local.id).unwrap();

    // A source in use can't be removed; once its rule is gone it can.
    let err = engine.remove_mock_source(&source.id).unwrap_err().to_string();
    assert!(err.contains("used by"), "{err}");
    engine.delete_rule(&rule.id).unwrap();
    engine.remove_mock_source(&source.id).unwrap();
    assert!(engine.mock_sources().is_empty());
    assert!(
        !dir.join("server/mocks")
            .join(format!("{}.proxymate", source.id))
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rules_keep_serving_after_a_restart() {
    let dir = temp_dir();
    let data = dir.join("data");
    let session = dir.join("one.proxymate");
    let upstream = Upstream::start().await;
    let url = upstream.url("/json");
    {
        let (recorder, addr) = proxy(&dir.join("recorder")).await;
        send(addr, "GET", &url, "").await;
        settled(&recorder, 1).await;
        save(&recorder, LIVE_STORE, &session);
    }
    drop(upstream);
    {
        let (engine, _) = proxy(&data).await;
        // A document's store copies in as well as a file does.
        let doc = engine.open_document(&session).unwrap();
        let source = engine
            .create_mock_source(doc.store, Some("From the tab".into()))
            .unwrap();
        assert_eq!((source.name.as_str(), source.flows), ("From the tab", 1));
        engine
            .create_rule(engine.draft_serve_from_rule(&source.id).unwrap())
            .unwrap();
        // The original file can go: the library has its own copy.
        engine.close_document(doc.store).unwrap();
        std::fs::remove_file(&session).unwrap();
    }
    let (engine, addr) = proxy(&data).await;
    assert_eq!(engine.mock_sources().len(), 1);
    let got = send(addr, "GET", &url, "").await;
    assert_eq!((got.status, &got.body[..]), (200, JSON_BODY.as_bytes()));
}

#[tokio::test]
async fn a_dry_run_says_what_a_recording_answers() {
    let dir = temp_dir();
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(dir.join("data")),
        ..Default::default()
    });
    let doc = engine.open_document(&fixture("charles/session.chlsj")).unwrap();
    let source = engine.create_mock_source(doc.store, None).unwrap();
    let rule = engine.draft_serve_from_rule(&source.id).unwrap();
    // The POST of the session, tested against itself.
    let result = engine.test_rule_in(doc.store, &rule, 1, None).await.unwrap();
    assert!(result.matched);
    assert!(
        result.notes[0].contains("Answered from the recording \"session\""),
        "{:?}",
        result.notes
    );
    assert_eq!(result.response.as_ref().map(|r| r.status), Some(201));
    assert_eq!(result.response_body.as_deref(), Some(r#"{"id":3,"name":"Alan"}"#));
    // The gzip'd GET is shown decoded.
    let result = engine.test_rule_in(doc.store, &rule, 0, None).await.unwrap();
    assert!(result.response_body.unwrap().contains("Grace"));
}

fn assert_charles_flows(engine: &Arc<Engine>, store: u32) {
    let flows = engine.selected_flows(store, &FlowSelection::All).unwrap();
    let urls: Vec<String> = flows
        .iter()
        .map(|f| format!("{} {}:{}{}?{}", f.method, f.host, f.port, f.path, f.query))
        .collect();
    assert_eq!(
        urls,
        [
            "GET api.example.com:443/v1/users?page=2&sort=name",
            "POST api.example.com:443/v1/users?",
            "GET cdn.example.com:8080/pixel.png?",
            "GET down.example.com:443/health?",
            "CONNECT pinned.example.com:443/?",
        ]
    );
    let states: Vec<FlowState> = flows.iter().map(|f| f.state).collect();
    assert_eq!(
        states,
        [
            FlowState::Completed,
            FlowState::Completed,
            FlowState::Completed,
            FlowState::Failed,
            FlowState::Tunnel
        ]
    );
    assert_eq!(flows[0].status, Some(200));
    assert_eq!(flows[1].status, Some(201));
    assert_eq!(flows[0].started_at_ms, 1_714_557_600_000.0);
    assert_eq!(flows[0].duration_ms, Some(120));

    let get = engine.flow_detail_in(store, 0).unwrap().unwrap();
    assert_eq!(get.request.url, "https://api.example.com/v1/users?page=2&sort=name");
    assert!(
        get.request
            .headers
            .iter()
            .any(|h| h.name == "Accept" && h.value == "application/json")
    );
    let response = get.response.unwrap();
    assert_eq!(response.reason, "OK");
    assert!(
        response
            .headers
            .iter()
            .any(|h| h.name == "X-Request-Id" && h.value == "r-1")
    );
    // Base64 bodies are the wire bytes: still gzip'd, decoded on demand.
    assert_eq!(get.response_body.encoding.as_deref(), Some("gzip"));
    let body = engine.body_head_in(store, 0, BodyPart::Response, true, None).unwrap();
    assert_eq!(body.decoded_from.as_deref(), Some("gzip"));
    assert!(String::from_utf8(body.bytes).unwrap().contains("\"Grace\""));
    assert_eq!(get.timing.dns_ms, Some(4.0));

    let post = engine.flow_detail_in(store, 1).unwrap().unwrap();
    assert_eq!(post.response.as_ref().unwrap().reason, "Created");
    let request_body = engine.body_head_in(store, 1, BodyPart::Request, true, None).unwrap();
    assert_eq!(request_body.bytes, br#"{"name":"Alan"}"#);
    assert_eq!(post.response_body.encoding, None);

    let png = engine.body_head_in(store, 2, BodyPart::Response, true, None).unwrap();
    assert!(png.bytes.starts_with(b"\x89PNG"));
    let failed = engine.flow_detail_in(store, 3).unwrap().unwrap();
    assert!(failed.error.unwrap().contains("Connection refused"));
}

#[test]
fn charles_sessions_open_as_documents_and_round_trip() {
    let dir = temp_dir();
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(dir.join("data")),
        ..Default::default()
    });
    for name in ["session.chlsj", "session.chlsx"] {
        let doc = engine.open_document(&fixture(&format!("charles/{name}"))).unwrap();
        assert_eq!((doc.kind, doc.flows), (DocumentKind::Charles, 5), "{name}");
        assert_charles_flows(&engine, doc.store);

        // Saved as a session and opened again, nothing changes.
        let saved = dir.join(format!("{name}.proxymate"));
        save(&engine, doc.store, &saved);
        let reopened = engine.open_document(&saved).unwrap();
        assert_eq!(reopened.kind, DocumentKind::Session);
        assert_charles_flows(&engine, reopened.store);
        for id in 0..5 {
            assert_eq!(
                engine.flow_detail_in(doc.store, id).unwrap(),
                engine.flow_detail_in(reopened.store, id).unwrap(),
                "{name} flow {id}"
            );
        }
    }
    // The JSON and XML exports of one session read the same.
    let json = engine.open_document(&fixture("charles/session.chlsj")).unwrap();
    let xml = engine.open_document(&fixture("charles/session.chlsx")).unwrap();
    for id in [0, 1, 2] {
        let (a, b) = (
            engine.flow_detail_in(json.store, id).unwrap().unwrap(),
            engine.flow_detail_in(xml.store, id).unwrap().unwrap(),
        );
        assert_eq!(a.request, b.request, "flow {id}");
        assert_eq!(a.response, b.response, "flow {id}");
        assert_eq!(
            engine
                .body_head_in(json.store, id, BodyPart::Response, false, None)
                .unwrap(),
            engine
                .body_head_in(xml.store, id, BodyPart::Response, false, None)
                .unwrap()
        );
    }
}

#[test]
fn binary_charles_sessions_say_how_to_export() {
    let engine = Engine::new();
    let err = engine
        .open_document(&fixture("charles/session.chls"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("Export Session") && err.contains(".chlsj"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_petstore_spec_labels_flows_and_its_mocks_answer() {
    let dir = temp_dir();
    let (engine, addr) = proxy(&dir).await;
    let spec = engine.import_openapi_file(&fixture("openapi/petstore.yaml")).unwrap();
    assert_eq!(
        (spec.name.as_str(), spec.openapi.as_str(), spec.operations),
        ("Swagger Petstore", "3.0.0", 3)
    );
    let rules = engine.generate_openapi_mocks(spec.id).unwrap();
    let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "listPets · Swagger Petstore",
            "createPets · Swagger Petstore",
            "showPetById · Swagger Petstore"
        ]
    );
    assert!(rules.iter().all(|r| !r.enabled), "disabled until reviewed");
    // Every request below is answered by a mock: nothing dials out.
    let list = "http://petstore.swagger.io/v1/pets?limit=2";
    for rule in &rules {
        engine.set_rule_enabled(&rule.id, true).unwrap();
    }
    let got = send(addr, "GET", list, "").await;
    assert_eq!(got.status, 200);
    assert_eq!(header(&got, "content-type"), Some("application/json"));
    let pets: serde_json::Value = serde_json::from_slice(&got.body).unwrap();
    assert_eq!(pets[1]["name"], "Tom");
    let got = send(addr, "GET", "http://petstore.swagger.io/v1/pets/7", "").await;
    let pet: serde_json::Value = serde_json::from_slice(&got.body).unwrap();
    assert_eq!(pet, serde_json::json!({"id": 0, "name": "string"}));
    let got = send(addr, "POST", "http://petstore.swagger.io/v1/pets", "{}").await;
    assert_eq!((got.status, got.body.len()), (201, 0));

    // Operations label the flows, and `op:` finds them.
    settled(&engine, 3).await;
    let page = engine.find_flows("op:showPetById", None, None, 10, true).unwrap();
    assert_eq!(page.flows.len(), 1);
    assert_eq!(page.flows[0].path, "/v1/pets/7");
    let op = engine.flow_operation_in(LIVE_STORE, page.flows[0].id).unwrap().unwrap();
    assert_eq!(op.operation_id.as_deref(), Some("showPetById"));
    assert_eq!(op.summary.as_deref(), Some("Info for a specific pet"));
    assert_eq!((op.method.as_str(), op.path.as_str()), ("GET", "/pets/{petId}"));
    assert_eq!(
        engine.find_flows("op:list*", None, None, 10, true).unwrap().flows.len(),
        1
    );
    assert_eq!(
        engine
            .find_flows("op:* -op:listPets", None, None, 10, true)
            .unwrap()
            .flows
            .len(),
        2
    );
    assert!(
        engine.find_flows("op:", None, None, 10, true).is_err(),
        "strict parse wants a value"
    );
    // The search box (lenient) binds too.
    let view = engine.create_view(
        FilterSpec {
            text: "op:createPets".into(),
            ..Default::default()
        },
        SortSpec::default(),
    );
    assert_eq!(view.len, 1);

    // Removing the spec unlabels flows; its rules stay.
    engine.remove_openapi_spec(spec.id).unwrap();
    assert!(engine.openapi_specs().is_empty());
    assert_eq!(engine.flow_operation_in(LIVE_STORE, page.flows[0].id).unwrap(), None);
    assert!(
        engine
            .find_flows("op:*", None, None, 10, true)
            .unwrap()
            .flows
            .is_empty()
    );
    assert_eq!(engine.rules().iter().filter(|r| r.name.contains("Petstore")).count(), 3);
}

#[test]
fn openapi_imports_report_what_is_wrong() {
    let engine = Engine::new();
    let err = engine.import_openapi("x", "swagger: '2.0'\n").unwrap_err().to_string();
    assert!(err.contains("Swagger 2.0"), "{err}");
    let err = engine.import_openapi("x", "{ not json").unwrap_err().to_string();
    assert!(err.contains("not valid JSON"), "{err}");
    let spec = engine
        .import_openapi(
            "fallback",
            r#"{"openapi":"3.1.0","info":{"version":"2"},"servers":[{"url":"/api"}],"paths":{"/a":{"get":{}}}}"#,
        )
        .unwrap();
    assert_eq!((spec.name.as_str(), spec.operations), ("fallback", 1));
    let rules = engine.generate_openapi_mocks(spec.id).unwrap();
    assert_eq!(rules[0].name, "GET /a · fallback");
    assert_eq!(rules[0].matcher.host, None, "a relative server names no host");
}
