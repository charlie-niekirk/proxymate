//! The rules matrix end to end: client → Proxymate (rules) → pm-testkit
//! upstreams, over plain h1, MITM h1 and MITM h2.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http::{HeaderMap, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::{TokioExecutor, TokioIo};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{
    Action, BlockMode, BreakpointResolution, FlowDetail, FlowId, FlowState, FlowSummary, Header, MapLocalSource,
    Matcher, MessageEdits, Pattern, Phase, PresetKind, Rule, RuleOrigin, RuleSettings,
};
use pm_testkit::client::{self, eventually, tls_config, tls_through};
use pm_testkit::upstream::{JSON_BODY, pattern_byte};
use pm_testkit::{Alpn, RawUpstream, TlsUpstream, Upstream};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use serde_json::json;

const WAIT: Duration = Duration::from_secs(10);

struct Proxy {
    engine: Arc<Engine>,
    addr: SocketAddr,
    dir: PathBuf,
}

impl Proxy {
    async fn start(upstream: Option<&TlsUpstream>) -> Proxy {
        let dir = std::env::temp_dir()
            .join("proxymate-tests")
            .join(uuid::Uuid::new_v4().to_string());
        Proxy::start_in(dir, upstream).await
    }

    async fn start_in(dir: PathBuf, upstream: Option<&TlsUpstream>) -> Proxy {
        let engine = Engine::with_config(EngineConfig {
            data_dir: Some(dir.clone()),
            extra_upstream_roots: upstream.map(|u| vec![u.ca_der()]).unwrap_or_default(),
            ..Default::default()
        });
        let addr = engine
            .start_proxy(ProxyConfig {
                ports: 0..=0,
                ..Default::default()
            })
            .await
            .expect("proxy starts");
        Proxy { engine, addr, dir }
    }

    fn rule(&self, matcher: Matcher, actions: Vec<Action>) -> Rule {
        self.engine
            .create_rule(Rule {
                id: String::new(),
                name: "test rule".into(),
                enabled: true,
                order: 0,
                origin: RuleOrigin::User,
                matcher,
                actions,
            })
            .expect("rule is valid")
    }

    fn ca(&self) -> CertificateDer<'static> {
        CertificateDer::from_pem_file(self.engine.ca_info().cert_path).expect("CA PEM")
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

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

async fn send(proxy: SocketAddr, req: Request<Full<Bytes>>) -> Reply {
    let response = client::send(proxy, req).await;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.expect("body").to_bytes();
    Reply { status, headers, body }
}

async fn get(proxy: SocketAddr, url: &str) -> Reply {
    send(proxy, Request::get(url).body(Full::default()).unwrap()).await
}

/// One request over TLS through the proxy's MITM, in whichever protocol
/// `alpn` gets negotiated.
async fn https(proxy: &Proxy, url: &str, alpn: &[u8], req: Request<Full<Bytes>>) -> Reply {
    let uri: http::Uri = url.parse().unwrap();
    let tls = tls_through(
        proxy.addr,
        uri.host().unwrap(),
        uri.port_u16().unwrap(),
        tls_config(&proxy.ca(), &[alpn]),
    )
    .await
    .expect("TLS through the proxy");
    let io = TokioIo::new(tls);
    let (mut parts, body) = req.into_parts();
    let response = if alpn == b"h2" {
        let (mut sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), io)
            .await
            .unwrap();
        tokio::spawn(conn);
        parts.uri = uri;
        sender.send_request(Request::from_parts(parts, body)).await.unwrap()
    } else {
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
        tokio::spawn(conn);
        parts.uri = uri.path_and_query().unwrap().as_str().parse().unwrap();
        parts
            .headers
            .insert(http::header::HOST, uri.authority().unwrap().as_str().parse().unwrap());
        sender.send_request(Request::from_parts(parts, body)).await.unwrap()
    };
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.expect("body").to_bytes();
    Reply { status, headers, body }
}

fn on_path(path: &str) -> Matcher {
    Matcher {
        path: Some(Pattern::exact(path)),
        ..Matcher::default()
    }
}

