//! `app.sqlite`: the app database (WAL). It holds the rules, the rule
//! settings, (M9) the LAN client allow-list and LAN access settings, and
//! (M7) the MCP settings, bearer token and audit log (`mcp.rs`);
//! the SSL lists stay in `ssl.json` until they move here. The rules engine
//! and the device registry each open their own connection.
//!
//! Rules are stored as JSON documents, one row each, so the schema doesn't
//! change every time an action gains a field.

use std::net::IpAddr;
use std::path::Path;

use pm_model::{Rule, RuleSettings};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub const APP_DB_FILE: &str = "app.sqlite";

const RULE_SETTINGS_KEY: &str = "rules";

pub(crate) struct AppDb {
    conn: Connection,
}

/// A LAN client the user answered for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SavedDevice {
    pub ip: IpAddr,
    pub nickname: Option<String>,
    pub allowed: bool,
}

impl AppDb {
    /// Opens (creating if needed) `<dir>/app.sqlite`.
    pub fn open(dir: &Path) -> rusqlite::Result<AppDb> {
        let _ = std::fs::create_dir_all(dir);
        let conn = Connection::open(dir.join(APP_DB_FILE))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        AppDb::init(conn)
    }

    /// A database that lives only as long as the engine (tests, and
    /// engines without a data directory).
    pub fn in_memory() -> rusqlite::Result<AppDb> {
        AppDb::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<AppDb> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS rules (id TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS devices (ip TEXT PRIMARY KEY, nickname TEXT, allowed INTEGER NOT NULL);",
        )?;
        Ok(AppDb { conn })
    }

    /// For tables owned by other modules (the MCP audit log).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Every stored rule. Rows that no longer parse are skipped (and
    /// reported), not fatal.
    pub fn rules(&self) -> rusqlite::Result<Vec<Rule>> {
        let mut stmt = self.conn.prepare("SELECT id, json FROM rules")?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        let mut rules = Vec::new();
        for row in rows {
            let (id, json) = row?;
            match serde_json::from_str(&json) {
                Ok(rule) => rules.push(rule),
                Err(err) => eprintln!("proxymate: skipping unreadable rule {id}: {err}"),
            }
        }
        Ok(rules)
    }

    /// Replaces the stored rules with `rules` in one transaction.
    pub fn save_rules(&mut self, rules: &[Rule]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM rules", [])?;
        {
            let mut insert = tx.prepare("INSERT INTO rules (id, json) VALUES (?1, ?2)")?;
            for rule in rules {
                let json =
                    serde_json::to_string(rule).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
                insert.execute(params![rule.id, json])?;
            }
        }
        tx.commit()
    }

    pub fn rule_settings(&self) -> rusqlite::Result<RuleSettings> {
        self.setting(RULE_SETTINGS_KEY)
    }

    pub fn save_rule_settings(&self, settings: &RuleSettings) -> rusqlite::Result<()> {
        self.save_setting(RULE_SETTINGS_KEY, settings)
    }

    /// A settings row as `T`; the default if it is missing or unreadable.
    pub fn setting<T: DeserializeOwned + Default>(&self, key: &str) -> rusqlite::Result<T> {
        let json: Option<String> = self
            .conn
            .query_row("SELECT json FROM settings WHERE key = ?1", params![key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
    }

    pub fn save_setting<T: Serialize>(&self, key: &str, value: &T) -> rusqlite::Result<()> {
        let json = serde_json::to_string(value).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
        self.conn.execute(
            "INSERT INTO settings (key, json) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET json = excluded.json",
            params![key, json],
        )?;
        Ok(())
    }

    /// The allow-list. Rows whose address no longer parses are skipped.
    pub fn devices(&self) -> rusqlite::Result<Vec<SavedDevice>> {
        let mut stmt = self
            .conn
            .prepare("SELECT ip, nickname, allowed FROM devices ORDER BY ip")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        let mut devices = Vec::new();
        for row in rows {
            let (ip, nickname, allowed) = row?;
            match ip.parse() {
                Ok(ip) => devices.push(SavedDevice { ip, nickname, allowed }),
                Err(_) => eprintln!("proxymate: skipping unreadable device {ip:?}"),
            }
        }
        Ok(devices)
    }

    /// Replaces the allow-list with `devices` in one transaction.
    pub fn save_devices(&mut self, devices: &[SavedDevice]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM devices", [])?;
        {
            let mut insert = tx.prepare("INSERT INTO devices (ip, nickname, allowed) VALUES (?1, ?2, ?3)")?;
            for d in devices {
                insert.execute(params![d.ip.to_string(), d.nickname, d.allowed])?;
            }
        }
        tx.commit()
    }
}
