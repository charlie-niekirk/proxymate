//! Device setup end to end: the onboarding server's routes, through the
//! proxy and addressed to it directly, and switching LAN access.

use std::net::SocketAddr;
use std::sync::Arc;

use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_testkit::client;

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

/// Splits a raw HTTP/1.1 response into its head (lower-cased) and body.
fn split(response: &[u8]) -> (String, Vec<u8>) {
    let at = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a complete response head");
    (
        String::from_utf8_lossy(&response[..at]).to_ascii_lowercase(),
        response[at + 4..].to_vec(),
    )
}

async fn origin_get(proxy: SocketAddr, path: &str) -> (String, Vec<u8>) {
    let request = format!("GET {path} HTTP/1.1\r\nHost: {proxy}\r\nConnection: close\r\n\r\n");
    split(&client::raw(proxy, request.as_bytes()).await)
}

#[tokio::test]
async fn onboarding_routes_serve_the_ca() {
    let (engine, proxy) = proxy().await;
    let ca_pem = std::fs::read_to_string(engine.ca_info().cert_path).unwrap();
    let ca_der = pm_core::tls::certs_from_pem_file(engine.ca_info().cert_path.as_ref())
        .unwrap()
        .remove(0);

    let (head, body) = origin_get(proxy, "/").await;
    assert!(
        head.starts_with("http/1.1 200") && head.contains("content-type: text/html"),
        "{head}"
    );
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("/proxymate.mobileconfig") && html.contains(&engine.ca_info().sha256));
    // The page names the address the device used as the proxy to set.
    assert!(html.contains(&proxy.to_string()));

    let (head, body) = origin_get(proxy, "/cert.pem").await;
    assert!(head.contains("content-type: application/x-pem-file"), "{head}");
    assert!(head.contains("content-disposition: attachment; filename=\"proxymate-ca.pem\""));
    assert_eq!(
        String::from_utf8(body).unwrap().replace('\n', ""),
        ca_pem.replace(['\n', '\r'], "")
    );

    let (head, body) = origin_get(proxy, "/cert.crt").await;
    assert!(head.contains("content-type: application/x-x509-ca-cert"), "{head}");
    assert_eq!(body, ca_der);

    let (head, body) = origin_get(proxy, "/proxymate.mobileconfig").await;
    assert!(
        head.contains("content-type: application/x-apple-aspen-config"),
        "{head}"
    );
    let profile = plist::Value::from_reader_xml(body.as_slice()).expect("a plist");
    let payload = profile.as_dictionary().unwrap()["PayloadContent"].as_array().unwrap()[0]
        .as_dictionary()
        .unwrap()
        .clone();
    assert_eq!(payload["PayloadContent"].as_data(), Some(ca_der.as_slice()));

    let (head, _) = origin_get(proxy, "/nope").await;
    assert!(head.starts_with("http/1.1 404"), "{head}");

    engine.flush();
    assert_eq!(engine.total(), 0, "onboarding requests aren't recorded");
}

#[tokio::test]
async fn the_friendly_host_is_answered_by_the_proxy() {
    let (engine, proxy) = proxy().await;
    let (status, body) = client::get(proxy, "http://proxy.mate/cert.crt").await;
    assert_eq!(status, 200);
    assert!(body.starts_with(&[0x30, 0x82]), "a DER certificate");
    let (status, _) = client::get(proxy, "http://PROXY.MATE:8080/").await;
    assert_eq!(status, 200);

    // https:// is refused so a browser falls back to the plain page.
    let response = client::raw(
        proxy,
        b"CONNECT proxy.mate:443 HTTP/1.1\r\nHost: proxy.mate:443\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 502"));
    engine.flush();
    assert_eq!(engine.total(), 0);
}

#[tokio::test]
async fn lan_access_rebinds_on_the_same_port() {
    let (engine, proxy) = proxy().await;
    assert!(!engine.device_access().enabled);
    assert_eq!(engine.proxy_status().listening, Some(proxy.to_string()));

    let access = engine.set_lan_access(true).await;
    assert!(access.enabled, "{:?}", access.error);
    assert_eq!(access.port, Some(proxy.port()));
    let listening = engine.proxy_status().listening.unwrap();
    assert!(listening.ends_with(&format!(":{}", proxy.port())), "{listening}");
    // Loopback still works, and so does our own LAN address.
    let (head, _) = origin_get(proxy, "/").await;
    assert!(head.starts_with("http/1.1 200"), "{head}");
    if let Some(lan) = &access.proxy {
        assert_eq!(&listening, lan);
        let url = access.setup_url.clone().unwrap();
        assert_eq!(url, format!("http://{lan}/"));
        assert!(access.qr_svg.as_deref().is_some_and(|svg| svg.contains("<svg")));
        let (head, _) = origin_get(lan.parse().unwrap(), "/").await;
        assert!(head.starts_with("http/1.1 200"), "{head}");
    }

    let access = engine.set_lan_access(false).await;
    assert!(!access.enabled && access.qr_svg.is_none());
    assert_eq!(engine.proxy_status().listening, Some(proxy.to_string()));
    let (head, _) = origin_get(proxy, "/").await;
    assert!(head.starts_with("http/1.1 200"), "{head}");
}
