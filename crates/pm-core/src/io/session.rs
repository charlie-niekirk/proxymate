//! Saving and opening `.proxymate` sessions (the tables are in
//! `store/sqlite.rs`). A save gathers each flow from whichever store it is
//! in, through the store's [`BlobBackend`], and writes a new file next to
//! the target that replaces it only once complete, so a failed save never
//! clobbers an older session.

use std::io;
use std::path::Path;

use pm_model::{BodyInfo, BodyPart, FlowDetail, FlowSummary, RequestHead, ResponseHead, Timing, WsQuery};
use rusqlite::{Connection, OpenFlags};

use crate::store::{BlobBackend, Opened, SavedFlow, SavedWs, SessionWriter, SqliteBackend};

/// The file extension of saved sessions.
pub const EXTENSION: &str = "proxymate";

/// Progress is reported every this many flows.
pub(crate) const PROGRESS_EVERY: u32 = 256;

/// Everything a store holds about one flow, ready to write.
pub(crate) fn collect(source: &dyn BlobBackend, summary: FlowSummary) -> io::Result<SavedFlow> {
    let id = summary.id;
    let detail = source.detail(id)?.unwrap_or_else(|| fallback_detail(&summary));
    let request_body = source.body(id, BodyPart::Request)?.unwrap_or_default();
    let response_body = source.body(id, BodyPart::Response)?.unwrap_or_default();
    let ws = match source.ws(id)? {
        Some(log) => {
            let page = log.page(source.segments(), &WsQuery::default(), 0, u32::MAX)?;
            let mut messages = Vec::with_capacity(page.messages.len());
            for meta in page.messages {
                let payload = log.payload(source.segments(), meta.index)?.unwrap_or_default();
                messages.push((meta, payload));
            }
            Some(SavedWs {
                messages,
                close: page.close,
            })
        }
        None => None,
    };
    let sse = source.sse(id)?.map(|log| log.events());
    Ok(SavedFlow {
        summary,
        detail,
        request_body,
        response_body,
        ws,
        sse,
    })
}

/// Writes `flows` to a new session at `path`. `progress` hears the number
/// written so far, every [`PROGRESS_EVERY`] flows.
pub(crate) fn write(
    path: &Path,
    flows: impl Iterator<Item = io::Result<SavedFlow>>,
    progress: &mut dyn FnMut(u32),
) -> io::Result<u32> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let partial = dir.join(format!(".{}.partial", uuid::Uuid::new_v4()));
    let written = write_to(&partial, flows, progress).and_then(|n| {
        std::fs::rename(&partial, path)?;
        Ok(n)
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written
}

fn write_to(
    path: &Path,
    flows: impl Iterator<Item = io::Result<SavedFlow>>,
    progress: &mut dyn FnMut(u32),
) -> io::Result<u32> {
    let conn = Connection::open(path).map_err(io::Error::other)?;
    // A fresh file that only replaces the target once complete: no journal needed.
    conn.execute_batch("PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;")
        .map_err(io::Error::other)?;
    let meta = [
        ("saved_at_ms", crate::proxy::now_ms().to_string()),
        ("app_version", env!("CARGO_PKG_VERSION").to_owned()),
    ];
    let mut writer = SessionWriter::create(conn, &meta).map_err(io::Error::other)?;
    let mut n = 0u32;
    for flow in flows {
        writer.add(flow?)?;
        n += 1;
        if n.is_multiple_of(PROGRESS_EVERY) {
            progress(n);
        }
    }
    let conn = writer.finish().map_err(io::Error::other)?;
    conn.close().map_err(|(_, e)| io::Error::other(e))?;
    progress(n);
    Ok(n)
}

/// Opens a saved session, read-only, and loads its summaries.
pub(crate) fn open(path: &Path) -> io::Result<Opened> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("can't open {}: {e}", path.display()),
            )
        })?;
    SqliteBackend::open(conn)
}

/// A detail for a flow that has none (synthetic flows): what the summary
/// says, with empty bodies.
pub(crate) fn fallback_detail(s: &FlowSummary) -> FlowDetail {
    let authority = if s.is_default_port() {
        s.host.to_string()
    } else {
        format!("{}:{}", s.host, s.port)
    };
    let query = if s.query.is_empty() {
        String::new()
    } else {
        format!("?{}", s.query)
    };
    FlowDetail {
        id: s.id,
        request: RequestHead {
            method: s.method.to_string(),
            url: format!("{}://{authority}{}{query}", s.scheme.as_str(), s.path),
            version: s.http_version,
            headers: Vec::new(),
        },
        response: s.status.map(|status| ResponseHead {
            status,
            reason: String::new(),
            version: s.http_version,
            headers: Vec::new(),
        }),
        request_body: BodyInfo {
            complete: true,
            ..Default::default()
        },
        response_body: BodyInfo {
            complete: true,
            ..Default::default()
        },
        error: None,
        tls: None,
        timing: Timing::default(),
        rules: Default::default(),
        trailers: None,
    }
}
