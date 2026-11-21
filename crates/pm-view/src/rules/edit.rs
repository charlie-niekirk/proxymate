//! Edits a rule one field at a time, by key: the rule form binds each text
//! input and picker to a key, and the dev remote drives the same keys.
//!
//! Keys:
//! - `name`.
//! - The matcher, `m.…`: `port`; the patterns `host`, `path`, `app`;
//!   `query.<i>.name`, `query.<i>.value`, `headers.<i>.…` likewise; the
//!   response matcher's `status` (ranges as text) and `ctype`.
//! - Action `i`, `a.<i>.…`: `name`, `value`, `pattern`, `replacement`,
//!   `body`, `ctype`, `pointer`, `raw` (JSON text), `status`, `ms`, `down`,
//!   `up`, `latency`, `host`, `port`, `path`, `inline`, `file`, `root`,
//!   `filter`, `text`.
//!
//! [`text`] reads a field, [`set_text`] writes one (refusing what doesn't
//! parse: a port, a status, JSON). [`choose`] sets a picker or a toggle:
//! `m.method` (toggles one), `m.scheme`, `<pattern key>.kind` and `.ci`,
//! `m.<query|headers>.<i>.negate`, and per action `phase`, `source`,
//! `scheme`, `preserve_host`, `request`, `response`, `mode`, `script`,
//! `direction`, `op`, `serve`, `query`, `body`, `pick`, `miss`. The
//! structural edits (rows and actions added, removed, moved) are their own
//! functions.

use pm_model::{
    Action, BlockMode, KvMatch, MapLocalSource, Pattern, PatternKind, Phase, ResponseMatcher, Rule, Scheme,
    WsDirection, WsOp,
};

use super::{
    PATTERN_KINDS, SERVE_MISS, SERVE_PICK, default_ws_op, format_status_ranges, json_error, json_set_text, move_item,
    new_pattern, parse_status_ranges,
};

/// A pattern slot: optional ones are "any" while empty.
enum Slot<'a> {
    Optional(&'a mut Option<Pattern>),
    Required(&'a mut Pattern),
}

impl Slot<'_> {
    fn get(&self) -> Option<&Pattern> {
        match self {
            Slot::Optional(p) => p.as_ref(),
            Slot::Required(p) => Some(p),
        }
    }

    /// Applies `f` to the pattern (a glob by default); an optional one with
    /// an empty value goes back to "any" (`PatternInput`).
    fn update(self, f: impl FnOnce(&mut Pattern)) {
        match self {
            Slot::Optional(slot) => {
                let mut p = slot.clone().unwrap_or_else(|| new_pattern(PatternKind::Glob, ""));
                f(&mut p);
                *slot = (!p.value.is_empty()).then_some(p);
            }
            Slot::Required(p) => f(p),
        }
    }
}

/// `a.<i>.<rest>`, `m.<query|headers>.<i>.<rest>`.
fn indexed<'k>(key: &'k str, prefix: &str) -> Option<(usize, &'k str)> {
    let rest = key.strip_prefix(prefix)?;
    let (i, rest) = rest.split_once('.')?;
    Some((i.parse().ok()?, rest))
}

fn kv_rows<'a>(rule: &'a mut Rule, which: &str) -> Option<&'a mut Vec<KvMatch>> {
    match which {
        "query" => Some(&mut rule.matcher.query),
        "headers" => Some(&mut rule.matcher.headers),
        _ => None,
    }
}

fn pattern_slot<'a>(rule: &'a mut Rule, key: &str) -> Option<Slot<'a>> {
    match key {
        "m.host" => return Some(Slot::Optional(&mut rule.matcher.host)),
        "m.path" => return Some(Slot::Optional(&mut rule.matcher.path)),
        "m.app" => return Some(Slot::Optional(&mut rule.matcher.client_app)),
        "m.ctype" => return rule.matcher.response.as_mut().map(|r| Slot::Optional(&mut r.content_type)),
        _ => {}
    }
    for which in ["query", "headers"] {
        if let Some((i, "value")) = indexed(key, &format!("m.{which}.")) {
            return kv_rows(rule, which)?.get_mut(i).map(|r| Slot::Optional(&mut r.value));
        }
    }
    let (i, field) = indexed(key, "a.")?;
    let action = rule.actions.get_mut(i)?;
    match (action, field) {
        (Action::ReplaceHeaderValue { pattern, .. } | Action::ReplaceBody { pattern, .. }, "pattern") => {
            Some(Slot::Required(pattern))
        }
        (
            Action::WsMessage {
                op: WsOp::Replace { pattern, .. },
                ..
            },
            "pattern",
        ) => Some(Slot::Required(pattern)),
        (Action::WsMessage { filter, .. }, "filter") => Some(Slot::Optional(filter)),
        _ => None,
    }
}

