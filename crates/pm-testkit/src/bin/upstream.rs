//! Runs the test upstream on its own, for trying the proxy by hand:
//!
//! ```sh
//! cargo run -p pm-testkit --bin upstream          # random port
//! curl -x http://127.0.0.1:9090 http://127.0.0.1:<port>/json
//! ```

use pm_testkit::Upstream;

#[tokio::main]
async fn main() {
    let upstream = Upstream::start().await;
    println!("test upstream on {}", upstream.url(""));
    println!("routes: /json /status/{{code}} /chunked /gzip /br /deflate /zstd /bytes/{{n}} /echo");
    let _ = tokio::signal::ctrl_c().await;
}