fn header<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

#[tokio::test]
async fn map_local_serves_a_file_and_a_directory() {
    let proxy = Proxy::start(None).await;
    let site = proxy.dir.join("site");
    std::fs::create_dir_all(site.join("v1")).unwrap();
    std::fs::write(site.join("v1/users.json"), br#"[{"id":1}]"#).unwrap();
    std::fs::write(site.join("mock.txt"), b"from a file").unwrap();
    proxy.rule(
        on_path("/file"),
        vec![Action::MapLocal {
            source: MapLocalSource::File {
                path: site.join("mock.txt").display().to_string(),
            },
            status: 201,
            headers: vec![Header::new("x-mock", "yes")],
            content_type: None,
        }],
    );
    proxy.rule(
        Matcher {
            path: Some(Pattern::glob("/api/*")),
            ..Matcher::default()
        },
        vec![Action::MapLocal {
            source: MapLocalSource::Directory {
                root: site.display().to_string(),
            },
            status: 200,
            headers: Vec::new(),
            content_type: None,
        }],
    );
    // Nothing listens here: the upstream is never contacted.
    let dead = "http://127.0.0.1:9";

    let reply = get(proxy.addr, &format!("{dead}/file")).await;
    assert_eq!(reply.status, 201);
    assert_eq!(reply.body, "from a file");
    assert_eq!(reply.headers["x-mock"], "yes");
    assert_eq!(reply.headers["content-type"], "text/plain; charset=utf-8");
    assert_eq!(reply.headers["content-length"], "11");

    let reply = get(proxy.addr, &format!("{dead}/api/v1/users.json")).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, r#"[{"id":1}]"#);
    assert_eq!(reply.headers["content-type"], "application/json");

    let reply = get(proxy.addr, &format!("{dead}/api/v1/missing.json")).await;
    assert_eq!(reply.status, 404);
    assert!(String::from_utf8_lossy(&reply.body).contains("no file for /api/v1/missing.json"));

    let (flow, detail) = proxy.finished(0).await;
    assert_eq!(
        (flow.state, flow.status, flow.edited),
        (FlowState::Completed, Some(201), true)
    );
    assert!(detail.rules.mocked);
    assert_eq!(detail.rules.applied.len(), 1);
    assert_eq!(detail.rules.applied[0].name, "test rule");
    assert_eq!(
        proxy.engine.body(0, pm_model::BodyPart::Response, false).unwrap().bytes,
        b"from a file"
    );
}

#[tokio::test]
async fn json_set_rewrites_a_gzipped_response() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    proxy.rule(
        on_path("/gzip"),
        vec![
            Action::JsonSet {
                phase: Phase::Response,
                pointer: "/name".into(),
                value: json!("patched"),
                raw: None,
            },
            Action::JsonRemove {
                phase: Phase::Response,
                pointer: "/nested".into(),
            },
        ],
    );
    let reply = get(proxy.addr, &upstream.url("/gzip")).await;
    assert_eq!(reply.status, 200);
    assert!(
        reply.headers.get("content-encoding").is_none(),
        "re-sent identity-encoded"
    );
    assert_eq!(reply.headers["content-length"], reply.body.len().to_string());
    let doc: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
    let mut expected: serde_json::Value = serde_json::from_str(JSON_BODY).unwrap();
    expected["name"] = json!("patched");
    expected.as_object_mut().unwrap().remove("nested");
    assert_eq!(doc, expected);

    let (flow, detail) = proxy.finished(0).await;
    assert!(flow.edited);
    assert_eq!(detail.rules.applied[0].phase, Phase::Response);
    assert_eq!(detail.response_body.encoding, None);
    assert_eq!(
        proxy.engine.body(0, pm_model::BodyPart::Response, false).unwrap().bytes,
        reply.body
    );
}

