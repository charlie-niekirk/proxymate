//! Rule evaluation, as documented in `docs/PLAN.md` ("Rules engine"):
//!
//! **Request phase.** Every matching rule runs, in `order` (not first
//! match). A `Block` anywhere stops the flow before anything else applies.
//! Header, query and body rewrites apply in sequence. The first `MapLocal`
//! wins and skips the upstream; otherwise the last `MapRemote` wins. Delays
//! add up and the last `Throttle` wins. The request breakpoint fires after
//! all of that (the caller pauses on [`RequestEffects::breakpoint`]), so the
//! user edits exactly what will be sent.
//!
//! **Response phase.** Only rules whose response matcher (if any) matches
//! the response as it arrived take part. Rewrites run in order, then the
//! response breakpoint.
//!
//! **Scripts (M14).** A `Script` action runs in place, in rule and action
//! order, so it sees the edits of the actions before it: a rewrite
//! script's `onRequest` or `onResponse`, or a mock script's `onMock`. A
//! mock answer is terminal like Map Local: the first answer (Map Local or
//! mock) wins and later script actions in the request phase don't run.
//! `Block` still runs first and the breakpoints last. A script that throws
//! or times out is skipped and reported in the effects' `script_issues`.
//!
//! Everything here works on drafts: the proxy fills a draft from the wire
//! (buffering and decoding the body first when a body action, script or
//! breakpoint needs it), evaluates, and sends what's left. The phases are
//! async only for scripts; a rule set without scripts never awaits.
//! [`super::Engine::test_rule`] runs the same code on a recorded flow.

use std::sync::Arc;

use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use pm_model::{Action, AppliedRule, BlockMode, MapLocalSource, Phase, Scheme, ScriptIssue, ScriptPhase};

use super::compile::{CompiledAction, CompiledRule, RequestInfo, percent_encode, query_pairs};
use super::json_edit::{self, Edit};
use crate::scripts::traffic::{apply_request, apply_response, mock_answer};
use crate::scripts::{JsRequest, MockAnswer, ScriptRun};

/// A request as rules see and change it.
#[derive(Debug, Clone)]
pub(crate) struct RequestDraft {
    pub method: String,
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    pub path: String,
    /// Raw, without the `?`.
    pub query: String,
    pub headers: HeaderMap,
    /// The decoded body, when it was buffered.
    pub body: Option<Vec<u8>>,
}

impl RequestDraft {
    pub fn info<'a>(&'a self, client_app: Option<&'a str>) -> RequestInfo<'a> {
        RequestInfo {
            method: &self.method,
            scheme: self.scheme,
            host: &self.host,
            port: self.port,
            path: &self.path,
            query: &self.query,
            headers: &self.headers,
            client_app,
        }
    }

    /// `path?query`.
    pub fn path_and_query(&self) -> String {
        if self.query.is_empty() {
            self.path.clone()
        } else {
            format!("{}?{}", self.path, self.query)
        }
    }

    /// The authority, without the default port.
    pub fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let default = match self.scheme {
            Scheme::Http | Scheme::Ws => 80,
            Scheme::Https | Scheme::Wss => 443,
        };
        if self.port == default {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    pub fn url(&self) -> String {
        format!(
            "{}://{}{}",
            self.scheme.as_str(),
            self.authority(),
            self.path_and_query()
        )
    }
}

/// A response as rules see and change it.
#[derive(Debug, Clone)]
pub(crate) struct ResponseDraft {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Option<Vec<u8>>,
}

/// Where a `MapRemote` sends the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Remap {
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    pub path: Option<String>,
    pub preserve_host: bool,
}

