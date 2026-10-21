//! End-to-end: client → Proxymate → pm-testkit upstream, checking what the
//! client gets and what the engine records.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::Request;
use http_body_util::{BodyExt, Full};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{BodyPart, ContentKind, FlowDetail, FlowId, FlowState, FlowSummary};
use pm_testkit::client::{self, eventually};
use pm_testkit::upstream::{self, CHUNKS, JSON_BODY};
use pm_testkit::{RawUpstream, Upstream};

const WAIT: Duration = Duration::from_secs(10);

async fn proxy() -> (Arc<Engine>, SocketAddr) {
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(
            std::env::temp_dir()
                .join("proxymate-tests")
                .join(uuid::Uuid::new_v4().to_string()),
        ),
        ..Default::default()
    });
    let config = ProxyConfig {
        ports: 0..=0,
        ..Default::default()
    };
    let addr = engine.start_proxy(config).await.expect("proxy starts");
    (engine, addr)
}

/// Waits for flow `id` to finish and returns its summary and detail.
async fn finished(engine: &Engine, id: FlowId) -> (FlowSummary, FlowDetail) {
    let summary = eventually(WAIT, || {
        engine.flush();
        engine.get_flow(id).filter(|f| f.state != FlowState::Active)
    })
    .await;
    let detail = engine.flow_detail(id).unwrap().expect("detail");
    (summary, detail)
}

