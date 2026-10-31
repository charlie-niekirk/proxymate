//! `Engine::send`, Replay, Edit & Replay and Copy as cURL, against
//! pm-testkit upstreams.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::Request;
use http_body_util::{BodyExt, Full};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{
    Action, BodyPart, FlowDetail, FlowId, FlowState, FlowSummary, Header, LIVE_STORE, Matcher, Phase, RequestSpec,
    Rule, RuleOrigin, SendOpts, SendOrigin, SentBy, TlsStatus,
};
use pm_testkit::client::{self, eventually};
use pm_testkit::upstream::{JSON_BODY, encode};
use pm_testkit::{Alpn, TlsUpstream, Upstream};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(10);

fn engine(upstream: Option<&TlsUpstream>) -> Arc<Engine> {
    let dir: PathBuf = std::env::temp_dir()
        .join("proxymate-tests")
        .join(uuid::Uuid::new_v4().to_string());
    Engine::with_config(EngineConfig {
        data_dir: Some(dir),
        extra_upstream_roots: upstream.map(|u| vec![u.ca_der()]).unwrap_or_default(),
        ..Default::default()
    })
}

async fn proxy(engine: &Arc<Engine>) -> SocketAddr {
    engine
        .start_proxy(ProxyConfig {
            ports: 0..=0,
            ..Default::default()
        })
        .await
        .expect("proxy starts")
}

fn wait() -> SendOpts {
    SendOpts {
        apply_rules: true,
        wait: true,
        origin: SendOrigin::User,
    }
}

async fn finished(engine: &Engine, id: FlowId) -> (FlowSummary, FlowDetail) {
    let summary = eventually(WAIT, || {
        engine.flush();
        engine
            .get_flow(id)
            .filter(|f| !matches!(f.state, FlowState::Active | FlowState::Paused))
    })
    .await;
    (summary, engine.flow_detail(id).unwrap().expect("detail"))
}

fn response_text(engine: &Engine, id: FlowId) -> String {
    String::from_utf8(engine.body(id, BodyPart::Response, true).unwrap().bytes).unwrap()
}

fn inspect(engine: &Engine, id: FlowId) -> Value {
    serde_json::from_str(&response_text(engine, id)).expect("an /inspect answer")
}

fn spec(method: &str, url: String, headers: &[(&str, &str)], body: Option<&str>) -> RequestSpec {
    RequestSpec {
        method: method.into(),
        url,
        headers: headers.iter().map(|(n, v)| Header::new(*n, *v)).collect(),
        body: body.map(str::to_owned),
        body_base64: false,
    }
}

#[tokio::test]
async fn sends_are_recorded_and_marked() {
    let upstream = Upstream::start().await;
    let engine = engine(None);
    let body = r#"{"hello":"world"}"#;
    let id = engine
        .send(
            spec(
                "post",
                upstream.url("/inspect?x=1"),
                &[("content-type", "application/json"), ("content-length", "999")],
                Some(body),
            ),
            wait(),
        )
        .await
        .unwrap();
    // `wait` resolves once the summary is final.
    let summary = engine.get_flow(id).unwrap();
    assert_eq!(summary.state, FlowState::Completed);
    assert_eq!((summary.status, summary.sent_by), (Some(200), Some(SentBy::User)));
    assert_eq!(summary.client.as_deref(), Some(pm_core::proxy::APP_CLIENT));
    assert_eq!((summary.method.as_ref(), summary.query.as_str()), ("POST", "x=1"));
    let seen = inspect(&engine, id);
    assert_eq!(seen["method"], "POST");
    assert_eq!(seen["body"], body);
    // Content-Length is the body's, whatever the spec said.
    assert_eq!(seen["headers"]["content-length"], body.len().to_string());
    let detail = engine.flow_detail(id).unwrap().unwrap();
    assert_eq!(
        engine.body(id, BodyPart::Request, false).unwrap().bytes,
        body.as_bytes()
    );
    assert_eq!(detail.request.url, upstream.url("/inspect?x=1"));

    // Agents are told apart, and sends are recorded while recording is paused.
    engine.set_recording(false);
    let opts = SendOpts {
        origin: SendOrigin::Agent {
            client: "claude-code".into(),
        },
        ..wait()
    };
    let id = engine
        .send(spec("GET", upstream.url("/json"), &[], None), opts)
        .await
        .unwrap();
    let summary = engine.get_flow(id).unwrap();
    assert_eq!(summary.sent_by, Some(SentBy::Agent));
    assert_eq!(summary.client.as_deref(), Some("claude-code"));
    assert_eq!(response_text(&engine, id), JSON_BODY);
}

