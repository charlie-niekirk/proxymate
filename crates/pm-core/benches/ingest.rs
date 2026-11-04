//! The hot path at 100k flows (R2): ingest through the same path live
//! traffic takes (recorder → events → ingest batches → views), sorted
//! views, eviction, refilter and sort, and a deep body search.
//!
//! `cargo bench -p pm-core --bench ingest`. Targets: ingest under 2 s per
//! 100k flows, a refilter or sort under 10 ms. `PM_BENCH_QUICK=1` trims
//! the measurement time for CI, whose budget is `benches/budget.json`
//! (checked by `scripts/bench-budget.py`).

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use criterion::{BatchSize, BenchmarkGroup, Criterion, criterion_group, criterion_main, measurement::WallTime};
use pm_core::{BodyEnd, Engine, EngineConfig};
use pm_model::{
    BodyInfo, BodyPart, ContentGroup, ContentKind, FilterSpec, FlowDetail, FlowSummary, Header, HttpVersion,
    ProtocolFilter, RequestHead, ResponseHead, SortDir, SortKey, SortSpec, StorageSettings,
};
use pm_testkit::synth::Generator;

const FLOWS: usize = 100_000;
const BODY: &[u8] = br#"{"id":42,"name":"proxymate","tags":["fast","small"],"ok":true}"#;
/// Live traffic arrives a few flows per ingest batch, not 50k at once.
const TRICKLE_BATCHES: usize = 200;
const TRICKLE_FLOWS: usize = 5;
/// Deep search: bodies past the search limit (4 MB) that it must not read whole.
const LARGE_BODIES: usize = 8;
const LARGE_BODY: usize = 16 * 1024 * 1024;

fn quick() -> bool {
    std::env::var_os("PM_BENCH_QUICK").is_some()
}

fn data_dir(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("proxymate-bench-{name}-{}", std::process::id()))
}

fn engine(name: &str) -> Arc<Engine> {
    Engine::with_config(EngineConfig {
        data_dir: Some(data_dir(name)),
        ..Default::default()
    })
}

/// The views the UI typically has open: everything, and a filtered one.
fn engine_with_views(name: &str) -> Arc<Engine> {
    let engine = engine(name);
    engine.create_view(FilterSpec::default(), SortSpec::default());
    engine.create_view(
        FilterSpec {
            protocol: ProtocolFilter::Https,
            content: vec![ContentGroup::Json],
            ..Default::default()
        },
        SortSpec::default(),
    );
    engine
}

fn sort(key: SortKey, dir: SortDir) -> SortSpec {
    SortSpec { key, dir, extra: 0 }
}

/// [`engine_with_views`] plus views sorted on keys a flow's response and
/// end change (status, duration) and one that never changes (URL).
fn engine_with_sorted_views(name: &str) -> Arc<Engine> {
    let engine = engine_with_views(name);
    engine.create_view(FilterSpec::default(), sort(SortKey::Duration, SortDir::Desc));
    engine.create_view(FilterSpec::default(), sort(SortKey::Url, SortDir::Asc));
    engine.create_view(
        FilterSpec {
            text: "api".into(),
            ..Default::default()
        },
        sort(SortKey::Status, SortDir::Asc),
    );
    engine
}

fn detail(flow: &FlowSummary) -> FlowDetail {
    FlowDetail {
        id: 0,
        request: RequestHead {
            method: flow.method.to_string(),
            url: format!("http://{}{}", flow.host, flow.path),
            version: HttpVersion::Http11,
            headers: vec![Header::new("host", &*flow.host), Header::new("accept", "*/*")],
        },
        response: None,
        request_body: BodyInfo::default(),
        response_body: BodyInfo::default(),
        error: None,
        tls: None,
        timing: Default::default(),
        rules: Default::default(),
        trailers: None,
        via: None,
    }
}

fn active_flows(n: usize) -> Vec<FlowSummary> {
    let mut generator = Generator::new(1);
    (0..n).map(|_| generator.active_flow()).collect()
}

/// Records `flows` as live traffic does: head, response, one body chunk, end.
fn record(engine: &Arc<Engine>, flows: Vec<FlowSummary>, body: &[u8]) {
    let response = ResponseHead {
        status: 200,
        reason: "OK".into(),
        version: HttpVersion::Http11,
        headers: vec![Header::new("content-type", "application/json")],
    };
    for flow in flows {
        let detail = detail(&flow);
        let recorder = engine.begin_flow(flow, detail).expect("recording");
        recorder.response(response.clone(), ContentKind::Json);
        recorder.body_data(BodyPart::Response, body);
        recorder.body_end(BodyPart::Response, BodyEnd::Complete);
    }
    engine.flush();
}

fn group<'a>(c: &'a mut Criterion, name: &str) -> BenchmarkGroup<'a, WallTime> {
    let mut group = c.benchmark_group(name);
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(if quick() { 5 } else { 20 }));
    group.warm_up_time(Duration::from_secs(if quick() { 1 } else { 3 }));
    group
}

