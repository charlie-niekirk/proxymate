//! Scripts (M14): JavaScript on an embedded QuickJS, with no network, file
//! access, `require` or timers. `proxymate.d.ts` is the API's source of
//! truth; `prelude.js` and `native.rs` implement it.
//!
//! - Rewrite and mock scripts run from a rule's `Action::Script`, in the
//!   order of `rules::eval`. A throw or timeout skips that action, records
//!   a [`ScriptIssue`] on the flow, and the flow goes on unmodified by it.
//! - View scripts run when the inspector shows a matching body
//!   ([`Engine::run_view_scripts`]), on the live capture, saved sessions
//!   and HARs alike. Results are cached for finished flows.
//!
//! Scripts live in the `scripts` table of `app.sqlite` (JSON documents,
//! like the rules). Each is evaluated once per worker and cached by source
//! hash; saving a script (a new hash) gives it a fresh `context.shared`.

mod dry_run;
mod file;
mod host;
mod native;
#[cfg(test)]
mod tests;
pub(crate) mod traffic;
mod views;
mod worker;

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};
use pm_model::{
    Action, ConsoleEntry, ConsoleLevel, FlowId, Phase, Rule, RuleOrigin, Script, ScriptId, ScriptKind, ScriptSettings,
    ViewConfig,
};

pub(crate) use host::{Call, CallInfo, Code, Exports, JsRequest, JsResponse, Output, RunResult, ScriptHost};
pub(crate) use traffic::{MockAnswer, ScriptRun};

use crate::notify::Notifier;
use crate::rules::compile::CompiledRule;
use crate::rules::db::AppDb;
use crate::{Engine, Error};

/// The script API's type definitions, for editors and agents.
pub const TYPE_DEFINITIONS: &str = include_str!("proxymate.d.ts");
/// The globals scripts see, one per line; checked against the runtime and
/// against `proxymate.d.ts` (`scripts/api_surface.txt`).
pub const API_SURFACE: &str = include_str!("../../../../scripts/api_surface.txt");

/// Bodies larger than this reach scripts as `null` with `bodyTooLarge`.
pub const SCRIPT_BODY_MAX: usize = 16 * 1024 * 1024;
/// Longest script source accepted.
pub const SOURCE_MAX: usize = 1024 * 1024;
/// Console entries kept.
const CONSOLE_MAX: usize = 2000;
const SETTINGS_KEY: &str = "scripts";

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("script {name:?}: {message}")]
    Invalid { name: String, message: String },
    #[error("no script with id {0}")]
    Unknown(ScriptId),
    #[error("script {script:?} is used by {}: remove it from {} first", rules_label(rules), if rules.len() == 1 { "that rule" } else { "those rules" })]
    InUse { script: String, rules: Vec<String> },
    #[error("scripts database: {0}")]
    Db(#[from] rusqlite::Error),
}

fn rules_label(rules: &[String]) -> String {
    match rules {
        [one] => format!("rule {one:?}"),
        many => format!(
            "rules {}",
            many.iter().map(|r| format!("{r:?}")).collect::<Vec<_>>().join(", ")
        ),
    }
}

fn invalid(script: &Script, message: impl Into<String>) -> ScriptError {
    ScriptError::Invalid {
        name: script.name.clone(),
        message: message.into(),
    }
}

/// A saved script, ready to run.
pub(crate) struct CompiledScript {
    pub code: Code,
    pub kind: ScriptKind,
    pub enabled: bool,
    pub exports: Exports,
}

impl CompiledScript {
    /// Runs `onMock` (an enabled mock script that exports it).
    pub fn runs_mock(&self) -> bool {
        self.enabled && self.kind == ScriptKind::Mock && self.exports.on_mock
    }

    /// Runs in `phase` of a rule: `onRequest` or `onMock` in the request
    /// phase, `onResponse` in the response phase.
    pub fn runs_in(&self, phase: Phase) -> bool {
        match phase {
            Phase::Request => {
                self.runs_mock() || (self.enabled && self.kind == ScriptKind::Rewrite && self.exports.on_request)
            }
            Phase::Response => self.enabled && self.kind == ScriptKind::Rewrite && self.exports.on_response,
        }
    }
}

