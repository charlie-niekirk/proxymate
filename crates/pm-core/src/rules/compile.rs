//! Rules compile into a [`CompiledRuleSet`]: every pattern becomes a
//! size-limited byte regex (globs via globset), and a host `RegexSet`
//! narrows each request to the rules that can match it.

use std::sync::Arc;

use http::HeaderMap;
use http::header::HeaderName;
use pm_model::{Action, AppliedRule, KvMatch, Pattern, PatternKind, Phase, Rule, Scheme, StatusRange, WsOp};
use regex::bytes::{Regex, RegexBuilder, RegexSet, RegexSetBuilder};

use super::RuleError;
use crate::mocks::{MockIndex, MockMap};
use crate::scripts::{CompiledScript, ScriptMap};

/// Compiled program size allowed per pattern. Generous for real patterns,
/// small enough that an agent can't make the proxy allocate gigabytes.
pub const REGEX_SIZE_LIMIT: usize = 256 * 1024;
/// Longest pattern source accepted.
pub const PATTERN_MAX_LEN: usize = 4096;

/// How a pattern is used.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Use {
    /// Does the whole value match? Exact and globs are anchored, regexes
    /// are not (anchor them with `^…$`).
    Match,
    /// Where in a value is it? Exact is a literal substring.
    Find,
}

/// Compiles `pattern` to a byte regex. `host` makes `*.example.com` also
/// match `example.com`.
pub(crate) fn compile_pattern(pattern: &Pattern, usage: Use, host: bool) -> Result<Regex, String> {
    regex_builder(&pattern_source(pattern, usage, host)?, pattern.case_insensitive)
        .build()
        .map_err(|e| regex_error(&e))
}

fn pattern_source(pattern: &Pattern, usage: Use, host: bool) -> Result<String, String> {
    let value = &pattern.value;
    if value.len() > PATTERN_MAX_LEN {
        return Err(format!(
            "the pattern is {} characters long; the limit is {PATTERN_MAX_LEN}",
            value.len()
        ));
    }
    Ok(match pattern.kind {
        PatternKind::Exact => match usage {
            Use::Match => format!("^{}$", regex::escape(value)),
            Use::Find => regex::escape(value),
        },
        PatternKind::Glob => {
            let mut source = format!("(?:{})", glob_regex(value, pattern.case_insensitive)?);
            if host && let Some(domain) = value.strip_prefix("*.") {
                source = format!("{source}|(?:{})", glob_regex(domain, pattern.case_insensitive)?);
            }
            source
        }
        PatternKind::Regex => value.clone(),
    })
}

fn glob_regex(glob: &str, case_insensitive: bool) -> Result<String, String> {
    globset::GlobBuilder::new(glob)
        .case_insensitive(case_insensitive)
        .literal_separator(false)
        .backslash_escape(true)
        .build()
        .map(|g| g.regex().to_owned())
        .map_err(|e| format!("invalid glob: {}", e.kind()))
}

fn regex_builder(source: &str, case_insensitive: bool) -> RegexBuilder {
    let mut builder = RegexBuilder::new(source);
    builder
        .case_insensitive(case_insensitive)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT);
    builder
}

fn regex_error(err: &regex::Error) -> String {
    match err {
        regex::Error::CompiledTooBig(limit) => format!(
            "the regex is too complex: it compiles to more than {} KB (the limit)",
            limit / 1024
        ),
        regex::Error::Syntax(text) => format!("invalid regex: {}", text.trim()),
        other => format!("invalid regex: {other}"),
    }
}

/// A query or header condition.
pub(crate) struct CompiledKv {
    name: String,
    value: Option<Regex>,
    negate: bool,
}

impl CompiledKv {
    fn compile(kv: &KvMatch, field: &str, rule: &Rule, header: bool) -> Result<CompiledKv, RuleError> {
        if kv.name.is_empty() {
            return Err(RuleError::invalid(rule, field, "the name is empty"));
        }
        if header && HeaderName::from_bytes(kv.name.as_bytes()).is_err() {
            return Err(RuleError::invalid(
                rule,
                field,
                format!("{:?} isn't a valid header name", kv.name),
            ));
        }
        let value = kv
            .value
            .as_ref()
            .map(|p| compile_pattern(p, Use::Match, false))
            .transpose()
            .map_err(|e| RuleError::invalid(rule, &format!("{field} {}", kv.name), e))?;
        Ok(CompiledKv {
            name: if header {
                kv.name.to_ascii_lowercase()
            } else {
                kv.name.clone()
            },
            value,
            negate: kv.negate,
        })
    }

