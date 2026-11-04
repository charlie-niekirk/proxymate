//! Pins the version 7 session format: a committed file
//! (`tests/fixtures/session-v7.proxymate`) that must keep decoding to what
//! [`flows`] describes, and the postcard index of every enum a session
//! stores by position.
//!
//! The round-trip tests in `tests.rs` save and reopen with the same code,
//! so they pass whatever the layout is. These tests catch a change that
//! should have bumped `VERSION`: a field moved or inserted, or an enum
//! variant added anywhere but the end.
//!
//! **Never regenerate the fixture to make these tests pass.** A file saved
//! by a version 7 release has exactly these bytes. Version 8 (R3) appended
//! `FlowDetail.error_kind`, so this file is now the version 7 case:
//! [`flows`] gives the kind reading it infers. A later layout change bumps
//! `VERSION` again and keeps this file as it is. `write_the_v7_fixture`
//! refuses to overwrite it.

use std::path::PathBuf;
use std::sync::Arc;

use pm_model::{
    AppliedRule, BodyInfo, BodyPart, CertSummary, ContentKind, ErrorKind, FlowDetail, FlowLabel, FlowRules, FlowState,
    FlowSummary, Header, HttpVersion, Phase, PresetKind, RequestHead, ResponseHead, RuleOrigin, Scheme, ScriptIssue,
    ScriptPhase, SentBy, SseEvent, Timing, TlsInfo, TlsStatus, Via, WsClose, WsDirection, WsMessage, WsOpcode, WsQuery,
};
use rusqlite::Connection;

use super::{BlobBackend, SavedFlow, SavedWs, SessionWriter, SqliteBackend, VERSION};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/session-v7.proxymate")
}

fn headers(pairs: &[(&str, &str)]) -> Vec<Header> {
    pairs.iter().map(|(n, v)| Header::new(*n, *v)).collect()
}

/// A summary with every optional field set, so each column is exercised.
fn summary(id: u32, scheme: Scheme, method: &str, path: &str, kind: ContentKind) -> FlowSummary {
    FlowSummary {
        id,
        state: FlowState::Completed,
        scheme,
        method: Arc::from(method),
        host: Arc::from("api.example.test"),
        port: 443,
        path: path.into(),
        query: "page=2&sort=desc".into(),
        status: Some(200),
        content_kind: kind,
        client: Some(Arc::from("Safari")),
        started_at_ms: 1_793_000_000_000.5,
        duration_ms: Some(42),
        req_size: 20_000,
        resp_size: 11,
        edited: true,
        http_version: HttpVersion::Http2,
        tls: TlsStatus::Intercepted,
        graphql_op: None,
        sent_by: None,
        pinned: false,
        grpc_status: None,
        label: None,
        comment: None,
        extra: Vec::new(),
    }
}

fn body(size: u64, kind: ContentKind, mime: &str) -> BodyInfo {
    BodyInfo {
        size,
        captured: size,
        truncated: false,
        encoding: None,
        complete: true,
        kind,
        mime: Some(mime.into()),
    }
}