fn bench_ingest(c: &mut Criterion) {
    let mut group = group(c, "ingest");

    group.bench_function("100k recorded flows", |b| {
        b.iter_batched(
            || (engine_with_views("recorded"), active_flows(FLOWS)),
            |(engine, flows)| {
                record(&engine, flows, BODY);
                assert_eq!(engine.total() as usize, FLOWS);
                engine
            },
            BatchSize::PerIteration,
        )
    });

    group.bench_function("100k recorded flows sorted views", |b| {
        b.iter_batched(
            || (engine_with_sorted_views("sorted"), active_flows(FLOWS)),
            |(engine, flows)| {
                record(&engine, flows, BODY);
                assert_eq!(engine.total() as usize, FLOWS);
                engine
            },
            BatchSize::PerIteration,
        )
    });

    group.bench_function("100k recorded flows 20k cap", |b| {
        b.iter_batched(
            || {
                let engine = engine_with_views("evict");
                engine
                    .set_storage_settings(StorageSettings {
                        max_flows: 20_000,
                        max_disk_mb: 0,
                        ..Default::default()
                    })
                    .expect("storage settings");
                (engine, active_flows(FLOWS))
            },
            |(engine, flows)| {
                record(&engine, flows, BODY);
                assert!(engine.total() as usize <= 20_000);
                engine
            },
            BatchSize::PerIteration,
        )
    });

    group.bench_function("100k summaries (synthetic insert)", |b| {
        b.iter_batched(
            || {
                let mut generator = Generator::new(1);
                let flows: Vec<FlowSummary> = (0..FLOWS).map(|_| generator.completed_flow()).collect();
                (engine_with_views("synthetic"), flows)
            },
            |(engine, flows)| {
                for batch in flows.chunks(10_000) {
                    engine.insert_flows(batch.to_vec());
                }
                engine
            },
            BatchSize::PerIteration,
        )
    });

    // Per-batch cost at 100k flows: a few flows per ingest batch, as live
    // traffic arrives, into sorted views.
    group.bench_function("trickle 1k flows into 100k sorted views", |b| {
        let mut generator = Generator::new(2);
        b.iter_batched(
            || {
                let engine = engine_with_sorted_views("trickle");
                let mut seed = Generator::new(1);
                for chunk in (0..FLOWS).collect::<Vec<_>>().chunks(25_000) {
                    engine.insert_flows(chunk.iter().map(|_| seed.completed_flow()).collect());
                }
                let batches: Vec<Vec<FlowSummary>> = (0..TRICKLE_BATCHES)
                    .map(|_| (0..TRICKLE_FLOWS).map(|_| generator.completed_flow()).collect())
                    .collect();
                (engine, batches)
            },
            |(engine, batches)| {
                for batch in batches {
                    engine.insert_flows(batch);
                }
                engine
            },
            BatchSize::PerIteration,
        )
    });
    group.finish();
}

fn bench_views(c: &mut Criterion) {
    let mut group = group(c, "views");
    let engine = engine_with_views("views");
    let mut generator = Generator::new(1);
    for _ in 0..4 {
        engine.insert_flows((0..FLOWS / 4).map(|_| generator.completed_flow()).collect());
    }
    let view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;

    let filters = ["api", "github", "status:200", "-image"];
    let mut i = 0;
    group.bench_function("refilter 100k", |b| {
        b.iter(|| {
            i += 1;
            let text = filters[i % filters.len()].to_owned();
            engine
                .set_filter(
                    view,
                    FilterSpec {
                        text,
                        ..Default::default()
                    },
                )
                .expect("filter")
        })
    });
    engine.set_filter(view, FilterSpec::default()).expect("filter");

    let sorts = [
        sort(SortKey::Url, SortDir::Asc),
        sort(SortKey::Duration, SortDir::Desc),
        sort(SortKey::Status, SortDir::Asc),
        sort(SortKey::Id, SortDir::Desc),
    ];
    let mut i = 0;
    group.bench_function("sort 100k", |b| {
        b.iter(|| {
            i += 1;
            engine.set_sort(view, sorts[i % sorts.len()]).expect("sort")
        })
    });
    group.finish();
}

/// Waits until the view's deep search has checked every candidate.
fn deep_done(engine: &Engine, rx: &mpsc::Receiver<pm_model::ViewDelta>) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        engine.publish_tick();
        while let Ok(delta) = rx.try_recv() {
            if let Some(deep) = delta.deep
                && !deep.running
                && deep.checked == deep.total
            {
                return deep.matched;
            }
        }
        assert!(Instant::now() < deadline, "deep search never finished");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn bench_deep(c: &mut Criterion) {
    let mut group = group(c, "deep");
    let engine = engine_with_views("deep");
    record(&engine, active_flows(FLOWS), BODY);
    // A few large bodies, past what a deep search reads of a body.
    let large: Vec<u8> = (0..LARGE_BODY).map(|i| b"abcdefghijklmnopqrstuvwxyz \n"[i % 28]).collect();
    record(&engine, active_flows(LARGE_BODIES), &large);
    let view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;
    let (tx, rx) = mpsc::channel();
    engine
        .subscribe(view, Box::new(move |delta| tx.send(delta).is_ok()))
        .expect("subscribe");

    let needles = ["body:proxymate", "body:no-such-needle"];
    let mut i = 0;
    group.bench_function("body search 100k flows", |b| {
        b.iter(|| {
            i += 1;
            let text = needles[i % needles.len()].to_owned();
            engine
                .set_filter(
                    view,
                    FilterSpec {
                        text,
                        ..Default::default()
                    },
                )
                .expect("filter");
            deep_done(&engine, &rx)
        })
    });
    group.finish();
    drop(engine);
    let _ = std::fs::remove_dir_all(data_dir("deep"));
}

criterion_group!(benches, bench_ingest, bench_views, bench_deep);
criterion_main!(benches);
