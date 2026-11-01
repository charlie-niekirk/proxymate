//! M21 end to end: the recording lists, the No Caching and Block Cookies
//! built-in rules, auto-save, and flows of an intercepted process tree.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderMap, Request};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{
    Action, AutoSaveSettings, FlowState, InterceptTarget, Matcher, NO_CACHE_RULE_ID, NO_COOKIES_RULE_ID, Phase,
    RecordingSettings, Rule, RuleOrigin,
};
use pm_testkit::client::{self, eventually, tls_config, tls_through};
use pm_testkit::upstream::JSON_BODY;
use pm_testkit::{Alpn, RawUpstream, TlsUpstream, Upstream};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;

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

    fn ca(&self) -> CertificateDer<'static> {
        CertificateDer::from_pem_file(self.engine.ca_info().cert_path).expect("CA PEM")
    }

    fn flows(&self) -> u32 {
        self.engine.flush();
        self.engine.total()
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One h1 request over TLS through the proxy's MITM.
async fn https_get(proxy: &Proxy, url: &str) -> (u16, HeaderMap, Bytes) {
    let uri: http::Uri = url.parse().unwrap();
    let tls = tls_through(
        proxy.addr,
        uri.host().unwrap(),
        uri.port_u16().unwrap(),
        tls_config(&proxy.ca(), &[b"http/1.1"]),
    )
    .await
    .expect("TLS through the proxy");
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
    tokio::spawn(conn);
    let req = Request::get(uri.path_and_query().unwrap().as_str())
        .header(http::header::HOST, uri.authority().unwrap().as_str())
        .body(Full::<Bytes>::default())
        .unwrap();
    let response = sender.send_request(req).await.unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, body)
}

#[tokio::test]
async fn a_never_recorded_host_is_proxied_but_makes_no_flow() {
    let upstream = Upstream::start().await;
    let tls_upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&tls_upstream)).await;
    // A rule still applies to the unrecorded traffic.
    proxy
        .engine
        .create_rule(Rule {
            id: String::new(),
            name: "tag".into(),
            enabled: true,
            order: 0,
            origin: RuleOrigin::User,
            matcher: Matcher::default(),
            actions: vec![Action::SetHeader {
                phase: Phase::Response,
                name: "x-rule".into(),
                value: "ran".into(),
            }],
        })
        .unwrap();
    let saved = proxy
        .engine
        .set_recording_settings(RecordingSettings {
            only: Vec::new(),
            never: vec![" HTTPS://LocalHost:443/ ".into()],
        })
        .unwrap();
    assert_eq!(saved.never, ["localhost"]);

    // MITM'd and rewritten, but not recorded: neither the CONNECT nor the request.
    let (status, headers, body) = https_get(&proxy, &tls_upstream.url("/json")).await;
    assert_eq!((status, &body[..]), (200, JSON_BODY.as_bytes()));
    assert_eq!(headers["x-rule"], "ran");
    assert_eq!(proxy.flows(), 0);
    assert!(proxy.engine.storage_status().unrecorded >= 1);

    // Other hosts still are.
    let (status, _) = client::get(proxy.addr, &upstream.url("/json")).await;
    assert_eq!(status, 200);
    assert_eq!(proxy.flows(), 1);
    assert_eq!(proxy.engine.get_flow(0).unwrap().host.as_ref(), "127.0.0.1");

    // "Never record this host" adds to the list; the lists survive a restart.
    proxy.engine.never_record("127.0.0.1").unwrap();
    let before = proxy.engine.storage_status().unrecorded;
    let (status, _) = client::get(proxy.addr, &upstream.url("/json")).await;
    assert_eq!(status, 200);
    assert_eq!(proxy.flows(), 1);
    assert_eq!(proxy.engine.storage_status().unrecorded, before + 1);
    let dir = proxy.dir.clone();
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(dir),
        ..Default::default()
    });
    assert_eq!(engine.recording_settings().never, ["localhost", "127.0.0.1"]);
}