#[tokio::test]
async fn rules_apply_only_when_asked() {
    let upstream = Upstream::start().await;
    let engine = engine(None);
    engine
        .create_rule(Rule {
            id: String::new(),
            name: "tag".into(),
            enabled: true,
            order: 0,
            origin: RuleOrigin::User,
            matcher: Matcher::default(),
            actions: vec![Action::SetHeader {
                phase: Phase::Request,
                name: "x-tagged".into(),
                value: "yes".into(),
            }],
        })
        .unwrap();
    let url = upstream.url("/inspect");

    let with = engine.send(spec("GET", url.clone(), &[], None), wait()).await.unwrap();
    assert_eq!(inspect(&engine, with)["headers"]["x-tagged"], "yes");
    let (summary, detail) = finished(&engine, with).await;
    assert!(summary.edited);
    assert_eq!(detail.rules.applied.len(), 1);

    let opts = SendOpts {
        apply_rules: false,
        ..wait()
    };
    let without = engine.send(spec("GET", url, &[], None), opts).await.unwrap();
    assert!(inspect(&engine, without)["headers"].get("x-tagged").is_none());
    assert!(!engine.get_flow(without).unwrap().edited);
}

#[tokio::test]
async fn without_wait_the_flow_finishes_later() {
    let upstream = Upstream::start().await;
    let engine = engine(None);
    let opts = SendOpts { wait: false, ..wait() };
    let id = engine
        .send(spec("GET", upstream.url("/json"), &[], None), opts)
        .await
        .unwrap();
    // In the store by the time the call returns, finished or not.
    assert!(engine.get_flow(id).is_some());
    let (summary, _) = finished(&engine, id).await;
    assert_eq!(summary.status, Some(200));
    assert_eq!(response_text(&engine, id), JSON_BODY);
}

#[tokio::test]
async fn https_sends_verify_the_upstream() {
    let tls = TlsUpstream::start(Alpn::Both).await;
    let engine = engine(Some(&tls));
    let id = engine
        .send(spec("GET", tls.url("/json"), &[], None), wait())
        .await
        .unwrap();
    let summary = engine.get_flow(id).unwrap();
    assert_eq!((summary.status, summary.tls), (Some(200), TlsStatus::Intercepted));
    assert_eq!(response_text(&engine, id), JSON_BODY);

    // An upstream we don't trust fails the flow, not the call.
    let untrusted = TlsUpstream::self_signed(Alpn::Http11).await;
    let id = engine
        .send(spec("GET", untrusted.url("/json"), &[], None), wait())
        .await
        .unwrap();
    let (summary, detail) = finished(&engine, id).await;
    assert_eq!(summary.state, FlowState::Failed);
    assert!(detail.error.is_some());
}

#[tokio::test]
async fn bad_specs_are_errors() {
    let engine = engine(None);
    for (method, url) in [
        ("GE T", "http://localhost/"),
        ("GET", "ftp://localhost/"),
        // A WebSocket handshake is a GET.
        ("POST", "ws://localhost/socket"),
        ("GET", "/relative"),
        ("CONNECT", "http://localhost:443/"),
    ] {
        let err = engine.send(spec(method, url.into(), &[], None), wait()).await;
        assert!(err.is_err(), "{method} {url}");
    }
    let mut binary = spec("POST", "http://localhost/".into(), &[], Some("not base64!"));
    binary.body_base64 = true;
    assert!(engine.send(binary, wait()).await.is_err());
    assert_eq!(engine.total(), 0);
}