#[tokio::test]
async fn set_status_and_header_rewrites_work_on_h2() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;
    proxy.rule(
        on_path("/json"),
        vec![
            Action::SetHeader {
                phase: Phase::Request,
                name: "x-added".into(),
                value: "request".into(),
            },
            Action::SetQuery {
                name: "page".into(),
                value: "3".into(),
            },
            Action::SetStatus { status: 418 },
            Action::SetHeader {
                phase: Phase::Response,
                name: "x-rewritten".into(),
                value: "yes".into(),
            },
            Action::RemoveHeader {
                phase: Phase::Response,
                name: "content-type".into(),
            },
        ],
    );
    let url = upstream.url("/json");
    let reply = https(&proxy, &url, b"h2", Request::get(&url).body(Full::default()).unwrap()).await;
    assert_eq!(reply.status, 418);
    assert_eq!(reply.headers["x-rewritten"], "yes");
    assert!(reply.headers.get("content-type").is_none());
    assert_eq!(reply.body, JSON_BODY);

    let (flow, detail) = proxy.finished(0).await;
    assert_eq!(flow.http_version, pm_model::HttpVersion::Http2);
    assert_eq!(flow.status, Some(418));
    assert!(flow.edited);
    assert_eq!(header(&detail.request.headers, "x-added"), Some("request"));
    assert!(detail.request.url.ends_with("/json?page=3"), "{}", detail.request.url);
    let phases: Vec<Phase> = detail.rules.applied.iter().map(|a| a.phase).collect();
    assert_eq!(phases, [Phase::Request, Phase::Response]);

    // Streams without a matching rule are untouched on the same proxy.
    let url = upstream.url("/version");
    let reply = https(&proxy, &url, b"h2", Request::get(&url).body(Full::default()).unwrap()).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, "HTTP/2.0");
}

#[tokio::test]
async fn map_local_works_inside_the_mitm() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;
    proxy.rule(
        on_path("/json"),
        vec![
            Action::MapLocal {
                source: MapLocalSource::Inline {
                    body: r#"{"mocked":true}"#.into(),
                },
                status: 200,
                headers: Vec::new(),
                content_type: Some("application/json".into()),
            },
            // Response-phase actions still run on Map Local's answer.
            Action::SetHeader {
                phase: Phase::Response,
                name: "x-after-local".into(),
                value: "1".into(),
            },
        ],
    );
    for alpn in [&b"h2"[..], b"http/1.1"] {
        let url = upstream.url("/json");
        let reply = https(&proxy, &url, alpn, Request::get(&url).body(Full::default()).unwrap()).await;
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body, r#"{"mocked":true}"#);
        assert_eq!(reply.headers["x-after-local"], "1");
    }
}

#[tokio::test]
async fn map_remote_sends_the_request_elsewhere() {
    let elsewhere = RawUpstream::start(b"HTTP/1.1 200 OK\r\ncontent-length: 9\r\n\r\nelsewhere").await;
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    proxy.rule(
        on_path("/json"),
        vec![Action::MapRemote {
            scheme: None,
            host: Some("127.0.0.1".into()),
            port: Some(elsewhere.addr().port()),
            path: Some("/moved".into()),
            preserve_host: false,
        }],
    );
    let reply = get(proxy.addr, &upstream.url("/json?x=1")).await;
    assert_eq!(reply.body, "elsewhere");
    let heads = elsewhere.heads().await;
    let head = String::from_utf8_lossy(&heads[0]).to_lowercase();
    assert!(head.starts_with("get /moved?x=1 http/1.1"), "{head}");
    assert!(
        head.contains(&format!("host: 127.0.0.1:{}", elsewhere.addr().port())),
        "{head}"
    );
}