#[tokio::test]
async fn an_only_list_records_just_its_hosts() {
    let upstream = Upstream::start().await;
    let tls_upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&tls_upstream)).await;
    proxy
        .engine
        .set_recording_settings(RecordingSettings {
            only: vec!["*.localhost".into()],
            never: Vec::new(),
        })
        .unwrap();
    let (status, _) = client::get(proxy.addr, &upstream.url("/json")).await;
    assert_eq!(status, 200);
    assert_eq!(proxy.flows(), 0);
    // `*.localhost` matches `localhost` itself.
    let (status, _, _) = https_get(&proxy, &tls_upstream.url("/json")).await;
    assert_eq!(status, 200);
    let flows = eventually(WAIT, || {
        proxy.engine.flush();
        let flows: Vec<_> = (0..proxy.engine.total())
            .filter_map(|id| proxy.engine.get_flow(id))
            .collect();
        flows.iter().any(|f| f.path == "/json").then_some(flows)
    })
    .await;
    assert!(flows.iter().all(|f| f.host.as_ref() == "localhost"));

    // A bad pattern is refused and the old lists stay.
    let err = proxy
        .engine
        .set_recording_settings(RecordingSettings {
            only: vec!["a{b".into()],
            never: Vec::new(),
        })
        .unwrap_err();
    assert!(err.to_string().contains("isn't a valid host pattern"), "{err}");
    assert_eq!(proxy.engine.recording_settings().only, ["*.localhost"]);
}

const CACHED: &[u8] = b"HTTP/1.1 200 OK\r\n\
Content-Type: text/plain\r\n\
Content-Length: 2\r\n\
Cache-Control: max-age=3600\r\n\
ETag: \"v1\"\r\n\
Last-Modified: Sun, 01 Nov 2026 10:00:00 GMT\r\n\
Expires: Sun, 01 Nov 2026 11:00:00 GMT\r\n\
Set-Cookie: session=abc; Path=/\r\n\
Set-Cookie: theme=dark\r\n\
Connection: close\r\n\
\r\n\
ok";

fn cached_request(upstream: &RawUpstream) -> Request<Full<Bytes>> {
    Request::get(format!("http://{}/page", upstream.addr()))
        .header("If-None-Match", "\"v0\"")
        .header("If-Modified-Since", "Sat, 31 Oct 2026 10:00:00 GMT")
        .header("If-Match", "\"keep\"")
        .header("Cache-Control", "max-age=0")
        .header("Cookie", "session=abc")
        .body(Full::default())
        .unwrap()
}

fn head_text(head: &[u8]) -> String {
    String::from_utf8_lossy(head).to_ascii_lowercase()
}

#[tokio::test]
async fn no_caching_strips_validators_both_ways() {
    let upstream = RawUpstream::start(CACHED).await;
    let proxy = Proxy::start(None).await;
    // Off by default: untouched.
    let response = client::send(proxy.addr, cached_request(&upstream)).await;
    assert_eq!(response.headers()["etag"], "\"v1\"");
    let head = head_text(&upstream.heads().await[0]);
    assert!(head.contains("if-none-match: \"v0\""), "{head}");

    proxy.engine.set_rule_enabled(NO_CACHE_RULE_ID, true).unwrap();
    let response = client::send(proxy.addr, cached_request(&upstream)).await;
    let headers = response.headers().clone();
    assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), "ok");
    assert_eq!(headers["cache-control"], "no-store");
    for gone in ["etag", "last-modified", "expires"] {
        assert!(headers.get(gone).is_none(), "{gone} in {headers:?}");
    }
    // Cookies are another toggle's business.
    assert_eq!(headers.get_all("set-cookie").iter().count(), 2);
    let head = head_text(&upstream.heads().await[1]);
    assert!(!head.contains("if-none-match"), "{head}");
    assert!(!head.contains("if-modified-since"), "{head}");
    assert!(head.contains("cache-control: no-cache\r\n"), "{head}");
    assert!(head.contains("pragma: no-cache\r\n"), "{head}");
    assert!(head.contains("if-match: \"keep\""), "write preconditions stay: {head}");
    assert!(head.contains("cookie: session=abc"), "{head}");

    let flow = eventually(WAIT, || {
        proxy.engine.flush();
        proxy.engine.get_flow(1).filter(|f| f.state != FlowState::Active)
    })
    .await;
    assert!(flow.edited);
    let detail = proxy.engine.flow_detail(1).unwrap().unwrap();
    assert!(detail.rules.applied.iter().any(|r| r.rule_id == NO_CACHE_RULE_ID));
}