/// A detail that sets every field. Each postcard enum takes its *last*
/// variant somewhere, so a variant inserted before it shifts the index and
/// the decoded value no longer matches.
fn detail(id: u32, url: &str) -> FlowDetail {
    FlowDetail {
        id,
        request: RequestHead {
            method: "POST".into(),
            url: url.into(),
            version: HttpVersion::Http2,
            headers: headers(&[("content-type", "application/cbor"), ("x-trace", "abc")]),
        },
        response: Some(ResponseHead {
            status: 200,
            reason: "OK".into(),
            version: HttpVersion::Http2,
            headers: headers(&[("content-type", "application/cbor")]),
        }),
        request_body: BodyInfo {
            encoding: Some("gzip".into()),
            truncated: true,
            captured: 16_384,
            ..body(20_000, ContentKind::MessagePack, "application/msgpack")
        },
        response_body: body(11, ContentKind::Cbor, "application/cbor"),
        error: Some("upstream reset the stream after the response".into()),
        // Not in the file (version 8 appended it): what reading a version 7
        // file infers from the message (`failure::legacy_kind`).
        error_kind: Some(ErrorKind::Other),
        tls: Some(TlsInfo {
            sni: Some("api.example.test".into()),
            client_alpn: vec!["h2".into(), "http/1.1".into()],
            alpn: Some("h2".into()),
            version: "TLSv1.3".into(),
            cipher: "TLS13_AES_128_GCM_SHA256".into(),
            upstream_verified: true,
            chain: vec![CertSummary {
                subject: "CN=api.example.test".into(),
                issuer: "CN=Example Issuing CA".into(),
                serial: "01:02:03".into(),
                not_before: "2026-01-01T00:00:00Z".into(),
                not_after: "2027-01-01T00:00:00Z".into(),
                san: vec!["api.example.test".into(), "*.example.test".into()],
                sha256: "ab".repeat(32),
            }],
        }),
        timing: Timing {
            dns_ms: Some(1.5),
            connect_ms: Some(2.25),
            tls_ms: Some(3.0),
            ttfb_ms: Some(10.75),
            transfer_ms: Some(0.5),
            reused_connection: true,
        },
        rules: FlowRules {
            applied: vec![
                AppliedRule {
                    rule_id: "r-user".into(),
                    name: "Tag requests".into(),
                    origin: RuleOrigin::User,
                    phase: Phase::Request,
                },
                AppliedRule {
                    rule_id: "r-serve".into(),
                    name: "Mock from session".into(),
                    origin: RuleOrigin::Preset {
                        preset: PresetKind::ServeFrom,
                    },
                    phase: Phase::Response,
                },
                AppliedRule {
                    rule_id: "r-openapi".into(),
                    name: "getItem".into(),
                    origin: RuleOrigin::Preset {
                        preset: PresetKind::OpenApi,
                    },
                    phase: Phase::Response,
                },
                AppliedRule {
                    rule_id: "r-agent".into(),
                    name: "Agent rule".into(),
                    origin: RuleOrigin::Agent {
                        client: "claude".into(),
                        session: Some("s1".into()),
                        created_at_ms: 1_793_000_000_000.25,
                    },
                    phase: Phase::Request,
                },
            ],
            rewrite_skipped: Some("the body was over the rewrite limit".into()),
            breakpoint_timeout: true,
            mocked: true,
            scripts: vec![ScriptIssue {
                script_id: "sc-1".into(),
                script_name: "Decode frames".into(),
                rule_id: Some("r-user".into()),
                phase: ScriptPhase::WsMessage,
                message: "TypeError: x is undefined".into(),
            }],
        },
        trailers: Some(headers(&[("grpc-status", "0"), ("grpc-message", "")])),
        via: Some(Via {
            proxy: Some("http://corp-proxy.test:3128".into()),
            addr: Some("203.0.113.7:443".into()),
            dns_override: Some("api.example.test → 203.0.113.7".into()),
            client_cert: Some("Dev client".into()),
            reverse_port: Some(8443),
        }),
    }
}

fn ws_message(index: u32, direction: WsDirection, opcode: WsOpcode, preview: &str) -> WsMessage {
    WsMessage {
        index,
        direction,
        opcode,
        len: preview.len() as u64,
        time_ms: 100.0 + f64::from(index),
        truncated: false,
        close_code: None,
        preview: preview.into(),
        edited: false,
        injected: false,
        dropped: false,
    }
}