fn header<'a>(headers: &'a [pm_model::Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

#[tokio::test]
async fn json_flow_is_recorded_with_heads_and_body() {
    let upstream = Upstream::start().await;
    let (engine, proxy) = proxy().await;

    let (status, body) = client::get(proxy, &upstream.url("/json?page=2")).await;
    assert_eq!(status, 200);
    assert_eq!(body, JSON_BODY.as_bytes());

    let (flow, detail) = finished(&engine, 0).await;
    assert_eq!(flow.state, FlowState::Completed);
    assert_eq!(flow.status, Some(200));
    assert_eq!(&*flow.method, "GET");
    assert_eq!(&*flow.host, "127.0.0.1");
    assert_eq!(flow.port, upstream.addr().port());
    assert_eq!((flow.path.as_str(), flow.query.as_str()), ("/json", "page=2"));
    assert_eq!(flow.content_kind, ContentKind::Json);
    assert_eq!(flow.resp_size, JSON_BODY.len() as u64);
    assert!(flow.duration_ms.is_some());

    assert_eq!(detail.request.url, upstream.url("/json?page=2"));
    assert_eq!(
        header(&detail.request.headers, "host"),
        Some(&*upstream.addr().to_string())
    );
    let response = detail.response.expect("response head");
    assert_eq!((response.status, response.reason.as_str()), (200, "OK"));
    assert_eq!(header(&response.headers, "content-type"), Some("application/json"));
    assert!(detail.response_body.complete);
    assert_eq!(detail.response_body.size, JSON_BODY.len() as u64);

    let recorded = engine.body(0, BodyPart::Response, true).unwrap();
    assert_eq!(recorded.bytes, JSON_BODY.as_bytes());
    assert_eq!(recorded.decoded_from, None);
}

#[tokio::test]
async fn status_codes_pass_through_and_are_recorded() {
    let upstream = Upstream::start().await;
    let (engine, proxy) = proxy().await;
    let codes = [200, 201, 204, 301, 304, 400, 404, 418, 500, 503];
    for code in codes {
        let (status, body) = client::get(proxy, &upstream.url(&format!("/status/{code}"))).await;
        assert_eq!(status, code);
        if !matches!(code, 204 | 304) {
            assert_eq!(body, format!("status {code}\n"));
        }
    }
    for (id, code) in codes.into_iter().enumerate() {
        let (flow, detail) = finished(&engine, id as FlowId).await;
        assert_eq!(
            (flow.state, flow.status),
            (FlowState::Completed, Some(code)),
            "flow {id}"
        );
        assert_eq!(detail.response.unwrap().status, code);
    }
}

#[tokio::test]
async fn head_requests_complete_without_a_body() {
    let upstream = Upstream::start().await;
    let (engine, proxy) = proxy().await;
    let req = Request::head(upstream.url("/json")).body(Full::default()).unwrap();
    let response = client::send(proxy, req).await;
    assert_eq!(response.status(), 200);
    let (flow, _) = finished(&engine, 0).await;
    assert_eq!((flow.state, flow.resp_size), (FlowState::Completed, 0));
}

#[tokio::test]
async fn chunked_bodies_are_forwarded_and_recorded_whole() {
    let upstream = Upstream::start().await;
    let (engine, proxy) = proxy().await;
    let (status, body) = client::get(proxy, &upstream.url("/chunked")).await;
    assert_eq!(status, 200);
    let expected = CHUNKS.concat();
    assert_eq!(body, expected.as_bytes());

    let (flow, detail) = finished(&engine, 0).await;
    assert_eq!(flow.resp_size, expected.len() as u64);
    // The recorded head is what the upstream sent, before hop-by-hop stripping.
    let response = detail.response.unwrap();
    assert_eq!(header(&response.headers, "transfer-encoding"), Some("chunked"));
    assert_eq!(
        engine.body(0, BodyPart::Response, false).unwrap().bytes,
        expected.as_bytes()
    );
}

#[tokio::test]
async fn encoded_bodies_are_stored_encoded_and_decoded_on_demand() {
    let upstream = Upstream::start().await;
    let (engine, proxy) = proxy().await;
    for (id, coding) in ["gzip", "br", "deflate", "zstd"].into_iter().enumerate() {
        let (status, body) = client::get(proxy, &upstream.url(&format!("/{coding}"))).await;
        assert_eq!(status, 200);
        // The proxy never touches the encoding on the wire.
        let encoded = upstream::encode(coding, JSON_BODY.as_bytes());
        assert_eq!(body, encoded, "{coding}");

        let id = id as FlowId;
        let (flow, detail) = finished(&engine, id).await;
        assert_eq!(flow.resp_size, encoded.len() as u64);
        assert_eq!(detail.response_body.encoding.as_deref(), Some(coding));
        assert_eq!(engine.body(id, BodyPart::Response, false).unwrap().bytes, encoded);
        let decoded = engine.body(id, BodyPart::Response, true).unwrap();
        assert_eq!(decoded.bytes, JSON_BODY.as_bytes(), "{coding}");
        assert_eq!(decoded.decoded_from.as_deref(), Some(coding));
    }
}

#[tokio::test]
async fn request_bodies_are_recorded() {
    let upstream = Upstream::start().await;
    let (engine, proxy) = proxy().await;
    let payload = r#"{"query":"proxymate"}"#;
    let req = Request::post(upstream.url("/echo"))
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from_static(payload.as_bytes())))
        .unwrap();
    let response = client::send(proxy, req).await;
    let echoed = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(echoed, payload.as_bytes());

    let (flow, detail) = finished(&engine, 0).await;
    assert_eq!((&*flow.method, flow.req_size), ("POST", payload.len() as u64));
    assert!(detail.request_body.complete);
    assert_eq!(
        engine.body(0, BodyPart::Request, false).unwrap().bytes,
        payload.as_bytes()
    );
    assert_eq!(
        engine.body(0, BodyPart::Response, false).unwrap().bytes,
        payload.as_bytes()
    );
}

