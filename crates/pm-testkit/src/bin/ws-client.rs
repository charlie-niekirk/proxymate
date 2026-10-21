//! A WebSocket client that talks to a pm-testkit echo server through the
//! proxy, for trying the WebSocket panel by hand:
//!
//! ```sh
//! cargo run -p pm-testkit --bin upstream            # or `-- --tls` for wss
//! cargo run -p pm-testkit --bin ws-client -- ws://127.0.0.1:<port>/ws
//! cargo run -p pm-testkit --bin ws-client -- wss://localhost:<port>/ws
//! ```
//!
//! Options: `--proxy <addr>` (default 127.0.0.1:9090), `--count <n>`
//! messages (default 20), `--interval-ms <ms>` between them (default 500),
//! `--absolute` to send a `ws://` handshake in absolute form instead of
//! through a CONNECT tunnel, `--ca <pem>` for the Proxymate CA (default: the
//! app's), and `--server-close` to have the server close instead.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use pm_testkit::client::{tls_config, tls_through, tunnel, ws_absolute, ws_over};
use pm_testkit::stream::{CLOSE_COMMAND, PING_COMMAND};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

struct Options {
    url: String,
    proxy: SocketAddr,
    count: u32,
    interval: Duration,
    absolute: bool,
    ca: Option<String>,
    server_close: bool,
}

fn options() -> Options {
    let mut args = std::env::args().skip(1);
    let mut options = Options {
        url: String::new(),
        proxy: "127.0.0.1:9090".parse().expect("default proxy"),
        count: 20,
        interval: Duration::from_millis(500),
        absolute: false,
        ca: None,
        server_close: false,
    };
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| panic!("{arg} needs a value"));
        match arg.as_str() {
            "--proxy" => options.proxy = value().parse().expect("--proxy host:port"),
            "--count" => options.count = value().parse().expect("--count number"),
            "--interval-ms" => options.interval = Duration::from_millis(value().parse().expect("--interval-ms number")),
            "--ca" => options.ca = Some(value()),
            "--absolute" => options.absolute = true,
            "--server-close" => options.server_close = true,
            url => options.url = url.to_owned(),
        }
    }
    assert!(
        !options.url.is_empty(),
        "usage: ws-client [options] ws[s]://host:port/ws"
    );
    options
}

#[tokio::main]
async fn main() {
    let options = options();
    let uri: http::Uri = options.url.parse().expect("valid url");
    let host = uri.host().expect("url host").to_owned();
    match uri.scheme_str() {
        Some("wss") => {
            let ca = options.ca.clone().unwrap_or_else(|| {
                let home = std::env::var("HOME").expect("HOME");
                format!("{home}/Library/Application Support/Proxymate/ca/proxymate-ca.pem")
            });
            let ca = CertificateDer::from_pem_file(&ca).unwrap_or_else(|e| panic!("read the CA {ca}: {e}"));
            let port = uri.port_u16().unwrap_or(443);
            let tls = tls_through(options.proxy, &host, port, tls_config(&ca, &[b"http/1.1"]))
                .await
                .expect("TLS through the proxy");
            run(ws_over(tls, &options.url).await, &options).await;
        }
        Some("ws") if options.absolute => {
            let http_url = options.url.replacen("ws://", "http://", 1);
            let (head, ws) = ws_absolute(options.proxy, &http_url, &[]).await;
            run(ws.unwrap_or_else(|| panic!("handshake answered {head}")), &options).await;
        }
        Some("ws") => {
            let authority = uri.authority().expect("url authority").as_str().to_owned();
            let stream = tunnel(options.proxy, &authority).await;
            run(ws_over(stream, &options.url).await, &options).await;
        }
        _ => panic!("expected a ws:// or wss:// url"),
    }
}

async fn run<S: AsyncRead + AsyncWrite + Unpin>(mut ws: WebSocketStream<S>, options: &Options) {
    for n in 0..options.count {
        let message = match n % 5 {
            3 => Message::binary((0..64u8).map(|b| b.wrapping_mul(n as u8 + 7)).collect::<Vec<_>>()),
            4 => Message::Ping(Bytes::from(format!("ping {n}"))),
            _ => Message::text(format!(
                r#"{{"op":"tick","n":{n},"payload":{{"price":{},"ok":true}}}}"#,
                100 + n
            )),
        };
        ws.send(message).await.expect("send");
        if n == options.count / 2 {
            ws.send(Message::text(PING_COMMAND)).await.expect("send");
        }
        println!("sent message {n}");
        tokio::time::sleep(options.interval).await;
        // Drain whatever the server sent back meanwhile.
        while let Ok(Some(Ok(reply))) = tokio::time::timeout(Duration::from_millis(1), ws.next()).await {
            if let Message::Text(text) = &reply {
                println!("received {text}");
            }
        }
    }
    if options.server_close {
        ws.send(Message::text(CLOSE_COMMAND)).await.expect("send");
    } else {
        ws.close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "done".into(),
        }))
        .await
        .expect("close");
    }
    while let Ok(Some(Ok(_))) = tokio::time::timeout(Duration::from_secs(5), ws.next()).await {}
    println!("closed");
}
