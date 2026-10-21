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
//! cargo run -p pm-testkit --bin upstream -- --zoo [PROXY]
//! # sends every body-viewer zoo fixture through PROXY (default 127.0.0.1:9090)
//! ```

use pm_testkit::{Alpn, TlsUpstream, Upstream, zoo};

const ROUTES: &str = "/json /version /status/{code} /chunked /gzip /br /deflate /zstd /bytes/{n} /echo /zoo/{name}";

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