    fn matches<'a>(&self, mut values: impl Iterator<Item = &'a [u8]>) -> bool {
        let hit = match &self.value {
            None => values.next().is_some(),
            Some(re) => values.any(|v| re.is_match(v)),
        };
        hit != self.negate
    }
}

/// A response matcher.
pub(crate) struct CompiledResponse {
    status: Vec<StatusRange>,
    content_type: Option<Regex>,
}

impl CompiledResponse {
    pub fn matches(&self, status: u16, headers: &HeaderMap) -> bool {
        if !self.status.is_empty() && !self.status.iter().any(|r| (r.from..=r.to).contains(&status)) {
            return false;
        }
        match &self.content_type {
            None => true,
            Some(re) => headers
                .get(http::header::CONTENT_TYPE)
                .is_some_and(|v| re.is_match(v.as_bytes())),
        }
    }
}

/// An action with what it needs precompiled.
pub(crate) struct CompiledAction {
    pub action: Action,
    /// `ReplaceHeaderValue` and `ReplaceBody`'s pattern, and a
    /// `WsMessage` replace's.
    pub find: Option<Regex>,
    /// A `WsMessage` action's message filter.
    pub filter: Option<Regex>,
    /// The header an action names.
    pub header: Option<HeaderName>,
    /// `JsonSet`'s value as JSON text (`raw`, or `value` serialised).
    pub json: Option<String>,
    /// The script an `Action::Script` runs, as it was when the rule set
    /// was compiled.
    pub script: Option<Arc<CompiledScript>>,
    /// The recording an `Action::ServeFrom` answers from.
    pub serve: Option<CompiledServe>,
}

/// A `ServeFrom`'s source, indexed when the rule set compiled.
pub(crate) struct CompiledServe {
    /// `None` for a rule that wasn't compiled to run (disabled, or only
    /// checked).
    pub index: Option<Arc<MockIndex>>,
    /// A round-robin's place, by the first flow of each candidate set.
    pub turns: parking_lot::Mutex<std::collections::HashMap<pm_model::FlowId, usize>>,
}

/// One enabled rule, ready to run.
pub(crate) struct CompiledRule {
    pub applied: AppliedRule,
    methods: Vec<String>,
    scheme: Option<Scheme>,
    host: Option<Regex>,
    port: Option<u16>,
    path: Option<Regex>,
    query: Vec<CompiledKv>,
    headers: Vec<CompiledKv>,
    client_app: Option<Regex>,
    pub response: Option<CompiledResponse>,
    pub actions: Vec<CompiledAction>,
}

/// What a request matcher looks at.
pub(crate) struct RequestInfo<'a> {
    pub method: &'a str,
    pub scheme: Scheme,
    pub host: &'a str,
    pub port: u16,
    pub path: &'a str,
    /// Raw, without the `?`.
    pub query: &'a str,
    pub headers: &'a HeaderMap,
    pub client_app: Option<&'a str>,
}

impl CompiledRule {
    /// Compiles `rule`, resolving its script actions against `scripts` and
    /// its `ServeFrom` sources against `mocks`.
    pub fn compile(rule: &Rule, scripts: &ScriptMap, mocks: &MockMap) -> Result<CompiledRule, RuleError> {
        let m = &rule.matcher;
        let pattern = |p: &Option<Pattern>, field: &str, host: bool| {
            p.as_ref()
                .map(|p| compile_pattern(p, Use::Match, host))
                .transpose()
                .map_err(|e| RuleError::invalid(rule, field, e))
        };
        let response = match &m.response {
            Some(r) => {
                for range in &r.status {
                    if range.from > range.to || !(100..=999).contains(&range.from) || range.to > 999 {
                        return Err(RuleError::invalid(
                            rule,
                            "response status",
                            format!("{}–{} isn't a valid status range", range.from, range.to),
                        ));
                    }
                }
                Some(CompiledResponse {
                    status: r.status.clone(),
                    content_type: pattern(&r.content_type, "response content type", false)?,
                })
            }
            None => None,
        };
        let actions = rule
            .actions
            .iter()
            .enumerate()
            .map(|(i, action)| compile_action(rule, i, action, scripts, mocks))
            .collect::<Result<_, _>>()?;
        Ok(CompiledRule {
            applied: AppliedRule {
                rule_id: rule.id.clone(),
                name: rule.name.clone(),
                origin: rule.origin.clone(),
                phase: Phase::Request,
            },
            methods: m.methods.iter().map(|s| s.trim().to_ascii_uppercase()).collect(),
            scheme: m.scheme,
            host: pattern(&m.host, "host", true)?,
            port: m.port,
            path: pattern(&m.path, "path", false)?,
            query: m
                .query
                .iter()
                .map(|kv| CompiledKv::compile(kv, "query", rule, false))
                .collect::<Result<_, _>>()?,
            headers: m
                .headers
                .iter()
                .map(|kv| CompiledKv::compile(kv, "header", rule, true))
                .collect::<Result<_, _>>()?,
            client_app: pattern(&m.client_app, "client app", false)?,
            response,
            actions,
        })
    }

