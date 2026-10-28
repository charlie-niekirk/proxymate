//! M17 through the engine: deleting flows (and their segment files),
//! labels, comments and pins in saved sessions, header columns, Repeat and
//! Copy as code.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use pm_core::{BodyEnd, Engine, EngineConfig};
use pm_model::{
    BodyInfo, BodyPart, CodeLanguage, ContentKind, FilterSpec, FlowDetail, FlowLabel, FlowSelection, Header,
    HeaderColumn, HttpVersion, LIVE_STORE, RepeatOpts, RepeatProgress, RequestHead, RequestSpec, ResponseHead, SentBy,
    SortDir, SortKey, SortSpec, StatusCount, Timing,
};
use pm_testkit::Upstream;
use pm_testkit::client::eventually;
use pm_testkit::synth::Generator;

const WAIT: Duration = Duration::from_secs(30);

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

/// Records `n` finished flows, each with an `x-flow: <i>` request header,
/// an `x-trace` response header on even flows, and a response body of
/// `body_len` bytes (big ones live in the segment files).
fn record(engine: &Arc<Engine>, n: usize, body_len: usize) {
    let mut generator = Generator::new(5);
    for i in 0..n {
        let summary = generator.active_flow();
        let detail = FlowDetail {
            id: 0,
            request: RequestHead {
                method: "GET".into(),
                url: format!("https://{}{}", summary.host, summary.path),
                version: HttpVersion::Http11,
                headers: vec![Header::new("X-Flow", i.to_string())],
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
        let mut headers = vec![Header::new("content-type", "application/octet-stream")];
        if i % 2 == 0 {
            headers.push(Header::new("x-trace", format!("t{i:03}")));
        }
        recorder.response(
            ResponseHead {
                status: 200,
                reason: "OK".into(),
                version: HttpVersion::Http11,
                headers,
            },
            ContentKind::Binary,
        );
        recorder.body_data(BodyPart::Response, &vec![(i % 251) as u8; body_len]);
        recorder.body_end(BodyPart::Response, BodyEnd::Complete);
    }
    engine.flush();
}

fn ids(selection: &[u32]) -> FlowSelection {
    FlowSelection::Ids {
        ids: selection.to_vec(),
    }
}

fn segment_bytes(dir: &Path) -> u64 {
    walk(&dir.join("live"))
}

fn walk(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let meta = e.metadata().unwrap();
            if meta.is_dir() {
                walk(&e.path())
            } else if e.file_name() == "lock" {
                0
            } else {
                meta.len()
            }
        })
        .sum()
}