#[tokio::test]
async fn a_breakpoint_resolved_programmatically_edits_the_request() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    proxy.rule(
        on_path("/echo"),
        vec![
            Action::SetHeader {
                phase: Phase::Request,
                name: "x-before-breakpoint".into(),
                value: "1".into(),
            },
            Action::Breakpoint {
                request: true,
                response: false,
            },
        ],
    );
    let url = upstream.url("/echo");
    let request = Request::post(&url)
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::from_static(b"original body")))
        .unwrap();
    let addr = proxy.addr;
    let reply = tokio::spawn(async move { send(addr, request).await });

    let paused = eventually(WAIT, || proxy.engine.breakpoints().into_iter().next()).await;
    assert_eq!(paused.phase, Phase::Request);
    assert_eq!(paused.flow_id, Some(0));
    assert!(paused.body.editable);
    // The breakpoint fires after the request rewrites.
    assert_eq!(header(&paused.request.headers, "x-before-breakpoint"), Some("1"));
    assert_eq!(proxy.engine.breakpoint_body(paused.id).unwrap(), b"original body");
    eventually(WAIT, || {
        proxy.engine.flush();
        proxy.engine.get_flow(0).filter(|f| f.state == FlowState::Paused)
    })
    .await;

    let mut headers = paused.request.headers.clone();
    headers.retain(|h| !h.name.eq_ignore_ascii_case("content-type"));
    headers.push(Header::new("content-type", "application/json"));
    proxy
        .engine
        .resolve_breakpoint(
            paused.id,
            BreakpointResolution::Continue {
                edits: Some(MessageEdits {
                    headers: Some(headers),
                    body: Some(r#"{"edited":true}"#.into()),
                    ..MessageEdits::default()
                }),
            },
        )
        .unwrap();
    // The first resolution wins.
    let again = proxy.engine.resolve_breakpoint(paused.id, BreakpointResolution::Abort);
    assert!(again.unwrap_err().to_string().contains("already resolved"));

    let reply = reply.await.unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, r#"{"edited":true}"#);
    assert_eq!(reply.headers["content-type"], "application/json");
    let (flow, detail) = proxy.finished(0).await;
    assert_eq!(flow.state, FlowState::Completed);
    assert_eq!(header(&detail.request.headers, "content-length"), Some("15"));
    assert!(!detail.rules.breakpoint_timeout);
    assert!(proxy.engine.breakpoints().is_empty());
}

#[tokio::test]
async fn a_response_breakpoint_can_answer_instead() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;
    proxy.rule(
        on_path("/json"),
        vec![Action::Breakpoint {
            request: false,
            response: true,
        }],
    );
    let url = upstream.url("/json");
    let request = Request::get(&url).body(Full::default()).unwrap();
    let reply = {
        let proxy = Proxy {
            engine: proxy.engine.clone(),
            addr: proxy.addr,
            dir: proxy.dir.clone(),
        };
        tokio::spawn(async move { https(&proxy, &url, b"h2", request).await })
    };
    let paused = eventually(WAIT, || proxy.engine.breakpoints().into_iter().next()).await;
    assert_eq!(paused.phase, Phase::Response);
    assert_eq!(paused.response.as_ref().map(|r| r.status), Some(200));
    assert_eq!(proxy.engine.breakpoint_body(paused.id).unwrap(), JSON_BODY.as_bytes());
    proxy
        .engine
        .resolve_breakpoint(
            paused.id,
            BreakpointResolution::Respond {
                status: 503,
                headers: vec![Header::new("x-from", "breakpoint")],
                body: "try later".into(),
            },
        )
        .unwrap();
    let reply = reply.await.unwrap();
    assert_eq!(reply.status, 503);
    assert_eq!(reply.headers["x-from"], "breakpoint");
    assert_eq!(reply.body, "try later");
    let (_, detail) = proxy.finished(0).await;
    assert!(detail.rules.mocked);
}

