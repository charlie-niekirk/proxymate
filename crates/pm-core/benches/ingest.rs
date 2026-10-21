//! Ingest throughput: 100k flows through the same path live traffic takes
//! (recorder → events → ingest batches → views), target under 2 s.
//!
//! `cargo bench -p pm-core --bench ingest`

use std::sync::Arc;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use pm_core::{BodyEnd, Engine, EngineConfig};
use pm_model::{
    BodyInfo, BodyPart, ContentGroup, ContentKind, FilterSpec, FlowDetail, FlowSummary, Header, HttpVersion,
    ProtocolFilter, RequestHead, ResponseHead, SortSpec,
};
use pm_testkit::synth::Generator;

const FLOWS: usize = 100_000;
const BODY: &[u8] = br#"{"id":42,"name":"proxymate","tags":["fast","small"],"ok":true}"#;

fn engine_with_views() -> Arc<Engine> {
    let engine = Engine::with_config(EngineConfig {
        data_dir: Some(std::env::temp_dir().join("proxymate-bench")),
    });
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
    }
}

fn bench_ingest(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingest");
    group.sample_size(10).measurement_time(Duration::from_secs(20));

    group.bench_function("100k recorded flows", |b| {
        b.iter_batched(
            || {
                let mut generator = Generator::new(1);
                let flows: Vec<FlowSummary> = (0..FLOWS).map(|_| generator.active_flow()).collect();
                (engine_with_views(), flows)
            },
            |(engine, flows)| {
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
                    recorder.body_data(BodyPart::Response, BODY);
                    recorder.body_end(BodyPart::Response, BodyEnd::Complete);
                }
                engine.flush();
                assert_eq!(engine.total() as usize, FLOWS);
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
                (engine_with_views(), flows)
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
    group.finish();
}

criterion_group!(benches, bench_ingest);
criterion_main!(benches);
