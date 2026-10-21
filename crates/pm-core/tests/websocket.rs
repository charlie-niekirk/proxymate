//! WebSockets through the proxy, against the pm-testkit echo server: plain
//! `ws://` in absolute form and inside a CONNECT, and `wss://` through the
//! MITM. Every message must be relayed and recorded, and the close must be
//! reported on the flow.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{FlowId, FlowState, FlowSummary, Scheme, WsDirection, WsMessage, WsOpcode, WsPage, WsQuery};
use pm_testkit::client::{eventually, tls_config, tls_through, tunnel, ws_absolute, ws_over};
use pm_testkit::stream::{
    CLOSE_COMMAND, PING_COMMAND, SEEN_EXTENSIONS, SERVER_CLOSE_CODE, SERVER_CLOSE_REASON, SERVER_PING,
};
use pm_testkit::{Alpn, TlsUpstream, Upstream};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

const WAIT: Duration = Duration::from_secs(10);

struct Proxy {
    engine: Arc<Engine>,
    addr: SocketAddr,
}

impl Proxy {
    async fn start(upstream: Option<&TlsUpstream>) -> Proxy {
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
        Proxy { engine, addr }
    }

    fn ca(&self) -> CertificateDer<'static> {
        CertificateDer::from_pem_file(self.engine.ca_info().cert_path).expect("CA PEM")
    }

    fn messages(&self, id: FlowId) -> WsPage {
        self.engine
            .websocket_messages(id, &WsQuery::default(), 0, 1_000)
            .expect("messages")
    }

    /// Waits for at least `n` recorded messages.
    async fn wait_messages(&self, id: FlowId, n: u32) -> WsPage {
        eventually(WAIT, || Some(self.messages(id)).filter(|p| p.total >= n)).await
    }

    async fn finished(&self, id: FlowId) -> FlowSummary {
        eventually(WAIT, || {
            self.engine.flush();
            self.engine.get_flow(id).filter(|f| f.state != FlowState::Active)
        })
        .await
    }
}

/// The next message that isn't a pong (the proxy and the echo server may
/// both answer a ping).
async fn next<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>) -> Message {
    loop {
        let message = tokio::time::timeout(WAIT, ws.next())
            .await
            .expect("a message in time")
            .expect("socket open")
            .expect("valid message");
        if !matches!(message, Message::Pong(_)) {
            return message;
        }
    }
}

/// Text, binary and a ping, each answered by the echo server.
async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>) {
    ws.send(Message::text("hello")).await.unwrap();
    assert_eq!(next(ws).await, Message::text("hello"));
    ws.send(Message::binary(vec![0u8, 1, 2, 255])).await.unwrap();
    assert_eq!(next(ws).await, Message::binary(vec![0u8, 1, 2, 255]));
    ws.send(Message::Ping(Bytes::from_static(b"are you there")))
        .await
        .unwrap();
    let pong = tokio::time::timeout(WAIT, ws.next()).await.unwrap().unwrap().unwrap();
    assert_eq!(pong, Message::Pong(Bytes::from_static(b"are you there")));
}

fn kinds(page: &WsPage) -> Vec<(WsDirection, WsOpcode)> {
    page.messages.iter().map(|m| (m.direction, m.opcode)).collect()
}

fn find(page: &WsPage, direction: WsDirection, opcode: WsOpcode) -> &WsMessage {
    page.messages
        .iter()
        .find(|m| m.direction == direction && m.opcode == opcode)
        .unwrap_or_else(|| panic!("no {direction:?} {opcode:?} in {:?}", kinds(page)))
}

fn assert_exchange_recorded(page: &WsPage) {
    use WsDirection::{Received, Sent};
    let text = find(page, Sent, WsOpcode::Text);
    assert_eq!((text.preview.as_str(), text.len), ("hello", 5));
    assert_eq!(find(page, Received, WsOpcode::Text).preview, "hello");
    let binary = find(page, Received, WsOpcode::Binary);
    assert_eq!((binary.len, binary.preview.as_str()), (4, ""));
    assert_eq!(find(page, Sent, WsOpcode::Ping).preview, "are you there");
    assert_eq!(find(page, Received, WsOpcode::Pong).preview, "are you there");
    assert!(page.messages.windows(2).all(|w| w[0].time_ms <= w[1].time_ms));
}

#[tokio::test]
async fn ws_in_absolute_form_is_relayed_and_recorded_until_the_client_closes() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    let url = upstream.url("/ws?room=1");
    let (head, ws) = ws_absolute(proxy.addr, &url, &[("Sec-WebSocket-Extensions", "permessage-deflate")]).await;
    let mut ws = ws.unwrap_or_else(|| panic!("handshake answered {head:?}"));
    // Frames must stay readable: the proxy drops the compression offer.
    assert!(
        head.to_ascii_lowercase().contains(&format!("{SEEN_EXTENSIONS}: none")),
        "{head}"
    );

    exchange(&mut ws).await;
    // The flow is live while the socket is open.
    let page = proxy.wait_messages(0, 6).await;
    assert!(page.open);
    proxy.engine.flush();
    let flow = proxy.engine.get_flow(0).unwrap();
    assert_eq!(
        (flow.scheme, flow.state, flow.status),
        (Scheme::Ws, FlowState::Active, Some(101))
    );
    assert_eq!((flow.path.as_str(), flow.query.as_str()), ("/ws", "room=1"));

    ws.close(Some(CloseFrame {
        code: CloseCode::Normal,
        reason: "done".into(),
    }))
    .await
    .unwrap();
    // The close completes: the proxy answers and the socket ends.
    while let Ok(Some(Ok(_))) = tokio::time::timeout(WAIT, ws.next()).await {}

    let flow = proxy.finished(0).await;
    assert_eq!(flow.state, FlowState::Completed);
    let page = proxy.messages(0);
    assert!(!page.open);
    let close = page.close.clone().expect("close recorded");
    assert_eq!(
        (close.code, close.reason.as_str(), close.by),
        (1000, "done", WsDirection::Sent)
    );
    assert_exchange_recorded(&page);
    let sent_close = find(&page, WsDirection::Sent, WsOpcode::Close);
    assert_eq!(
        (sent_close.close_code, sent_close.preview.as_str()),
        (Some(1000), "done")
    );
    // Payload bytes are the flow's sizes.
    let sent: u64 = page
        .messages
        .iter()
        .filter(|m| m.direction == WsDirection::Sent)
        .map(|m| m.len)
        .sum();
    assert_eq!(flow.req_size, sent);
    assert_eq!(
        proxy
            .engine
            .websocket_payload(0, find(&page, WsDirection::Received, WsOpcode::Binary).index)
            .unwrap(),
        vec![0u8, 1, 2, 255]
    );
}

