//! OS integration through the engine: client app detection for loopback
//! connections, per-app SSL exclusions, the sidebar aggregates and the
//! certificate trust check. Nothing here changes the machine's settings.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{FlowState, KeychainTrust, SidebarSnapshot, TlsStatus};
use pm_testkit::client::{self, eventually, https_get};
use pm_testkit::{Alpn, TlsUpstream, Upstream};

const WAIT: Duration = Duration::from_secs(10);

async fn proxy(upstream: Option<&TlsUpstream>) -> (Arc<Engine>, SocketAddr) {
    let dir = std::env::temp_dir()
        .join("proxymate-tests")
        .join(uuid::Uuid::new_v4().to_string());
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(dir),
        extra_upstream_roots: upstream.map(|u| vec![u.ca_der()]).unwrap_or_default(),
        ..Default::default()
    });
    let config = ProxyConfig {
        ports: 0..=0,
        ..Default::default()
    };
    let addr = engine.start_proxy(config).await.expect("proxy starts");
    (engine, addr)
}

/// What app detection calls this test binary.
fn own_name() -> String {
    let exe = std::env::current_exe().unwrap();
    exe.file_name().unwrap().to_string_lossy().into_owned()
}

#[tokio::test]
async fn loopback_clients_are_named() {
    let upstream = Upstream::start().await;
    let (engine, addr) = proxy(None).await;
    let (status, _) = client::get(addr, &upstream.url("/json")).await;
    assert_eq!(status, 200);

    if cfg!(target_os = "macos") {
        // The flow may start before the scan finishes; it is patched then.
        let flow = eventually(WAIT, || {
            engine.flush();
            engine.get_flow(0).filter(|f| f.client.is_some())
        })
        .await;
        assert_eq!(flow.client.as_deref(), Some(own_name().as_str()));
        // A plain executable has no bundle, so no icon.
        assert!(engine.app_icon(&own_name()).is_none());
    } else {
        engine.flush();
        assert_eq!(engine.get_flow(0).unwrap().client, None);
    }
}

#[tokio::test]
async fn excluded_apps_are_tunnelled() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let (engine, addr) = proxy(Some(&upstream)).await;
    engine.set_app_ssl_excluded(&own_name(), true).unwrap();

    // Passthrough: the client sees the upstream's own certificate.
    let ca = rustls::pki_types::CertificateDer::from(upstream.ca_der());
    let response = https_get(addr, &upstream.url("/json"), &ca, &[b"http/1.1"]).await;
    assert_eq!(response.status, 200);
    let flow = eventually(WAIT, || {
        engine.flush();
        engine.get_flow(0).filter(|f| f.state != FlowState::Active)
    })
    .await;
    assert_eq!((flow.method.as_ref(), flow.tls), ("CONNECT", TlsStatus::Passthrough));
    assert_eq!(flow.client.as_deref(), Some(own_name().as_str()));

    let settings = engine.set_app_ssl_excluded(&own_name().to_uppercase(), false).unwrap();
    assert!(settings.exclude_apps.is_empty());
}

#[tokio::test]
async fn the_trust_check_handshakes_without_recording() {
    let (engine, _addr) = proxy(None).await;
    let status = engine.trust_status().await;
    // A fresh CA in a temp dir is trusted by no one.
    assert!(!status.verified);
    let detail = status.detail.unwrap();
    assert!(detail.contains("doesn't trust the Proxymate CA"), "{detail}");
    let expected = if cfg!(target_os = "macos") {
        KeychainTrust::NotTrusted
    } else {
        KeychainTrust::Unsupported
    };
    assert_eq!(status.keychain, expected);
    engine.flush();
    assert_eq!(engine.total(), 0, "the probe must not show up as a flow");
}

#[tokio::test]
async fn the_sidebar_is_pushed_when_it_changes() {
    let (engine, _addr) = proxy(None).await;
    let mut generator = pm_core::synth::Generator::new(3);
    engine.insert_flows((0..500).map(|_| generator.completed_flow()).collect());

    let (tx, rx) = mpsc::channel::<SidebarSnapshot>();
    engine.subscribe_sidebar(Box::new(move |s| tx.send(s).is_ok()));
    engine.publish_tick();
    let first = rx.recv_timeout(WAIT).unwrap();
    assert_eq!(first.total, 500);
    let apps: u32 = first.apps.iter().map(|a| a.count).sum();
    let domains: u32 = first.domains.iter().map(|d| d.count).sum();
    assert_eq!(domains, 500);
    assert!(apps > 0 && apps <= 500);
    assert!(
        first
            .apps
            .windows(2)
            .all(|w| w[0].name.to_lowercase() <= w[1].name.to_lowercase())
    );

    // Nothing changed: nothing is sent.
    engine.publish_tick();
    engine.publish_tick();
    while let Ok(extra) = rx.try_recv() {
        assert_eq!(extra, first, "only a racing publisher tick may resend, unchanged");
    }

    engine.clear();
    engine.publish_tick();
    let cleared = eventually(WAIT, || rx.try_recv().ok().filter(|s| s.total == 0)).await;
    assert!(cleared.apps.is_empty() && cleared.domains.is_empty());
}