    /// Checks everything but the host (the rule set's prefilter did that).
    /// On a mismatch, says which condition failed.
    pub fn mismatch(&self, req: &RequestInfo<'_>, check_host: bool) -> Option<String> {
        if !self.methods.is_empty() && !self.methods.iter().any(|m| m.eq_ignore_ascii_case(req.method)) {
            return Some(format!(
                "method {} isn't one of {}",
                req.method,
                self.methods.join(", ")
            ));
        }
        if let Some(scheme) = self.scheme
            && scheme != req.scheme
        {
            return Some(format!("scheme is {}, not {}", req.scheme.as_str(), scheme.as_str()));
        }
        if check_host
            && let Some(host) = &self.host
            && !host.is_match(req.host.as_bytes())
        {
            return Some(format!("host {} doesn't match", req.host));
        }
        if let Some(port) = self.port
            && port != req.port
        {
            return Some(format!("port is {}, not {port}", req.port));
        }
        if let Some(path) = &self.path
            && !path.is_match(req.path.as_bytes())
        {
            return Some(format!("path {} doesn't match", req.path));
        }
        if !self.query.is_empty() {
            let pairs = query_pairs(req.query);
            for kv in &self.query {
                let values = pairs.iter().filter(|(k, _)| *k == kv.name).map(|(_, v)| v.as_bytes());
                if !kv.matches(values) {
                    return Some(format!("query parameter {} doesn't match", kv.name));
                }
            }
        }
        for kv in &self.headers {
            let values = req.headers.get_all(kv.name.as_str()).iter().map(|v| v.as_bytes());
            if !kv.matches(values) {
                return Some(format!("header {} doesn't match", kv.name));
            }
        }
        if let Some(app) = &self.client_app
            && !req.client_app.is_some_and(|a| app.is_match(a.as_bytes()))
        {
            return Some(match req.client_app {
                Some(a) => format!("client app {a} doesn't match"),
                None => "the client app isn't known".into(),
            });
        }
        None
    }

    pub fn needs_client_app(&self) -> bool {
        self.client_app.is_some()
    }

    /// True if the rule edits or pauses a body in `phase`.
    pub fn touches_body(&self, phase: Phase) -> bool {
        self.actions.iter().any(|a| match &a.action {
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
            Action::Script { .. } => a.script.as_ref().is_some_and(|s| s.runs_in(phase)),
            Action::ServeFrom { key, .. } => phase == Phase::Request && key.body,
            _ => false,
        })
    }

    /// True if the rule acts on WebSocket messages: a `WsMessage` action,
    /// or a script that exports `onWsMessage`.
    pub fn acts_on_messages(&self) -> bool {
        self.actions.iter().any(|a| match &a.action {
            Action::WsMessage { .. } => true,
            Action::Script { .. } => a.script.as_ref().is_some_and(|s| s.runs_ws()),
            _ => false,
        })
    }

    /// True if some action applies to HTTP exchanges (anything but a
    /// `WsMessage`).
    pub fn acts_on_http(&self) -> bool {
        self.actions
            .iter()
            .any(|a| !matches!(a.action, Action::WsMessage { .. }))
    }

    pub fn applied(&self, phase: Phase) -> AppliedRule {
        AppliedRule {
            phase,
            ..self.applied.clone()
        }
    }
}

