//! Every body-viewer zoo fixture through the proxy: the detected kind lands
//! in the summary (driving the filter chips) and in the body info.

use std::sync::Arc;
use std::time::Duration;

use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{BodyPart, ContentGroup, ContentKind, FilterSpec, FlowState, SortSpec};
use pm_testkit::Upstream;
use pm_testkit::client::eventually;
use pm_testkit::zoo;

const WAIT: Duration = Duration::from_secs(20);

#[tokio::test]
async fn every_zoo_fixture_is_detected() {
    let upstream = Upstream::start().await;
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(
            std::env::temp_dir()
                .join("proxymate-tests")
                .join(uuid::Uuid::new_v4().to_string()),
        ),
        ..Default::default()
    });
    let proxy = engine
        .start_proxy(ProxyConfig {
            ports: 0..=0,
            ..Default::default()
        })
        .await
        .unwrap();

    let sent = zoo::send_all(proxy, upstream.addr()).await;
    let fixtures = zoo::fixtures();
    assert_eq!(sent.len(), fixtures.len());
    let all_done = |engine: &Arc<Engine>| {
        engine.flush();
        (0..fixtures.len() as u32).all(|id| engine.get_flow(id).is_some_and(|f| f.state != FlowState::Active))
    };
    eventually(WAIT, || all_done(&engine).then_some(())).await;

    for (id, fixture) in fixtures.iter().enumerate() {
        let id = id as u32;
        let flow = engine.get_flow(id).unwrap();
        assert_eq!(flow.state, FlowState::Completed, "{}", fixture.name);
        assert_eq!(flow.path, fixture.path());
        let kind = serde_json::to_value(flow.content_kind).unwrap();
        assert_eq!(kind, fixture.kind.as_str(), "{}: flow kind", fixture.name);
        assert_eq!(
            flow.graphql_op.as_deref(),
            fixture.graphql_op.as_deref(),
            "{}",
            fixture.name
        );

        let detail = engine.flow_detail(id).unwrap().unwrap();
        if fixture.request.is_none() {
            assert_eq!(
                detail.response_body.kind, flow.content_kind,
                "{}: response kind",
                fixture.name
            );
            // The decoded body is what the fixture holds.
            let body = engine.body(id, BodyPart::Response, true).unwrap();
            let expected = match &fixture.encoding {
                Some(_) => zoo::file_bytes(fixture.file.as_deref().unwrap()).to_vec(),
                None => fixture.response_body(),
            };
            assert_eq!(body.bytes.len(), expected.len(), "{}", fixture.name);
        } else {
            assert_eq!(
                detail.request_body.kind, flow.content_kind,
                "{}: request kind",
                fixture.name
            );
            assert_eq!(detail.response_body.kind, ContentKind::Json, "{}", fixture.name);
        }
    }

    let mime = |name: &str| {
        let id = fixtures.iter().position(|f| f.name == name).unwrap() as u32;
        engine.flow_detail(id).unwrap().unwrap().response_body.mime
    };
    // Mislabelled and headerless bodies follow their bytes.
    assert_eq!(mime("png-mislabelled").as_deref(), Some("image/png"));
    assert_eq!(mime("json-headerless").as_deref(), Some("application/json"));
    assert_eq!(mime("webp").as_deref(), Some("image/webp"));
    assert_eq!(mime("gif-animated").as_deref(), Some("image/gif"));
    assert_eq!(mime("font").as_deref(), Some("font/woff2"));

    // The chips filter on the detected kind.
    let chip = |group| {
        let filter = FilterSpec {
            content: vec![group],
            ..Default::default()
        };
        engine.create_view(filter, SortSpec::default()).len
    };
    assert_eq!(chip(ContentGroup::GraphQl), 1);
    assert_eq!(chip(ContentGroup::Form), 2);
    assert_eq!(chip(ContentGroup::Json), 4);
    assert_eq!(chip(ContentGroup::Media), 12);
    assert_eq!(chip(ContentGroup::Document), 3);
}

#[tokio::test]
async fn huge_bodies_can_be_read_from_the_head() {
    let upstream = Upstream::start().await;
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(
            std::env::temp_dir()
                .join("proxymate-tests")
                .join(uuid::Uuid::new_v4().to_string()),
        ),
        ..Default::default()
    });
    let proxy = engine
        .start_proxy(ProxyConfig {
            ports: 0..=0,
            ..Default::default()
        })
        .await
        .unwrap();
    pm_testkit::client::get(proxy, &upstream.url("/zoo/json-10mb")).await;
    pm_testkit::client::get(proxy, &upstream.url("/zoo/json-gzip")).await;
    eventually(WAIT, || {
        engine.flush();
        engine.get_flow(1).filter(|f| f.state == FlowState::Completed)
    })
    .await;

    let head = engine.body_head(0, BodyPart::Response, true, Some(1024)).unwrap();
    assert_eq!(head.bytes.len(), 1024);
    assert!(head.cut);
    assert!(head.bytes.starts_with(br#"{"generated":"proxymate zoo""#));
    let whole = engine
        .body_head(0, BodyPart::Response, true, Some(usize::MAX - 1))
        .unwrap();
    assert!(!whole.cut);
    assert_eq!(whole.bytes.len(), zoo::big_json(zoo::BIG_JSON_LEN).len());

    // Decoded heads come from the start of the decoded body.
    let decoded = engine.body_head(1, BodyPart::Response, true, Some(10)).unwrap();
    assert_eq!(decoded.decoded_from.as_deref(), Some("gzip"));
    assert_eq!(decoded.bytes, &zoo::file_bytes("nested.json")[..10]);
    assert!(decoded.cut);
}
