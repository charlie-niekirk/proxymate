//! Network plumbing (M18) end to end: chaining to an HTTP proxy (absolute
//! form and CONNECT, Basic auth) and a SOCKS5 proxy, the loop guard, the
//! SOCKS5 listener into the MITM, reverse proxies over plain HTTP and TLS,
//! client certificates for an mTLS upstream, DNS overrides for names that
//! don't exist, and the proxy that was set before Proxymate.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::Request;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{
    ClientCert, ClientIdentity, DnsOverride, FlowDetail, FlowId, FlowState, FlowSummary, ListenerKind, NetworkSettings,
    ProxyServer, RequestSpec, ReverseProxy, Scheme, SendOpts, SendOrigin, TlsStatus, UpstreamProxy, UpstreamRule,
    UpstreamSettings,
};
use pm_testkit::client::{self, eventually, https_get, tls_config};
use pm_testkit::proxies::socks5_connect;
use pm_testkit::upstream::JSON_BODY;
use pm_testkit::{Alpn, HttpProxy, Socks5Server, TlsUpstream, Upstream};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const WAIT: Duration = Duration::from_secs(10);
const H1: &[u8] = b"http/1.1";

struct Proxy {
    engine: Arc<Engine>,
    addr: SocketAddr,
    dir: PathBuf,
}

impl Proxy {
    async fn start(roots: Vec<Vec<u8>>) -> Proxy {
        Self::start_in(temp_dir(), roots).await
    }

    async fn start_in(dir: PathBuf, roots: Vec<Vec<u8>>) -> Proxy {
        let engine = Engine::with_config(EngineConfig {
            data_dir: Some(dir.clone()),
            extra_upstream_roots: roots,
            ..Default::default()
        });
        let config = ProxyConfig {
            ports: 0..=0,
            ..Default::default()
        };
        let addr = engine.start_proxy(config).await.expect("proxy starts");
        Proxy { engine, addr, dir }
    }

    fn ca(&self) -> CertificateDer<'static> {
        CertificateDer::from_pem_file(self.engine.ca_info().cert_path).expect("CA PEM")
    }

    async fn network(&self, settings: NetworkSettings) -> NetworkSettings {
        self.engine
            .set_network_settings(settings)
            .await
            .expect("network settings save")
    }

    /// The newest flow, once finished.
    async fn last(&self) -> (FlowSummary, FlowDetail) {
        let id = eventually(WAIT, || {
            self.engine.flush();
            self.engine.total().checked_sub(1)
        })
        .await;
        self.finished(id).await
    }

    async fn finished(&self, id: FlowId) -> (FlowSummary, FlowDetail) {
        let summary = eventually(WAIT, || {
            self.engine.flush();
            self.engine.get_flow(id).filter(|f| f.state != FlowState::Active)
        })
        .await;
        let detail = self.engine.flow_detail(id).unwrap().expect("detail");
        (summary, detail)
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn temp_dir() -> PathBuf {
    std::env::temp_dir()
        .join("proxymate-tests")
        .join(uuid::Uuid::new_v4().to_string())
}

fn server(port: u16, user: Option<(&str, &str)>) -> ProxyServer {
    ProxyServer {
        host: "127.0.0.1".into(),
        port,
        username: user.map(|(u, _)| u.into()),
        password: user.map(|(_, p)| p.into()),
        has_password: false,
    }
}

/// Every host in `hosts` goes through `proxy`; nothing is bypassed.
fn upstream(hosts: &[&str], proxy: UpstreamProxy) -> NetworkSettings {
    NetworkSettings {
        upstream: UpstreamSettings {
            rules: vec![UpstreamRule {
                enabled: true,
                hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
                proxy,
            }],
            bypass: Vec::new(),
            use_system_previous: false,
        },
        ..Default::default()
    }
}

/// A port nothing listens on right now.
async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

/// `GET /json` over h1 on `stream`, which already reaches the origin.
async fn get_json<S>(stream: S, host: &str) -> (u16, Bytes)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("h1 handshake");
    tokio::spawn(conn);
    let req = Request::get("/json")
        .header(http::header::HOST, host)
        .body(Full::<Bytes>::default())
        .unwrap();
    let response = sender.send_request(req).await.expect("request");
    let status = response.status().as_u16();
    (status, response.into_body().collect().await.unwrap().to_bytes())
}