/// A `MapLocal` to serve.
#[derive(Debug, Clone)]
pub(crate) struct LocalAnswer {
    pub source: MapLocalSource,
    pub status: u16,
    pub headers: Vec<pm_model::Header>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ThrottleSpec {
    /// Bytes per second, 0 for unlimited.
    pub down_bps: u64,
    pub up_bps: u64,
    pub latency_ms: u32,
}

/// What the request phase decided, beyond the edits made to the draft.
#[derive(Default)]
pub(crate) struct RequestEffects {
    pub block: Option<BlockMode>,
    pub map_local: Option<LocalAnswer>,
    /// A mock script's answer.
    pub mock: Option<MockAnswer>,
    pub map_remote: Option<Remap>,
    pub delay_ms: u64,
    pub throttle: Option<ThrottleSpec>,
    /// The rule whose request breakpoint fires.
    pub breakpoint: Option<Arc<CompiledRule>>,
    /// The head (method, URL, headers) changed.
    pub head_changed: bool,
    pub body_changed: bool,
    /// Body actions that couldn't run (no buffered body, not JSON…).
    pub body_skipped: Vec<String>,
    pub applied: Vec<AppliedRule>,
    /// Scripts that threw, timed out or returned something unusable.
    pub script_issues: Vec<ScriptIssue>,
}

#[derive(Default)]
pub(crate) struct ResponseEffects {
    pub delay_ms: u64,
    pub breakpoint: Option<Arc<CompiledRule>>,
    pub head_changed: bool,
    pub body_changed: bool,
    pub body_skipped: Vec<String>,
    pub applied: Vec<AppliedRule>,
    pub script_issues: Vec<ScriptIssue>,
}

/// True if a script in `rules` runs `onResponse`, so the response phase
/// needs the request as it was sent.
pub(crate) fn needs_request_for_response(rules: &[Arc<CompiledRule>]) -> bool {
    rules.iter().any(|r| {
        r.actions
            .iter()
            .any(|a| a.script.as_ref().is_some_and(|s| s.runs_in(Phase::Response)))
    })
}

/// True if any rule edits or pauses the body in `phase`, so it must be
/// buffered.
pub(crate) fn needs_body(rules: &[Arc<CompiledRule>], phase: Phase) -> bool {
    rules.iter().any(|r| r.touches_body(phase))
}

/// Like [`needs_body`], ignoring scripts: an event stream is never
/// buffered for a script.
pub(crate) fn needs_body_without_scripts(rules: &[Arc<CompiledRule>], phase: Phase) -> bool {
    rules.iter().any(|r| {
        r.actions.iter().any(|a| match &a.action {
            Action::ReplaceBody { phase: p, .. }
            | Action::SetBody { phase: p, .. }
            | Action::JsonSet { phase: p, .. }
            | Action::JsonRemove { phase: p, .. }
            | Action::FormSet { phase: p, .. }
            | Action::FormRemove { phase: p, .. } => *p == phase,
            Action::Breakpoint { request, response } => match phase {
                Phase::Request => *request,
                Phase::Response => *response,
            },
            _ => false,
        })
    })
}

/// Runs the request phase of `rules` (the matching ones, in order).
pub(crate) async fn request_phase(
    rules: &[Arc<CompiledRule>],
    draft: &mut RequestDraft,
    run: ScriptRun<'_>,
) -> RequestEffects {
    let mut fx = RequestEffects::default();
    // A block stops the flow before any rewrite.
    for rule in rules {
        if let Some(mode) = rule.actions.iter().find_map(|a| match &a.action {
            Action::Block { mode } => Some(*mode),
            _ => None,
        }) {
            fx.block = Some(mode);
            fx.applied.push(rule.applied(Phase::Request));
            return fx;
        }
    }
    for rule in rules {
        let mut applied = false;
        let mut actions = rule.actions.as_slice();
        while let Some(ca) = actions.first() {
            if let Some(n) = json_run(actions, Phase::Request) {
                applied |= edit_json(
                    rule,
                    &actions[..n],
                    &mut draft.body,
                    &mut fx.body_changed,
                    &mut fx.body_skipped,
                );
                actions = &actions[n..];
                continue;
            }
            actions = &actions[1..];
            applied |= match &ca.action {
                Action::SetHeader {
                    phase: Phase::Request, ..
                }
                | Action::AddHeader {
                    phase: Phase::Request, ..
                }
                | Action::RemoveHeader {
                    phase: Phase::Request, ..
                }
                | Action::ReplaceHeaderValue {
                    phase: Phase::Request, ..
                } => {
                    let changed = edit_headers(ca, &mut draft.headers);
                    fx.head_changed |= changed;
                    changed
                }
                Action::SetQuery { name, value } => {
                    draft.query = set_query(&draft.query, name, Some(value));
                    fx.head_changed = true;
                    true
                }
                Action::RemoveQuery { name } => {
                    let query = set_query(&draft.query, name, None);
                    let changed = query != draft.query;
                    draft.query = query;
                    fx.head_changed |= changed;
                    changed
                }
                Action::ReplaceBody {
                    phase: Phase::Request, ..
                }
                | Action::SetBody {
                    phase: Phase::Request, ..
                }
                | Action::FormSet {
                    phase: Phase::Request, ..
                }
                | Action::FormRemove {
                    phase: Phase::Request, ..
                } => match edit_body(ca, &mut draft.body, &mut draft.headers) {
                    Ok(changed) => {
                        fx.body_changed |= changed;
                        changed
                    }
                    Err(reason) => {
                        fx.body_skipped.push(format!("{}: {reason}", rule.applied.name));
                        false
                    }
                },
                Action::MapLocal {
                    source,
                    status,
                    headers,
                    content_type,
                } => {
                    if fx.map_local.is_none() && fx.mock.is_none() {
                        fx.map_local = Some(LocalAnswer {
                            source: source.clone(),
                            status: *status,
                            headers: headers.clone(),
                            content_type: content_type.clone(),
                        });
                    }
                    true
                }
                Action::MapRemote {
                    scheme,
                    host,
                    port,
                    path,
                    preserve_host,
                } => {
                    let scheme = scheme.unwrap_or(draft.scheme);
                    let default_port = if scheme.is_tls() { 443 } else { 80 };
                    // A new scheme or host without a port gets that scheme's default.
                    let port = port.unwrap_or(if host.is_some() || scheme != draft.scheme {
                        default_port
                    } else {
                        draft.port
                    });
                    fx.map_remote = Some(Remap {
                        scheme,
                        host: host.clone().unwrap_or_else(|| draft.host.clone()),
                        port,
                        path: path.clone(),
                        preserve_host: *preserve_host,
                    });
                    true
                }
                Action::Breakpoint { request: true, .. } => {
                    fx.breakpoint.get_or_insert_with(|| rule.clone());
                    true
                }
                Action::Delay {
                    phase: Phase::Request,
                    ms,
                } => {
                    fx.delay_ms += u64::from(*ms);
                    true
                }
                Action::Throttle {
                    download_kbps,
                    upload_kbps,
                    latency_ms,
                } => {
                    fx.throttle = Some(ThrottleSpec {
                        down_bps: u64::from(*download_kbps) * 1000 / 8,
                        up_bps: u64::from(*upload_kbps) * 1000 / 8,
                        latency_ms: *latency_ms,
                    });
                    true
                }
                Action::Script { .. } => match &ca.script {
                    // After a terminal answer nothing more is sent, so no
                    // later script runs.
                    Some(script) if fx.map_local.is_some() || fx.mock.is_some() || !script.enabled => false,
                    Some(script) if script.runs_mock() => match run.mock(script, rule, draft).await {
                        Ok(Some(out)) => match mock_answer(out) {
                            Ok(answer) => {
                                fx.mock = Some(answer);
                                true
                            }
                            Err(message) => {
                                fx.script_issues
                                    .push(run.issue(script, rule, ScriptPhase::Mock, message));
                                false
                            }
                        },
                        Ok(None) => false,
                        Err(issue) => {
                            fx.script_issues.push(issue);
                            false
                        }
                    },
                    Some(script) if script.runs_in(Phase::Request) => match run.request(script, rule, draft).await {
                        Ok(Some(out)) => match apply_request(draft, out) {
                            Ok(changed) => {
                                fx.head_changed |= changed.head;
                                fx.body_changed |= changed.body;
                                changed.head || changed.body
                            }
                            Err(message) => {
                                fx.script_issues
                                    .push(run.issue(script, rule, ScriptPhase::Request, message));
                                false
                            }
                        },
                        Ok(None) => false,
                        Err(issue) => {
                            fx.script_issues.push(issue);
                            false
                        }
                    },
                    _ => false,
                },
                _ => false,
            };
        }
        if applied {
            fx.applied.push(rule.applied(Phase::Request));
        }
    }
    if fx.map_local.is_some() || fx.mock.is_some() {
        fx.map_remote = None;
    }
    fx
}

/// The rules (of those that matched the request) whose response matcher
/// matches this response.
pub(crate) fn response_rules(rules: &[Arc<CompiledRule>], status: u16, headers: &HeaderMap) -> Vec<Arc<CompiledRule>> {
    rules
        .iter()
        .filter(|r| r.response.as_ref().is_none_or(|m| m.matches(status, headers)))
        .cloned()
        .collect()
}

/// Runs the response phase of `rules` (already gated by
/// [`response_rules`]). `request` is the request as it was sent, for
/// `onResponse` scripts ([`needs_request_for_response`]).
pub(crate) async fn response_phase(
    rules: &[Arc<CompiledRule>],
    draft: &mut ResponseDraft,
    run: ScriptRun<'_>,
    request: Option<&JsRequest>,
) -> ResponseEffects {
    let mut fx = ResponseEffects::default();
    for rule in rules {
        let mut applied = false;
        let mut actions = rule.actions.as_slice();
        while let Some(ca) = actions.first() {
            if let Some(n) = json_run(actions, Phase::Response) {
                applied |= edit_json(
                    rule,
                    &actions[..n],
                    &mut draft.body,
                    &mut fx.body_changed,
                    &mut fx.body_skipped,
                );
                actions = &actions[n..];
                continue;
            }
            actions = &actions[1..];
            applied |= match &ca.action {
                Action::SetHeader {
                    phase: Phase::Response, ..
                }
                | Action::AddHeader {
                    phase: Phase::Response, ..
                }
                | Action::RemoveHeader {
                    phase: Phase::Response, ..
                }
                | Action::ReplaceHeaderValue {
                    phase: Phase::Response, ..
                } => {
                    let changed = edit_headers(ca, &mut draft.headers);
                    fx.head_changed |= changed;
                    changed
                }
                Action::SetStatus { status } => {
                    fx.head_changed |= draft.status != *status;
                    draft.status = *status;
                    true
                }
                Action::ReplaceBody {
                    phase: Phase::Response, ..
                }
                | Action::SetBody {
                    phase: Phase::Response, ..
                }
                | Action::FormSet {
                    phase: Phase::Response, ..
                }
                | Action::FormRemove {
                    phase: Phase::Response, ..
                } => match edit_body(ca, &mut draft.body, &mut draft.headers) {
                    Ok(changed) => {
                        fx.body_changed |= changed;
                        changed
                    }
                    Err(reason) => {
                        fx.body_skipped.push(format!("{}: {reason}", rule.applied.name));
                        false
                    }
                },
                Action::Breakpoint { response: true, .. } => {
                    fx.breakpoint.get_or_insert_with(|| rule.clone());
                    true
                }
                Action::Delay {
                    phase: Phase::Response,
                    ms,
                } => {
                    fx.delay_ms += u64::from(*ms);
                    true
                }
                Action::Script { .. } => match (&ca.script, request) {
                    (Some(script), Some(request)) if script.runs_in(Phase::Response) => {
                        match run.response(script, rule, request, draft).await {
                            Ok(Some(out)) => match apply_response(draft, out) {
                                Ok(changed) => {
                                    fx.head_changed |= changed.head;
                                    fx.body_changed |= changed.body;
                                    changed.head || changed.body
                                }
                                Err(message) => {
                                    fx.script_issues
                                        .push(run.issue(script, rule, ScriptPhase::Response, message));
                                    false
                                }
                            },
                            Ok(None) => false,
                            Err(issue) => {
                                fx.script_issues.push(issue);
                                false
                            }
                        }
                    }
                    _ => false,
                },
                _ => false,
            };
        }
        if applied {
            fx.applied.push(rule.applied(Phase::Response));
        }
    }
    fx
}

/// Applies a header action; returns whether anything changed.
fn edit_headers(ca: &CompiledAction, headers: &mut HeaderMap) -> bool {
    let Some(name) = &ca.header else {
        return false;
    };
    match &ca.action {
        Action::SetHeader { value, .. } => {
            let Ok(value) = HeaderValue::from_str(value) else {
                return false;
            };
            let unchanged = headers.get_all(name).iter().eq(std::iter::once(&value));
            headers.insert(name.clone(), value);
            !unchanged
        }
        Action::AddHeader { value, .. } => {
            let Ok(value) = HeaderValue::from_str(value) else {
                return false;
            };
            headers.append(name.clone(), value);
            true
        }
        Action::RemoveHeader { .. } => headers.remove(name).is_some(),
        Action::ReplaceHeaderValue { replacement, .. } => {
            let Some(find) = &ca.find else {
                return false;
            };
            let values: Vec<HeaderValue> = headers.get_all(name).iter().cloned().collect();
            let mut changed = false;
            let replaced: Vec<HeaderValue> = values
                .into_iter()
                .map(|v| {
                    let new = find.replace_all(v.as_bytes(), replacement.as_bytes());
                    match HeaderValue::from_bytes(&new) {
                        Ok(new) if new != v => {
                            changed = true;
                            new
                        }
                        _ => v,
                    }
                })
                .collect();
            if changed {
                headers.remove(name);
                for v in replaced {
                    headers.append(name.clone(), v);
                }
            }
            changed
        }
        _ => false,
    }
}

/// Applies a body action to a buffered, decoded body.
fn edit_body(ca: &CompiledAction, body: &mut Option<Vec<u8>>, headers: &mut HeaderMap) -> Result<bool, String> {
    if let Action::SetBody {
        body: new,
        content_type,
        ..
    } = &ca.action
    {
        if let Some(ct) = content_type
            && let Ok(ct) = HeaderValue::from_str(ct)
        {
            headers.insert(header::CONTENT_TYPE, ct);
        }
        *body = Some(new.clone().into_bytes());
        return Ok(true);
    }
    let Some(bytes) = body.as_mut() else {
        return Err("the body wasn't buffered".into());
    };
    match &ca.action {
        Action::ReplaceBody { replacement, .. } => {
            let Some(find) = &ca.find else {
                return Ok(false);
            };
            let new = find.replace_all(bytes, replacement.as_bytes());
            if new.as_ref() == bytes.as_slice() {
                return Ok(false);
            }
            *bytes = new.into_owned();
            Ok(true)
        }
        Action::FormSet { name, value, .. } => edit_form(bytes, name, Some(value)),
        Action::FormRemove { name, .. } => edit_form(bytes, name, None),
        _ => Ok(false),
    }
}

/// Sets or removes a field of a form-urlencoded body.
fn edit_form(bytes: &mut Vec<u8>, name: &str, value: Option<&str>) -> Result<bool, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "the body isn't form text".to_owned())?;
    let new = set_query(text, name, value);
    if new == text {
        return Ok(false);
    }
    *bytes = new.into_bytes();
    Ok(true)
}