/// What the version 7 fixture holds: an HTTP/2 flow with every detail
/// field set and annotated, a WebSocket with edited, injected and dropped
/// messages, and an event stream.
fn flows() -> Vec<SavedFlow> {
    let http = SavedFlow {
        summary: FlowSummary {
            graphql_op: Some(Arc::from("GetItem")),
            sent_by: Some(SentBy::Repeat),
            pinned: true,
            grpc_status: Some(0),
            label: Some(FlowLabel::Purple),
            comment: Some(Arc::from("the slow one")),
            ..summary(0, Scheme::Https, "POST", "/v1/items", ContentKind::Cbor)
        },
        detail: detail(0, "https://api.example.test/v1/items?page=2&sort=desc"),
        request_body: vec![7u8; 20_000],
        response_body: b"\xa1\x62ok\xf5 cbor".to_vec(),
        ws: None,
        sse: None,
    };
    let edited = WsMessage {
        edited: true,
        ..ws_message(0, WsDirection::Sent, WsOpcode::Text, "hello")
    };
    let injected = WsMessage {
        injected: true,
        ..ws_message(1, WsDirection::Received, WsOpcode::Binary, "")
    };
    let dropped = WsMessage {
        dropped: true,
        truncated: true,
        close_code: Some(4001),
        ..ws_message(2, WsDirection::Received, WsOpcode::Close, "bye")
    };
    let websocket = SavedFlow {
        summary: FlowSummary {
            status: Some(101),
            http_version: HttpVersion::Http11,
            ..summary(1, Scheme::Wss, "GET", "/socket", ContentKind::None)
        },
        detail: detail(1, "wss://api.example.test/socket"),
        request_body: Vec::new(),
        response_body: Vec::new(),
        ws: Some(SavedWs {
            messages: vec![
                (edited, b"hello".to_vec()),
                (injected, vec![0, 1, 2, 255]),
                (dropped, b"bye".to_vec()),
            ],
            originals: vec![(0, b"HELLO".to_vec())],
            close: Some(WsClose {
                code: 4001,
                reason: "going away".into(),
                by: WsDirection::Received,
            }),
        }),
        sse: None,
    };
    let events = SavedFlow {
        summary: FlowSummary {
            state: FlowState::Failed,
            ..summary(2, Scheme::Https, "GET", "/events", ContentKind::Text)
        },
        detail: detail(2, "https://api.example.test/events"),
        request_body: Vec::new(),
        response_body: b"id: 1\nevent: tick\ndata: {\"n\":1}\nretry: 500\n\n".to_vec(),
        ws: None,
        sse: Some(vec![SseEvent {
            index: 0,
            time_ms: Some(12.5),
            id: Some("1".into()),
            event: Some("tick".into()),
            data: "{\"n\":1}".into(),
            retry: Some(500),
            truncated: true,
        }]),
    };
    vec![http, websocket, events]
}