#[tokio::test]
async fn a_paused_h2_stream_doesnt_hold_up_the_others() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;
    proxy.rule(
        on_path("/json"),
        vec![Action::Breakpoint {
            request: true,
            response: false,
        }],
    );
    let uri: http::Uri = upstream.url("/").parse().unwrap();
    let tls = tls_through(
        proxy.addr,
        uri.host().unwrap(),
        uri.port_u16().unwrap(),
        tls_config(&proxy.ca(), &[b"h2"]),
    )
    .await
    .unwrap();
    let (sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(conn);
    let paused_request = {
        let mut sender = sender.clone();
        let url = upstream.url("/json");
        tokio::spawn(async move {
            let req = Request::get(url).body(Full::<Bytes>::default()).unwrap();
            let response = sender.send_request(req).await.unwrap();
            response.into_body().collect().await.unwrap().to_bytes()
        })
    };
    let paused = eventually(WAIT, || proxy.engine.breakpoints().into_iter().next()).await;
    // Another stream on the same connection completes meanwhile.
    let mut other = sender.clone();
    let req = Request::get(upstream.url("/version"))
        .body(Full::<Bytes>::default())
        .unwrap();
    let response = tokio::time::timeout(WAIT, other.send_request(req))
        .await
        .unwrap()
        .unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body, "HTTP/2.0");
    assert!(!paused_request.is_finished());

    proxy
        .engine
        .resolve_breakpoint(paused.id, BreakpointResolution::Continue { edits: None })
        .unwrap();
    assert_eq!(paused_request.await.unwrap(), JSON_BODY);
}

#[tokio::test]
async fn a_breakpoint_timeout_lets_the_flow_continue_unmodified() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    proxy
        .engine
        .set_rule_settings(RuleSettings {
            breakpoint_timeout_secs: 1,
        })
        .unwrap();
    proxy.rule(
        on_path("/json"),
        vec![Action::Breakpoint {
            request: true,
            response: true,
        }],
    );
    let started = Instant::now();
    let reply = get(proxy.addr, &upstream.url("/json")).await;
    assert!(started.elapsed() >= Duration::from_secs(2), "both breakpoints waited");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, JSON_BODY);
    let (flow, detail) = proxy.finished(0).await;
    assert_eq!(flow.state, FlowState::Completed);
    assert!(detail.rules.breakpoint_timeout);
}

#[tokio::test]
async fn an_over_cap_body_streams_through_with_a_warning() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    proxy.rule(
        Matcher {
            path: Some(Pattern::glob("/bytes/*")),
            ..Matcher::default()
        },
        vec![Action::ReplaceBody {
            phase: Phase::Response,
            pattern: Pattern::exact("x"),
            replacement: "y".into(),
        }],
    );
    let len = pm_core::rules::REWRITE_BODY_CAP as u64 + 1024 * 1024;
    let response = client::send(
        proxy.addr,
        Request::get(upstream.url(&format!("/bytes/{len}")))
            .body(Full::default())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let mut received = 0u64;
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.unwrap().into_data() {
            for (i, b) in data.iter().enumerate() {
                assert_eq!(*b, pattern_byte(received + i as u64), "byte {}", received + i as u64);
            }
            received += data.len() as u64;
        }
    }
    assert_eq!(received, len);
    let (flow, detail) = proxy.finished(0).await;
    assert_eq!(flow.state, FlowState::Completed);
    let warning = detail.rules.rewrite_skipped.expect("marked rewrite_skipped");
    assert!(warning.contains("larger than 32 MB"), "{warning}");
}

#[tokio::test]
async fn block_answers_with_a_status_or_resets() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    proxy.rule(
        on_path("/json"),
        vec![Action::Block {
            mode: BlockMode::Status { status: 451 },
        }],
    );
    proxy.rule(on_path("/chunked"), vec![Action::Block { mode: BlockMode::Reset }]);
    let reply = get(proxy.addr, &upstream.url("/json")).await;
    assert_eq!(reply.status, 451);
    let (flow, _) = proxy.finished(0).await;
    assert_eq!((flow.state, flow.status), (FlowState::Blocked, Some(451)));

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\n\r\n",
        upstream.url("/chunked"),
        upstream.addr()
    );
    let raw = client::raw(proxy.addr, request.as_bytes()).await;
    assert!(raw.is_empty(), "the connection was closed without an answer: {raw:?}");
    let (flow, detail) = proxy.finished(1).await;
    assert_eq!(flow.state, FlowState::Blocked);
    assert!(detail.error.unwrap().contains("reset"));
}