fn compile_action(
    rule: &Rule,
    index: usize,
    action: &Action,
    scripts: &ScriptMap,
    mocks: &MockMap,
) -> Result<CompiledAction, RuleError> {
    let field = format!("action {}", index + 1);
    let invalid = |message: String| RuleError::invalid(rule, &field, message);
    let header = |name: &str| {
        HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| invalid(format!("{name:?} isn't a valid header name")))
    };
    let status = |status: u16| {
        if (100..=999).contains(&status) {
            Ok(())
        } else {
            Err(invalid(format!("{status} isn't a valid status code")))
        }
    };
    let pointer = |pointer: &str| {
        if pointer.is_empty() || pointer.starts_with('/') {
            Ok(())
        } else {
            Err(invalid(format!(
                "{pointer:?} isn't a JSON pointer: it must start with /"
            )))
        }
    };
    let mut compiled = CompiledAction {
        action: action.clone(),
        find: None,
        filter: None,
        header: None,
        json: None,
        script: None,
        serve: None,
    };
    match action {
        Action::SetHeader { name, value, .. } | Action::AddHeader { name, value, .. } => {
            compiled.header = Some(header(name)?);
            if http::HeaderValue::from_str(value).is_err() {
                return Err(invalid(format!("{value:?} isn't a valid header value")));
            }
        }
        Action::RemoveHeader { name, .. } => compiled.header = Some(header(name)?),
        Action::ReplaceHeaderValue { name, pattern, .. } => {
            compiled.header = Some(header(name)?);
            compiled.find = Some(compile_pattern(pattern, Use::Find, false).map_err(invalid)?);
        }
        Action::SetQuery { name, .. } | Action::RemoveQuery { name } if name.is_empty() => {
            return Err(invalid("the query parameter name is empty".into()));
        }
        Action::FormSet { name, .. } | Action::FormRemove { name, .. } if name.is_empty() => {
            return Err(invalid("the form field name is empty".into()));
        }
        Action::ReplaceBody { pattern, .. } => {
            compiled.find = Some(compile_pattern(pattern, Use::Find, false).map_err(invalid)?);
        }
        Action::SetBody { content_type, .. } => {
            if let Some(ct) = content_type
                && http::HeaderValue::from_str(ct).is_err()
            {
                return Err(invalid(format!("{ct:?} isn't a valid content type")));
            }
        }
        Action::JsonSet {
            pointer: p, value, raw, ..
        } => {
            pointer(p)?;
            compiled.json = Some(match raw {
                Some(raw) => {
                    serde_json::from_str::<serde::de::IgnoredAny>(raw)
                        .map_err(|e| invalid(format!("the raw value isn't JSON: {e}")))?;
                    raw.trim().to_owned()
                }
                None => value.to_string(),
            });
        }
        Action::JsonRemove { pointer: p, .. } => pointer(p)?,
        Action::SetStatus { status: s } => status(*s)?,
        Action::MapLocal { status: s, headers, .. } => {
            status(*s)?;
            for h in headers {
                header(&h.name)?;
            }
        }
        Action::MapRemote { host, path, .. } => {
            if host
                .as_deref()
                .is_some_and(|h| h.trim().is_empty() || h.contains(['/', ' ']))
            {
                return Err(invalid(format!(
                    "{:?} isn't a host name",
                    host.as_deref().unwrap_or("")
                )));
            }
            if path.as_deref().is_some_and(|p| !p.starts_with('/')) {
                return Err(invalid("the path must start with /".into()));
            }
        }
        Action::Block {
            mode: pm_model::BlockMode::Status { status: s },
        } => status(*s)?,
        Action::Script { script_id } => {
            let script = scripts
                .get(script_id)
                .ok_or_else(|| invalid(format!("there is no script with id {script_id}")))?;
            if script.kind == pm_model::ScriptKind::View {
                return Err(invalid(format!(
                    "{:?} is a view script; rules run rewrite and mock scripts",
                    script.code.name
                )));
            }
            compiled.script = Some(script.clone());
        }
        Action::WsMessage { filter, op, .. } => {
            compiled.filter = filter
                .as_ref()
                .map(|p| compile_pattern(p, Use::Find, false))
                .transpose()
                .map_err(|e| invalid(format!("message filter: {e}")))?;
            match op {
                WsOp::Replace { pattern, .. } => {
                    compiled.find = Some(compile_pattern(pattern, Use::Find, false).map_err(invalid)?);
                }
                WsOp::JsonSet { pointer: p, value, raw } => {
                    pointer(p)?;
                    compiled.json = Some(match raw {
                        Some(raw) => {
                            serde_json::from_str::<serde::de::IgnoredAny>(raw)
                                .map_err(|e| invalid(format!("the raw value isn't JSON: {e}")))?;
                            raw.trim().to_owned()
                        }
                        None => value.to_string(),
                    });
                }
                WsOp::JsonRemove { pointer: p } => pointer(p)?,
                WsOp::Drop | WsOp::Delay { .. } | WsOp::Breakpoint => {}
            }
        }
        Action::ServeFrom { source, .. } => {
            compiled.serve = Some(CompiledServe {
                index: mocks.resolve(source).map_err(invalid)?,
                turns: Default::default(),
            });
        }
        _ => {}
    }
    Ok(compiled)
}

