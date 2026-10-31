//! Runs a test upstream on its own, for trying the proxy by hand:
//!
//! ```sh
//! cargo run -p pm-testkit --bin upstream          # plain HTTP, random port
//! curl -x http://127.0.0.1:9090 http://127.0.0.1:<port>/json
//!
//! cargo run -p pm-testkit --bin upstream -- --tls # h2 + http/1.1 over TLS
//! PROXYMATE_EXTRA_UPSTREAM_CA=<printed ca.pem> npm run tauri dev
//! curl -x http://127.0.0.1:9090 --cacert <Proxymate CA> https://localhost:<port>/json --http2 -v
//!
//! cargo run -p pm-testkit --bin upstream -- --grpc [PORT]  # shop.v1.Orders over TLS (h2, gRPC-web on h1)
//! PROXYMATE_EXTRA_UPSTREAM_CA=<printed ca.pem> npm run tauri dev
//! HTTPS_PROXY=http://127.0.0.1:9090 grpcurl -cacert <Proxymate CA> \
//!   -import-path crates/pm-testkit/proto -proto shop.proto \
//!   -d '{"id":"o1"}' localhost:<port> shop.v1.Orders/GetOrder
//!
//! cargo run -p pm-testkit --bin upstream -- --zoo [PROXY]
//! # sends every body-viewer zoo fixture through PROXY (default 127.0.0.1:9090)
//!
//! cargo run -p pm-testkit --bin upstream -- --http-proxy [USER:PASSWORD]  # an HTTP proxy to chain to (M18)
//! cargo run -p pm-testkit --bin upstream -- --socks5 [USER:PASSWORD]      # a SOCKS5 proxy to chain to (M18)
//! # then point a rule in Settings › Network at the printed port; every request is logged
//! ```

use pm_testkit::{Alpn, GrpcUpstream, HttpProxy, Socks5Server, TlsUpstream, Upstream, zoo};

const ROUTES: &str = "/json /version /status/{code} /chunked /gzip /br /deflate /zstd /bytes/{n} /echo /zoo/{name} /ws \
                      /sse?count=N&interval_ms=M";

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--zoo") {
        let proxy = args
            .get(i + 1)
            .map_or("127.0.0.1:9090", String::as_str)
            .parse()
            .expect("PROXY is ip:port");
        let upstream = Upstream::start().await;
        println!(
            "zoo upstream on {}; sending every fixture through {proxy}",
            upstream.url("")
        );
        for (name, status) in zoo::send_all(proxy, upstream.addr()).await {
            println!("  {status} {}", upstream.url(&format!("/zoo/{name}")));
        }
        println!("done; serving until Ctrl-C");
        let _ = tokio::signal::ctrl_c().await;
    } else if let Some(i) = args.iter().position(|a| a == "--grpc") {
        let port: u16 = args.get(i + 1).map_or(0, |p| p.parse().expect("PORT is a number"));
        let upstream = GrpcUpstream::bind(&format!("127.0.0.1:{port}")).await;
        let ca_path = std::env::temp_dir().join("pm-testkit-grpc-ca.pem");
        std::fs::write(&ca_path, upstream.ca().pem()).expect("write test CA");
        println!(
            "gRPC test upstream (shop.v1.Orders; gRPC-web too) on localhost:{}",
            upstream.port()
        );
        println!(
            "its CA: {}  (set PROXYMATE_EXTRA_UPSTREAM_CA to this)",
            ca_path.display()
        );
        println!("schema: crates/pm-testkit/proto/shop.proto");
        println!("methods: GetOrder (id \"missing\" → NOT_FOUND), ListOrders (server stream: count, interval_ms)");
        let _ = tokio::signal::ctrl_c().await;
    } else if let Some(i) = args.iter().position(|a| a == "--http-proxy" || a == "--socks5") {
        let credentials = args.get(i + 1).and_then(|c| c.split_once(':'));
        if args[i] == "--http-proxy" {
            let proxy = match credentials {
                Some((user, password)) => HttpProxy::with_basic_auth(user, password).await,
                None => HttpProxy::start().await,
            };
            println!("HTTP proxy (CONNECT and absolute form) on 127.0.0.1:{}", proxy.port());
            let mut seen = 0;
            loop {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                        for r in proxy.requests().iter().skip(seen) {
                            println!("  {} {}{}", r.method, r.target, if r.allowed { "" } else { "  → 407" });
                            seen += 1;
                        }
                    }
                }
            }
        } else {
            let server = match credentials {
                Some((user, password)) => Socks5Server::with_auth(user, password).await,
                None => Socks5Server::start().await,
            };
            println!("SOCKS5 proxy on 127.0.0.1:{}", server.port());
            let mut seen = 0;
            loop {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                        for target in server.targets().iter().skip(seen) {
                            println!("  CONNECT {target}");
                            seen += 1;
                        }
                    }
                }
            }
        }
    } else if args.iter().any(|a| a == "--tls") {
        let upstream = TlsUpstream::start(Alpn::Both).await;
        let ca_path = std::env::temp_dir().join("pm-testkit-ca.pem");
        std::fs::write(&ca_path, upstream.ca().expect("test CA").pem()).expect("write test CA");
        println!("TLS test upstream (h2, http/1.1) on {}", upstream.url(""));
        println!(
            "its CA: {}  (set PROXYMATE_EXTRA_UPSTREAM_CA to this)",
            ca_path.display()
        );
        println!("routes: {ROUTES}");
        let _ = tokio::signal::ctrl_c().await;
    } else {
        let upstream = Upstream::start().await;
        println!("test upstream on {}", upstream.url(""));
        println!("routes: {ROUTES}");
        let _ = tokio::signal::ctrl_c().await;
    }
}
