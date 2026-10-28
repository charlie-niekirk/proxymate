//! Storage limits and deep search (M10): the flow cap evicts the oldest
//! flows but keeps pinned ones, eviction deletes the segment files only
//! evicted flows used, the disk cap evicts by bytes, and a view whose
//! filter has header or body terms streams its matches in.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pm_core::{BodyEnd, Engine, EngineConfig, FlowRecorder};
use pm_model::{
    BodyInfo, BodyPart, ContentKind, FilterSpec, FlowDetail, FlowId, Header, HttpVersion, RequestHead, ResponseHead,
    SortSpec, StorageSettings, Timing, ViewDelta,
};
use pm_testkit::synth::Generator;

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir()
        .join("proxymate-tests")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn engine(dir: &Path) -> Arc<Engine> {
    Engine::with_config(EngineConfig {
        data_dir: Some(dir.to_owned()),
        ..Default::default()
    })
}

/// Starts recording one flow with a request header `x-flow: <i>`.
fn begin(engine: &Arc<Engine>, generator: &mut Generator, i: usize, headers: Vec<Header>) -> FlowRecorder {
    let summary = generator.active_flow();
    let mut all = vec![Header::new("x-flow", i.to_string())];
    all.extend(headers);
    let detail = FlowDetail {
        id: 0,
        request: RequestHead {
            method: summary.method.to_string(),
            url: format!("https://{}{}", summary.host, summary.path),
            version: HttpVersion::Http11,
            headers: all,
        },
        response: None,
        request_body: BodyInfo::default(),
        response_body: BodyInfo::default(),
        error: None,
        tls: None,
        timing: Timing::default(),
        rules: Default::default(),
        trailers: None,
    };
    let recorder = engine.begin_flow(summary, detail).expect("recording");
    recorder.body_end(BodyPart::Request, BodyEnd::Complete);
    recorder.response(
        ResponseHead {
            status: 200,
            reason: "OK".into(),
            version: HttpVersion::Http11,
            headers: vec![Header::new("content-type", "application/json")],
        },
        ContentKind::Json,
    );
    recorder
}

/// Records a finished flow whose response body is `body`.
fn record(engine: &Arc<Engine>, generator: &mut Generator, i: usize, body: &[u8], headers: Vec<Header>) -> FlowId {
    let recorder = begin(engine, generator, i, headers);
    recorder.body_data(BodyPart::Response, body);
    recorder.body_end(BodyPart::Response, BodyEnd::Complete);
    recorder.id()
}

