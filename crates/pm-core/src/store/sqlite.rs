//! The `.proxymate` session format, and the store backend that reads it.
//! An imported HAR is converted into the same tables, in memory.
//!
//! One SQLite file:
//! - `meta(key, value)`: the format name and version, when it was saved,
//!   by which version of Proxymate.
//! - `flows`: one row per flow, ids `0..n` in table order. The summary
//!   fields are columns (enums as their serde names), the [`FlowDetail`]
//!   is a postcard blob, and the WebSocket message list and the parsed
//!   server-sent events are zstd'd postcard blobs (NULL when the flow has
//!   none).
//! - `bodies(flow, part, data)`: each body zstd-compressed, still
//!   content-encoded as recorded. Empty bodies have no row.
//! - `messages(flow, idx, data)`: WebSocket payloads, zstd-compressed.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use moka::sync::Cache;
use parking_lot::Mutex;
use pm_model::{BodyPart, FlowDetail, FlowId, FlowSummary, SseEvent, WsClose, WsMessage};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::backend::BlobBackend;
use super::messages::{SseLog, WsLog};
use super::segments::SegmentStore;

/// `meta.format` of a saved session.
pub(crate) const FORMAT: &str = "proxymate-session";
/// Bumped whenever the tables or the postcard layouts change. Version 2
/// (M14) appended `FlowRules.scripts`, the last field of a `FlowDetail`.
pub(crate) const VERSION: u32 = 2;
/// The oldest version this Proxymate still reads.
const OLDEST: u32 = 1;

const ZSTD_LEVEL: i32 = 3;
const CACHE_ENTRIES: u64 = 1_024;

const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE flows (
    id INTEGER PRIMARY KEY,
    state TEXT NOT NULL,
    scheme TEXT NOT NULL,
    method TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    path TEXT NOT NULL,
    query TEXT NOT NULL,
    status INTEGER,
    content_kind TEXT NOT NULL,
    client TEXT,
    started_at_ms REAL NOT NULL,
    duration_ms INTEGER,
    req_size INTEGER NOT NULL,
    resp_size INTEGER NOT NULL,
    edited INTEGER NOT NULL,
    http_version TEXT NOT NULL,
    tls TEXT NOT NULL,
    graphql_op TEXT,
    sent_by TEXT,
    detail BLOB NOT NULL,
    ws BLOB,
    sse BLOB
);
CREATE TABLE bodies (flow INTEGER NOT NULL, part INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (flow, part)) WITHOUT ROWID;
CREATE TABLE messages (flow INTEGER NOT NULL, idx INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (flow, idx)) WITHOUT ROWID;
";

/// One flow to write, whatever store it comes from.
pub(crate) struct SavedFlow {
    pub summary: FlowSummary,
    pub detail: FlowDetail,
    pub request_body: Vec<u8>,
    pub response_body: Vec<u8>,
    pub ws: Option<SavedWs>,
    pub sse: Option<Vec<SseEvent>>,
}

/// A WebSocket flow's messages with their payloads, and how it closed.
pub(crate) struct SavedWs {
    pub messages: Vec<(WsMessage, Vec<u8>)>,
    pub close: Option<WsClose>,
}

/// What the `ws` column holds; the payloads are in `messages`.
#[derive(Serialize, Deserialize)]
struct WsColumn {
    messages: Vec<WsMessage>,
    close: Option<WsClose>,
}

/// Appends flows to a new session database inside one transaction.
pub(crate) struct SessionWriter {
    conn: Connection,
    next: FlowId,
}

impl SessionWriter {
    /// Creates the tables in an empty database and writes `meta`.
    pub fn create(conn: Connection, meta: &[(&str, String)]) -> rusqlite::Result<SessionWriter> {
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch("BEGIN")?;
        {
            let mut insert = conn.prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")?;
            insert.execute(params!["format", FORMAT])?;
            insert.execute(params!["version", VERSION.to_string()])?;
            for (key, value) in meta {
                insert.execute(params![key, value])?;
            }
        }
        Ok(SessionWriter { conn, next: 0 })
    }