#[tokio::test]
async fn block_resets_an_h2_stream() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;
    proxy.rule(on_path("/json"), vec![Action::Block { mode: BlockMode::Reset }]);
    let uri: http::Uri = upstream.url("/").parse().unwrap();
    let tls = tls_through(
        proxy.addr,
        uri.host().unwrap(),
        uri.port_u16().unwrap(),
        tls_config(&proxy.ca(), &[b"h2"]),
    )
    .await
    .unwrap();
    let (mut sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(conn);
    let req = Request::get(upstream.url("/json"))
        .body(Full::<Bytes>::default())
        .unwrap();
    assert!(sender.send_request(req).await.is_err(), "the stream was reset");
    // The connection itself survives.
    let req = Request::get(upstream.url("/version"))
        .body(Full::<Bytes>::default())
        .unwrap();
    assert_eq!(sender.send_request(req).await.unwrap().status(), 200);
}

#[tokio::test]
async fn throttle_shapes_the_throughput() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    // 800 kbps = 100 KB/s.
    proxy.rule(
        Matcher::default(),
        vec![Action::Throttle {
            download_kbps: 800,
            upload_kbps: 0,
            latency_ms: 200,
        }],
    );
    let started = Instant::now();
    let reply = get(proxy.addr, &upstream.url("/bytes/250000")).await;
    let elapsed = started.elapsed();
    assert_eq!(reply.body.len(), 250_000);
    // 250 KB at 100 KB/s with a 10 KB burst, plus 200 ms of latency.
    assert!(elapsed >= Duration::from_millis(2500), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(8), "{elapsed:?}");
}

#[tokio::test]
async fn invalid_or_oversized_regexes_are_rejected_clearly() {
    let proxy = Proxy::start(None).await;
    let before = proxy.engine.rules();
    let attempt = |pattern: &str| {
        proxy
            .engine
            .create_rule(Rule {
                id: String::new(),
                name: "agent rule".into(),
                enabled: true,
                order: 0,
                origin: RuleOrigin::User,
                matcher: Matcher {
                    host: Some(Pattern::regex(pattern)),
                    ..Matcher::default()
                },
                actions: Vec::new(),
            })
            .unwrap_err()
            .to_string()
    };
    let err = attempt("api.(example");
    assert!(err.contains("rule \"agent rule\", host: invalid regex"), "{err}");
    let err = attempt(r"(\w{500}){500}");
    assert!(err.contains("host: the regex is too complex"), "{err}");
    assert_eq!(proxy.engine.rules(), before, "nothing was saved");
}

#[tokio::test]
async fn rules_persist_and_presets_draft_from_a_flow() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    get(proxy.addr, &upstream.url("/json?page=1")).await;
    proxy.finished(0).await;

    let draft = proxy.engine.draft_rule(0, PresetKind::MapLocal).unwrap();
    assert_eq!(draft.matcher.path, Some(Pattern::exact("/json")));
    assert_eq!(draft.matcher.methods, ["GET"]);
    match &draft.actions[0] {
        Action::MapLocal {
            source: MapLocalSource::Inline { body },
            status,
            content_type,
            ..
        } => {
            assert_eq!(body, JSON_BODY);
            assert_eq!(*status, 200);
            assert_eq!(content_type.as_deref(), Some("application/json"));
        }
        other => panic!("{other:?}"),
    }
    let saved = proxy.engine.create_rule(draft).unwrap();

    // Dry run against the recorded flow.
    let mut probe = saved.clone();
    probe.actions.push(Action::SetHeader {
        phase: Phase::Response,
        name: "x-dry".into(),
        value: "run".into(),
    });
    let result = proxy.engine.test_rule(&probe, 0).await.unwrap();
    assert!(result.matched);
    assert_eq!(result.response_matched, Some(true));
    assert_eq!(result.response_body.as_deref(), Some(JSON_BODY));
    assert_eq!(header(&result.response.unwrap().headers, "x-dry"), Some("run"));
    assert!(
        result.notes.iter().any(|n| n.contains("Map Local")),
        "{:?}",
        result.notes
    );
    let mut other = probe.clone();
    other.matcher.path = Some(Pattern::exact("/other"));
    let result = proxy.engine.test_rule(&other, 0).await.unwrap();
    assert_eq!(result.mismatch.as_deref(), Some("path /json doesn't match"));

    let reopened = Proxy::start_in(proxy.dir.clone(), None).await;
    let rules = reopened.engine.rules();
    assert!(rules.iter().any(|r| r.id == saved.id && r.actions == saved.actions));
    assert!(
        rules
            .iter()
            .any(|r| r.id == pm_model::GLOBAL_THROTTLE_RULE_ID && !r.enabled)
    );
    assert!(reopened.engine.delete_rule(pm_model::GLOBAL_THROTTLE_RULE_ID).is_err());
}