fn segment_files(engine: &Engine) -> usize {
    let dir = PathBuf::from(engine.storage_status().dir);
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "seg"))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn the_flow_cap_evicts_the_oldest_but_keeps_pinned_and_active_flows() {
    let dir = temp_dir();
    let engine = engine(&dir);
    engine
        .set_storage_settings(StorageSettings {
            max_flows: 0,
            ..Default::default()
        })
        .unwrap();
    let mut generator = Generator::new(1);
    let ids: Vec<FlowId> = (0..90)
        .map(|i| record(&engine, &mut generator, i, b"{\"small\":true}", vec![]))
        .collect();
    let in_flight = begin(&engine, &mut generator, 90, vec![]);
    engine.flush();
    engine.pin_flows(vec![ids[3], ids[10]], true);
    let view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;

    for i in 91..140 {
        record(&engine, &mut generator, i, b"{\"small\":true}", vec![]);
    }
    engine.flush();
    assert_eq!(engine.total(), 140);
    engine
        .set_storage_settings(StorageSettings {
            max_flows: 100,
            ..Default::default()
        })
        .unwrap();

    // 140 flows are past 100: down to 95, keeping the two pinned and the
    // one in flight.
    assert_eq!(engine.total(), 95);
    let status = engine.storage_status();
    assert_eq!((status.flows, status.pinned, status.evicted), (95, 2, 45));
    assert!(engine.get_flow(ids[3]).is_some_and(|f| f.pinned));
    assert!(engine.get_flow(ids[10]).is_some());
    assert!(engine.get_flow(in_flight.id()).is_some());
    assert!(engine.get_flow(ids[0]).is_none());
    assert!(engine.flow_detail(ids[0]).unwrap().is_none());
    // The view follows; its rows are what the store holds, oldest first.
    let rows = engine.get_rows(view, 0, 1000).unwrap();
    assert_eq!(rows.len, 95);
    assert_eq!(rows.rows[0].0, ids[3]);
    assert_eq!(rows.rows[1].0, ids[10]);
    assert_eq!(engine.sidebar().total, 95);
    let counted: u32 = engine.sidebar().domains.iter().map(|d| d.count).sum();
    assert_eq!(counted, 95);

    // The in-flight flow still finishes; ids are never reused.
    in_flight.body_end(BodyPart::Response, BodyEnd::Complete);
    let next = record(&engine, &mut generator, 141, b"{}", vec![]);
    engine.flush();
    assert_eq!(next, 140);
    assert!(
        engine
            .find_flows("is:pinned", None, None, 10, false)
            .unwrap()
            .flows
            .len()
            == 2
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn eviction_deletes_segment_files_only_evicted_flows_used() {
    let dir = temp_dir();
    let engine = engine(&dir);
    engine
        .set_storage_settings(StorageSettings {
            max_flows: 0,
            max_disk_mb: 0,
            ..Default::default()
        })
        .unwrap();
    let mut generator = Generator::new(2);
    let mb: Vec<u8> = (0..1024 * 1024u32).map(|b| (b % 251) as u8).collect();
    // 1 MB bodies: about 64 flows per 64 MB segment, so 250 flows fill
    // segments 0 to 3.
    let ids: Vec<FlowId> = (0..250)
        .map(|i| record(&engine, &mut generator, i, &mb, vec![]))
        .collect();
    engine.flush();
    assert_eq!(segment_files(&engine), 4);
    // Flow 70 lives in segment 1.
    engine.pin_flows(vec![ids[70]], true);
    let before = engine.storage_status().disk_bytes;

    // A cap of 100 evicts all but 95 (the pinned one among them): segment 0 only
    // held evicted flows, segment 1 still has the pinned one.
    engine
        .set_storage_settings(StorageSettings {
            max_flows: 100,
            max_disk_mb: 0,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(engine.total(), 95);
    let after = engine.storage_status().disk_bytes;
    assert_eq!(segment_files(&engine), 3, "{before} -> {after}");
    assert!(after < before - 60 * 1024 * 1024, "{before} -> {after}");
    assert_eq!(engine.body(ids[70], BodyPart::Response, false).unwrap().bytes, mb);
    assert_eq!(engine.body(ids[249], BodyPart::Response, false).unwrap().bytes, mb);
    assert!(engine.body(ids[0], BodyPart::Response, false).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_disk_cap_evicts_by_bytes() {
    let dir = temp_dir();
    let engine = engine(&dir);
    engine
        .set_storage_settings(StorageSettings {
            max_flows: 0,
            max_disk_mb: 128,
            ..Default::default()
        })
        .unwrap();
    let mut generator = Generator::new(3);
    let mb = vec![7u8; 1024 * 1024];
    for i in 0..300 {
        record(&engine, &mut generator, i, &mb, vec![]);
    }
    engine.flush();
    let status = engine.storage_status();
    // Over 128 MB it drops whole segments down to 90%; the current one is
    // still filling.
    assert!(status.disk_bytes <= 192 * 1024 * 1024, "{status:?}");
    assert!(status.evicted >= 100, "{status:?}");
    assert_eq!(status.error, None);
    assert!(engine.total() < 300);
    let _ = std::fs::remove_dir_all(dir);
}

/// Waits for a view's deep search to have checked all `total` candidates.
fn settled(rx: &mpsc::Receiver<ViewDelta>, total: u32) -> ViewDelta {
    settled_where(rx, total, |_| true)
}

/// Like [`settled`], ignoring deltas whose progress fails `this_search`.
fn settled_where(
    rx: &mpsc::Receiver<ViewDelta>,
    total: u32,
    this_search: impl Fn(&pm_model::DeepProgress) -> bool,
) -> ViewDelta {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let delta = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("deep search progress");
        if delta
            .deep
            .is_some_and(|d| this_search(&d) && !d.running && d.checked == total && d.total == total)
        {
            return delta;
        }
    }
}

#[test]
fn deep_search_streams_header_and_body_matches_into_the_view() {
    let dir = temp_dir();
    let engine = engine(&dir);
    let mut generator = Generator::new(4);
    let mut needles = Vec::new();
    for i in 0..3000 {
        let body = if i % 100 == 7 {
            needles.push(i as FlowId);
            format!("{{\"token\":\"NeedleInHay-{i}\"}}")
        } else {
            format!("{{\"i\":{i}}}")
        };
        let headers = if i % 500 == 1 {
            vec![Header::new("x-trace", "deep-trace-id")]
        } else {
            vec![]
        };
        record(&engine, &mut generator, i, body.as_bytes(), headers);
    }
    // A big gzip body that only matches once decoded.
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut gz, &[b"x".repeat(200_000), b"needleinhay-gz".to_vec()].concat()).unwrap();
    let gz = gz.finish().unwrap();
    let recorder = begin(&engine, &mut generator, 3000, vec![]);
    let gz_id = recorder.id();
    recorder.response(
        ResponseHead {
            status: 200,
            reason: "OK".into(),
            version: HttpVersion::Http11,
            headers: vec![Header::new("content-encoding", "gzip")],
        },
        ContentKind::Json,
    );
    recorder.body_data(BodyPart::Response, &gz);
    recorder.body_end(BodyPart::Response, BodyEnd::Complete);
    needles.push(gz_id);
    engine.flush();

    let info = engine.create_view(
        FilterSpec {
            text: "body:needleinhay".into(),
            ..Default::default()
        },
        SortSpec::default(),
    );
    let deep = info.deep.expect("a deep search started");
    assert_eq!(deep.total, 3001);
    let view = info.view;
    let (tx, rx) = mpsc::channel();
    engine.subscribe(view, Box::new(move |d| tx.send(d).is_ok())).unwrap();
    let done = settled(&rx, 3001);
    assert_eq!(done.deep.unwrap().matched as usize, needles.len());
    let rows = engine.get_rows(view, 0, 1000).unwrap();
    assert_eq!(rows.rows.iter().map(|r| r.0).collect::<Vec<_>>(), needles);

    // New flows are checked as they finish.
    let late = begin(&engine, &mut generator, 4000, vec![]);
    engine.flush();
    late.body_data(BodyPart::Response, b"late needleinhay");
    late.body_end(BodyPart::Response, BodyEnd::Complete);
    let late_id = late.id();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !engine
        .get_rows(view, 0, 1000)
        .unwrap()
        .rows
        .iter()
        .any(|r| r.0 == late_id)
    {
        assert!(Instant::now() < deadline, "the late flow never joined the view");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Header terms, negation and summary terms combine.
    let info = engine
        .set_filter(
            view,
            FilterSpec {
                text: "header:deep-trace -body:needleinhay".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(info.len, 0, "rows wait for the deep search");
    // The first search also ends at 3002 checked (31 matches): skip its
    // deltas still in flight.
    let done = settled_where(&rx, 3002, |d| d.matched <= 6);
    assert_eq!(done.deep.unwrap().matched, 6);
    assert_eq!(engine.get_rows(view, 0, 100).unwrap().len, 6);

    // MCP's list evaluates the same terms inline.
    let page = engine.find_flows("reqheader:x-trace", None, None, 3, false).unwrap();
    assert_eq!(page.flows.len(), 3);
    assert!(page.next.is_some());

    // Dropping the filter's deep terms stops the search.
    let info = engine.set_filter(view, FilterSpec::default()).unwrap();
    assert_eq!(info.deep, None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_flow_that_matches_only_after_it_finished_is_still_searched() {
    let dir = temp_dir();
    let engine = engine(&dir);
    let mut generator = Generator::new(5);
    let info = engine.create_view(
        FilterSpec {
            text: "app:latecomer body:needle".into(),
            ..Default::default()
        },
        SortSpec::default(),
    );
    let id = record(&engine, &mut generator, 0, b"a needle", vec![]);
    engine.flush();
    // Its client resolves after it finished (a fast request), as
    // `ClientResolved` can arrive late.
    let mut flow = engine.get_flow(id).unwrap();
    flow.client = Some("latecomer".into());
    engine.update_flows(vec![flow]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while engine.get_rows(info.view, 0, 10).unwrap().len != 1 {
        assert!(Instant::now() < deadline, "the flow never joined the deep view");
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = std::fs::remove_dir_all(dir);
}