#[tokio::test]
async fn wss_through_the_mitm_records_a_server_close() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;
    let config = tls_config(&proxy.ca(), &[b"http/1.1"]);
    let tls = tls_through(proxy.addr, "localhost", upstream.port(), config)
        .await
        .expect("TLS through the proxy");
    let mut ws = ws_over(tls, &format!("wss://localhost:{}/ws", upstream.port())).await;

    exchange(&mut ws).await;
    // The server pings; the client library answers it.
    ws.send(Message::text(PING_COMMAND)).await.unwrap();
    assert_eq!(next(&mut ws).await, Message::Ping(Bytes::from_static(SERVER_PING)));
    ws.send(Message::text(CLOSE_COMMAND)).await.unwrap();
    match next(&mut ws).await {
        Message::Close(Some(frame)) => assert_eq!(u16::from(frame.code), SERVER_CLOSE_CODE),
        other => panic!("expected the server's close, got {other:?}"),
    }
    while let Ok(Some(Ok(_))) = tokio::time::timeout(WAIT, ws.next()).await {}

    let flow = proxy.finished(0).await;
    assert_eq!(
        (flow.scheme, flow.state, flow.status),
        (Scheme::Wss, FlowState::Completed, Some(101))
    );
    let detail = proxy.engine.flow_detail(0).unwrap().unwrap();
    assert!(
        detail.request.url.starts_with("wss://localhost:"),
        "{}",
        detail.request.url
    );
    assert!(detail.tls.is_some());
    let page = proxy.messages(0);
    assert_exchange_recorded(&page);
    assert_eq!(
        find(&page, WsDirection::Received, WsOpcode::Ping).preview,
        "server ping"
    );
    let close = page.close.expect("close recorded");
    assert_eq!(
        (close.code, close.reason.as_str(), close.by),
        (SERVER_CLOSE_CODE, SERVER_CLOSE_REASON, WsDirection::Received)
    );

    // The connection is usable for ordinary requests again afterwards.
    let response = pm_testkit::client::https_get(proxy.addr, &upstream.url("/json"), &proxy.ca(), &[b"http/1.1"]).await;
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn ws_inside_connect_records_a_dropped_connection() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    let stream = tunnel(proxy.addr, &upstream.addr().to_string()).await;
    let mut ws = ws_over(stream, &format!("ws://{}/ws", upstream.addr())).await;
    ws.send(Message::text("one")).await.unwrap();
    assert_eq!(next(&mut ws).await, Message::text("one"));
    proxy.wait_messages(0, 2).await;
    drop(ws);

    let flow = proxy.finished(0).await;
    assert_eq!((flow.scheme, flow.state), (Scheme::Ws, FlowState::Completed));
    let close = proxy.messages(0).close.expect("close recorded");
    assert_eq!((close.code, close.by), (1006, WsDirection::Sent));
}

#[tokio::test]
async fn messages_can_be_searched_and_filtered_by_direction() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    let (_, ws) = ws_absolute(proxy.addr, &upstream.url("/ws"), &[]).await;
    let mut ws = ws.expect("handshake");
    for text in ["alpha", "beta", "ALPHABET"] {
        ws.send(Message::text(text)).await.unwrap();
        next(&mut ws).await;
    }
    proxy.wait_messages(0, 6).await;
    let query = |direction, text: &str| WsQuery {
        direction,
        text: text.into(),
    };
    let page = proxy
        .engine
        .websocket_messages(0, &query(Some(WsDirection::Received), "alpha"), 0, 10)
        .unwrap();
    assert_eq!((page.total, page.len), (6, 2));
    assert!(page.messages.iter().all(|m| m.direction == WsDirection::Received));
    let window = proxy.engine.websocket_messages(0, &query(None, ""), 4, 10).unwrap();
    assert_eq!(window.messages.iter().map(|m| m.index).collect::<Vec<_>>(), vec![4, 5]);
}

#[tokio::test]
async fn a_refused_upgrade_is_an_ordinary_response() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    let (head, ws) = ws_absolute(proxy.addr, &upstream.url("/status/403"), &[]).await;
    assert!(ws.is_none());
    assert!(head.starts_with("HTTP/1.1 403 "), "{head}");
    let flow = proxy.finished(0).await;
    assert_eq!((flow.scheme, flow.status), (Scheme::Ws, Some(403)));
    let page = proxy.messages(0);
    assert_eq!((page.total, page.open), (0, false));
}