#[test]
fn deleting_flows_frees_segments_no_kept_flow_uses() {
    let dir = temp_dir();
    let engine = engine(&dir);
    // 2 MB bodies: about 32 flows per 64 MB segment, so 100 flows fill
    // four segments (about 200 MB, removed at the end).
    record(&engine, 100, 2 * 1024 * 1024);
    let before = segment_bytes(&dir);
    assert!(before > 150 * 1024 * 1024, "{before}");

    // Deleting a flow in the middle of a segment frees nothing on disk.
    assert_eq!(engine.remove_flows_in(LIVE_STORE, &ids(&[40])).unwrap(), 1);
    assert_eq!(engine.total(), 99);
    assert!(engine.get_flow(40).is_none());
    assert_eq!(segment_bytes(&dir), before);
    assert!(engine.flow_detail(40).unwrap().is_none());

    // Pinned flows go when asked; the first two segments are freed, the
    // one shared with flow 70 stays.
    engine.pin_flows_in(LIVE_STORE, &ids(&[3]), true).unwrap();
    let first: Vec<u32> = (0..70).collect();
    assert_eq!(engine.remove_flows_in(LIVE_STORE, &ids(&first)).unwrap(), 69);
    let after = segment_bytes(&dir);
    assert!(after < before - 120 * 1024 * 1024, "{after} of {before}");
    let kept = engine.body(70, BodyPart::Response, false).unwrap();
    assert_eq!(kept.bytes.len(), 2 * 1024 * 1024);
    // Flow 70's body is filled with its id (below 251).
    assert_eq!(kept.bytes[0], 70);

    // A whole view, but for the exceptions.
    let view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;
    let removed = engine
        .remove_flows_in(
            LIVE_STORE,
            &FlowSelection::View {
                view,
                except: vec![80, 81],
            },
        )
        .unwrap();
    assert_eq!(removed, 28);
    let left: Vec<u32> = engine.selected_ids(LIVE_STORE, &FlowSelection::All).unwrap();
    assert_eq!(left, vec![80, 81]);
    assert_eq!(engine.get_rows(view, 0, 10).unwrap().len, 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn labels_comments_and_pins_survive_a_session_and_filter() {
    let dir = temp_dir();
    let engine = engine(&dir);
    record(&engine, 6, 16);
    engine
        .label_flows_in(LIVE_STORE, &ids(&[1, 2]), Some(FlowLabel::Red))
        .unwrap();
    engine
        .label_flows_in(LIVE_STORE, &ids(&[2]), Some(FlowLabel::Blue))
        .unwrap();
    engine.comment_flow_in(LIVE_STORE, 4, "  Check this one  ").unwrap();
    engine.pin_flows_in(LIVE_STORE, &ids(&[5]), true).unwrap();
    assert_eq!(engine.get_flow(4).unwrap().comment.as_deref(), Some("Check this one"));
    assert!(engine.comment_flow_in(LIVE_STORE, 99, "x").is_err());

    let matching = |store, text: &str| {
        let view = engine
            .create_view_in(
                store,
                FilterSpec {
                    text: text.into(),
                    ..Default::default()
                },
                SortSpec::default(),
            )
            .unwrap()
            .view;
        let rows = engine.get_rows(view, 0, 100).unwrap().rows;
        rows.iter().map(|r| r.0).collect::<Vec<_>>()
    };
    assert_eq!(matching(LIVE_STORE, "label:red"), vec![1]);
    assert_eq!(matching(LIVE_STORE, "is:labelled"), vec![1, 2]);
    assert_eq!(matching(LIVE_STORE, "comment:check"), vec![4]);

    let path = dir.join("annotated.proxymate");
    let (tx, rx) = mpsc::channel();
    engine
        .save_session(
            LIVE_STORE,
            &FlowSelection::All,
            path.clone(),
            Box::new(move |p| {
                if p.finished {
                    let _ = tx.send(p);
                }
            }),
        )
        .unwrap();
    let saved = rx.recv_timeout(WAIT).unwrap();
    assert_eq!(saved.error, None);

    let doc = engine.open_document(&path).unwrap();
    let flow = |id| engine.get_flow_in(doc.store, id).unwrap();
    assert_eq!(flow(1).label, Some(FlowLabel::Red));
    assert_eq!(flow(2).label, Some(FlowLabel::Blue));
    assert_eq!(flow(4).comment.as_deref(), Some("Check this one"));
    assert!(flow(5).pinned && !flow(4).pinned);
    assert_eq!(matching(doc.store, "is:pinned"), vec![5]);
    assert_eq!(matching(doc.store, "is:commented"), vec![4]);

    // Documents take annotations and deletions in memory.
    engine
        .label_flows_in(doc.store, &ids(&[0]), Some(FlowLabel::Green))
        .unwrap();
    engine.comment_flow_in(doc.store, 4, "").unwrap();
    assert_eq!(flow(0).label, Some(FlowLabel::Green));
    assert_eq!(flow(4).comment, None);
    assert_eq!(engine.remove_flows_in(doc.store, &ids(&[0, 1])).unwrap(), 2);
    assert!(engine.get_flow_in(doc.store, 0).is_none());

    // A version 3 session (before annotations) still opens, without them.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE annotations; UPDATE meta SET value = '3' WHERE key = 'version';")
            .unwrap();
    }
    let old = engine.open_document(&path).expect("a v3 session opens");
    assert_eq!(engine.get_flow_in(old.store, 1).unwrap().label, None);
    assert!(!engine.get_flow_in(old.store, 5).unwrap().pinned);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn header_columns_fill_backfill_and_sort() {
    let dir = temp_dir();
    let engine = engine(&dir);
    record(&engine, 10, 16);
    let columns = engine
        .set_header_columns(vec![
            HeaderColumn {
                part: BodyPart::Request,
                name: "X-Flow".into(),
            },
            HeaderColumn {
                part: BodyPart::Response,
                name: "x-trace".into(),
            },
        ])
        .unwrap();
    assert_eq!(columns[0].name, "x-flow");
    assert!(
        engine
            .set_header_columns(vec![HeaderColumn {
                part: BodyPart::Request,
                name: "bad name".into(),
            }])
            .is_err()
    );

    // Existing flows are backfilled in the background.
    let view = engine
        .create_view(
            FilterSpec::default(),
            SortSpec {
                key: SortKey::Extra,
                dir: SortDir::Desc,
                extra: 1,
            },
        )
        .view;
    let rows = || engine.get_rows(view, 0, 100).unwrap().rows;
    let filled = || {
        engine.flush();
        let rows = rows();
        rows.iter()
            .all(|r| r.24.first().cloned().flatten().is_some())
            .then_some(rows)
    };
    let backfilled = eventually_sync(filled);
    assert_eq!(backfilled[0].24, vec![Some("8".into()), Some("t008".into())]);
    // Odd flows have no x-trace: they sort last when descending.
    assert!(backfilled[5..].iter().all(|r| r.24.get(1).cloned().flatten().is_none()));

    // New flows fill theirs as they arrive.
    record(&engine, 2, 16);
    let newest = engine.get_flow(10).unwrap();
    assert_eq!(newest.extra, vec![Some("0".into()), Some("t000".into())]);

    // Documents opened later are filled too.
    let path = dir.join("columns.proxymate");
    let (tx, rx) = mpsc::channel();
    engine
        .save_session(
            LIVE_STORE,
            &FlowSelection::All,
            path.clone(),
            Box::new(move |p| {
                if p.finished {
                    let _ = tx.send(p);
                }
            }),
        )
        .unwrap();
    rx.recv_timeout(WAIT).unwrap();
    let doc = engine.open_document(&path).unwrap();
    let doc_flow = eventually_sync(|| engine.get_flow_in(doc.store, 3).filter(|f| !f.extra.is_empty()));
    assert_eq!(doc_flow.extra, vec![Some("3".into())]);

    // Changing the columns drops the old values.
    engine.set_header_columns(Vec::new()).unwrap();
    assert!(engine.get_flow(10).unwrap().extra.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

fn eventually_sync<T>(mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

async fn repeat(engine: &Arc<Engine>, spec: RequestSpec, opts: RepeatOpts) -> (RepeatProgress, usize) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    engine
        .repeat(
            spec,
            opts,
            Box::new(move |p| {
                let _ = tx.send(p);
            }),
        )
        .unwrap();
    let mut updates = 0;
    loop {
        let p = tokio::time::timeout(WAIT * 4, rx.recv()).await.unwrap().unwrap();
        updates += 1;
        if p.finished {
            return (p, updates);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_thousand_repeats_report_the_right_counts() {
    let upstream = Upstream::start().await;
    let engine = engine(&temp_dir());
    let spec = RequestSpec {
        method: "GET".into(),
        url: upstream.url("/status/201"),
        ..Default::default()
    };
    let (done, updates) = repeat(
        &engine,
        spec,
        RepeatOpts {
            iterations: 1_000,
            concurrency: 16,
            delay_ms: 0,
            apply_rules: false,
        },
    )
    .await;
    assert_eq!((done.done, done.total, done.failed), (1_000, 1_000, 0));
    assert_eq!(
        done.statuses,
        vec![StatusCount {
            status: Some(201),
            count: 1_000
        }]
    );
    assert!(!done.cancelled && done.error.is_none());
    let (min, p50, p95, max) = (
        done.min_ms.unwrap(),
        done.p50_ms.unwrap(),
        done.p95_ms.unwrap(),
        done.max_ms.unwrap(),
    );
    assert!(min <= p50 && p50 <= p95 && p95 <= max);
    assert!(updates >= 2);
    engine.flush();
    let flows = engine.selected_flows(LIVE_STORE, &FlowSelection::All).unwrap();
    assert_eq!(flows.len(), 1_000);
    assert!(
        flows
            .iter()
            .all(|f| f.sent_by == Some(SentBy::Repeat) && f.status == Some(201))
    );
    assert_eq!((done.first_flow, done.last_flow), (Some(0), Some(999)));

    // Nothing listening: every request fails, and is counted as such.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let (failed, _) = repeat(
        &engine,
        RequestSpec {
            method: "GET".into(),
            url: format!("http://{closed}/"),
            ..Default::default()
        },
        RepeatOpts {
            iterations: 5,
            concurrency: 2,
            delay_ms: 1,
            apply_rules: true,
        },
    )
    .await;
    assert_eq!((failed.done, failed.failed), (5, 5));
    assert_eq!(failed.statuses, vec![StatusCount { status: None, count: 5 }]);

    // A spec that can't be sent stops at once with the reason.
    let (bad, _) = repeat(
        &engine,
        RequestSpec {
            method: "GET".into(),
            url: "not a url".into(),
            ..Default::default()
        },
        RepeatOpts::default(),
    )
    .await;
    assert_eq!(bad.done, 0);
    assert!(bad.error.is_some());
    assert!(
        engine
            .repeat(
                RequestSpec::default(),
                RepeatOpts {
                    iterations: 0,
                    ..Default::default()
                },
                Box::new(|_| {})
            )
            .is_err()
    );
    assert!(
        engine
            .repeat(
                RequestSpec::default(),
                RepeatOpts {
                    concurrency: 65,
                    ..Default::default()
                },
                Box::new(|_| {})
            )
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repeat_can_be_cancelled() {
    let upstream = Upstream::start().await;
    let engine = engine(&temp_dir());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let id = engine
        .repeat(
            RequestSpec {
                method: "GET".into(),
                url: upstream.url("/json"),
                ..Default::default()
            },
            RepeatOpts {
                iterations: 10_000,
                concurrency: 1,
                delay_ms: 20,
                apply_rules: false,
            },
            Box::new(move |p| {
                let _ = tx.send(p);
            }),
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(engine.cancel_repeat(id));
    let last = eventually(WAIT, || {
        let mut last = None;
        while let Ok(p) = rx.try_recv() {
            last = Some(p);
        }
        last.filter(|p| p.finished)
    })
    .await;
    assert!(last.cancelled);
    assert!(last.done < 10_000);
    assert!(!engine.cancel_repeat(id));
}

#[test]
fn code_snippets_come_from_the_recorded_request() {
    let dir = temp_dir();
    let engine = engine(&dir);
    record(&engine, 1, 16);
    let python = engine.code_snippet(LIVE_STORE, 0, CodeLanguage::Python).unwrap();
    assert!(
        python.contains("requests.get(") && python.contains("\"X-Flow\": \"0\""),
        "{python}"
    );
    let curl = engine.code_snippet(LIVE_STORE, 0, CodeLanguage::Curl).unwrap();
    assert!(curl.starts_with("curl "), "{curl}");
    assert!(engine.code_snippet(LIVE_STORE, 9, CodeLanguage::Go).is_err());
    let _ = std::fs::remove_dir_all(dir);
}