/// How many actions from the start of `actions` are JSON edits of `phase`
/// in a row (`None` for none). They are applied in one pass.
fn json_run(actions: &[CompiledAction], phase: Phase) -> Option<usize> {
    let n = actions
        .iter()
        .take_while(|ca| {
            matches!(&ca.action, Action::JsonSet { phase: p, .. } | Action::JsonRemove { phase: p, .. } if *p == phase)
        })
        .count();
    (n > 0).then_some(n)
}

/// Applies a run of JSON edits; returns whether any changed the body.
fn edit_json(
    rule: &CompiledRule,
    run: &[CompiledAction],
    body: &mut Option<Vec<u8>>,
    body_changed: &mut bool,
    skipped: &mut Vec<String>,
) -> bool {
    let Some(bytes) = body.as_mut() else {
        skipped.extend(
            run.iter()
                .map(|_| format!("{}: the body wasn't buffered", rule.applied.name)),
        );
        return false;
    };
    let edits: Vec<Edit<'_>> = run
        .iter()
        .map(|ca| match &ca.action {
            Action::JsonSet { pointer, .. } => Edit::Set {
                pointer,
                raw: ca.json.as_deref().unwrap_or("null"),
            },
            Action::JsonRemove { pointer, .. } => Edit::Remove { pointer },
            _ => unreachable!("json_run takes only JSON edits"),
        })
        .collect();
    let mut changed = false;
    for result in json_edit::apply(bytes, &edits) {
        match result {
            Ok(c) => changed |= c,
            Err(reason) => skipped.push(format!("{}: {reason}", rule.applied.name)),
        }
    }
    *body_changed |= changed;
    changed
}

