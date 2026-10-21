//! What the engine keeps for the MCP server (`pm-mcp`): its settings, the
//! per-install bearer token, the audit log of tool calls (all in
//! `app.sqlite`) and the live status (where it serves, which clients are
//! connected) that the toolbar and Settings show. The server itself is an
//! adapter like the Tauri commands; it reports its state here.

use std::path::Path;

use parking_lot::{Mutex, RwLock};
use pm_model::{McpAuditEntry, McpAuditPage, McpCallOutcome, McpClient, McpSettings, McpStatus};
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{OptionalExtension, params};
use tokio::sync::watch;

use crate::notify::{Notifier, Sink};
use crate::rules::RuleError;
use crate::rules::db::{APP_DB_FILE, AppDb};
use crate::{Engine, Error};

const SETTINGS_KEY: &str = "mcp";
const TOKEN_KEY: &str = "mcp_token";
/// The audit log keeps this many calls; older ones are dropped.
const AUDIT_KEEP: i64 = 10_000;
/// Arguments longer than this are cut in the log.
const AUDIT_ARGS_MAX: usize = 4096;

/// Receives the MCP status whenever it changes. Returning `false`
/// unsubscribes.
pub type McpSink = Sink<McpStatus>;

/// One finished tool call, for [`Engine::record_mcp_call`].
#[derive(Debug, Clone)]
pub struct McpCall {
    pub client: String,
    pub session: Option<String>,
    pub tool: String,
    pub arguments: String,
    pub outcome: McpCallOutcome,
    pub error: Option<String>,
    pub duration_ms: u32,
}

pub(crate) struct McpState {
    db: Mutex<AppDb>,
    settings: watch::Sender<McpSettings>,
    token: RwLock<String>,
    serving: Mutex<(Option<String>, Option<String>)>,
    clients: Mutex<Vec<McpClient>>,
    changes: Notifier<McpStatus>,
}

impl McpState {
    pub fn load(dir: Option<&Path>) -> McpState {
        let db = dir
            .map(AppDb::open)
            .unwrap_or_else(AppDb::in_memory)
            .or_else(|err| {
                eprintln!("proxymate: can't open the app database for MCP: {err}; using defaults");
                AppDb::in_memory()
            })
            .expect("in-memory SQLite");
        if let Err(err) = create_audit_table(&db) {
            eprintln!("proxymate: can't create the MCP audit log: {err}");
        }
        let settings: McpSettings = db.setting(SETTINGS_KEY).unwrap_or_default();
        let token = stored_token(&db).unwrap_or_else(|| {
            let token = new_token();
            if let Err(err) = db.save_setting(TOKEN_KEY, &token) {
                eprintln!("proxymate: can't save the MCP token: {err}");
            }
            token
        });
        McpState {
            db: Mutex::new(db),
            settings: watch::Sender::new(settings),
            token: RwLock::new(token),
            serving: Mutex::new((None, None)),
            clients: Mutex::new(Vec::new()),
            changes: Notifier::new(),
        }
    }

    fn status(&self) -> McpStatus {
        let (url, error) = self.serving.lock().clone();
        McpStatus {
            settings: self.settings.borrow().clone(),
            url,
            error,
            clients: self.clients.lock().clone(),
        }
    }
}

fn create_audit_table(db: &AppDb) -> rusqlite::Result<()> {
    db.conn().execute_batch(
        "CREATE TABLE IF NOT EXISTS mcp_audit (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             time_ms REAL NOT NULL,
             client TEXT NOT NULL,
             session TEXT,
             tool TEXT NOT NULL,
             arguments TEXT NOT NULL,
             outcome TEXT NOT NULL,
             error TEXT,
             duration_ms INTEGER NOT NULL
         );",
    )
}

fn stored_token(db: &AppDb) -> Option<String> {
    db.setting::<String>(TOKEN_KEY).ok().filter(|t| t.len() >= 32)
}

/// 32 random bytes, hex: the bearer token MCP clients send.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    SystemRandom::new().fill(&mut bytes).expect("system randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads the bearer token from `<dir>/app.sqlite` without an engine, for
/// the stdio bridge (a separate process). `None` if the app never ran.
pub fn read_token(dir: &Path) -> Option<String> {
    if !dir.join(APP_DB_FILE).exists() {
        return None;
    }
    stored_token(&AppDb::open(dir).ok()?)
}

