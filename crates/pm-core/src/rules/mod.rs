//! The rules engine: persistence (`app.sqlite`), compilation into an
//! atomically swapped [`CompiledRuleSet`], evaluation ([`eval`]), Map Local
//! answers ([`local`]) and paused flows ([`breakpoints`]). The proxy side
//! (buffering, re-sending, throttling) is `proxy/exchange.rs`.
//!
//! Each flow takes a snapshot of the rule set when it starts, so editing
//! rules never changes a flow halfway through.

mod breakpoints;
pub mod compile;
pub(crate) mod db;
pub(crate) mod eval;
pub(crate) mod json_edit;
pub(crate) mod local;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};
use pm_model::{
    Action, BodyPart, BreakpointId, BreakpointInfo, BreakpointResolution, FlowId, GLOBAL_THROTTLE_RULE_ID,
    MapLocalSource, Matcher, Pattern, PresetKind, RequestHead, ResponseHead, Rule, RuleId, RuleOrigin, RuleSettings,
    RuleTestResult, StoreId,
};

pub use breakpoints::BreakpointSink;
pub(crate) use breakpoints::{BreakpointManager, Outcome};
pub use compile::{CompiledRuleSet, PATTERN_MAX_LEN, REGEX_SIZE_LIMIT};
pub use db::APP_DB_FILE;

use crate::notify::Notifier;
use crate::scripts::{ScriptMap, ScriptRun};
use crate::{Engine, Error};
use compile::CompiledRule;
use db::AppDb;
use eval::{RequestDraft, ResponseDraft};

/// Bodies are buffered up to this size for body actions and breakpoints;
/// larger ones are forwarded untouched and the flow is marked
/// `rewrite_skipped`.
pub const REWRITE_BODY_CAP: usize = 32 * 1024 * 1024;
/// A paused body up to this size can be edited; above it only the head.
pub const BREAKPOINT_EDIT_CAP: usize = 16 * 1024 * 1024;
/// Dry-run bodies are shown up to this size unless the caller asks for
/// more.
const TEST_BODY_MAX: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RuleError {
    #[error("{}{field}: {message}", rule_label(rule))]
    Invalid {
        rule: String,
        field: String,
        message: String,
    },
    #[error("no rule with id {0}")]
    Unknown(RuleId),
    #[error("the built-in rule {0} can't be deleted")]
    Builtin(RuleId),
    #[error("breakpoint {0} was already resolved")]
    AlreadyResolved(BreakpointId),
    #[error("no breakpoint {0}")]
    UnknownBreakpoint(BreakpointId),
    #[error("rules database: {0}")]
    Db(#[from] rusqlite::Error),
}

fn rule_label(rule: &str) -> String {
    if rule.is_empty() {
        String::new()
    } else {
        format!("rule {rule:?}, ")
    }
}

impl RuleError {
    pub(crate) fn invalid(rule: &Rule, field: &str, message: impl Into<String>) -> RuleError {
        RuleError::Invalid {
            rule: rule.name.clone(),
            field: field.to_owned(),
            message: message.into(),
        }
    }
}

/// Rules state owned by the engine.
pub(crate) struct Rules {
    db: Mutex<AppDb>,
    /// Write-locked across a whole change: validate, persist, swap.
    list: RwLock<Vec<Rule>>,
    compiled: ArcSwap<CompiledRuleSet>,
    settings: RwLock<RuleSettings>,
    pub breakpoints: BreakpointManager,
    /// The Rules Editor and MCP sessions, told when anyone (the UI or an
    /// agent) changes the list.
    changes: Notifier<Vec<Rule>>,
}

/// Receives the rule list (in evaluation order) whenever it changes.
/// Returning `false` unsubscribes.
pub type RulesSink = crate::notify::Sink<Vec<Rule>>;

impl Rules {
    /// The app database, for settings that aren't the rules' (`updates.rs`).
    pub(crate) fn db(&self) -> parking_lot::MutexGuard<'_, AppDb> {
        self.db.lock()
    }

    /// Loads `<dir>/app.sqlite`, or an in-memory database without a
    /// directory. A database that can't be opened falls back to memory.
    pub fn load(dir: Option<&Path>, scripts: &ScriptMap) -> Rules {
        let db = dir
            .map(AppDb::open)
            .unwrap_or_else(AppDb::in_memory)
            .or_else(|err| {
                eprintln!("proxymate: can't open the app database: {err}; rules won't be saved");
                AppDb::in_memory()
            })
            .expect("in-memory SQLite");
        let mut rules = db.rules().unwrap_or_else(|err| {
            eprintln!("proxymate: can't read rules: {err}");
            Vec::new()
        });
        if !rules.iter().any(|r| r.id == GLOBAL_THROTTLE_RULE_ID) {
            rules.push(global_throttle_rule());
        }
        // One broken rule must not take the others down: drop it from the
        // compiled set (it stays listed, disabled).
        let compiled = CompiledRuleSet::compile(&rules, scripts).unwrap_or_else(|err| {
            eprintln!("proxymate: {err}; disabling rules that don't compile");
            for rule in &mut rules {
                if CompiledRule::compile(rule, scripts).is_err() {
                    rule.enabled = false;
                }
            }
            CompiledRuleSet::compile(&rules, scripts).unwrap_or_else(|_| CompiledRuleSet::empty())
        });
        let settings = db.rule_settings().unwrap_or_default();
        Rules {
            breakpoints: BreakpointManager::new(settings.breakpoint_timeout_secs),
            db: Mutex::new(db),
            list: RwLock::new(rules),
            compiled: ArcSwap::from_pointee(compiled),
            settings: RwLock::new(settings),
            changes: Notifier::new(),
        }
    }

    /// Part of the publisher tick.
    pub fn publish(&self) {
        self.changes.publish(|| sorted(self.list.read().clone()));
    }
}

