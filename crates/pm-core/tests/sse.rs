//! Server-sent events: an event stream is parsed while it is still
//! streaming, so events show up before the response ends.

use std::time::Duration;

use http::Request;
use http_body_util::{BodyExt, Full};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{FlowState, SsePage};
use pm_testkit::Upstream;
use pm_testkit::client::{self, eventually};
use pm_testkit::stream::sse_event;

const WAIT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn events_are_recorded_incrementally_before_the_stream_ends() {
    let upstream = Upstream::start().await;
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
        .unwrap();

    // Six events, 300 ms apart: the stream stays open for almost 2 s.
    let req = Request::get(upstream.url("/sse?count=6&interval_ms=300"))
        .body(Full::default())
        .unwrap();
    let response = client::send(addr, req).await;
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body();
    let reader = tokio::spawn(async move {
        let mut received = Vec::new();
        while let Some(frame) = body.frame().await {
            if let Some(data) = frame.unwrap().data_ref() {
                received.extend_from_slice(data);
            }
        }
        received
    });

    let page = |engine: &Engine| engine.sse_events(0, "", 0, 100).expect("events");
    let early: SsePage = eventually(WAIT, || Some(page(&engine)).filter(|p| p.total >= 2)).await;
    assert!(early.open, "events must arrive while the stream is open");
    assert!(early.total < 6);
    engine.flush();
    assert_eq!(engine.get_flow(0).unwrap().state, FlowState::Active);
    assert_eq!(early.events[0].id.as_deref(), Some("0"));
    assert_eq!(early.events[0].event.as_deref(), Some("tick"));
    assert_eq!(early.events[0].data, r#"{"n":0}"#);

    let received = reader.await.unwrap();
    let expected: String = (0..6).map(sse_event).collect();
    assert!(String::from_utf8(received).unwrap().ends_with(&expected));
    let done = eventually(WAIT, || Some(page(&engine)).filter(|p| !p.open)).await;
    assert_eq!(done.total, 6);
    let times: Vec<f64> = done
        .events
        .iter()
        .map(|e| e.time_ms.expect("live events are timed"))
        .collect();
    // The events were stamped as they arrived, ~300 ms apart.
    assert!(times.last().unwrap() - times[0] >= 1_000.0, "{times:?}");

    let search = engine.sse_events(0, r#""n":4"#, 0, 10).unwrap();
    assert_eq!((search.len, search.events[0].index), (1, 4));
}