/// The enabled rules, in order, with a host prefilter.
pub struct CompiledRuleSet {
    rules: Vec<Arc<CompiledRule>>,
    hosts: RegexSet,
    /// `hosts` pattern i belongs to `rules[host_rules[i]]`.
    host_rules: Vec<usize>,
    /// Rules without a host condition.
    any_host: Vec<usize>,
    needs_client_app: bool,
}

impl CompiledRuleSet {
    pub fn empty() -> CompiledRuleSet {
        CompiledRuleSet {
            rules: Vec::new(),
            hosts: RegexSet::empty(),
            host_rules: Vec::new(),
            any_host: Vec::new(),
            needs_client_app: false,
        }
    }

    /// Compiles every rule (so a broken one is reported even if disabled)
    /// and keeps the enabled ones.
    pub(crate) fn compile(rules: &[Rule], scripts: &ScriptMap, mocks: &MockMap) -> Result<CompiledRuleSet, RuleError> {
        let mut sorted: Vec<&Rule> = rules.iter().collect();
        sorted.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        let mut compiled = Vec::new();
        let mut host_sources = Vec::new();
        let mut host_rules = Vec::new();
        let mut any_host = Vec::new();
        for rule in sorted {
            let c = CompiledRule::compile(rule, scripts, mocks)?;
            if !rule.enabled {
                continue;
            }
            let index = compiled.len();
            match &rule.matcher.host {
                Some(p) => {
                    let source =
                        pattern_source(p, Use::Match, true).map_err(|e| RuleError::invalid(rule, "host", e))?;
                    let flags = if p.case_insensitive { "(?i)" } else { "" };
                    host_sources.push(format!("{flags}(?:{source})"));
                    host_rules.push(index);
                }
                None => any_host.push(index),
            }
            compiled.push(Arc::new(c));
        }
        let hosts = RegexSetBuilder::new(&host_sources)
            .size_limit(REGEX_SIZE_LIMIT * host_sources.len().max(1))
            .dfa_size_limit(REGEX_SIZE_LIMIT * host_sources.len().max(1))
            .build()
            .map_err(|e| RuleError::Invalid {
                rule: String::new(),
                field: "host patterns".into(),
                message: regex_error(&e),
            })?;
        Ok(CompiledRuleSet {
            needs_client_app: compiled.iter().any(|r| r.needs_client_app()),
            rules: compiled,
            hosts,
            host_rules,
            any_host,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// True if some rule matches on the client app, so it's worth waiting
    /// for it to be resolved.
    pub fn needs_client_app(&self) -> bool {
        self.needs_client_app
    }

    /// The rules whose request matcher matches, in order.
    pub(crate) fn matching(&self, req: &RequestInfo<'_>) -> Vec<Arc<CompiledRule>> {
        if self.rules.is_empty() {
            return Vec::new();
        }
        let mut candidates: Vec<usize> = self
            .hosts
            .matches(req.host.as_bytes())
            .into_iter()
            .map(|i| self.host_rules[i])
            .chain(self.any_host.iter().copied())
            .collect();
        candidates.sort_unstable();
        candidates
            .into_iter()
            .map(|i| &self.rules[i])
            .filter(|r| r.mismatch(req, false).is_none())
            .cloned()
            .collect()
    }
}

/// `a=1&b=x%20y` as decoded pairs.
pub(crate) fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes a query component.
pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