#[tokio::test]
async fn block_cookies_strips_cookie_and_set_cookie() {
    let upstream = RawUpstream::start(CACHED).await;
    let proxy = Proxy::start(None).await;
    proxy.engine.set_rule_enabled(NO_COOKIES_RULE_ID, true).unwrap();
    let response = client::send(proxy.addr, cached_request(&upstream)).await;
    let headers = response.headers().clone();
    assert!(headers.get("set-cookie").is_none(), "{headers:?}");
    assert_eq!(headers["etag"], "\"v1\"");
    let head = head_text(&upstream.heads().await[0]);
    assert!(!head.contains("cookie:"), "{head}");
    assert!(head.contains("if-none-match"), "{head}");
    // Both are built in: listed, off at first, and can't be deleted.
    let fresh = Proxy::start(None).await;
    let rules = fresh.engine.rules();
    for id in [NO_CACHE_RULE_ID, NO_COOKIES_RULE_ID] {
        assert!(rules.iter().any(|r| r.id == id && !r.enabled), "{id}");
        assert!(fresh.engine.delete_rule(id).is_err());
    }
}

#[tokio::test]
async fn auto_save_writes_a_session_that_opens_and_keeps_the_newest() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    let folder = proxy.dir.join("autosaves");
    std::fs::create_dir_all(&folder).unwrap();
    for _ in 0..3 {
        let (status, _) = client::get(proxy.addr, &upstream.url("/json")).await;
        assert_eq!(status, 200);
    }
    eventually(WAIT, || {
        proxy.engine.flush();
        (0..3)
            .all(|id| {
                proxy
                    .engine
                    .get_flow(id)
                    .is_some_and(|f| f.state == FlowState::Completed)
            })
            .then_some(())
    })
    .await;

    // Refused without a real folder.
    let bad = AutoSaveSettings {
        enabled: true,
        interval_minutes: 5,
        folder: proxy.dir.join("missing").display().to_string(),
        keep: 2,
    };
    assert!(proxy.engine.set_autosave_settings(bad).is_err());
    let settings = AutoSaveSettings {
        enabled: true,
        interval_minutes: 5,
        folder: folder.display().to_string(),
        keep: 2,
    };
    proxy.engine.set_autosave_settings(settings.clone()).unwrap();
    assert_eq!(proxy.engine.autosave_settings(), settings);

    let mut paths = Vec::new();
    for _ in 0..3 {
        assert_eq!(proxy.engine.autosave_now().unwrap(), 3);
        let status = eventually(WAIT, || {
            let status = proxy.engine.storage_status().autosave;
            (!status.saving && status.last_path.as_ref() != paths.last()).then_some(status)
        })
        .await;
        assert_eq!(status.error, None);
        paths.push(status.last_path.unwrap());
        // Rotation goes by modification time.
        std::thread::sleep(Duration::from_millis(20));
    }
    let left: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().path().display().to_string())
        .collect();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(!left.contains(&paths[0]));
    assert!(paths[0].contains("Proxymate Autosave "), "{}", paths[0]);

    let doc = proxy.engine.open_document(std::path::Path::new(&paths[2])).unwrap();
    assert_eq!(doc.flows, 3);
}

/// The test process stands in for a launched browser: its own connections
/// are from an intercepted tree.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn flows_from_an_intercepted_process_tree_are_labelled() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    let (status, _) = client::get(proxy.addr, &upstream.url("/json")).await;
    assert_eq!(status, 200);
    let plain = eventually(WAIT, || {
        proxy.engine.flush();
        proxy.engine.get_flow(0)?.client
    })
    .await;
    assert!(!plain.ends_with("(Intercepted)"), "{plain}");

    let session = proxy
        .engine
        .track_intercept(InterceptTarget::Chrome, std::process::id());
    assert_eq!(proxy.engine.intercepts(), [session]);
    let (status, _) = client::get(proxy.addr, &upstream.url("/json")).await;
    assert_eq!(status, 200);
    let labelled = eventually(WAIT, || {
        proxy.engine.flush();
        proxy.engine.get_flow(1)?.client
    })
    .await;
    assert_eq!(labelled.as_ref(), format!("{plain} (Intercepted)"));
}

#[test]
fn every_intercept_target_is_listed() {
    let engine = Engine::new();
    let apps = engine.intercept_apps();
    assert_eq!(apps.len(), InterceptTarget::ALL.len());
    assert_eq!(apps[2].name, "Google Chrome");
    assert!(engine.intercepts().is_empty());
}