/// Every saved script by id: what a rule set resolves `Action::Script`
/// against. A flow keeps the snapshot it started with.
pub(crate) type ScriptMap = HashMap<ScriptId, Arc<CompiledScript>>;

/// An enabled view script and where it applies.
pub(crate) struct ViewScript {
    pub script: Arc<CompiledScript>,
    pub matcher: CompiledRule,
    pub config: ViewConfig,
}

/// Scripts state owned by the engine.
pub(crate) struct Scripts {
    db: Mutex<AppDb>,
    /// Write-locked across a whole change: validate, persist, swap.
    list: RwLock<Vec<Script>>,
    compiled: ArcSwap<ScriptMap>,
    view_scripts: ArcSwap<Vec<Arc<ViewScript>>>,
    settings: RwLock<ScriptSettings>,
    pub host: ScriptHost,
    changes: Notifier<Vec<Script>>,
    console: Console,
    pub(crate) views: views::ViewCache,
}

/// Receives the script list whenever it changes. Returning `false`
/// unsubscribes.
pub type ScriptsSink = crate::notify::Sink<Vec<Script>>;
/// Told the newest console sequence number when entries are added (or
/// the console is cleared); fetch them with [`Engine::script_console`].
pub type ConsoleSink = crate::notify::Sink<u64>;

impl Scripts {
    /// Loads the scripts from `<dir>/app.sqlite` (in memory without a
    /// directory) and evaluates each once to learn its exports.
    pub fn load(dir: Option<&Path>) -> Scripts {
        let db = dir
            .map(AppDb::open)
            .unwrap_or_else(AppDb::in_memory)
            .or_else(|err| {
                eprintln!("proxymate: can't open the app database for scripts: {err}; scripts won't be saved");
                AppDb::in_memory()
            })
            .expect("in-memory SQLite");
        let list = db.scripts().unwrap_or_else(|err| {
            eprintln!("proxymate: can't read scripts: {err}");
            Vec::new()
        });
        let settings: ScriptSettings = db.setting(SETTINGS_KEY).unwrap_or_default();
        let scripts = Scripts {
            db: Mutex::new(db),
            list: RwLock::new(Vec::new()),
            compiled: ArcSwap::from_pointee(HashMap::new()),
            view_scripts: ArcSwap::from_pointee(Vec::new()),
            settings: RwLock::new(settings),
            host: ScriptHost::new(),
            changes: Notifier::new(),
            console: Console::new(),
            views: views::ViewCache::new(),
        };
        let mut map = ScriptMap::new();
        let mut view_scripts = Vec::new();
        for script in &list {
            // A script that no longer evaluates stays listed but does
            // nothing until it is fixed.
            let compiled = match scripts.compile(script) {
                Ok(c) => c,
                Err(err) => {
                    eprintln!("proxymate: {err}");
                    Arc::new(CompiledScript {
                        code: Code::new(script.id.clone(), script.name.clone(), script.source.as_str().into()),
                        kind: script.kind,
                        enabled: false,
                        exports: Exports::default(),
                    })
                }
            };
            if let Some(view) = view_script(script, &compiled) {
                view_scripts.push(Arc::new(view));
            }
            map.insert(script.id.clone(), compiled);
        }
        *scripts.list.write() = list;
        scripts.compiled.store(Arc::new(map));
        scripts.view_scripts.store(Arc::new(view_scripts));
        scripts
    }

    /// The saved scripts, for compiling rules.
    pub fn map(&self) -> Arc<ScriptMap> {
        self.compiled.load_full()
    }

    pub fn view_scripts(&self) -> Arc<Vec<Arc<ViewScript>>> {
        self.view_scripts.load_full()
    }

    pub fn settings(&self) -> ScriptSettings {
        *self.settings.read()
    }

    pub fn traffic_timeout(&self) -> Duration {
        Duration::from_millis(u64::from(self.settings().traffic_timeout_ms.max(1)))
    }

    pub fn view_timeout(&self) -> Duration {
        Duration::from_millis(u64::from(self.settings().view_timeout_ms.max(1)))
    }