    /// Writes one flow as the next id. The flow's own ids are replaced.
    pub fn add(&mut self, mut flow: SavedFlow) -> io::Result<()> {
        let id = self.next;
        flow.summary.id = id;
        flow.detail.id = id;
        let s = &flow.summary;
        let detail = postcard::to_allocvec(&flow.detail).map_err(io::Error::other)?;
        let ws = match &flow.ws {
            Some(ws) => Some(pack(&WsColumn {
                messages: ws.messages.iter().map(|(m, _)| m.clone()).collect(),
                close: ws.close.clone(),
            })?),
            None => None,
        };
        let sse = flow.sse.as_ref().map(pack).transpose()?;
        self.conn
            .prepare_cached(
                "INSERT INTO flows (id, state, scheme, method, host, port, path, query, status, content_kind, client,
                    started_at_ms, duration_ms, req_size, resp_size, edited, http_version, tls, graphql_op, sent_by,
                    detail, ws, sse)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                    ?21, ?22, ?23)",
            )
            .and_then(|mut insert| {
                insert.execute(params![
                    id,
                    enum_text(&s.state),
                    enum_text(&s.scheme),
                    &*s.method,
                    &*s.host,
                    s.port,
                    s.path,
                    s.query,
                    s.status,
                    enum_text(&s.content_kind),
                    s.client.as_deref(),
                    s.started_at_ms,
                    s.duration_ms,
                    s.req_size as i64,
                    s.resp_size as i64,
                    s.edited,
                    enum_text(&s.http_version),
                    enum_text(&s.tls),
                    s.graphql_op.as_deref(),
                    s.sent_by.as_ref().map(enum_text),
                    detail,
                    ws,
                    sse,
                ])
            })
            .map_err(io::Error::other)?;
        for (part, bytes) in [(0u8, &flow.request_body), (1u8, &flow.response_body)] {
            if bytes.is_empty() {
                continue;
            }
            let data = zstd::bulk::compress(bytes, ZSTD_LEVEL)?;
            self.conn
                .prepare_cached("INSERT INTO bodies (flow, part, data) VALUES (?1, ?2, ?3)")
                .and_then(|mut insert| insert.execute(params![id, part, data]))
                .map_err(io::Error::other)?;
        }
        if let Some(ws) = &flow.ws {
            for (meta, payload) in &ws.messages {
                if payload.is_empty() {
                    continue;
                }
                let data = zstd::bulk::compress(payload, ZSTD_LEVEL)?;
                self.conn
                    .prepare_cached("INSERT INTO messages (flow, idx, data) VALUES (?1, ?2, ?3)")
                    .and_then(|mut insert| insert.execute(params![id, meta.index, data]))
                    .map_err(io::Error::other)?;
            }
        }
        self.next += 1;
        Ok(())
    }

    /// Commits and hands the database back.
    pub fn finish(self) -> rusqlite::Result<Connection> {
        self.conn.execute_batch("COMMIT")?;
        Ok(self.conn)
    }
}

/// Reads flows back out of a session database.
pub(crate) struct SqliteBackend {
    conn: Mutex<Connection>,
    /// The format version it was saved with.
    version: u32,
    details: Cache<FlowId, Arc<FlowDetail>>,
    ws: Cache<FlowId, Option<Arc<WsLog>>>,
    sse: Cache<FlowId, Option<Arc<SseLog>>>,
}

/// An opened session: its summaries (ids `0..n`) and the backend for
/// everything else.
pub(crate) struct Opened {
    pub summaries: Vec<FlowSummary>,
    pub backend: SqliteBackend,
}