#[tokio::test]
async fn plain_http_is_chained_through_an_http_proxy_with_basic_auth() {
    let origin = Upstream::start().await;
    let corp = HttpProxy::with_basic_auth("me", "s3cret").await;
    let proxy = Proxy::start(Vec::new()).await;
    let saved = proxy
        .network(upstream(
            &["127.0.0.1"],
            UpstreamProxy::Http(server(corp.port(), Some(("me", "s3cret")))),
        ))
        .await;
    // The password went to the secret store, not the settings row.
    let UpstreamProxy::Http(s) = &saved.upstream.rules[0].proxy else {
        panic!("{saved:?}")
    };
    assert_eq!(s.password, None);
    assert!(s.has_password);
    assert!(
        std::fs::read_to_string(proxy.dir.join("secrets.json"))
            .unwrap()
            .contains("s3cret")
    );

    let (status, body) = client::get(proxy.addr, &origin.url("/json")).await;
    assert_eq!(status, 200);
    assert_eq!(body, JSON_BODY.as_bytes());
    let seen = corp.requests();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].target, origin.url("/json"), "absolute form");
    assert!(seen[0].allowed);
    let (_, detail) = proxy.last().await;
    let via = detail.via.expect("via");
    assert_eq!(
        via.proxy.as_deref(),
        Some(format!("http://127.0.0.1:{}", corp.port()).as_str())
    );
    assert_eq!(via.addr.as_deref(), Some(corp.addr().to_string().as_str()));
    assert!(detail.timing.connect_ms.is_some());

    // Engine::send goes the same way.
    let spec = RequestSpec {
        method: "GET".into(),
        url: origin.url("/json"),
        headers: Vec::new(),
        body: None,
        body_base64: false,
    };
    let opts = SendOpts {
        apply_rules: true,
        wait: true,
        origin: SendOrigin::User,
    };
    proxy.engine.send(spec, opts).await.unwrap();
    assert_eq!(corp.requests().len(), 2);
}

#[tokio::test]
async fn https_goes_through_connect_and_auth_failures_explain_themselves() {
    let origin = TlsUpstream::start(Alpn::Both).await;
    let corp = HttpProxy::with_basic_auth("me", "s3cret").await;
    let proxy = Proxy::start(vec![origin.ca_der()]).await;
    proxy
        .network(upstream(
            &["localhost"],
            UpstreamProxy::Http(server(corp.port(), Some(("me", "s3cret")))),
        ))
        .await;
    let response = https_get(proxy.addr, &origin.url("/json"), &proxy.ca(), &[H1]).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, JSON_BODY.as_bytes());
    let seen = corp.requests();
    assert_eq!(seen[0].method, "CONNECT");
    assert_eq!(seen[0].target, format!("localhost:{}", origin.port()));
    let (summary, detail) = proxy.last().await;
    assert_eq!(summary.tls, TlsStatus::Intercepted);
    assert!(detail.via.unwrap().proxy.unwrap().starts_with("http://127.0.0.1:"));

    // Wrong password: a readable failure, not a hang.
    proxy
        .network(upstream(
            &["localhost"],
            UpstreamProxy::Http(server(corp.port(), Some(("me", "nope")))),
        ))
        .await;
    let response = https_get(proxy.addr, &origin.url("/json"), &proxy.ca(), &[H1]).await;
    assert_eq!(response.status, 502);
    let (summary, detail) = proxy.last().await;
    assert_eq!(summary.state, FlowState::Failed);
    let error = detail.error.unwrap();
    assert!(error.contains("rejected the username and password"), "{error}");

    // No credentials at all.
    proxy
        .network(upstream(&["localhost"], UpstreamProxy::Http(server(corp.port(), None))))
        .await;
    let response = https_get(proxy.addr, &origin.url("/json"), &proxy.ca(), &[H1]).await;
    assert_eq!(response.status, 502);
    let (_, detail) = proxy.last().await;
    assert!(detail.error.unwrap().contains("asks for a username and password"));
}