    /// Part of the publisher tick.
    pub fn publish(&self) {
        self.changes.publish(|| sorted(self.list.read().clone()));
        self.console.notifier.publish(|| self.console.latest());
    }

    /// Records what a run logged, and its error, in the console.
    pub fn log_run(&self, code: &Code, result: &RunResult, flow_id: Option<FlowId>) {
        let error = result.output.as_ref().err().map(|e| (ConsoleLevel::Error, e.clone()));
        self.console
            .push(code, result.console.iter().cloned().chain(error), flow_id);
    }

    pub fn log_error(&self, code: &Code, message: &str, flow_id: Option<FlowId>) {
        self.console
            .push(code, [(ConsoleLevel::Error, message.to_owned())], flow_id);
    }

    /// Evaluates `script` to check it and learn its exports.
    fn compile(&self, script: &Script) -> Result<Arc<CompiledScript>, ScriptError> {
        if script.source.len() > SOURCE_MAX {
            return Err(invalid(
                script,
                format!(
                    "the source is {} KB; the limit is {} KB",
                    script.source.len() / 1024,
                    SOURCE_MAX / 1024
                ),
            ));
        }
        let code = Code::new(script.id.clone(), script.name.clone(), script.source.as_str().into());
        let result = self
            .host
            .run_blocking(code.clone(), Call::Inspect, self.traffic_timeout());
        let exports = match result.output {
            Ok(Output::Exports(exports)) => exports,
            Ok(_) => Exports::default(),
            Err(message) => return Err(invalid(script, message)),
        };
        let ok = match script.kind {
            ScriptKind::Rewrite => exports.on_request || exports.on_response,
            ScriptKind::Mock => exports.on_mock,
            ScriptKind::View => exports.on_view,
        };
        if !ok {
            return Err(invalid(
                script,
                match script.kind {
                    ScriptKind::Rewrite => "a rewrite script must export onRequest or onResponse",
                    ScriptKind::Mock => "a mock script must export onMock",
                    ScriptKind::View => "a view script must export onView",
                },
            ));
        }
        if script.kind == ScriptKind::View {
            CompiledRule::compile(&matcher_rule(script), &ScriptMap::new())
                .map_err(|e| invalid(script, e.to_string()))?;
        }
        Ok(Arc::new(CompiledScript {
            code,
            kind: script.kind,
            enabled: script.enabled,
            exports,
        }))
    }
}

/// A view script's matcher, compiled through the rules compiler.
fn matcher_rule(script: &Script) -> Rule {
    Rule {
        id: script.id.clone(),
        name: script.name.clone(),
        enabled: true,
        order: 0,
        origin: script.origin.clone(),
        matcher: script.view.clone().unwrap_or_default().matcher,
        actions: Vec::new(),
    }
}

fn view_script(script: &Script, compiled: &Arc<CompiledScript>) -> Option<ViewScript> {
    if script.kind != ScriptKind::View || !compiled.enabled {
        return None;
    }
    let matcher = CompiledRule::compile(&matcher_rule(script), &ScriptMap::new()).ok()?;
    Some(ViewScript {
        script: compiled.clone(),
        matcher,
        config: script.view.clone().unwrap_or_default(),
    })
}

/// Grouped by kind, then by name.
fn sorted(mut scripts: Vec<Script>) -> Vec<Script> {
    let rank = |k: ScriptKind| match k {
        ScriptKind::View => 0,
        ScriptKind::Rewrite => 1,
        ScriptKind::Mock => 2,
    };
    scripts.sort_by(|a, b| {
        rank(a.kind)
            .cmp(&rank(b.kind))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    scripts
}

/// The rules whose actions name `id`.
fn rules_using<'a>(rules: &'a [Rule], id: &str) -> Vec<&'a Rule> {
    rules
        .iter()
        .filter(|r| {
            r.actions
                .iter()
                .any(|a| matches!(a, Action::Script { script_id } if script_id == id))
        })
        .collect()
}

pub(crate) fn now_ms() -> f64 {
    crate::proxy::now_ms()
}

/// The script console: the newest entries, oldest first.
struct Console {
    entries: Mutex<VecDeque<ConsoleEntry>>,
    next: AtomicU64,
    notifier: Notifier<u64>,
}