/// The query with `name` set to `value` (every occurrence replaced, or
/// appended), or removed with `None`. Other pairs keep their raw form.
pub(crate) fn set_query(query: &str, name: &str, value: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut placed = false;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let key = pair.split_once('=').map_or(pair, |(k, _)| k);
        let decoded = query_pairs(key).into_iter().next().map(|(k, _)| k).unwrap_or_default();
        if decoded != name {
            out.push(pair.to_owned());
            continue;
        }
        if let (Some(value), false) = (value, placed) {
            out.push(format!("{}={}", percent_encode(name), percent_encode(value)));
            placed = true;
        }
    }
    if let (Some(value), false) = (value, placed) {
        out.push(format!("{}={}", percent_encode(name), percent_encode(value)));
    }
    out.join("&")
}

/// Rewrites a body's framing headers for a body re-sent identity-encoded.
pub(crate) fn reframe(headers: &mut HeaderMap, len: usize) {
    headers.remove(header::CONTENT_ENCODING);
    headers.remove(header::TRANSFER_ENCODING);
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
}

/// `Content-Type` for a file served by Map Local.
pub(crate) fn guess_content_type(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "json" => "application/json",
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wasm" => "application/wasm",
        "txt" | "log" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Header name check shared with the breakpoint edits.
pub(crate) fn header_map(headers: &[pm_model::Header]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for h in headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(h.name.trim().as_bytes()),
            HeaderValue::from_str(&h.value),
        ) {
            map.append(name, value);
        }
    }
    map
}