/// The toolbar's network conditions: one rule matching everything,
/// disabled until a profile is chosen.
fn global_throttle_rule() -> Rule {
    Rule {
        id: GLOBAL_THROTTLE_RULE_ID.into(),
        name: "Network conditions".into(),
        enabled: false,
        order: i64::MIN / 2,
        origin: RuleOrigin::Preset {
            preset: PresetKind::Throttle,
        },
        matcher: Matcher::default(),
        actions: vec![Action::Throttle {
            download_kbps: 780,
            upload_kbps: 330,
            latency_ms: 100,
        }],
    }
}

fn script_note(issue: &pm_model::ScriptIssue) -> String {
    format!("Script {:?} failed: {}", issue.script_name, issue.message)
}

fn sorted(mut rules: Vec<Rule>) -> Vec<Rule> {
    rules.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    rules
}

impl Engine {
    // ---- Rules ----

    /// Every rule, in evaluation order.
    pub fn rules(&self) -> Vec<Rule> {
        sorted(self.rules.list.read().clone())
    }

    pub fn rule(&self, id: &str) -> Option<Rule> {
        self.rules.list.read().iter().find(|r| r.id == id).cloned()
    }

    /// Checks that `rule` compiles, without saving it.
    pub fn validate_rule(&self, rule: &Rule) -> Result<(), Error> {
        CompiledRule::compile(rule, &self.script_map())?;
        Ok(())
    }

    /// Saves a new rule at the end of the list. It gets a fresh id; the
    /// `id` and `order` passed in are ignored.
    pub fn create_rule(&self, mut rule: Rule) -> Result<Rule, Error> {
        rule.id = uuid::Uuid::new_v4().to_string();
        self.change_rules(|rules| {
            rule.order = rules.iter().map(|r| r.order).max().unwrap_or(0).max(0) + 1;
            rules.push(rule.clone());
            Ok(rule)
        })
    }

    /// Replaces the rule with the same id.
    pub fn update_rule(&self, rule: Rule) -> Result<Rule, Error> {
        self.change_rules(|rules| {
            let slot = rules
                .iter_mut()
                .find(|r| r.id == rule.id)
                .ok_or_else(|| RuleError::Unknown(rule.id.clone()))?;
            *slot = rule.clone();
            Ok(rule)
        })
    }

    pub fn set_rule_enabled(&self, id: &str, enabled: bool) -> Result<Rule, Error> {
        self.change_rules(|rules| {
            let rule = rules
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or_else(|| RuleError::Unknown(id.to_owned()))?;
            rule.enabled = enabled;
            Ok(rule.clone())
        })
    }