impl Console {
    fn new() -> Console {
        Console {
            entries: Mutex::new(VecDeque::new()),
            next: AtomicU64::new(1),
            notifier: Notifier::new(),
        }
    }

    fn latest(&self) -> u64 {
        self.next.load(Ordering::Acquire) - 1
    }

    fn push(&self, code: &Code, lines: impl IntoIterator<Item = (ConsoleLevel, String)>, flow_id: Option<FlowId>) {
        let mut entries = self.entries.lock();
        let mut pushed = false;
        for (level, message) in lines {
            let seq = self.next.fetch_add(1, Ordering::AcqRel);
            entries.push_back(ConsoleEntry {
                seq,
                time_ms: now_ms(),
                script_id: code.id.clone(),
                script_name: code.name.clone(),
                level,
                message,
                flow_id,
            });
            if entries.len() > CONSOLE_MAX {
                entries.pop_front();
            }
            pushed = true;
        }
        if pushed {
            self.notifier.changed();
        }
    }
}

impl Engine {
    // ---- Scripts ----

    /// Every script, grouped by kind (view, rewrite, mock), then by name.
    pub fn scripts(&self) -> Vec<Script> {
        sorted(self.scripts.list.read().clone())
    }

    pub fn script(&self, id: &str) -> Option<Script> {
        self.scripts.list.read().iter().find(|s| s.id == id).cloned()
    }

    /// Saves a new script. It gets a fresh id; the `id` passed in is
    /// ignored. The source is evaluated once to check it, which runs its
    /// top-level code (with the traffic timeout).
    pub fn create_script(&self, mut script: Script) -> Result<Script, Error> {
        script.id = uuid::Uuid::new_v4().to_string();
        script.updated_at_ms = now_ms().round();
        if script.name.trim().is_empty() {
            script.name = "Untitled script".into();
        }
        self.change_scripts(|scripts, _| {
            scripts.push(script.clone());
            Ok(script)
        })
    }

    /// Replaces the script with the same id. Its `context.shared` starts
    /// over if the source changed.
    pub fn update_script(&self, mut script: Script) -> Result<Script, Error> {
        script.updated_at_ms = now_ms().round();
        self.change_scripts(|scripts, rules| {
            let slot = scripts
                .iter_mut()
                .find(|s| s.id == script.id)
                .ok_or_else(|| ScriptError::Unknown(script.id.clone()))?;
            if slot.kind != script.kind && script.kind == ScriptKind::View {
                let users = rules_using(rules, &script.id);
                if !users.is_empty() {
                    return Err(ScriptError::InUse {
                        script: script.name.clone(),
                        rules: users.iter().map(|r| r.name.clone()).collect(),
                    }
                    .into());
                }
            }
            *slot = script.clone();
            Ok(script)
        })
    }

    pub fn set_script_enabled(&self, id: &str, enabled: bool) -> Result<Script, Error> {
        self.change_scripts(|scripts, _| {
            let script = scripts
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or_else(|| ScriptError::Unknown(id.to_owned()))?;
            script.enabled = enabled;
            script.updated_at_ms = now_ms().round();
            Ok(script.clone())
        })
    }

    /// Deletes a script. A script some rule runs can't be deleted; the
    /// error names those rules.
    pub fn delete_script(&self, id: &str) -> Result<(), Error> {
        self.change_scripts(|scripts, rules| {
            let script = scripts
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(|| ScriptError::Unknown(id.to_owned()))?;
            let users = rules_using(rules, id);
            if !users.is_empty() {
                return Err(ScriptError::InUse {
                    script: script.name.clone(),
                    rules: users.iter().map(|r| r.name.clone()).collect(),
                }
                .into());
            }
            scripts.retain(|s| s.id != id);
            Ok(())
        })
    }

    /// Deletes every script an agent created that no rule runs; returns
    /// how many.
    pub fn delete_agent_scripts(&self) -> Result<u32, Error> {
        self.change_scripts(|scripts, rules| {
            let before = scripts.len();
            scripts.retain(|s| !matches!(s.origin, RuleOrigin::Agent { .. }) || !rules_using(rules, &s.id).is_empty());
            Ok((before - scripts.len()) as u32)
        })
    }