#[tokio::test]
async fn the_loop_guard_refuses_proxymate_as_its_own_upstream() {
    let proxy = Proxy::start(Vec::new()).await;
    let port = proxy.addr.port();
    for host in ["127.0.0.1", "localhost"] {
        let mut settings = upstream(&["*"], UpstreamProxy::Http(server(port, None)));
        if let UpstreamProxy::Http(s) = &mut settings.upstream.rules[0].proxy {
            s.host = host.into();
        }
        let err = proxy.engine.set_network_settings(settings).await.unwrap_err();
        assert!(err.to_string().contains("loop"), "{err}");
    }
    // A name that only resolves to us gets through the check, and is
    // caught when dialling.
    let origin = Upstream::start().await;
    let mut settings = upstream(&["127.0.0.1"], UpstreamProxy::Socks5(server(port, None)));
    if let UpstreamProxy::Socks5(s) = &mut settings.upstream.rules[0].proxy {
        s.host = "me.proxymate-nowhere.test".into();
    }
    settings.dns.push(DnsOverride {
        enabled: true,
        host: "me.proxymate-nowhere.test".into(),
        ip: "127.0.0.1".into(),
    });
    proxy.network(settings).await;
    let (status, _) = client::get(proxy.addr, &origin.url("/json")).await;
    assert_eq!(status, 502);
    let (_, detail) = proxy.last().await;
    let error = detail.error.unwrap();
    assert!(error.contains("is Proxymate itself"), "{error}");
}

#[tokio::test]
async fn a_socks5_upstream_resolves_names_remotely() {
    let origin = TlsUpstream::start(Alpn::Both).await;
    let plain = Upstream::start().await;
    let socks = Socks5Server::with_auth("me", "pw").await;
    let proxy = Proxy::start(vec![origin.ca_der()]).await;
    proxy
        .network(upstream(
            &["localhost"],
            UpstreamProxy::Socks5(server(socks.port(), Some(("me", "pw")))),
        ))
        .await;
    let response = https_get(proxy.addr, &origin.url("/json"), &proxy.ca(), &[H1]).await;
    assert_eq!(response.status, 200);
    assert_eq!(socks.targets(), vec![format!("localhost:{}", origin.port())]);
    let (_, detail) = proxy.last().await;
    assert!(detail.via.unwrap().proxy.unwrap().starts_with("socks5://"));

    let url = format!("http://localhost:{}/json", plain.addr().port());
    let (status, body) = client::get(proxy.addr, &url).await;
    assert_eq!(status, 200);
    assert_eq!(body, JSON_BODY.as_bytes());
    assert_eq!(socks.targets().len(), 2);

    // Bad credentials fail the flow with why.
    proxy
        .network(upstream(
            &["localhost"],
            UpstreamProxy::Socks5(server(socks.port(), Some(("me", "x")))),
        ))
        .await;
    let (status, _) = client::get(proxy.addr, &url).await;
    assert_eq!(status, 502);
    let (_, detail) = proxy.last().await;
    assert!(detail.error.unwrap().contains("rejected the username and password"));
}

