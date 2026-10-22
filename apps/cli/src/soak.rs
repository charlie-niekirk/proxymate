//! `proxymate-cli bench --rps N --duration SECS`: a soak test. Sends a
//! steady mix of requests (small and large JSON, compressed bodies, 20 KB
//! and 300 KB binaries, POSTed bodies, errors, the body-viewer zoo)
//! through a proxy to a local test upstream, and prints one CSV line every
//! interval: throughput, latency, failures, the proxy process's RSS and,
//! for an in-process engine, flows kept, disk use and evictions.
//!
//! Without `--proxy` it runs its own engine (with the default storage
//! limits, or `--max-flows`); with `--proxy HOST:PORT --pid PID` it drives
//! the running app and samples that process.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::rt::TokioIo;
use parking_lot::Mutex;
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::StorageSettings;
use pm_testkit::Upstream;
use tokio::net::TcpStream;

pub struct SoakConfig {
    pub rps: u32,
    pub duration: Duration,
    pub interval: Duration,
    pub proxy: Option<SocketAddr>,
    pub pid: Option<u32>,
    pub max_flows: Option<u32>,
}

/// Connections kept open to the proxy; each sends one request at a time.
const CONNECTIONS: usize = 48;

/// Request paths, weighted: about 3 MB/s of bodies at 500 rps.
const MIX: &[(u32, &str)] = &[
    (40, "/json"),
    (8, "/gzip"),
    (7, "/br"),
    (14, "/bytes/20000"),
    (1, "/bytes/300000"),
    (10, "POST /echo"),
    (5, "/status/404"),
    (2, "/status/500"),
    (13, "/zoo/"),
];

#[derive(Default)]
struct Stats {
    done: AtomicU64,
    failed: AtomicU64,
    bytes: AtomicU64,
    latencies_us: Mutex<Vec<u64>>,
}

pub fn run(config: SoakConfig) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(soak(config));
}

async fn soak(config: SoakConfig) {
    let upstream = Upstream::start().await;
    let zoo: Vec<String> = pm_testkit::zoo::fixtures()
        .iter()
        .filter(|f| f.name != "json-10mb")
        .map(|f| f.name.clone())
        .collect();
    let data_dir = std::env::temp_dir().join(format!("proxymate-soak-{}", std::process::id()));
    let (engine, proxy, pid) = match config.proxy {
        Some(proxy) => (None, proxy, config.pid),
        None => {
            let engine = Engine::with_config(EngineConfig {
                data_dir: Some(data_dir.clone()),
                ..Default::default()
            });
            if let Some(max_flows) = config.max_flows {
                engine
                    .set_storage_settings(StorageSettings {
                        max_flows,
                        ..engine.storage_settings()
                    })
                    .expect("valid limits");
            }
            let addr = engine
                .start_proxy(ProxyConfig {
                    ports: 0..=0,
                    ..Default::default()
                })
                .await
                .expect("start the proxy");
            (Some(engine), addr, Some(std::process::id()))
        }
    };
    eprintln!(
        "soak: {} rps for {:?} through {proxy} to {} ({} connections){}",
        config.rps,
        config.duration,
        upstream.addr(),
        CONNECTIONS,
        pid.map(|p| format!(", sampling pid {p}")).unwrap_or_default()
    );

    let stats = Arc::new(Stats::default());
    let (tx, rx) = async_channel(config.rps as usize * 2);
    for worker in 0..CONNECTIONS {
        let rx = rx.clone();
        let stats = stats.clone();
        let upstream = upstream.addr();
        let zoo = zoo.clone();
        tokio::spawn(async move { worker_loop(worker, proxy, upstream, zoo, rx, stats).await });
    }

    // One tick per request; a slow proxy builds a backlog that shows up
    // as latency, never as a lower send rate.
    let started = Instant::now();
    let ticker = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(config.rps)));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
        let mut n: u64 = 0;
        while started.elapsed() < config.duration {
            tick.tick().await;
            if tx.send(n).await.is_err() {
                return;
            }
            n += 1;
        }
    });

    println!("elapsed_s,requests,rps,failed,mb_per_s,p50_ms,p99_ms,rss_mb,flows,disk_mb,evicted");
    let mut last = (Instant::now(), 0u64, 0u64);
    let mut next = Instant::now() + config.interval;
    while !ticker.is_finished() {
        tokio::time::sleep_until(next.into()).await;
        next += config.interval;
        let now = Instant::now();
        let done = stats.done.load(Ordering::Relaxed);
        let bytes = stats.bytes.load(Ordering::Relaxed);
        let secs = now.duration_since(last.0).as_secs_f64();
        let mut lat = std::mem::take(&mut *stats.latencies_us.lock());
        lat.sort_unstable();
        let pct = |p: f64| {
            lat.get(((lat.len() as f64 * p) as usize).min(lat.len().saturating_sub(1)))
                .copied()
        };
        let ms = |v: Option<u64>| v.map_or("-".into(), |us| format!("{:.1}", us as f64 / 1000.0));
        let (flows, disk, evicted) = match &engine {
            Some(engine) => {
                let s = engine.storage_status();
                (
                    s.flows.to_string(),
                    format!("{:.0}", s.disk_bytes as f64 / 1e6),
                    s.evicted.to_string(),
                )
            }
            None => ("-".into(), "-".into(), "-".into()),
        };
        println!(
            "{:.0},{done},{:.0},{},{:.2},{},{},{},{flows},{disk},{evicted}",
            started.elapsed().as_secs_f64(),
            (done - last.1) as f64 / secs,
            stats.failed.load(Ordering::Relaxed),
            (bytes - last.2) as f64 / secs / 1e6,
            ms(pct(0.5)),
            ms(pct(0.99)),
            pid.and_then(rss_mb).map_or("-".into(), |mb| format!("{mb:.0}")),
        );
        last = (now, done, bytes);
    }
    if let Some(engine) = engine {
        engine.shutdown();
    }
    let _ = std::fs::remove_dir_all(data_dir);
}