    /// Validates, saves and swaps in a changed script list, then
    /// recompiles the rules so they run the new versions. Flows in flight
    /// keep the scripts they started with.
    fn change_scripts<T>(
        &self,
        change: impl FnOnce(&mut Vec<Script>, &[Rule]) -> Result<T, Error>,
    ) -> Result<T, Error> {
        // Rules first, then scripts: the same order `change_rules` takes.
        let rules = self.rules_write();
        let mut list = self.scripts.list.write();
        let mut next = list.clone();
        let out = change(&mut next, &rules)?;
        let previous = self.scripts.map();
        let mut map = ScriptMap::new();
        let mut view_scripts = Vec::new();
        for script in &next {
            let compiled = match (previous.get(&script.id), list.iter().find(|s| s.id == script.id)) {
                // Unchanged source and kind: keep what was learned.
                (Some(old), Some(saved)) if saved.source == script.source && saved.kind == script.kind => {
                    Arc::new(CompiledScript {
                        code: Code::new(script.id.clone(), script.name.clone(), script.source.as_str().into()),
                        kind: script.kind,
                        enabled: script.enabled,
                        exports: old.exports,
                    })
                }
                _ => self.scripts.compile(script)?,
            };
            if let Some(view) = view_script(script, &compiled) {
                view_scripts.push(Arc::new(view));
            }
            map.insert(script.id.clone(), compiled);
        }
        self.scripts.db.lock().save_scripts(&next).map_err(ScriptError::from)?;
        *list = next;
        self.scripts.compiled.store(Arc::new(map));
        self.scripts.view_scripts.store(Arc::new(view_scripts));
        self.scripts.views.clear();
        self.scripts.changes.changed();
        drop(list);
        self.recompile_rules(rules);
        Ok(out)
    }

    /// Sends the script list to `sink` on the next tick and whenever it
    /// changes.
    pub fn subscribe_scripts(&self, sink: ScriptsSink) {
        self.scripts.changes.subscribe(sink);
    }

    pub fn script_settings(&self) -> ScriptSettings {
        self.scripts.settings()
    }

    pub fn set_script_settings(&self, settings: ScriptSettings) -> Result<ScriptSettings, Error> {
        for (what, ms) in [
            ("traffic timeout", settings.traffic_timeout_ms),
            ("view timeout", settings.view_timeout_ms),
        ] {
            if !(10..=60_000).contains(&ms) {
                return Err(Error::Invalid(format!(
                    "the script {what} must be between 10 ms and 60 s, not {ms} ms"
                )));
            }
        }
        let mut current = self.scripts.settings.write();
        self.scripts
            .db
            .lock()
            .save_setting(SETTINGS_KEY, &settings)
            .map_err(ScriptError::from)?;
        *current = settings;
        Ok(settings)
    }

    /// Console entries newer than `since` (all kept ones without it),
    /// oldest first, at most `limit`, the newest ones when there are more.
    pub fn script_console(&self, since: Option<u64>, limit: u32) -> Vec<ConsoleEntry> {
        let entries = self.scripts.console.entries.lock();
        let fresh: Vec<&ConsoleEntry> = entries.iter().filter(|e| since.is_none_or(|s| e.seq > s)).collect();
        let skip = fresh.len().saturating_sub(limit as usize);
        fresh.into_iter().skip(skip).cloned().collect()
    }

    pub fn clear_script_console(&self) {
        self.scripts.console.entries.lock().clear();
        self.scripts.console.notifier.changed();
    }

    /// Tells `sink` the newest console sequence number on the next tick and
    /// whenever entries are added.
    pub fn subscribe_script_console(&self, sink: ConsoleSink) {
        self.scripts.console.notifier.subscribe(sink);
    }

    /// `proxymate.d.ts`: the script API, for editors and agents.
    pub fn script_type_definitions(&self) -> &'static str {
        TYPE_DEFINITIONS
    }

    pub(crate) fn script_map(&self) -> Arc<ScriptMap> {
        self.scripts.map()
    }

    pub(crate) fn scripts_state(&self) -> &Scripts {
        &self.scripts
    }
}
