//! Saved sessions and HAR documents through the engine: save a store in
//! the background, open it as a document, read it like the live store.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use pm_core::{BodyEnd, Engine, EngineConfig, ProxyConfig};
use pm_model::{
    BodyInfo, BodyPart, ContentKind, DocumentKind, FilterSpec, FlowDetail, FlowSelection, FlowState, Header,
    HttpVersion, LIVE_STORE, RequestHead, ResponseHead, SaveProgress, SortSpec, StoreId, Timing, WsQuery,
};
use pm_testkit::Upstream;
use pm_testkit::client::{self, eventually};
use pm_testkit::synth::Generator;
use pm_testkit::upstream::JSON_BODY;
use tokio_tungstenite::tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);

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

/// Runs a background save and waits for its last update.
fn finish(start: impl FnOnce(pm_core::ProgressSink) -> u32) -> (u32, Vec<SaveProgress>) {
    let (tx, rx) = mpsc::channel();
    let total = start(Box::new(move |p| {
        let _ = tx.send(p);
    }));
    let mut updates = Vec::new();
    loop {
        let update = rx.recv_timeout(Duration::from_secs(120)).expect("save progress");
        let done = update.finished;
        updates.push(update);
        if done {
            return (total, updates);
        }
    }
}

fn body_for(i: usize) -> (Vec<u8>, Vec<u8>) {
    let request = if i.is_multiple_of(10) {
        format!("{{\"flow\":{i}}}").into_bytes()
    } else {
        Vec::new()
    };
    // Every thousandth body is big enough to live in the segment files.
    let response = if i.is_multiple_of(1000) {
        (0..20_000u32).map(|b| (b % 251) as u8).collect()
    } else {
        format!("{{\"ok\":true,\"i\":{i}}}").into_bytes()
    };
    (request, response)
}