fn outcome_str(outcome: McpCallOutcome) -> &'static str {
    match outcome {
        McpCallOutcome::Ok => "ok",
        McpCallOutcome::Error => "error",
        McpCallOutcome::Refused => "refused",
    }
}

fn parse_outcome(s: &str) -> McpCallOutcome {
    match s {
        "ok" => McpCallOutcome::Ok,
        "refused" => McpCallOutcome::Refused,
        _ => McpCallOutcome::Error,
    }
}

/// Cuts `s` to at most `max` bytes on a character boundary.
fn cut(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('…');
    }
    s
}

impl Engine {
    // ---- MCP (M7) ----

    pub fn mcp_settings(&self) -> McpSettings {
        self.mcp.settings.borrow().clone()
    }

    /// Saves and applies MCP settings. The server picks them up at once:
    /// it starts or stops, and read-only changes the tool list of every
    /// connected client.
    pub fn set_mcp_settings(&self, settings: McpSettings) -> Result<McpSettings, Error> {
        self.mcp
            .db
            .lock()
            .save_setting(SETTINGS_KEY, &settings)
            .map_err(RuleError::from)?;
        self.mcp.settings.send_replace(settings.clone());
        self.mcp.changes.changed();
        Ok(settings)
    }

    /// Follows the MCP settings (for the server).
    pub fn watch_mcp_settings(&self) -> watch::Receiver<McpSettings> {
        self.mcp.settings.subscribe()
    }

    /// The bearer token MCP clients must send.
    pub fn mcp_token(&self) -> String {
        self.mcp.token.read().clone()
    }

    /// Replaces the bearer token. Clients using the old one get 401 from
    /// their next request on.
    pub fn regenerate_mcp_token(&self) -> Result<String, Error> {
        let token = new_token();
        let mut current = self.mcp.token.write();
        self.mcp
            .db
            .lock()
            .save_setting(TOKEN_KEY, &token)
            .map_err(RuleError::from)?;
        *current = token.clone();
        Ok(token)
    }

    pub fn mcp_status(&self) -> McpStatus {
        self.mcp.status()
    }

    /// Sends the MCP status to `sink` on the next tick and whenever it
    /// changes (clients connect or leave, settings change).
    pub fn subscribe_mcp(&self, sink: McpSink) {
        self.mcp.changes.subscribe(sink);
    }

    pub(crate) fn publish_mcp(&self) {
        self.mcp.changes.publish(|| self.mcp.status());
    }

    /// The server reports where it serves (`url`) or why it can't.
    pub fn set_mcp_serving(&self, url: Option<String>, error: Option<String>) {
        if url.is_none() {
            self.mcp.clients.lock().clear();
        }
        *self.mcp.serving.lock() = (url, error);
        self.mcp.changes.changed();
    }

    /// A client finished `initialize`.
    pub fn mcp_client_connected(&self, client: McpClient) {
        let mut clients = self.mcp.clients.lock();
        clients.retain(|c| c.session != client.session);
        clients.push(client);
        self.mcp.changes.changed();
    }

    /// A client's session ended.
    pub fn mcp_client_disconnected(&self, session: &str) {
        self.mcp.clients.lock().retain(|c| c.session != session);
        self.mcp.changes.changed();
    }