    pub fn delete_rule(&self, id: &str) -> Result<(), Error> {
        if id.starts_with("builtin-") {
            return Err(RuleError::Builtin(id.to_owned()).into());
        }
        self.change_rules(|rules| {
            let before = rules.len();
            rules.retain(|r| r.id != id);
            if rules.len() == before {
                return Err(RuleError::Unknown(id.to_owned()).into());
            }
            Ok(())
        })
    }

    /// Deletes every rule an agent created; returns how many.
    pub fn delete_agent_rules(&self) -> Result<u32, Error> {
        self.change_rules(|rules| {
            let before = rules.len();
            rules.retain(|r| !matches!(r.origin, RuleOrigin::Agent { .. }));
            Ok((before - rules.len()) as u32)
        })
    }

    /// Puts the listed rules first, in that order; the others follow in
    /// their current order. The built-in rules stay ahead of everything.
    pub fn reorder_rules(&self, ids: &[RuleId]) -> Result<Vec<Rule>, Error> {
        self.change_rules(|rules| {
            let current = sorted(std::mem::take(rules));
            let (builtin, user): (Vec<Rule>, Vec<Rule>) =
                current.into_iter().partition(|r| r.id.starts_with("builtin-"));
            let mut ordered: Vec<Rule> = ids
                .iter()
                .filter_map(|id| user.iter().find(|r| &r.id == id).cloned())
                .collect();
            ordered.extend(user.into_iter().filter(|r| !ids.contains(&r.id)));
            for (i, rule) in ordered.iter_mut().enumerate() {
                rule.order = i as i64 + 1;
            }
            *rules = builtin.into_iter().chain(ordered).collect();
            Ok(sorted(rules.clone()))
        })
    }

    pub fn rule_settings(&self) -> RuleSettings {
        self.rules.settings.read().clone()
    }

    pub fn set_rule_settings(&self, settings: RuleSettings) -> Result<RuleSettings, Error> {
        let mut current = self.rules.settings.write();
        self.rules
            .db
            .lock()
            .save_rule_settings(&settings)
            .map_err(RuleError::from)?;
        self.rules
            .breakpoints
            .set_timeout_secs(settings.breakpoint_timeout_secs);
        *current = settings.clone();
        Ok(settings)
    }

    /// Validates, saves and swaps in a changed rule list. New flows use it
    /// straight away; flows in flight keep the set they started with.
    fn change_rules<T>(&self, change: impl FnOnce(&mut Vec<Rule>) -> Result<T, Error>) -> Result<T, Error> {
        let mut list = self.rules.list.write();
        let mut next = list.clone();
        let out = change(&mut next)?;
        let compiled = CompiledRuleSet::compile(&next, &self.script_map())?;
        self.rules.db.lock().save_rules(&next).map_err(RuleError::from)?;
        *list = next;
        self.rules.compiled.store(Arc::new(compiled));
        self.rules.changes.changed();
        Ok(out)
    }