/// A gzipped POST through the proxy, replayed as is and as an edit.
#[tokio::test]
async fn replay_and_edit_and_replay() {
    let upstream = Upstream::start().await;
    let engine = engine(None);
    let addr = proxy(&engine).await;
    let text = r#"{"note":"it's \"here\""}"#;
    let request = Request::post(upstream.url("/inspect"))
        .header("content-type", "application/json")
        .header("content-encoding", "gzip")
        .header("x-trace", "abc")
        .body(Full::new(Bytes::from(encode("gzip", text.as_bytes()))))
        .unwrap();
    let response = client::send(addr, request).await;
    let _ = response.into_body().collect().await.unwrap();
    let original = eventually(WAIT, || {
        engine.flush();
        engine.get_flow(0).filter(|f| f.state == FlowState::Completed)
    })
    .await;
    assert_eq!(original.sent_by, None);

    // Replay sends the recorded bytes, still gzipped.
    let replayed = engine.replay(LIVE_STORE, 0, wait()).await.unwrap();
    let seen = inspect(&engine, replayed);
    assert_eq!(seen["headers"]["content-encoding"], "gzip");
    assert_eq!(seen["headers"]["x-trace"], "abc");
    assert_eq!(
        engine.body(replayed, BodyPart::Request, true).unwrap().bytes,
        text.as_bytes()
    );

    // Edit & Replay gets the body decoded, without the encoding header.
    let editable = engine.request_spec(LIVE_STORE, 0, true).unwrap();
    assert_eq!(editable.body.as_deref(), Some(text));
    assert!(!editable.body_base64);
    let names: Vec<&str> = editable.headers.iter().map(|h| h.name.as_str()).collect();
    assert!(!names.contains(&"content-encoding") && !names.contains(&"host") && !names.contains(&"content-length"));
    let raw = engine.request_spec(LIVE_STORE, 0, false).unwrap();
    assert!(raw.body_base64);

    let mut edited = editable;
    edited.method = "PUT".into();
    edited.body = Some(r#"{"note":"edited"}"#.into());
    let id = engine.send(edited, wait()).await.unwrap();
    let seen = inspect(&engine, id);
    assert_eq!(
        (seen["method"].as_str(), seen["body"].as_str()),
        (Some("PUT"), Some(r#"{"note":"edited"}"#))
    );
    assert!(seen["headers"].get("content-encoding").is_none());
}

/// The copied command, run by a shell, gets the response the flow got.
#[tokio::test]
async fn copied_curl_commands_send_the_same_request() {
    if std::process::Command::new("curl").arg("--version").output().is_err() {
        eprintln!("curl isn't installed; skipping");
        return;
    }
    let upstream = Upstream::start().await;
    let engine = engine(None);
    let addr = proxy(&engine).await;
    type Case<'a> = (&'a str, Vec<(&'a str, &'a str)>, Vec<u8>);
    let cases: Vec<Case> = vec![
        (
            "GET",
            vec![("x-quote", "it's \"$HOME\" `id`"), ("accept-encoding", "gzip, br")],
            Vec::new(),
        ),
        (
            "POST",
            vec![("content-type", "text/plain")],
            "line one\nit's line two $PATH\n\ttabbed".as_bytes().to_vec(),
        ),
        (
            "PATCH",
            vec![("content-type", "application/octet-stream")],
            vec![0, 1, 2, 39, 10, 255, 254],
        ),
    ];
    for (i, (method, headers, body)) in cases.into_iter().enumerate() {
        let mut request = Request::builder()
            .method(method)
            .uri(upstream.url("/inspect?q=a%20b&r=[1]"));
        for (name, value) in &headers {
            request = request.header(*name, *value);
        }
        let response = client::send(addr, request.body(Full::new(Bytes::from(body))).unwrap()).await;
        let _ = response.into_body().collect().await.unwrap();
        let id = i as FlowId;
        let (summary, _) = finished(&engine, id).await;
        assert_eq!(summary.status, Some(200));
        let recorded = inspect(&engine, id);

        let command = engine.curl_command(LIVE_STORE, id).unwrap();
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{command} --silent --show-error"))
            .env("HTTP_PROXY", "")
            .env("http_proxy", "")
            .env("ALL_PROXY", "")
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{command}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let replayed: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("{command}\n{e}: {}", String::from_utf8_lossy(&output.stdout)));
        for key in ["method", "uri", "body", "body_len"] {
            assert_eq!(replayed[key], recorded[key], "{key} of {command}");
        }
        for (name, value) in headers
            .iter()
            .filter(|(n, _)| n.starts_with("x-") || *n == "content-type")
        {
            assert_eq!(replayed["headers"][*name], *value, "{name} of {command}");
        }
    }
}