/// Writes the fixture, once. Run with `cargo test -p pm-core --lib
/// write_the_v7_fixture -- --ignored` only when the file doesn't exist.
#[test]
#[ignore = "writes tests/fixtures/session-v7.proxymate; it is committed and must not change"]
fn write_the_v7_fixture() {
    assert_eq!(VERSION, 7, "this writes a version 7 file; keep the committed one");
    let path = fixture();
    assert!(
        !path.exists(),
        "{} exists: it is what version 7 wrote and must not be regenerated",
        path.display()
    );
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode = OFF;").unwrap();
    let meta = [
        ("saved_at_ms", "1793000000000".to_owned()),
        ("app_version", "0.4.0".to_owned()),
    ];
    let mut writer = SessionWriter::create(conn, &meta).unwrap();
    for flow in flows() {
        writer.add(flow).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn the_committed_v7_session_still_decodes_as_it_was_written() {
    let conn = Connection::open_with_flags(fixture(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("tests/fixtures/session-v7.proxymate is committed");
    let opened = SqliteBackend::open(conn).unwrap();
    assert_eq!(opened.backend.version, 7);
    let expected = flows();
    assert_eq!(opened.summaries.len(), expected.len());
    for (i, (read, want)) in opened.summaries.iter().zip(&expected).enumerate() {
        assert_eq!(read, &want.summary, "summary of flow {i}");
        let detail = opened.backend.detail(i as u32).unwrap().unwrap();
        assert_eq!(detail, want.detail, "detail of flow {i}");
        for (part, bytes) in [
            (BodyPart::Request, &want.request_body),
            (BodyPart::Response, &want.response_body),
        ] {
            let stored = opened.backend.body(i as u32, part).unwrap().unwrap_or_default();
            assert_eq!(&stored, bytes, "{part:?} body of flow {i}");
        }
        match &want.ws {
            Some(ws) => {
                let log = opened.backend.ws(i as u32).unwrap().expect("a WebSocket log");
                let page = log.page(None, &WsQuery::default(), 0, 10).unwrap();
                let messages: Vec<WsMessage> = ws.messages.iter().map(|(m, _)| m.clone()).collect();
                assert_eq!(page.messages, messages, "messages of flow {i}");
                assert_eq!(page.close, ws.close, "close of flow {i}");
                for (m, payload) in &ws.messages {
                    let read = log.payload(None, m.index).unwrap().unwrap_or_default();
                    assert_eq!(&read, payload, "payload {} of flow {i}", m.index);
                }
                for (index, original) in &ws.originals {
                    assert_eq!(&log.original(None, *index).unwrap().unwrap(), original);
                }
            }
            None => assert!(
                opened.backend.ws(i as u32).unwrap().is_none(),
                "flow {i} has no WebSocket log"
            ),
        }
        let events = opened.backend.sse(i as u32).unwrap().map(|log| log.events());
        assert_eq!(events, want.sse, "events of flow {i}");
    }
}

/// Checks each listed variant's postcard tag. `index` is an exhaustive
/// match, so a new variant doesn't compile until it is listed here.
macro_rules! pin {
    ($ty:ident { $($variant:ident = $index:literal),+ $(,)? }) => {{
        fn index(v: &$ty) -> u8 {
            match v {
                $($ty::$variant => $index),+
            }
        }
        for v in [$($ty::$variant),+] {
            let bytes = postcard::to_allocvec(&v).unwrap();
            assert_eq!(bytes, [index(&v)], "{}::{v:?}", stringify!($ty));
        }
    }};
}

/// postcard writes an enum as its variant's position. These enums are
/// inside a session's postcard columns (`FlowDetail`, the `ws` and `sse`
/// columns), so their order is part of the format. A new variant must go at
/// the end, with the next index here, and a `VERSION` bump: an older
/// Proxymate can't read it. Never renumber a variant.
#[test]
fn session_enums_keep_their_postcard_indices() {
    pin!(HttpVersion { Http10 = 0, Http11 = 1, Http2 = 2 });
    pin!(ContentKind {
        None = 0,
        Json = 1,
        Xml = 2,
        Html = 3,
        FormUrlEncoded = 4,
        Multipart = 5,
        GraphQl = 6,
        Javascript = 7,
        Css = 8,
        Text = 9,
        Image = 10,
        Svg = 11,
        Video = 12,
        Audio = 13,
        Font = 14,
        Pdf = 15,
        Binary = 16,
        Grpc = 17,
        Protobuf = 18,
        MessagePack = 19,
        Cbor = 20,
    });
    pin!(Phase { Request = 0, Response = 1 });
    pin!(PresetKind {
        MapLocal = 0,
        Breakpoint = 1,
        Rewrite = 2,
        MapRemote = 3,
        Block = 4,
        Throttle = 5,
        ServeFrom = 6,
        OpenApi = 7,
    });
    pin!(ScriptPhase { Request = 0, Response = 1, Mock = 2, View = 3, WsMessage = 4 });
    pin!(WsDirection { Sent = 0, Received = 1 });
    pin!(WsOpcode { Text = 0, Binary = 1, Ping = 2, Pong = 3, Close = 4 });
    // Version 8 (R3): `FlowDetail.error_kind`.
    pin!(ErrorKind {
        Other = 0,
        ClientRejectedCert = 1,
        ClientTls = 2,
        ClientGone = 3,
        Dns = 4,
        Refused = 5,
        Timeout = 6,
        ClientCertRequired = 7,
        UpstreamCertUntrusted = 8,
        UpstreamCertInvalid = 9,
        UpstreamTlsVersion = 10,
        UpstreamTls = 11,
        UpstreamProxy = 12,
        UpstreamProxyAuth = 13,
        UpstreamProxyLoop = 14,
        Upstream = 15,
    });
}