    /// The rule list, write-locked, for a script change (which must check
    /// and then recompile the rules).
    pub(crate) fn rules_write(&self) -> parking_lot::RwLockWriteGuard<'_, Vec<Rule>> {
        self.rules.list.write()
    }

    /// Recompiles the rules against the current scripts, so they run the
    /// new versions.
    pub(crate) fn recompile_rules(&self, list: parking_lot::RwLockWriteGuard<'_, Vec<Rule>>) {
        match CompiledRuleSet::compile(&list, &self.script_map()) {
            Ok(compiled) => self.rules.compiled.store(Arc::new(compiled)),
            Err(err) => eprintln!("proxymate: rules no longer compile after a script change: {err}"),
        }
    }

    /// Sends the rule list to `sink` on the next tick and whenever it
    /// changes, so an open Rules Editor follows edits made by agents.
    pub fn subscribe_rules(&self, sink: RulesSink) {
        self.rules.changes.subscribe(sink);
    }

    /// The rule set a new flow evaluates against.
    pub(crate) fn rule_set(&self) -> Arc<CompiledRuleSet> {
        self.rules.compiled.load_full()
    }

    /// A new, unsaved rule pre-filled from a live flow: it matches the
    /// flow's method, host and path. The Rules Editor (or an agent) then
    /// edits and saves it.
    pub fn draft_rule(&self, flow: FlowId, preset: PresetKind) -> Result<Rule, Error> {
        self.draft_rule_in(pm_model::LIVE_STORE, flow, preset)
    }

    /// [`Engine::draft_rule`] from a flow in any store (a saved session or
    /// HAR too). The Rewrite preset has no actions: the Rewrite dialog
    /// fills them in from the user's edits.
    pub fn draft_rule_in(&self, store: StoreId, flow: FlowId, preset: PresetKind) -> Result<Rule, Error> {
        let summary = self.get_flow_in(store, flow).ok_or(Error::UnknownFlow(flow))?;
        let detail = self.flow_detail_in(store, flow)?.ok_or(Error::UnknownFlow(flow))?;
        let matcher = Matcher {
            methods: vec![summary.method.to_string()],
            host: Some(Pattern::exact(summary.host.to_string())),
            path: Some(Pattern::exact(summary.path.clone())),
            ..Matcher::default()
        };
        let target = format!("{} {}{}", summary.method, summary.host, summary.path);
        let response_type = detail
            .response
            .as_ref()
            .and_then(|r| r.headers.iter().find(|h| h.name.eq_ignore_ascii_case("content-type")))
            .map(|h| h.value.clone());
        let (name, actions) = match preset {
            PresetKind::MapLocal => {
                let body = self
                    .body_head_in(store, flow, BodyPart::Response, true, Some(TEST_BODY_MAX * 4))
                    .ok()
                    .filter(|b| !b.cut)
                    .and_then(|b| String::from_utf8(b.bytes).ok())
                    .unwrap_or_default();
                (
                    format!("Map Local · {target}"),
                    vec![Action::MapLocal {
                        source: MapLocalSource::Inline { body },
                        status: summary.status.unwrap_or(200),
                        headers: Vec::new(),
                        content_type: response_type,
                    }],
                )
            }
            PresetKind::Breakpoint => (
                format!("Breakpoint · {target}"),
                vec![Action::Breakpoint {
                    request: true,
                    response: false,
                }],
            ),
            PresetKind::Rewrite => (format!("Rewrite · {target}"), Vec::new()),
            PresetKind::MapRemote => (
                format!("Map Remote · {target}"),
                vec![Action::MapRemote {
                    scheme: None,
                    host: Some(summary.host.to_string()),
                    port: None,
                    path: None,
                    preserve_host: false,
                }],
            ),
            PresetKind::Block => (
                format!("Block · {target}"),
                vec![Action::Block {
                    mode: pm_model::BlockMode::Status { status: 403 },
                }],
            ),
            PresetKind::Throttle => (
                format!("Throttle · {target}"),
                vec![Action::Throttle {
                    download_kbps: 780,
                    upload_kbps: 330,
                    latency_ms: 100,
                }],
            ),
        };
        Ok(Rule {
            id: String::new(),
            name,
            enabled: true,
            order: 0,
            origin: RuleOrigin::Preset { preset },
            matcher,
            actions,
        })
    }

    /// Dry run: what `rule` (saved or not, enabled or not) would do to a
    /// live flow, computed with the proxy's own evaluator. Nothing is
    /// sent.
    pub async fn test_rule(&self, rule: &Rule, flow: FlowId) -> Result<RuleTestResult, Error> {
        self.test_rule_in(pm_model::LIVE_STORE, rule, flow, None).await
    }

    /// [`Engine::test_rule`] on a flow in any store. The bodies in the
    /// result are cut at `body_limit` bytes (default 256 KB); the rule
    /// itself runs on whole bodies, up to the proxy's buffering cap.
    pub async fn test_rule_in(
        &self,
        store: StoreId,
        rule: &Rule,
        flow: FlowId,
        body_limit: Option<usize>,
    ) -> Result<RuleTestResult, Error> {
        let summary = self.get_flow_in(store, flow).ok_or(Error::UnknownFlow(flow))?;
        let detail = self.flow_detail_in(store, flow)?.ok_or(Error::UnknownFlow(flow))?;
        let compiled = Arc::new(CompiledRule::compile(rule, &self.script_map())?);
        // The console names live flows only: a document's ids are its own.
        let run = ScriptRun::new(self.scripts_state(), (store == pm_model::LIVE_STORE).then_some(flow));
        let limit = body_limit.unwrap_or(TEST_BODY_MAX);
        let body = |part| {
            self.body_head_in(store, flow, part, true, Some(REWRITE_BODY_CAP))
                .ok()
                .filter(|b| !b.cut)
                .map(|b| b.bytes)
        };
        let mut request = RequestDraft {
            method: detail.request.method.clone(),
            scheme: summary.scheme,
            host: summary.host.to_string(),
            port: summary.port,
            path: summary.path.clone(),
            query: summary.query.clone(),
            headers: eval::header_map(&detail.request.headers),
            body: body(BodyPart::Request),
        };
        let mismatch = compiled.mismatch(&request.info(summary.client.as_deref()), true);
        let mut notes = Vec::new();
        let mut response = detail.response.as_ref().map(|r| ResponseDraft {
            status: r.status,
            headers: eval::header_map(&r.headers),
            body: body(BodyPart::Response),
        });
        let mut response_matched = None;
        if mismatch.is_none() {
            let rules = [compiled.clone()];
            let fx = eval::request_phase(&rules, &mut request, run).await;
            notes.extend(fx.body_skipped.iter().map(|s| format!("Body action skipped: {s}")));
            notes.extend(fx.script_issues.iter().map(script_note));
            if let Some(mode) = fx.block {
                notes.push(match mode {
                    pm_model::BlockMode::Status { status } => format!("Blocked with status {status}."),
                    pm_model::BlockMode::Reset => "Blocked: the connection is reset.".into(),
                });
                response = None;
            } else {
                if let Some(local) = &fx.map_local {
                    notes.push("Answered by Map Local; the upstream isn't contacted.".into());
                    response = Some(local::answer(local, &request.path).await);
                } else if let Some(mock) = &fx.mock {
                    notes.push("Answered by a mock script; the upstream isn't contacted.".into());
                    response = Some(mock.clone());
                } else if let Some(remap) = &fx.map_remote {
                    let path = remap.path.as_deref().unwrap_or(&request.path);
                    notes.push(format!(
                        "Sent to {}://{}:{}{path} instead.",
                        remap.scheme.as_str(),
                        remap.host,
                        remap.port
                    ));
                }
                if fx.breakpoint.is_some() {
                    notes.push("Pauses at a request breakpoint.".into());
                }
                if fx.delay_ms > 0 {
                    notes.push(format!("Delays the request by {} ms.", fx.delay_ms));
                }
                if let Some(t) = fx.throttle {
                    notes.push(format!(
                        "Throttled to {} kbps down, {} kbps up, {} ms latency.",
                        t.down_bps * 8 / 1000,
                        t.up_bps * 8 / 1000,
                        t.latency_ms
                    ));
                }
                if let Some(draft) = &mut response {
                    let gated = eval::response_rules(&rules, draft.status, &draft.headers);
                    response_matched = Some(!gated.is_empty());
                    let sent =
                        eval::needs_request_for_response(&gated).then(|| crate::scripts::traffic::js_request(&request));
                    let fx = eval::response_phase(&gated, draft, run, sent.as_ref()).await;
                    notes.extend(fx.body_skipped.iter().map(|s| format!("Body action skipped: {s}")));
                    notes.extend(fx.script_issues.iter().map(script_note));
                    if fx.breakpoint.is_some() {
                        notes.push("Pauses at a response breakpoint.".into());
                    }
                    if fx.delay_ms > 0 {
                        notes.push(format!("Delays the response by {} ms.", fx.delay_ms));
                    }
                }
            }
        }
        let text = |b: &Option<Vec<u8>>| {
            b.as_ref().map(|b| {
                let cut = &b[..b.len().min(limit)];
                String::from_utf8_lossy(cut).into_owned()
            })
        };
        let cut = |b: Option<&Vec<u8>>| b.is_some_and(|b| b.len() > limit);
        let version = detail.request.version;
        Ok(RuleTestResult {
            matched: mismatch.is_none(),
            mismatch,
            response_matched,
            request: RequestHead {
                method: request.method.clone(),
                url: request.url(),
                version,
                headers: crate::proxy::headers_of(&request.headers),
            },
            request_body: text(&request.body),
            response: response.as_ref().map(|r| ResponseHead {
                status: r.status,
                reason: http::StatusCode::from_u16(r.status)
                    .ok()
                    .and_then(|s| s.canonical_reason())
                    .unwrap_or_default()
                    .to_owned(),
                version,
                headers: crate::proxy::headers_of(&r.headers),
            }),
            response_body: response.as_ref().and_then(|r| text(&r.body)),
            notes,
            request_body_truncated: cut(request.body.as_ref()),
            response_body_truncated: cut(response.as_ref().and_then(|r| r.body.as_ref())),
        })
    }

    /// Up to `limit` flows of `store` (newest first) that `rule`'s matcher
    /// matches, for reopening a saved rule on a flow it applies to.
    pub fn flows_matching_rule(&self, store: StoreId, rule: &Rule, limit: usize) -> Result<Vec<FlowId>, Error> {
        let scripts = self.script_map();
        let compiled = CompiledRule::compile(rule, &scripts)?;
        // Header and response conditions need each flow's detail: narrow by
        // the summary first, under the lock, then check those outside it.
        let summary_only = CompiledRule::compile(
            &Rule {
                matcher: Matcher {
                    headers: Vec::new(),
                    response: None,
                    ..rule.matcher.clone()
                },
                actions: Vec::new(),
                ..rule.clone()
            },
            &scripts,
        )?;
        let needs_detail = !rule.matcher.headers.is_empty() || rule.matcher.response.is_some();
        let no_headers = http::HeaderMap::new();
        let candidates: Vec<FlowId> = self.read_store(store, |s| {
            s.flows()
                .iter()
                .rev()
                .filter(|f| {
                    &*f.method != "CONNECT" && !matches!(f.scheme, pm_model::Scheme::Ws | pm_model::Scheme::Wss)
                })
                .filter(|f| {
                    let info = compile::RequestInfo {
                        method: &f.method,
                        scheme: f.scheme,
                        host: &f.host,
                        port: f.port,
                        path: &f.path,
                        query: &f.query,
                        headers: &no_headers,
                        client_app: f.client.as_deref(),
                    };
                    summary_only.mismatch(&info, true).is_none()
                })
                .map(|f| f.id)
                .take(if needs_detail { usize::MAX } else { limit })
                .collect()
        })?;
        if !needs_detail {
            return Ok(candidates);
        }
        let mut out = Vec::new();
        for id in candidates {
            if out.len() == limit {
                break;
            }
            let (Some(f), Ok(Some(detail))) = (self.get_flow_in(store, id), self.flow_detail_in(store, id)) else {
                continue;
            };
            let headers = eval::header_map(&detail.request.headers);
            let info = compile::RequestInfo {
                method: &f.method,
                scheme: f.scheme,
                host: &f.host,
                port: f.port,
                path: &f.path,
                query: &f.query,
                headers: &headers,
                client_app: f.client.as_deref(),
            };
            if compiled.mismatch(&info, true).is_some() {
                continue;
            }
            let response_ok = match (&compiled.response, &detail.response) {
                (Some(m), Some(r)) => m.matches(r.status, &eval::header_map(&r.headers)),
                _ => true,
            };
            if response_ok {
                out.push(id);
            }
        }
        Ok(out)
    }

    // ---- Breakpoints ----

    /// Flows paused at a breakpoint, oldest first.
    pub fn breakpoints(&self) -> Vec<BreakpointInfo> {
        self.rules.breakpoints.list()
    }

    pub fn breakpoint(&self, id: BreakpointId) -> Option<BreakpointInfo> {
        self.rules.breakpoints.get(id)
    }

    /// The paused message's body, decoded.
    pub fn breakpoint_body(&self, id: BreakpointId) -> Result<Vec<u8>, Error> {
        self.rules
            .breakpoints
            .body(id)
            .map(|b| b.as_ref().clone())
            .ok_or_else(|| RuleError::UnknownBreakpoint(id).into())
    }

    /// Lets a paused flow go on. The first resolution wins; later ones get
    /// an "already resolved" error.
    pub fn resolve_breakpoint(&self, id: BreakpointId, resolution: BreakpointResolution) -> Result<(), Error> {
        Ok(self.rules.breakpoints.resolve(id, resolution)?)
    }

    /// Sends the breakpoint queue to `sink` on the next tick and whenever it
    /// changes. Every subscriber (the UI, each MCP session) gets its own
    /// pushes.
    pub fn subscribe_breakpoints(&self, sink: BreakpointSink) {
        self.rules.breakpoints.subscribe(sink);
    }

    pub(crate) fn breakpoint_manager(&self) -> &BreakpointManager {
        &self.rules.breakpoints
    }
}