async fn worker_loop(
    worker: usize,
    proxy: SocketAddr,
    upstream: SocketAddr,
    zoo: Vec<String>,
    rx: SharedRx,
    stats: Arc<Stats>,
) {
    let total: u32 = MIX.iter().map(|(w, _)| w).sum();
    let mut sender = None;
    while let Ok(n) = rx.recv().await {
        // Spread the mix evenly over the request numbers.
        let mut pick = ((n.wrapping_mul(2_654_435_761) >> 7) % u64::from(total)) as u32;
        let path = MIX
            .iter()
            .find(|(w, _)| {
                if pick < *w {
                    true
                } else {
                    pick -= w;
                    false
                }
            })
            .map(|(_, p)| *p)
            .unwrap_or("/json");
        let req = match path {
            "POST /echo" => Request::post(format!("http://{upstream}/echo"))
                .header("content-type", "application/json")
                .body(Full::new(Bytes::from(format!(
                    "{{\"worker\":{worker},\"n\":{n},\"pad\":\"{}\"}}",
                    "x".repeat(2000)
                )))),
            "/zoo/" => Request::get(format!(
                "http://{upstream}/zoo/{}",
                zoo[(n as usize) % zoo.len().max(1)]
            ))
            .body(Full::default()),
            p => Request::get(format!("http://{upstream}{p}")).body(Full::default()),
        }
        .expect("valid request");
        let started = Instant::now();
        match send(&mut sender, proxy, upstream, req).await {
            Ok(len) => {
                stats.done.fetch_add(1, Ordering::Relaxed);
                stats.bytes.fetch_add(len, Ordering::Relaxed);
                stats.latencies_us.lock().push(started.elapsed().as_micros() as u64);
            }
            Err(_) => {
                stats.failed.fetch_add(1, Ordering::Relaxed);
                sender = None;
            }
        }
    }
}

type Sender = hyper::client::conn::http1::SendRequest<Full<Bytes>>;

/// Sends on the worker's keep-alive connection to the proxy, opening it
/// first if needed. Returns the response body length.
async fn send(
    sender: &mut Option<Sender>,
    proxy: SocketAddr,
    upstream: SocketAddr,
    mut req: Request<Full<Bytes>>,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    if sender.as_ref().is_none_or(|s| s.is_closed()) {
        let stream = TcpStream::connect(proxy).await?;
        let _ = stream.set_nodelay(true);
        let (s, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
        tokio::spawn(conn);
        *sender = Some(s);
    }
    let s = sender.as_mut().expect("connected");
    req.headers_mut()
        .insert(http::header::HOST, upstream.to_string().parse().expect("valid host"));
    s.ready().await?;
    let response = s.send_request(req).await?;
    let body = response.into_body().collect().await?.to_bytes();
    Ok(body.len() as u64)
}

/// Resident memory of `pid` in MB, from `ps`.
fn rss_mb(pid: u32) -> Option<f64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let kb: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(kb / 1024.0)
}

// A small multi-consumer channel over tokio primitives: the ticker hands
// each request number to whichever worker is free.
type AsyncReceiver = Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<u64>>>;

fn async_channel(capacity: usize) -> (tokio::sync::mpsc::Sender<u64>, SharedRx) {
    let (tx, rx) = tokio::sync::mpsc::channel(capacity);
    (tx, SharedRx(Arc::new(tokio::sync::Mutex::new(rx))))
}

#[derive(Clone)]
struct SharedRx(AsyncReceiver);

impl SharedRx {
    async fn recv(&self) -> Result<u64, ()> {
        self.0.lock().await.recv().await.ok_or(())
    }
}