#[tokio::test]
async fn agent_rules_are_tagged_and_removed_together() {
    let proxy = Proxy::start(None).await;
    let agent = |name: &str| Rule {
        id: String::new(),
        name: name.into(),
        enabled: true,
        order: 0,
        origin: RuleOrigin::Agent {
            client: "claude-code".into(),
            session: None,
            created_at_ms: 0.0,
        },
        matcher: Matcher::default(),
        actions: Vec::new(),
    };
    let a = proxy.engine.create_rule(agent("a")).unwrap();
    proxy.engine.create_rule(agent("b")).unwrap();
    let user = proxy.rule(Matcher::default(), Vec::new());
    let ordered = proxy.engine.reorder_rules(&[user.id.clone(), a.id.clone()]).unwrap();
    let names: Vec<&str> = ordered.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Network conditions", "test rule", "a", "b"]);
    assert_eq!(proxy.engine.delete_agent_rules().unwrap(), 2);
    assert_eq!(proxy.engine.rules().len(), 2);
}

/// A raw HTTP/1.1 JSON response, leaked so a `RawUpstream` can serve it.
fn json_response(body: &str) -> &'static [u8] {
    let raw = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    Box::leak(raw.into_bytes().into_boxed_slice())
}

#[tokio::test]
async fn a_visual_rewrite_patches_only_its_paths() {
    // What was captured, and what the server answers later: the same shape,
    // other values, 64-bit ids and its own formatting.
    let captured = r#"{"id":1,"user":{"name":"Ana","role":"user"},"items":[1]}"#;
    let later = "{\n  \"id\": 9007199254740993123,\n  \"user\": {\n    \"name\": \"Zoe\",\n    \"role\": \"admin\",\n    \"since\": 1.50\n  },\n  \"items\": [7, 8]\n}";
    let first = RawUpstream::start(json_response(captured)).await;
    let second = RawUpstream::start(json_response(later)).await;
    let proxy = Proxy::start(None).await;
    get(proxy.addr, &format!("http://{}/me", first.addr())).await;
    proxy.finished(0).await;

    // What the Rewrite dialog saves for: name edited, role renamed to
    // `kind`, a key added (exact JSON text, not a parsed value).
    let mut rule = proxy.engine.draft_rule(0, PresetKind::Rewrite).unwrap();
    assert!(rule.actions.is_empty(), "the Rewrite preset drafts no actions");
    rule.matcher = on_path("/me");
    rule.actions = vec![
        Action::JsonSet {
            phase: Phase::Response,
            pointer: "/user/name".into(),
            value: serde_json::Value::Null,
            raw: Some(r#""Bea""#.into()),
        },
        Action::JsonRemove {
            phase: Phase::Response,
            pointer: "/user/role".into(),
        },
        Action::JsonSet {
            phase: Phase::Response,
            pointer: "/user/kind".into(),
            value: serde_json::Value::Null,
            raw: Some(r#""user""#.into()),
        },
        Action::JsonSet {
            phase: Phase::Response,
            pointer: "/big".into(),
            value: serde_json::Value::Null,
            raw: Some("12345678901234567890123".into()),
        },
    ];
    proxy.engine.create_rule(rule).unwrap();

    let reply = get(proxy.addr, &format!("http://{}/me", second.addr())).await;
    assert_eq!(
        std::str::from_utf8(&reply.body).unwrap(),
        "{\n  \"id\": 9007199254740993123,\n  \"user\": {\n    \"name\": \"Bea\",\n    \"since\": 1.50,\n    \"kind\": \"user\"\n  },\n  \"items\": [7, 8],\n  \"big\": 12345678901234567890123\n}",
        "only the edited paths change; everything else is byte-identical"
    );
    assert_eq!(reply.headers["content-length"], reply.body.len().to_string());
}

#[tokio::test]
async fn form_and_repeated_header_actions() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    proxy.rule(
        on_path("/echo"),
        vec![
            Action::FormSet {
                phase: Phase::Request,
                name: "q".into(),
                value: "a b".into(),
            },
            Action::FormRemove {
                phase: Phase::Request,
                name: "drop".into(),
            },
            Action::RemoveHeader {
                phase: Phase::Request,
                name: "x-tag".into(),
            },
            Action::AddHeader {
                phase: Phase::Request,
                name: "x-tag".into(),
                value: "one".into(),
            },
            Action::AddHeader {
                phase: Phase::Request,
                name: "x-tag".into(),
                value: "two".into(),
            },
        ],
    );
    let req = Request::post(upstream.url("/echo"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("x-tag", "zero")
        .body(Full::new(Bytes::from_static(b"q=old&drop=1&keep=x%2By")))
        .unwrap();
    send(proxy.addr, req).await;
    let (_, detail) = proxy.finished(0).await;
    let tags: Vec<&str> = detail
        .request
        .headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("x-tag"))
        .map(|h| h.value.as_str())
        .collect();
    assert_eq!(tags, ["one", "two"]);
    let body = proxy.engine.body(0, pm_model::BodyPart::Request, true).unwrap().bytes;
    assert_eq!(body, b"q=a%20b&keep=x%2By");
}

#[tokio::test]
async fn dry_runs_work_on_a_saved_session() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    get(proxy.addr, &upstream.url("/json")).await;
    proxy.finished(0).await;
    let path = proxy.dir.join("dry.proxymate");
    let (tx, rx) = std::sync::mpsc::channel();
    proxy
        .engine
        .save_session(
            pm_model::LIVE_STORE,
            &pm_model::FlowSelection::All,
            path.clone(),
            Box::new(move |p| {
                if p.finished {
                    let _ = tx.send(p);
                }
            }),
        )
        .unwrap();
    rx.recv_timeout(WAIT).expect("saved");
    proxy.engine.clear();
    let store = proxy.engine.open_document(&path).unwrap().store;

    let mut rule = proxy.engine.draft_rule_in(store, 0, PresetKind::Rewrite).unwrap();
    rule.actions.push(Action::JsonSet {
        phase: Phase::Response,
        pointer: "/name".into(),
        value: json!("dry"),
        raw: None,
    });
    assert!(
        proxy.engine.test_rule(&rule, 0).await.is_err(),
        "the live capture is empty"
    );
    let result = proxy.engine.test_rule_in(store, &rule, 0, Some(8)).await.unwrap();
    assert!(result.matched);
    assert!(result.response_body_truncated);
    assert_eq!(result.response_body.as_deref().map(str::len), Some(8));
    let full = proxy.engine.test_rule_in(store, &rule, 0, None).await.unwrap();
    assert!(!full.response_body_truncated);
    let doc: serde_json::Value = serde_json::from_str(full.response_body.as_deref().unwrap()).unwrap();
    assert_eq!(doc["name"], "dry");
    assert_eq!(proxy.engine.flows_matching_rule(store, &rule, 10).unwrap(), [0]);
    rule.matcher.path = Some(Pattern::exact("/nope"));
    assert!(proxy.engine.flows_matching_rule(store, &rule, 10).unwrap().is_empty());
}
