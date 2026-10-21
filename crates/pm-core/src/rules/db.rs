//! `app.sqlite`: the app database (WAL). It holds the rules and the rule
//! settings for now; the SSL lists stay in `ssl.json` until they move here.
//!
//! Rules are stored as JSON documents, one row each, so the schema doesn't
//! change every time an action gains a field.

use std::path::Path;

use pm_model::{Rule, RuleSettings};
use rusqlite::{Connection, OptionalExtension, params};

pub const APP_DB_FILE: &str = "app.sqlite";

const RULE_SETTINGS_KEY: &str = "rules";

pub(crate) struct AppDb {
    conn: Connection,
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
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, json TEXT NOT NULL);",
        )?;
        Ok(AppDb { conn })
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
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT json FROM settings WHERE key = ?1",
                params![RULE_SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
    }

    pub fn save_rule_settings(&self, settings: &RuleSettings) -> rusqlite::Result<()> {
        let json = serde_json::to_string(settings).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
        self.conn.execute(
            "INSERT INTO settings (key, json) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET json = excluded.json",
            params![RULE_SETTINGS_KEY, json],
        )?;
        Ok(())
    }
}