impl SqliteBackend {
    /// Checks the format and loads every summary.
    pub fn open(conn: Connection) -> io::Result<Opened> {
        let bad = |what: String| io::Error::new(io::ErrorKind::InvalidData, what);
        let meta: HashMap<String, String> = conn
            .prepare("SELECT key, value FROM meta")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()
            })
            .map_err(|_| bad("not a Proxymate session".into()))?;
        if meta.get("format").map(String::as_str) != Some(FORMAT) {
            return Err(bad("not a Proxymate session".into()));
        }
        let version: u32 = meta.get("version").and_then(|v| v.parse().ok()).unwrap_or(0);
        if !(OLDEST..=VERSION).contains(&version) {
            return Err(bad(format!(
                "this session is format version {version}; this Proxymate reads versions {OLDEST} to {VERSION}"
            )));
        }
        let summaries = conn
            .prepare(
                "SELECT id, state, scheme, method, host, port, path, query, status, content_kind, client,
                    started_at_ms, duration_ms, req_size, resp_size, edited, http_version, tls, graphql_op, sent_by
                 FROM flows ORDER BY id",
            )
            .and_then(|mut stmt| {
                let mut interned = Interner::default();
                stmt.query_map([], |row| summary(row, &mut interned))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| bad(format!("can't read the session's flows: {e}")))?;
        if summaries.iter().enumerate().any(|(i, s)| s.id as usize != i) {
            return Err(bad("the session's flow ids aren't consecutive".into()));
        }
        Ok(Opened {
            summaries,
            backend: SqliteBackend {
                conn: Mutex::new(conn),
                version,
                details: Cache::new(CACHE_ENTRIES),
                ws: Cache::new(64),
                sse: Cache::new(64),
            },
        })
    }

    fn blob(&self, sql: &str, args: impl rusqlite::Params) -> io::Result<Option<Vec<u8>>> {
        self.conn
            .lock()
            .prepare_cached(sql)
            .and_then(|mut stmt| stmt.query_row(args, |row| row.get::<_, Option<Vec<u8>>>(0)).optional())
            .map(Option::flatten)
            .map_err(io::Error::other)
    }
}

impl BlobBackend for SqliteBackend {
    fn detail(&self, id: FlowId) -> io::Result<Option<FlowDetail>> {
        if let Some(detail) = self.details.get(&id) {
            return Ok(Some(FlowDetail::clone(&detail)));
        }
        let Some(mut bytes) = self.blob("SELECT detail FROM flows WHERE id = ?1", [id])? else {
            return Ok(None);
        };
        if self.version == 1 {
            // Version 1 had no `FlowRules.scripts`: an empty list is one 0.
            bytes.push(0);
        }
        let detail: FlowDetail = postcard::from_bytes(&bytes).map_err(io::Error::other)?;
        self.details.insert(id, Arc::new(detail.clone()));
        Ok(Some(detail))
    }

    fn body(&self, id: FlowId, part: BodyPart) -> io::Result<Option<Vec<u8>>> {
        let part = match part {
            BodyPart::Request => 0u8,
            BodyPart::Response => 1u8,
        };
        match self.blob(
            "SELECT data FROM bodies WHERE flow = ?1 AND part = ?2",
            params![id, part],
        )? {
            Some(data) => zstd::stream::decode_all(&data[..]).map(Some),
            None => Ok(self.exists(id)?.then(Vec::new)),
        }
    }

