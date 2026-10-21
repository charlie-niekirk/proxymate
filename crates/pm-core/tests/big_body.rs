//! A 50 MB response streams through the proxy and into the segment files
//! without the proxy holding it in memory. Its own test binary, so the RSS
//! measurement sees no other test's allocations.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use http::Request;
use http_body_util::{BodyExt, Full};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{BodyPart, FlowState};
use pm_testkit::Upstream;
use pm_testkit::client::{self, eventually};
use pm_testkit::upstream::pattern_byte;

const LEN: u64 = 50 * 1024 * 1024;
/// Peak RSS growth allowed while the body streams. Buffering it would cost
/// at least `LEN`.
const MAX_GROWTH: u64 = 24 * 1024 * 1024;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fifty_megabytes_pass_through_with_bounded_memory() {
    let upstream = Upstream::start().await;
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(std::env::temp_dir().join("proxymate-tests")),
        ..Default::default()
    });
    let proxy = engine
        .start_proxy(ProxyConfig {
            ports: 0..=0,
            ..Default::default()
        })
        .await
        .unwrap();

    let baseline = rss_bytes();
    let peak = Arc::new(AtomicU64::new(baseline));
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (peak, stop) = (peak.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                peak.fetch_max(rss_bytes(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    };

    let req = Request::get(upstream.url(&format!("/bytes/{LEN}")))
        .body(Full::default())
        .unwrap();
    let mut body = client::send(proxy, req).await.into_body();
    let mut received = 0u64;
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.unwrap().into_data() {
            for (i, byte) in data.iter().enumerate() {
                assert_eq!(*byte, pattern_byte(received + i as u64), "byte {}", received + i as u64);
            }
            received += data.len() as u64;
        }
    }
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();
    assert_eq!(received, LEN);

    let growth = peak.load(Ordering::Relaxed).saturating_sub(baseline);
    eprintln!("peak RSS growth while streaming: {:.1} MB", growth as f64 / 1048576.0);
    assert!(
        growth < MAX_GROWTH,
        "RSS grew by {} MB while streaming",
        growth / 1024 / 1024
    );

    let flow = eventually(Duration::from_secs(10), || {
        engine.flush();
        engine.get_flow(0).filter(|f| f.state != FlowState::Active)
    })
    .await;
    assert_eq!((flow.state, flow.resp_size), (FlowState::Completed, LEN));
    let detail = engine.flow_detail(0).unwrap().unwrap();
    assert_eq!(detail.response_body.captured, LEN);
    assert!(!detail.response_body.truncated);

    let recorded = engine.body(0, BodyPart::Response, false).unwrap().bytes;
    assert_eq!(recorded.len() as u64, LEN);
    assert!(
        recorded
            .iter()
            .enumerate()
            .step_by(4093)
            .all(|(i, b)| *b == pattern_byte(i as u64))
    );
}

/// Resident set size of this process, via `ps` (macOS and Linux).
fn rss_bytes() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("run ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .expect("ps rss")
        * 1024
}