/// `curl --socks5-hostname 127.0.0.1:9090 https://localhost:<port>/json`:
/// the SOCKS5 listener on the proxy port, then the MITM.
#[tokio::test]
async fn the_socks5_listener_feeds_the_mitm_plain_http_and_raw_tunnels() {
    let origin = TlsUpstream::start(Alpn::Both).await;
    let plain = Upstream::start().await;
    let proxy = Proxy::start(vec![origin.ca_der()]).await;

    let stream = socks5_connect(proxy.addr, "localhost", origin.port()).await.unwrap();
    let name = ServerName::try_from("localhost").unwrap();
    let tls = tokio_rustls::TlsConnector::from(tls_config(&proxy.ca(), &[H1]))
        .connect(name, stream)
        .await
        .expect("TLS with the Proxymate leaf");
    let (status, body) = get_json(tls, "localhost").await;
    assert_eq!(status, 200);
    assert_eq!(body, JSON_BODY.as_bytes());
    let (summary, detail) = proxy.last().await;
    assert_eq!(summary.scheme, Scheme::Https);
    assert_eq!(summary.tls, TlsStatus::Intercepted);
    assert_eq!(&*summary.host, "localhost");
    assert_eq!(detail.response.unwrap().status, 200);

    // Plain HTTP over SOCKS is served as HTTP.
    let stream = socks5_connect(proxy.addr, "127.0.0.1", plain.addr().port())
        .await
        .unwrap();
    let (status, body) = get_json(stream, &plain.addr().to_string()).await;
    assert_eq!(status, 200);
    assert_eq!(body, JSON_BODY.as_bytes());
    let (summary, _) = proxy.last().await;
    assert_eq!(summary.scheme, Scheme::Http);
    assert_eq!(summary.path, "/json");

    // Anything else is relayed byte for byte as a tunnel flow.
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = echo.accept().await.unwrap();
        let mut buf = [0u8; 64];
        let n = s.read(&mut buf).await.unwrap();
        s.write_all(&buf[..n]).await.unwrap();
    });
    let mut stream = socks5_connect(proxy.addr, "127.0.0.1", echo_port).await.unwrap();
    stream.write_all(&[0x00, 0x01, 0xfe, 0xff]).await.unwrap();
    let mut back = [0u8; 4];
    stream.read_exact(&mut back).await.unwrap();
    assert_eq!(back, [0x00, 0x01, 0xfe, 0xff]);
    drop(stream);
    let (summary, _) = proxy.last().await;
    assert_eq!(&*summary.method, "CONNECT");
    assert_eq!(summary.tls, TlsStatus::Passthrough);
    assert_eq!(summary.port, echo_port);

    // A closed port gets SOCKS reply 5 (connection refused) via passthrough.
    proxy.engine.add_ssl_bypass("127.0.0.1").unwrap();
    let closed = free_port().await;
    let err = socks5_connect(proxy.addr, "127.0.0.1", closed).await.unwrap_err();
    assert!(err.to_string().contains("0x05"), "{err}");

    // The status lists the SOCKS5 listener.
    let status = proxy.engine.proxy_status();
    assert_eq!(status.listeners[0].kind, ListenerKind::Socks5);
    assert_eq!(status.listeners[0].addr, status.listening);
}