    fn ws(&self, id: FlowId) -> io::Result<Option<Arc<WsLog>>> {
        if let Some(log) = self.ws.get(&id) {
            return Ok(log);
        }
        let log = match self.blob("SELECT ws FROM flows WHERE id = ?1", [id])? {
            Some(packed) => {
                let column: WsColumn = unpack(&packed)?;
                let mut payloads: HashMap<u32, Vec<u8>> = self
                    .conn
                    .lock()
                    .prepare_cached("SELECT idx, data FROM messages WHERE flow = ?1")
                    .and_then(|mut stmt| {
                        stmt.query_map([id], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, Vec<u8>>(1)?)))?
                            .collect::<rusqlite::Result<_>>()
                    })
                    .map_err(io::Error::other)?;
                let mut messages = Vec::with_capacity(column.messages.len());
                for meta in column.messages {
                    let payload = match payloads.remove(&meta.index) {
                        Some(data) => zstd::stream::decode_all(&data[..])?,
                        None => Vec::new(),
                    };
                    messages.push((meta, payload));
                }
                Some(Arc::new(WsLog::restore(messages, column.close)))
            }
            None => None,
        };
        self.ws.insert(id, log.clone());
        Ok(log)
    }

    fn sse(&self, id: FlowId) -> io::Result<Option<Arc<SseLog>>> {
        if let Some(log) = self.sse.get(&id) {
            return Ok(log);
        }
        let log = match self.blob("SELECT sse FROM flows WHERE id = ?1", [id])? {
            Some(packed) => Some(Arc::new(SseLog::restore(unpack(&packed)?))),
            None => None,
        };
        self.sse.insert(id, log.clone());
        Ok(log)
    }

    fn segments(&self) -> Option<&SegmentStore> {
        None
    }
}

impl SqliteBackend {
    fn exists(&self, id: FlowId) -> io::Result<bool> {
        self.conn
            .lock()
            .prepare_cached("SELECT 1 FROM flows WHERE id = ?1")
            .and_then(|mut stmt| stmt.exists([id]))
            .map_err(io::Error::other)
    }
}

/// Repeated hosts, methods and clients share one allocation, as in the
/// live store.
#[derive(Default)]
struct Interner(std::collections::HashSet<Arc<str>>);

impl Interner {
    fn get(&mut self, s: String) -> Arc<str> {
        if let Some(existing) = self.0.get(s.as_str()) {
            return existing.clone();
        }
        let s: Arc<str> = s.into();
        self.0.insert(s.clone());
        s
    }
}

fn summary(row: &Row<'_>, interned: &mut Interner) -> rusqlite::Result<FlowSummary> {
    Ok(FlowSummary {
        id: row.get(0)?,
        state: text_enum(row, 1)?,
        scheme: text_enum(row, 2)?,
        method: interned.get(row.get(3)?),
        host: interned.get(row.get(4)?),
        port: row.get(5)?,
        path: row.get(6)?,
        query: row.get(7)?,
        status: row.get(8)?,
        content_kind: text_enum(row, 9)?,
        client: row.get::<_, Option<String>>(10)?.map(|c| interned.get(c)),
        started_at_ms: row.get(11)?,
        duration_ms: row.get(12)?,
        req_size: row.get::<_, i64>(13)? as u64,
        resp_size: row.get::<_, i64>(14)? as u64,
        edited: row.get(15)?,
        http_version: text_enum(row, 16)?,
        tls: text_enum(row, 17)?,
        graphql_op: row.get::<_, Option<String>>(18)?.map(Arc::from),
        sent_by: match row.get::<_, Option<String>>(19)? {
            Some(_) => Some(text_enum(row, 19)?),
            None => None,
        },
        pinned: false,
    })
}

/// A fieldless enum as its serde name (`"completed"`, `"HTTP/2"`).
fn enum_text<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

fn text_enum<T: DeserializeOwned>(row: &Row<'_>, idx: usize) -> rusqlite::Result<T> {
    let text: String = row.get(idx)?;
    serde_json::from_value(serde_json::Value::String(text))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e)))
}

fn pack<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    let bytes = postcard::to_allocvec(value).map_err(io::Error::other)?;
    zstd::bulk::compress(&bytes, ZSTD_LEVEL)
}