/// Records `n` finished flows through the recorder, as the proxy would.
fn record(engine: &Arc<Engine>, n: usize) {
    let mut generator = Generator::new(11);
    for i in 0..n {
        let summary = generator.active_flow();
        let url = format!(
            "{}://{}{}{}",
            summary.scheme.as_str(),
            summary.host,
            summary.path,
            if summary.query.is_empty() {
                String::new()
            } else {
                format!("?{}", summary.query)
            }
        );
        let detail = FlowDetail {
            id: 0,
            request: RequestHead {
                method: summary.method.to_string(),
                url,
                version: HttpVersion::Http11,
                headers: vec![Header::new("x-flow", i.to_string())],
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
        let (request, response) = body_for(i);
        if !request.is_empty() {
            recorder.body_data(BodyPart::Request, &request);
        }
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
        recorder.body_data(BodyPart::Response, &response);
        recorder.body_end(BodyPart::Response, BodyEnd::Complete);
    }
    engine.flush();
}

#[test]
fn a_100k_flow_session_saves_and_reopens() {
    const N: usize = 100_000;
    let dir = temp_dir();
    let engine = engine(&dir);
    record(&engine, N);
    assert_eq!(engine.total() as usize, N);

    let path = dir.join("big.proxymate");
    let started = Instant::now();
    let (total, updates) = finish(|sink| {
        engine
            .save_session(LIVE_STORE, &FlowSelection::All, path.clone(), sink)
            .unwrap()
    });
    let last = updates.last().unwrap();
    assert_eq!(last.error, None);
    assert_eq!((total, last.done, last.total), (N as u32, N as u32, N as u32));
    assert!(updates.len() > 3, "progress is reported along the way");
    eprintln!("saved {N} flows in {:?}", started.elapsed());

    let started = Instant::now();
    let doc = engine.open_document(&path).unwrap();
    eprintln!("opened {N} flows in {:?}", started.elapsed());
    assert_eq!(
        (doc.kind, doc.flows as usize, doc.name.as_str()),
        (DocumentKind::Session, N, "big.proxymate")
    );
    let store = doc.store;

    for id in 0..N as u32 {
        let live = engine.get_flow(id).unwrap();
        let saved = engine.get_flow_in(store, id).unwrap();
        assert_eq!(saved, live, "summary {id}");
    }
    for id in (0..N as u32).step_by(997).chain([0, 1000, 99_000, N as u32 - 1]) {
        assert_eq!(
            engine.flow_detail_in(store, id).unwrap(),
            engine.flow_detail(id).unwrap(),
            "detail {id}"
        );
        for part in [BodyPart::Request, BodyPart::Response] {
            let expected = engine.body(id, part, false).unwrap();
            assert_eq!(
                engine.body_head_in(store, id, part, false, None).unwrap(),
                expected,
                "body {id}"
            );
        }
    }

    // The document has its own views, filtered and sorted in Rust.
    let all = engine
        .create_view_in(store, FilterSpec::default(), SortSpec::default())
        .unwrap();
    assert_eq!(all.len as usize, N);
    let rows = engine.get_rows(all.view, 0, 3).unwrap();
    assert_eq!(rows.rows.iter().map(|r| r.0).collect::<Vec<_>>(), vec![0, 1, 2]);
    let filter = FilterSpec {
        text: "github".into(),
        ..Default::default()
    };
    let live = engine.create_view(filter.clone(), SortSpec::default());
    let filtered = engine.set_filter(all.view, filter).unwrap();
    assert_eq!(filtered.len, live.len);

    engine.close_document(store).unwrap();
    assert!(engine.get_rows(all.view, 0, 1).is_err());
    assert!(engine.documents().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// A selection of a store, then a document saved again: ids are renumbered
/// from 0, bodies and message logs come along.
#[tokio::test]
async fn selections_and_message_logs_are_saved() {
    let dir = temp_dir();
    let engine = engine(&dir);
    let upstream = Upstream::start().await;
    let addr = engine
        .start_proxy(ProxyConfig {
            ports: 0..=0,
            ..Default::default()
        })
        .await
        .unwrap();
    let (status, _) = client::get(addr, &upstream.url("/json")).await;
    assert_eq!(status, 200);
    let (status, _) = client::get(addr, &upstream.url("/gzip")).await;
    assert_eq!(status, 200);
    let (_, ws) = client::ws_absolute(addr, &upstream.url("/ws"), &[]).await;
    let mut ws = ws.expect("websocket");
    ws.send(Message::text("hello")).await.unwrap();
    let _ = ws.next().await;
    ws.send(Message::binary(vec![0u8, 1, 2])).await.unwrap();
    let _ = ws.next().await;
    ws.close(None).await.unwrap();
    while ws.next().await.is_some() {}
    let (status, _) = client::get(addr, &upstream.url("/sse?count=3&interval_ms=5")).await;
    assert_eq!(status, 200);
    eventually(WAIT, || {
        engine.flush();
        (engine.total() == 4 && (0..4).all(|id| engine.get_flow(id).unwrap().state != FlowState::Active)).then_some(())
    })
    .await;

    let path = dir.join("pick.proxymate");
    let selection = FlowSelection::Ids { ids: vec![1, 2, 3] };
    let (total, updates) = finish(|sink| engine.save_session(LIVE_STORE, &selection, path.clone(), sink).unwrap());
    assert_eq!((total, updates.last().unwrap().error.clone()), (3, None));
    let store = engine.open_document(&path).unwrap().store;

    // Flow 1 (the gzipped one) is now flow 0, still encoded.
    let gz = engine.body_head_in(store, 0, BodyPart::Response, true, None).unwrap();
    assert_eq!(
        (gz.bytes.as_slice(), gz.decoded_from.as_deref()),
        (JSON_BODY.as_bytes(), Some("gzip"))
    );
    let all = WsQuery::default();
    let live_ws = engine.websocket_messages(2, &all, 0, 100).unwrap();
    let saved_ws = engine.websocket_messages_in(store, 1, &all, 0, 100).unwrap();
    assert_eq!(saved_ws.messages, live_ws.messages);
    assert_eq!((saved_ws.close, saved_ws.open), (live_ws.close, false));
    assert_eq!(
        engine.websocket_payload_in(store, 1, 1).unwrap(),
        engine.websocket_payload(2, 1).unwrap()
    );
    let live_sse = engine.sse_events(3, "", 0, 100).unwrap();
    let saved_sse = engine.sse_events_in(store, 2, "", 0, 100).unwrap();
    assert_eq!(saved_sse.events, live_sse.events);
    assert_eq!(saved_sse.total, 3);

    // Saving a document again (and over an existing file) works the same.
    let (total, updates) = finish(|sink| {
        engine
            .save_session(store, &FlowSelection::All, path.clone(), sink)
            .unwrap()
    });
    assert_eq!((total, updates.last().unwrap().error.clone()), (3, None));
    let again = engine.open_document(&path).unwrap().store;
    assert_eq!(engine.get_flow_in(again, 2), engine.get_flow_in(store, 2));
    let _ = std::fs::remove_dir_all(dir);
}

fn chrome_har() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chrome-devtools.har")
}

#[test]
fn har_files_open_as_documents_and_export_views() {
    let dir = temp_dir();
    let engine = engine(&dir);
    let doc = engine.open_document(&chrome_har()).unwrap();
    assert_eq!((doc.kind, doc.flows), (DocumentKind::Har, 6));
    let store: StoreId = doc.store;
    assert_ne!(store, LIVE_STORE);
    assert_eq!(engine.documents(), vec![doc.clone()]);

    let png = engine.body_head_in(store, 2, BodyPart::Response, true, None).unwrap();
    assert!(png.bytes.starts_with(b"\x89PNG"));
    let ws = engine
        .websocket_messages_in(store, 3, &WsQuery::default(), 0, 10)
        .unwrap();
    assert_eq!(ws.total, 3);
    assert_eq!(engine.websocket_payload_in(store, 3, 2).unwrap(), b"\x00\x01\x02binary");

    // Export the filtered view: only the HTTPS GETs to example.test hosts.
    let view = engine
        .create_view_in(
            store,
            FilterSpec {
                text: "cdn.example".into(),
                ..Default::default()
            },
            SortSpec::default(),
        )
        .unwrap();
    assert_eq!(view.len, 1);
    let out = dir.join("view.har");
    let (total, updates) = finish(|sink| {
        engine
            .export_har(store, &FlowSelection::View { view: view.view }, out.clone(), sink)
            .unwrap()
    });
    assert_eq!((total, updates.last().unwrap().error.clone()), (1, None));
    let har: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let entries = har["log"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["request"]["url"], "https://cdn.example.test/img/dot.png");
    assert_eq!(entries[0]["response"]["content"]["encoding"], "base64");

    // A view of another store isn't a selection of this one.
    let live_view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;
    assert!(
        engine
            .selected_flows(store, &FlowSelection::View { view: live_view })
            .is_err()
    );

    // The exported file opens again.
    let reopened = engine.open_document(&out).unwrap();
    assert_eq!((reopened.kind, reopened.flows), (DocumentKind::Har, 1));
    let json = engine.har_json(store, &FlowSelection::Ids { ids: vec![0] }).unwrap();
    assert!(json.contains("\"url\": \"https://example.test/\""));

    assert!(engine.open_document(&dir.join("missing.har")).is_err());
    std::fs::write(dir.join("junk.har"), b"not json").unwrap();
    assert!(engine.open_document(&dir.join("junk.har")).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_failed_save_leaves_the_old_file() {
    let dir = temp_dir();
    let engine = engine(&dir);
    record(&engine, 3);
    let path = dir.join("keep.proxymate");
    std::fs::write(&path, b"old").unwrap();
    // Unknown flows are skipped; an unknown store is an error up front.
    assert!(
        engine
            .save_session(42, &FlowSelection::All, path.clone(), Box::new(|_| {}))
            .is_err()
    );
    let missing_dir = dir.join("nope").join("x.proxymate");
    let (_, updates) = finish(|sink| {
        engine
            .save_session(LIVE_STORE, &FlowSelection::All, missing_dir.clone(), sink)
            .unwrap()
    });
    assert!(updates.last().unwrap().error.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
    let _ = std::fs::remove_dir_all(dir);
}

/// A session saved by M8–M13 (format version 1, before
/// `FlowRules.scripts`) still opens, with its rules record intact.
#[test]
fn a_version_1_session_still_opens() {
    let dir = temp_dir();
    let engine = engine(&dir);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/session-v1.proxymate");
    let doc = engine.open_document(&fixture).expect("a v1 session opens");
    assert_eq!(doc.flows, 2);
    let detail = engine.flow_detail_in(doc.store, 0).unwrap().unwrap();
    assert_eq!(detail.request.url, "https://api.example.test/v1/profile");
    assert_eq!(detail.rules.applied.len(), 1);
    assert_eq!(detail.rules.applied[0].name, "Tag");
    assert_eq!(detail.rules.rewrite_skipped.as_deref(), Some("a v1 note"));
    assert!(detail.rules.scripts.is_empty());
    let body = engine
        .body_head_in(doc.store, 0, BodyPart::Response, true, None)
        .unwrap();
    assert!(body.bytes.starts_with(br#"{"ok":true"#));
    let health = engine.flow_detail_in(doc.store, 1).unwrap().unwrap();
    assert_eq!(health.response.unwrap().status, 200);
}