#[tokio::test]
async fn reverse_proxies_serve_plain_http_and_tls() {
    let origin = Upstream::start().await;
    let proxy = Proxy::start(Vec::new()).await;
    let (plain_port, tls_port) = (free_port().await, free_port().await);
    let target = format!("http://{}", origin.addr());
    let reverse = |listen_port, tls| ReverseProxy {
        enabled: true,
        listen_port,
        target_url: target.clone(),
        preserve_host: false,
        tls,
    };
    proxy
        .network(NetworkSettings {
            reverse_proxies: vec![reverse(plain_port, false), reverse(tls_port, true)],
            ..Default::default()
        })
        .await;

    let stream = TcpStream::connect(("127.0.0.1", plain_port)).await.unwrap();
    let (status, body) = get_json(stream, &format!("127.0.0.1:{plain_port}")).await;
    assert_eq!(status, 200);
    assert_eq!(body, JSON_BODY.as_bytes());
    let (summary, detail) = proxy.last().await;
    assert_eq!(
        summary.client.as_deref(),
        Some(format!("Reverse :{plain_port}").as_str())
    );
    assert_eq!(detail.request.url, format!("{target}/json"));
    assert_eq!(summary.port, origin.addr().port());
    assert_eq!(detail.via.unwrap().reverse_port, Some(plain_port));

    let stream = TcpStream::connect(("127.0.0.1", tls_port)).await.unwrap();
    let name = ServerName::try_from("localhost").unwrap();
    let tls = tokio_rustls::TlsConnector::from(tls_config(&proxy.ca(), &[H1]))
        .connect(name, stream)
        .await
        .expect("TLS with a localhost leaf");
    let (status, body) = get_json(tls, "localhost").await;
    assert_eq!(status, 200);
    assert_eq!(body, JSON_BODY.as_bytes());
    let (summary, _) = proxy.last().await;
    assert_eq!(summary.tls, TlsStatus::Intercepted);
    assert_eq!(summary.client.as_deref(), Some(format!("Reverse :{tls_port}").as_str()));

    let listeners = proxy.engine.proxy_status().listeners;
    let kinds: Vec<_> = listeners.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![ListenerKind::Socks5, ListenerKind::Reverse, ListenerKind::ReverseTls]
    );
    assert_eq!(
        listeners[1].addr.as_deref(),
        Some(format!("127.0.0.1:{plain_port}").as_str())
    );
    assert_eq!(listeners[1].target.as_deref(), Some(target.as_str()));

    // Turning them off frees the ports.
    proxy.network(NetworkSettings::default()).await;
    assert_eq!(proxy.engine.proxy_status().listeners.len(), 1);
    TcpListener::bind(("127.0.0.1", plain_port)).await.expect("port freed");
}

#[tokio::test]
async fn client_certificates_open_an_mtls_upstream() {
    let origin = TlsUpstream::mtls(Alpn::Both).await;
    let identity = origin.ca().unwrap().issue_client("pm-client");
    let proxy = Proxy::start(vec![origin.ca_der()]).await;

    // Without one, the failure says what to do.
    let response = https_get(proxy.addr, &origin.url("/json"), &proxy.ca(), &[H1]).await;
    assert_eq!(response.status, 502);
    let (_, detail) = proxy.last().await;
    let error = detail.error.unwrap();
    assert!(error.contains("asks for a client certificate"), "{error}");

    // PEM files.
    let cert_path = proxy.dir.join("client.pem");
    let key_path = proxy.dir.join("client-key.pem");
    std::fs::write(&cert_path, &identity.cert_pem).unwrap();
    std::fs::write(&key_path, &identity.key_pem).unwrap();
    let pem = ClientIdentity::Pem {
        cert_path: cert_path.display().to_string(),
        key_path: key_path.display().to_string(),
    };
    let cert = |identity| ClientCert {
        enabled: true,
        hosts: vec!["localhost".into()],
        identity,
    };
    proxy
        .network(NetworkSettings {
            client_certs: vec![cert(pem)],
            ..Default::default()
        })
        .await;
    for alpn in [&[H1][..], &[b"h2".as_slice()][..]] {
        let response = https_get(proxy.addr, &origin.url("/json"), &proxy.ca(), alpn).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.body, JSON_BODY.as_bytes());
    }
    let (_, detail) = proxy.last().await;
    let via = detail.via.unwrap();
    assert!(via.client_cert.unwrap().contains("pm-client"));

    // A PKCS#12 file with a password, kept in the secret store.
    let p12 = proxy.dir.join("client.p12");
    std::fs::write(&p12, identity.pkcs12("hunter2")).unwrap();
    let pkcs12 = |password: &str| ClientIdentity::Pkcs12 {
        path: p12.display().to_string(),
        password: Some(password.into()),
        has_password: false,
    };
    let err = proxy
        .engine
        .set_network_settings(NetworkSettings {
            client_certs: vec![cert(pkcs12("wrong"))],
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("wrong password"), "{err}");
    let saved = proxy
        .network(NetworkSettings {
            client_certs: vec![cert(pkcs12("hunter2"))],
            ..Default::default()
        })
        .await;
    assert!(matches!(
        &saved.client_certs[0].identity,
        ClientIdentity::Pkcs12 {
            password: None,
            has_password: true,
            ..
        }
    ));
    let response = https_get(proxy.addr, &origin.url("/json"), &proxy.ca(), &[H1]).await;
    assert_eq!(response.status, 200);

    // A fresh engine on the same data dir reads the password back lazily.
    let dir = proxy.dir.clone();
    let again = Proxy::start_in(dir, vec![origin.ca_der()]).await;
    std::mem::forget(proxy);
    let response = https_get(again.addr, &origin.url("/json"), &again.ca(), &[H1]).await;
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn dns_overrides_reach_names_that_dont_exist() {
    let plain = Upstream::start().await;
    let origin = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(vec![origin.ca_der()]).await;
    let override_ = |host: &str| DnsOverride {
        enabled: true,
        host: host.into(),
        ip: "127.0.0.1".into(),
    };
    proxy
        .network(NetworkSettings {
            dns: vec![override_("*.proxymate-nowhere.test")],
            ..Default::default()
        })
        .await;

    let url = format!("http://api.proxymate-nowhere.test:{}/inspect", plain.addr().port());
    let (status, body) = client::get(proxy.addr, &url).await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let seen: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // `Host` is unchanged.
    assert_eq!(
        seen["headers"]["host"],
        format!("api.proxymate-nowhere.test:{}", plain.addr().port())
    );
    let (_, detail) = proxy.last().await;
    let via = detail.via.unwrap();
    assert_eq!(
        via.dns_override.as_deref(),
        Some("*.proxymate-nowhere.test → 127.0.0.1")
    );
    assert!(via.addr.unwrap().starts_with("127.0.0.1:"));

    // HTTPS keeps the name for SNI; the test certificate is for localhost,
    // so verification is relaxed for this host.
    proxy
        .engine
        .set_insecure_upstream("secure.proxymate-nowhere.test", true)
        .unwrap();
    let url = format!("https://secure.proxymate-nowhere.test:{}/json", origin.port());
    let response = https_get(proxy.addr, &url, &proxy.ca(), &[H1]).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, JSON_BODY.as_bytes());
}