fn unpack<T: DeserializeOwned>(packed: &[u8]) -> io::Result<T> {
    let bytes = zstd::stream::decode_all(packed)?;
    postcard::from_bytes(&bytes).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Generator;
    use pm_model::{WsDirection, WsOpcode, WsQuery};

    #[test]
    fn flows_round_trip_through_the_tables() {
        let mut generator = Generator::new(3);
        let mut writer = SessionWriter::create(Connection::open_in_memory().unwrap(), &[]).unwrap();
        let mut originals = Vec::new();
        for i in 0..3u32 {
            let mut summary = generator.completed_flow();
            summary.id = 100 + i;
            summary.sent_by = (i == 1).then_some(pm_model::SentBy::Agent);
            let detail = crate::io::session::fallback_detail(&summary);
            let ws = (i == 2).then(|| SavedWs {
                messages: vec![(
                    WsMessage {
                        index: 0,
                        direction: WsDirection::Sent,
                        opcode: WsOpcode::Text,
                        len: 5,
                        time_ms: 1.0,
                        truncated: false,
                        close_code: None,
                        preview: "hello".into(),
                    },
                    b"hello".to_vec(),
                )],
                close: None,
            });
            originals.push(summary.clone());
            writer
                .add(SavedFlow {
                    summary,
                    detail,
                    request_body: if i == 0 { Vec::new() } else { vec![i as u8; 20_000] },
                    response_body: b"{\"ok\":true}".to_vec(),
                    ws,
                    sse: None,
                })
                .unwrap();
        }
        let opened = SqliteBackend::open(writer.finish().unwrap()).unwrap();
        assert_eq!(opened.summaries.len(), 3);
        for (i, (read, original)) in opened.summaries.iter().zip(&originals).enumerate() {
            let mut expected = original.clone();
            expected.id = i as u32;
            assert_eq!(read, &expected);
        }
        let backend = opened.backend;
        assert_eq!(backend.detail(1).unwrap().unwrap().id, 1);
        assert_eq!(backend.body(0, BodyPart::Request).unwrap(), Some(Vec::new()));
        assert_eq!(backend.body(1, BodyPart::Request).unwrap().unwrap(), vec![1u8; 20_000]);
        assert_eq!(backend.body(9, BodyPart::Request).unwrap(), None);
        let log = backend.ws(2).unwrap().unwrap();
        let page = log.page(None, &WsQuery::default(), 0, 10).unwrap();
        assert_eq!((page.total, page.open), (1, false));
        assert_eq!(log.payload(None, 0).unwrap().unwrap(), b"hello");
        assert!(backend.ws(1).unwrap().is_none());
    }

    /// Every origin survives postcard (it can't read serde's internally
    /// tagged derive), and JSON keeps the tagged shape.
    #[test]
    fn rule_origins_round_trip_through_postcard_and_json() {
        let origins = [
            pm_model::RuleOrigin::User,
            pm_model::RuleOrigin::Preset {
                preset: pm_model::PresetKind::MapLocal,
            },
            pm_model::RuleOrigin::Agent {
                client: "claude".into(),
                session: Some("s1".into()),
                created_at_ms: 12.5,
            },
        ];
        let rules = pm_model::FlowRules {
            applied: origins
                .iter()
                .map(|origin| pm_model::AppliedRule {
                    rule_id: "r".into(),
                    name: "n".into(),
                    origin: origin.clone(),
                    phase: pm_model::Phase::Response,
                })
                .collect(),
            scripts: vec![pm_model::ScriptIssue {
                script_id: "s".into(),
                script_name: "Decode".into(),
                rule_id: None,
                phase: pm_model::ScriptPhase::Mock,
                message: "boom".into(),
            }],
            ..Default::default()
        };
        let bytes = postcard::to_allocvec(&rules).unwrap();
        assert_eq!(postcard::from_bytes::<pm_model::FlowRules>(&bytes).unwrap(), rules);
        assert_eq!(
            serde_json::to_value(&origins[2]).unwrap(),
            serde_json::json!({"kind": "agent", "client": "claude", "session": "s1", "created_at_ms": 12.5})
        );
        let back: pm_model::RuleOrigin = serde_json::from_str(r#"{"kind":"preset","preset":"map_local"}"#).unwrap();
        assert_eq!(back, origins[1]);
    }

    #[test]
    fn other_databases_are_refused() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE meta (key TEXT, value TEXT)").unwrap();
        let err = SqliteBackend::open(conn).err().unwrap();
        assert!(err.to_string().contains("not a Proxymate session"), "{err}");
    }
}