fn num<T: std::str::FromStr>(value: &str, what: &str) -> Result<T, String> {
    value.trim().parse().map_err(|_| format!("{what} must be a number"))
}

/// An optional number: empty is `None`.
fn opt_num<T: std::str::FromStr>(value: &str, what: &str) -> Result<Option<T>, String> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        num(value, what).map(Some)
    }
}

/// A number whose empty field means `fallback` (`NumberInput`'s `?? n`).
fn num_or<T: std::str::FromStr>(value: &str, fallback: T, what: &str) -> Result<T, String> {
    if value.trim().is_empty() {
        Ok(fallback)
    } else {
        num(value, what)
    }
}

fn opt_text(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

/// The text a field shows.
pub fn text(rule: &Rule, key: &str) -> Option<String> {
    if key == "name" {
        return Some(rule.name.clone());
    }
    if key == "m.port" {
        return Some(rule.matcher.port.map(|p| p.to_string()).unwrap_or_default());
    }
    if key == "m.status" {
        return rule.matcher.response.as_ref().map(|r| format_status_ranges(&r.status));
    }
    // Patterns read through a scratch clone: the slot needs `&mut`.
    if key.starts_with("m.") || key.ends_with(".pattern") || key.ends_with(".filter") {
        let mut scratch = rule.clone();
        if let Some(slot) = pattern_slot(&mut scratch, key) {
            return Some(slot.get().map(|p| p.value.clone()).unwrap_or_default());
        }
    }
    for which in ["query", "headers"] {
        if let Some((i, "name")) = indexed(key, &format!("m.{which}.")) {
            let rows = if which == "query" {
                &rule.matcher.query
            } else {
                &rule.matcher.headers
            };
            return rows.get(i).map(|r| r.name.clone());
        }
    }
    let (i, field) = indexed(key, "a.")?;
    action_text(rule.actions.get(i)?, field)
}

fn action_text(action: &Action, field: &str) -> Option<String> {
    let n = |n: u32| Some(n.to_string());
    Some(match (action, field) {
        (
            Action::SetHeader { name, .. }
            | Action::AddHeader { name, .. }
            | Action::RemoveHeader { name, .. }
            | Action::ReplaceHeaderValue { name, .. }
            | Action::SetQuery { name, .. }
            | Action::RemoveQuery { name }
            | Action::FormSet { name, .. }
            | Action::FormRemove { name, .. },
            "name",
        ) => name.clone(),
        (
            Action::SetHeader { value, .. }
            | Action::AddHeader { value, .. }
            | Action::SetQuery { value, .. }
            | Action::FormSet { value, .. },
            "value",
        ) => value.clone(),
        (
            Action::ReplaceHeaderValue { replacement, .. }
            | Action::ReplaceBody { replacement, .. }
            | Action::WsMessage {
                op: WsOp::Replace { replacement, .. },
                ..
            },
            "replacement",
        ) => replacement.clone(),
        (Action::SetBody { body, .. }, "body") => body.clone(),
        (Action::SetBody { content_type, .. } | Action::MapLocal { content_type, .. }, "ctype") => {
            content_type.clone().unwrap_or_default()
        }
        (
            Action::JsonSet { pointer, .. }
            | Action::JsonRemove { pointer, .. }
            | Action::WsMessage {
                op: WsOp::JsonSet { pointer, .. } | WsOp::JsonRemove { pointer },
                ..
            },
            "pointer",
        ) => pointer.clone(),
        (
            Action::JsonSet { value, raw, .. }
            | Action::WsMessage {
                op: WsOp::JsonSet { value, raw, .. },
                ..
            },
            "raw",
        ) => json_set_text(value, raw.as_deref()),
        (
            Action::SetStatus { status }
            | Action::MapLocal { status, .. }
            | Action::Block {
                mode: BlockMode::Status { status },
            },
            "status",
        ) => status.to_string(),
        (
            Action::Delay { ms, .. }
            | Action::WsMessage {
                op: WsOp::Delay { ms }, ..
            },
            "ms",
        ) => return n(*ms),
        (Action::Throttle { download_kbps, .. }, "down") => return n(*download_kbps),
        (Action::Throttle { upload_kbps, .. }, "up") => return n(*upload_kbps),
        (Action::Throttle { latency_ms, .. }, "latency") => return n(*latency_ms),
        (Action::MapRemote { host, .. }, "host") => host.clone().unwrap_or_default(),
        (Action::MapRemote { port, .. }, "port") => port.map(|p| p.to_string()).unwrap_or_default(),
        (Action::MapRemote { path, .. }, "path") => path.clone().unwrap_or_default(),
        (
            Action::MapLocal {
                source: MapLocalSource::Inline { body },
                ..
            },
            "inline",
        ) => body.clone(),
        (
            Action::MapLocal {
                source: MapLocalSource::File { path },
                ..
            },
            "file",
        ) => path.clone(),
        (
            Action::MapLocal {
                source: MapLocalSource::Directory { root },
                ..
            },
            "root",
        ) => root.clone(),
        (Action::Comment { text }, "text") => text.clone(),
        (
            Action::ReplaceHeaderValue { pattern, .. }
            | Action::ReplaceBody { pattern, .. }
            | Action::WsMessage {
                op: WsOp::Replace { pattern, .. },
                ..
            },
            "pattern",
        ) => pattern.value.clone(),
        (Action::WsMessage { filter, .. }, "filter") => filter.as_ref().map(|f| f.value.clone()).unwrap_or_default(),
        _ => return None,
    })
}

/// Writes a field. A value that doesn't parse (a port, a status, JSON
/// text, status ranges) is refused and leaves the rule as it was.
pub fn set_text(rule: &mut Rule, key: &str, value: &str) -> Result<(), String> {
    match key {
        "name" => {
            rule.name = value.to_owned();
            return Ok(());
        }
        "m.port" => {
            rule.matcher.port = opt_num(value, "The port")?;
            return Ok(());
        }
        "m.status" => {
            let ranges = parse_status_ranges(value).ok_or("Status ranges look like 2xx, 404, 500-599")?;
            let response = rule.matcher.response.as_mut().ok_or("no response matcher")?;
            response.status = ranges;
            return Ok(());
        }
        _ => {}
    }
    if let Some(slot) = pattern_slot(rule, key) {
        slot.update(|p| p.value = value.to_owned());
        return Ok(());
    }
    for which in ["query", "headers"] {
        if let Some((i, "name")) = indexed(key, &format!("m.{which}.")) {
            let row = kv_rows(rule, which).and_then(|rows| rows.get_mut(i)).ok_or("no such row")?;
            row.name = value.to_owned();
            return Ok(());
        }
    }
    let (i, field) = indexed(key, "a.").ok_or_else(|| format!("no field {key}"))?;
    let action = rule.actions.get_mut(i).ok_or_else(|| format!("no action {i}"))?;
    set_action_text(action, field, value).ok_or_else(|| format!("no field {key}"))?
}

fn set_action_text(action: &mut Action, field: &str, value: &str) -> Option<Result<(), String>> {
    let s = || value.to_owned();
    match (action, field) {
        (
            Action::SetHeader { name, .. }
            | Action::AddHeader { name, .. }
            | Action::RemoveHeader { name, .. }
            | Action::ReplaceHeaderValue { name, .. }
            | Action::SetQuery { name, .. }
            | Action::RemoveQuery { name }
            | Action::FormSet { name, .. }
            | Action::FormRemove { name, .. },
            "name",
        ) => *name = s(),
        (
            Action::SetHeader { value: v, .. }
            | Action::AddHeader { value: v, .. }
            | Action::SetQuery { value: v, .. }
            | Action::FormSet { value: v, .. },
            "value",
        ) => *v = s(),
        (
            Action::ReplaceHeaderValue { replacement, .. }
            | Action::ReplaceBody { replacement, .. }
            | Action::WsMessage {
                op: WsOp::Replace { replacement, .. },
                ..
            },
            "replacement",
        ) => *replacement = s(),
        (Action::SetBody { body, .. }, "body") => *body = s(),
        (Action::SetBody { content_type, .. } | Action::MapLocal { content_type, .. }, "ctype") => {
            *content_type = opt_text(value);
        }
        (
            Action::JsonSet { pointer, .. }
            | Action::JsonRemove { pointer, .. }
            | Action::WsMessage {
                op: WsOp::JsonSet { pointer, .. } | WsOp::JsonRemove { pointer },
                ..
            },
            "pointer",
        ) => *pointer = s(),
        (
            Action::JsonSet { value: v, raw, .. }
            | Action::WsMessage {
                op: WsOp::JsonSet { value: v, raw, .. },
                ..
            },
            "raw",
        ) => {
            if let Some(err) = json_error(value) {
                return Some(Err(format!("Not JSON yet: {err}")));
            }
            *v = serde_json::Value::Null;
            *raw = Some(value.trim().to_owned());
        }
        (Action::SetStatus { status } | Action::MapLocal { status, .. }, "status") => {
            match num_or(value, 200, "The status") {
                Ok(n) => *status = n,
                Err(e) => return Some(Err(e)),
            }
        }
        (
            Action::Block {
                mode: BlockMode::Status { status },
            },
            "status",
        ) => match num_or(value, 403, "The status") {
            Ok(n) => *status = n,
            Err(e) => return Some(Err(e)),
        },
        (
            Action::Delay { ms, .. }
            | Action::WsMessage {
                op: WsOp::Delay { ms }, ..
            },
            "ms",
        )
        | (Action::Throttle { download_kbps: ms, .. }, "down")
        | (Action::Throttle { upload_kbps: ms, .. }, "up")
        | (Action::Throttle { latency_ms: ms, .. }, "latency") => match num_or(value, 0, "It") {
            Ok(n) => *ms = n,
            Err(e) => return Some(Err(e)),
        },
        (Action::MapRemote { host, .. }, "host") => *host = opt_text(value),
        (Action::MapRemote { port, .. }, "port") => match opt_num(value, "The port") {
            Ok(n) => *port = n,
            Err(e) => return Some(Err(e)),
        },
        (Action::MapRemote { path, .. }, "path") => *path = opt_text(value),
        (
            Action::MapLocal {
                source: MapLocalSource::Inline { body },
                ..
            },
            "inline",
        ) => *body = s(),
        (
            Action::MapLocal {
                source: MapLocalSource::File { path },
                ..
            },
            "file",
        ) => *path = s(),
        (
            Action::MapLocal {
                source: MapLocalSource::Directory { root },
                ..
            },
            "root",
        ) => *root = s(),
        (Action::Comment { text }, "text") => *text = s(),
        _ => return None,
    }
    Some(Ok(()))
}

fn flag(value: &str) -> Result<bool, String> {
    match value {
        "true" | "on" | "1" => Ok(true),
        "false" | "off" | "0" => Ok(false),
        other => Err(format!("{other:?} isn't on or off")),
    }
}

fn bad(key: &str, value: &str) -> String {
    format!("{value:?} isn't a choice for {key}")
}

/// What a picker or toggle shows now (the same keys as [`choose`]).
pub fn chosen(rule: &Rule, key: &str) -> Option<String> {
    if let Some(base) = key.strip_suffix(".kind").or_else(|| key.strip_suffix(".ci")) {
        let mut scratch = rule.clone();
        let slot = pattern_slot(&mut scratch, base)?;
        let p = slot.get().cloned().unwrap_or_else(|| new_pattern(PatternKind::Glob, ""));
        return Some(if key.ends_with(".kind") {
            super::pattern_kind_id(p.kind).to_owned()
        } else {
            p.case_insensitive.to_string()
        });
    }
    match key {
        "m.scheme" => return Some(rule.matcher.scheme.map_or("any", Scheme::as_str).to_owned()),
        "m.response" => return Some(rule.matcher.response.is_some().to_string()),
        _ => {}
    }
    for which in ["query", "headers"] {
        if let Some((i, "negate")) = indexed(key, &format!("m.{which}.")) {
            let rows = if which == "query" {
                &rule.matcher.query
            } else {
                &rule.matcher.headers
            };
            return rows.get(i).map(|r| r.negate.to_string());
        }
    }
    let (i, field) = indexed(key, "a.")?;
    let action = rule.actions.get(i)?;
    if field == "phase" {
        return phase(action).map(|p| super::phase_id(p).to_owned());
    }
    Some(match (action, field) {
        (Action::MapLocal { source, .. }, "source") => match source {
            MapLocalSource::Inline { .. } => "inline",
            MapLocalSource::File { .. } => "file",
            MapLocalSource::Directory { .. } => "directory",
        }
        .to_owned(),
        (Action::MapRemote { scheme, .. }, "scheme") => scheme.map_or("same", Scheme::as_str).to_owned(),
        (Action::MapRemote { preserve_host, .. }, "preserve_host") => preserve_host.to_string(),
        (Action::Breakpoint { request, .. }, "request") => request.to_string(),
        (Action::Breakpoint { response, .. }, "response") => response.to_string(),
        (Action::Block { mode }, "mode") => match mode {
            BlockMode::Status { .. } => "status",
            BlockMode::Reset => "reset",
        }
        .to_owned(),
        (Action::Script { script_id }, "script") => script_id.clone(),
        (Action::WsMessage { direction, .. }, "direction") => match direction {
            None => "both",
            Some(WsDirection::Sent) => "sent",
            Some(WsDirection::Received) => "received",
        }
        .to_owned(),
        (Action::WsMessage { op, .. }, "op") => super::ws_op_kind(op).to_owned(),
        (Action::ServeFrom { source, .. }, "serve") => source.clone(),
        (Action::ServeFrom { key, .. }, "query") => key.query.to_string(),
        (Action::ServeFrom { key, .. }, "body") => key.body.to_string(),
        (Action::ServeFrom { pick, .. }, "pick") => SERVE_PICK.iter().find(|p| p.0 == *pick)?.1.to_owned(),
        (Action::ServeFrom { on_miss, .. }, "miss") => SERVE_MISS.iter().find(|p| p.0 == *on_miss)?.1.to_owned(),
        _ => return None,
    })
}

/// The phase of an action that has one.
pub fn phase(action: &Action) -> Option<Phase> {
    match action {
        Action::SetHeader { phase, .. }
        | Action::RemoveHeader { phase, .. }
        | Action::ReplaceHeaderValue { phase, .. }
        | Action::ReplaceBody { phase, .. }
        | Action::SetBody { phase, .. }
        | Action::JsonSet { phase, .. }
        | Action::JsonRemove { phase, .. }
        | Action::FormSet { phase, .. }
        | Action::FormRemove { phase, .. }
        | Action::AddHeader { phase, .. }
        | Action::Delay { phase, .. } => Some(*phase),
        _ => None,
    }
}

fn phase_mut(action: &mut Action) -> Option<&mut Phase> {
    match action {
        Action::SetHeader { phase, .. }
        | Action::RemoveHeader { phase, .. }
        | Action::ReplaceHeaderValue { phase, .. }
        | Action::ReplaceBody { phase, .. }
        | Action::SetBody { phase, .. }
        | Action::JsonSet { phase, .. }
        | Action::JsonRemove { phase, .. }
        | Action::FormSet { phase, .. }
        | Action::FormRemove { phase, .. }
        | Action::AddHeader { phase, .. }
        | Action::Delay { phase, .. } => Some(phase),
        _ => None,
    }
}

/// Sets a picker or toggle (see the module docs for the keys).
pub fn choose(rule: &mut Rule, key: &str, value: &str) -> Result<(), String> {
    if let Some(base) = key.strip_suffix(".kind") {
        let kind = PATTERN_KINDS
            .iter()
            .find(|k| k.1 == value)
            .ok_or_else(|| bad(key, value))?
            .0;
        let slot = pattern_slot(rule, base).ok_or_else(|| format!("no pattern {base}"))?;
        slot.update(|p| p.kind = kind);
        return Ok(());
    }
    if let Some(base) = key.strip_suffix(".ci") {
        let on = flag(value)?;
        let slot = pattern_slot(rule, base).ok_or_else(|| format!("no pattern {base}"))?;
        slot.update(|p| p.case_insensitive = on);
        return Ok(());
    }
    match key {
        "m.method" => {
            let m = value.to_ascii_uppercase();
            let methods = &mut rule.matcher.methods;
            match methods.iter().position(|x| *x == m) {
                Some(i) => {
                    methods.remove(i);
                }
                None => methods.push(m),
            }
            return Ok(());
        }
        "m.scheme" => {
            rule.matcher.scheme = match value {
                "any" => None,
                "http" => Some(Scheme::Http),
                "https" => Some(Scheme::Https),
                _ => return Err(bad(key, value)),
            };
            return Ok(());
        }
        "m.response" => {
            set_response(rule, flag(value)?);
            return Ok(());
        }
        _ => {}
    }
    for which in ["query", "headers"] {
        if let Some((i, "negate")) = indexed(key, &format!("m.{which}.")) {
            let on = flag(value)?;
            let row = kv_rows(rule, which).and_then(|rows| rows.get_mut(i)).ok_or("no such row")?;
            row.negate = on;
            return Ok(());
        }
    }
    let (i, field) = indexed(key, "a.").ok_or_else(|| format!("no picker {key}"))?;
    let action = rule.actions.get_mut(i).ok_or_else(|| format!("no action {i}"))?;
    if field == "phase" {
        let p = phase_mut(action).ok_or_else(|| format!("{key}: this action has no phase"))?;
        *p = match value {
            "request" => Phase::Request,
            "response" => Phase::Response,
            _ => return Err(bad(key, value)),
        };
        return Ok(());
    }
    match (action, field) {
        (Action::MapLocal { source, .. }, "source") => {
            *source = match value {
                "inline" => MapLocalSource::Inline { body: String::new() },
                "file" => MapLocalSource::File { path: String::new() },
                "directory" => MapLocalSource::Directory { root: String::new() },
                _ => return Err(bad(key, value)),
            }
        }
        (Action::MapRemote { scheme, .. }, "scheme") => {
            *scheme = match value {
                "same" => None,
                "http" => Some(Scheme::Http),
                "https" => Some(Scheme::Https),
                _ => return Err(bad(key, value)),
            }
        }
        (Action::MapRemote { preserve_host, .. }, "preserve_host") => *preserve_host = flag(value)?,
        (Action::Breakpoint { request, .. }, "request") => *request = flag(value)?,
        (Action::Breakpoint { response, .. }, "response") => *response = flag(value)?,
        (Action::Block { mode }, "mode") => {
            *mode = match value {
                "status" => BlockMode::Status { status: 403 },
                "reset" => BlockMode::Reset,
                _ => return Err(bad(key, value)),
            }
        }
        (Action::Script { script_id }, "script") => *script_id = value.to_owned(),
        (Action::WsMessage { direction, .. }, "direction") => {
            *direction = match value {
                "both" => None,
                "sent" => Some(WsDirection::Sent),
                "received" => Some(WsDirection::Received),
                _ => return Err(bad(key, value)),
            }
        }
        (Action::WsMessage { op, .. }, "op") => *op = default_ws_op(value).ok_or_else(|| bad(key, value))?,
        (Action::ServeFrom { source, .. }, "serve") => *source = value.to_owned(),
        (Action::ServeFrom { key: k, .. }, "query") => k.query = flag(value)?,
        (Action::ServeFrom { key: k, .. }, "body") => k.body = flag(value)?,
        (Action::ServeFrom { pick, .. }, "pick") => {
            *pick = SERVE_PICK
                .iter()
                .find(|p| p.1 == value)
                .ok_or_else(|| bad(key, value))?
                .0;
        }
        (Action::ServeFrom { on_miss, .. }, "miss") => {
            *on_miss = SERVE_MISS
                .iter()
                .find(|p| p.1 == value)
                .ok_or_else(|| bad(key, value))?
                .0;
        }
        _ => return Err(format!("no picker {key}")),
    }
    Ok(())
}

/// Adds the response matcher ("Only for some responses") or removes it.
pub fn set_response(rule: &mut Rule, on: bool) {
    rule.matcher.response = on.then(|| rule.matcher.response.take().unwrap_or_else(ResponseMatcher::default));
}

/// A blank query (`"query"`) or header (`"headers"`) condition.
pub fn add_row(rule: &mut Rule, which: &str) -> bool {
    let Some(rows) = kv_rows(rule, which) else { return false };
    rows.push(KvMatch {
        name: String::new(),
        value: None,
        negate: false,
    });
    true
}

pub fn remove_row(rule: &mut Rule, which: &str, i: usize) -> bool {
    match kv_rows(rule, which) {
        Some(rows) if i < rows.len() => {
            rows.remove(i);
            true
        }
        _ => false,
    }
}

/// Appends a new action of `type_id` with its defaults.
pub fn add_action(rule: &mut Rule, type_id: &str) -> bool {
    match super::default_action(type_id) {
        Some(a) => {
            rule.actions.push(a);
            true
        }
        None => false,
    }
}

pub fn remove_action(rule: &mut Rule, i: usize) -> bool {
    if i < rule.actions.len() {
        rule.actions.remove(i);
        true
    } else {
        false
    }
}

/// Moves action `i` one place up or down.
pub fn move_action(rule: &mut Rule, i: usize, down: bool) -> bool {
    let to = if down { i + 1 } else { i.wrapping_sub(1) };
    if i >= rule.actions.len() || to >= rule.actions.len() {
        return false;
    }
    move_item(&mut rule.actions, i, to);
    true
}