#[tokio::test]
async fn header_case_is_preserved_both_ways() {
    let upstream = RawUpstream::start(
        b"HTTP/1.1 200 OK\r\nX-Upstream-MiXeD: 1\r\ncontent-TYPE: text/plain\r\nContent-Length: 2\r\n\r\nok",
    )
    .await;
    let (engine, proxy) = proxy().await;
    let request = format!(
        "GET http://{0}/case HTTP/1.1\r\nHost: {0}\r\nX-Custom-CASE: yes\r\nuser-agent: lower\r\nConnection: close\r\n\r\n",
        upstream.addr()
    );
    let response = String::from_utf8(client::raw(proxy, request.as_bytes()).await).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.contains("\r\nX-Upstream-MiXeD: 1\r\n"), "{response}");
    assert!(response.contains("\r\ncontent-TYPE: text/plain\r\n"), "{response}");
    assert!(response.ends_with("\r\n\r\nok"), "{response}");

    let heads = upstream.heads().await;
    let head = String::from_utf8(heads[0].clone()).unwrap();
    assert!(
        head.starts_with("GET /case HTTP/1.1\r\n"),
        "origin-form upstream: {head}"
    );
    assert!(head.contains("\r\nX-Custom-CASE: yes\r\n"), "{head}");
    assert!(head.contains("\r\nuser-agent: lower\r\n"), "{head}");
    assert!(head.contains(&format!("\r\nHost: {}\r\n", upstream.addr())), "{head}");

    let (flow, _) = finished(&engine, 0).await;
    assert_eq!(flow.content_kind, ContentKind::Text);
}

#[tokio::test]
async fn unreachable_upstreams_fail_the_flow_with_502() {
    // Bind and drop to get a port nothing listens on.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (engine, proxy) = proxy().await;
    let (status, body) = client::get(proxy, &format!("http://127.0.0.1:{port}/")).await;
    assert_eq!(status, 502);
    assert!(String::from_utf8_lossy(&body).contains("upstream error"));
    let (flow, detail) = finished(&engine, 0).await;
    assert_eq!((flow.state, flow.status), (FlowState::Failed, None));
    assert!(detail.error.unwrap().to_lowercase().contains("refused"));
}

#[tokio::test]
async fn requests_to_the_proxy_itself_get_the_info_page() {
    let (engine, proxy) = proxy().await;
    let (status, body) = client::get(proxy, &format!("http://{proxy}/")).await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("Proxymate is running"));
    let origin_form = client::raw(proxy, b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert!(String::from_utf8_lossy(&origin_form).contains("Proxymate is running"));
    engine.flush();
    assert_eq!(engine.total(), 0);
}

#[tokio::test]
async fn busy_ports_fall_back_to_the_next_one() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let engine = Engine::new();
    let config = ProxyConfig {
        ports: port..=port.saturating_add(20),
        ..Default::default()
    };
    let addr = engine.start_proxy(config).await.unwrap();
    assert!(addr.port() > port && addr.port() <= port.saturating_add(20));
    assert_eq!(engine.proxy_status().listening, Some(addr.to_string()));

    // With every port taken, the status explains why.
    let other = Engine::new();
    let config = ProxyConfig {
        ports: port..=port,
        ..Default::default()
    };
    assert!(other.start_proxy(config).await.is_err());
    assert!(other.proxy_status().error.unwrap().contains(&port.to_string()));
}

#[tokio::test]
async fn paused_recording_forwards_without_recording() {
    let upstream = Upstream::start().await;
    let (engine, proxy) = proxy().await;
    engine.set_recording(false);
    let (status, _) = client::get(proxy, &upstream.url("/json")).await;
    assert_eq!(status, 200);
    engine.flush();
    assert_eq!(engine.total(), 0);
    assert!(!engine.proxy_status().recording);
}

#[tokio::test]
async fn clear_drops_flows_and_bodies() {
    let upstream = Upstream::start().await;
    let (engine, proxy) = proxy().await;
    client::get(proxy, &upstream.url("/bytes/100000")).await;
    finished(&engine, 0).await;
    assert_eq!(engine.body(0, BodyPart::Response, false).unwrap().bytes.len(), 100_000);

    engine.clear();
    assert_eq!(engine.total(), 0);
    assert!(engine.flow_detail(0).unwrap().is_none());
    assert!(engine.body(0, BodyPart::Response, false).is_err());

    // New flows get new ids.
    client::get(proxy, &upstream.url("/json")).await;
    let (flow, _) = finished(&engine, 1).await;
    assert_eq!(flow.id, 1);
}