    /// Appends one tool call to the audit log (keeping the newest 10,000).
    pub fn record_mcp_call(&self, call: McpCall) {
        let db = self.mcp.db.lock();
        let result = db
            .conn()
            .execute(
                "INSERT INTO mcp_audit (time_ms, client, session, tool, arguments, outcome, error, duration_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    crate::proxy::now_ms(),
                    call.client,
                    call.session,
                    call.tool,
                    cut(call.arguments, AUDIT_ARGS_MAX),
                    outcome_str(call.outcome),
                    call.error.map(|e| cut(e, AUDIT_ARGS_MAX)),
                    call.duration_ms,
                ],
            )
            .and_then(|_| {
                let id = db.conn().last_insert_rowid();
                if id % 100 == 0 {
                    db.conn()
                        .execute("DELETE FROM mcp_audit WHERE id <= ?1", params![id - AUDIT_KEEP])?;
                }
                Ok(())
            });
        if let Err(err) = result {
            eprintln!("proxymate: can't write the MCP audit log: {err}");
        }
    }

    /// The newest `limit` audit entries older than `before` (all, without).
    pub fn mcp_audit(&self, before: Option<i64>, limit: u32) -> Result<McpAuditPage, Error> {
        let db = self.mcp.db.lock();
        let read = || -> rusqlite::Result<Vec<McpAuditEntry>> {
            let mut stmt = db.conn().prepare(
                "SELECT id, time_ms, client, session, tool, arguments, outcome, error, duration_ms
                 FROM mcp_audit WHERE id < ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![before.unwrap_or(i64::MAX), limit.min(1000)], |row| {
                Ok(McpAuditEntry {
                    id: row.get(0)?,
                    time_ms: row.get(1)?,
                    client: row.get(2)?,
                    session: row.get(3)?,
                    tool: row.get(4)?,
                    arguments: row.get(5)?,
                    outcome: parse_outcome(&row.get::<_, String>(6)?),
                    error: row.get(7)?,
                    duration_ms: row.get(8)?,
                })
            })?;
            rows.collect()
        };
        let entries = read().map_err(RuleError::from)?;
        let more = match entries.last() {
            Some(last) => db
                .conn()
                .query_row(
                    "SELECT 1 FROM mcp_audit WHERE id < ?1 LIMIT 1",
                    params![last.id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(RuleError::from)?
                .is_some(),
            None => false,
        };
        let next = entries.last().filter(|_| more).map(|e| e.id);
        Ok(McpAuditPage { entries, next })
    }

    pub fn clear_mcp_audit(&self) -> Result<(), Error> {
        self.mcp
            .db
            .lock()
            .conn()
            .execute("DELETE FROM mcp_audit", [])
            .map_err(RuleError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(tool: &str) -> McpCall {
        McpCall {
            client: "claude-code".into(),
            session: Some("s1".into()),
            tool: tool.into(),
            arguments: "{}".into(),
            outcome: McpCallOutcome::Ok,
            error: None,
            duration_ms: 3,
        }
    }

    #[test]
    fn the_audit_log_pages_newest_first() {
        let engine = Engine::new();
        for i in 0..5 {
            engine.record_mcp_call(call(&format!("tool{i}")));
        }
        let first = engine.mcp_audit(None, 3).unwrap();
        let tools: Vec<&str> = first.entries.iter().map(|e| e.tool.as_str()).collect();
        assert_eq!(tools, ["tool4", "tool3", "tool2"]);
        let rest = engine.mcp_audit(first.next, 3).unwrap();
        assert_eq!(rest.entries.len(), 2);
        assert_eq!(rest.next, None);
        engine.clear_mcp_audit().unwrap();
        assert!(engine.mcp_audit(None, 10).unwrap().entries.is_empty());
    }

    #[test]
    fn the_token_persists_and_regenerates() {
        let dir = std::env::temp_dir().join(format!("pm-mcp-token-{}", uuid::Uuid::new_v4()));
        let engine = Engine::with_config(crate::EngineConfig {
            data_dir: Some(dir.clone()),
            ..Default::default()
        });
        let token = engine.mcp_token();
        assert_eq!(token.len(), 64);
        assert_eq!(read_token(&dir).as_deref(), Some(token.as_str()));
        let fresh = engine.regenerate_mcp_token().unwrap();
        assert_ne!(fresh, token);
        assert_eq!(read_token(&dir), Some(fresh));
        engine.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn settings_changes_reach_watchers() {
        let engine = Engine::new();
        let watch = engine.watch_mcp_settings();
        assert!(!watch.borrow().read_only);
        engine
            .set_mcp_settings(McpSettings {
                enabled: true,
                read_only: true,
            })
            .unwrap();
        assert!(watch.borrow().read_only);
        assert!(engine.mcp_status().settings.read_only);
    }
}