#[tokio::test]
async fn the_proxy_set_before_proxymate_is_used_when_asked() {
    let origin = TlsUpstream::start(Alpn::Http11).await;
    let corp = HttpProxy::start().await;
    let dir = temp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    // What the system proxy backup looks like while Proxymate is the
    // system proxy (it never touches the real settings here: this
    // process owns the backup, and nothing restores it).
    let backup = serde_json::json!({
        "pid": std::process::id(),
        "host": "127.0.0.1",
        "port": 1,
        "services": [{
            "service": "pm-test",
            "web": { "enabled": true, "server": "127.0.0.1", "port": corp.port() },
            "secure": { "enabled": true, "server": "127.0.0.1", "port": corp.port() },
            "bypass": ["*.bypassed.test"],
        }],
    });
    std::fs::write(dir.join("sysproxy-backup.json"), backup.to_string()).unwrap();
    let proxy = Proxy::start_in(dir, vec![origin.ca_der()]).await;
    let host = "corp.proxymate-nowhere.test";
    proxy.engine.set_insecure_upstream(host, true).unwrap();
    proxy
        .network(NetworkSettings {
            upstream: UpstreamSettings {
                rules: Vec::new(),
                bypass: Vec::new(),
                use_system_previous: true,
            },
            dns: vec![DnsOverride {
                enabled: true,
                host: host.into(),
                ip: "127.0.0.1".into(),
            }],
            ..Default::default()
        })
        .await;
    let url = format!("https://{host}:{}/json", origin.port());
    let response = https_get(proxy.addr, &url, &proxy.ca(), &[H1]).await;
    assert_eq!(response.status, 200);
    // The override applies to what the proxy is asked for.
    assert_eq!(corp.requests()[0].target, format!("127.0.0.1:{}", origin.port()));
    let (_, detail) = proxy.last().await;
    assert_eq!(
        detail.via.unwrap().proxy.as_deref(),
        Some(format!("http://127.0.0.1:{}", corp.port()).as_str())
    );
}
