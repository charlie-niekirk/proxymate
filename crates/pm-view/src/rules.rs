//! Rules helpers (`lib/rules.ts`, G7): the "Add action" list with each
//! action's defaults, the one-line summaries the Rules window lists, origin
//! labels, the toolbar's throttle presets, status ranges, the list's
//! reordering, and [`edit`], which changes a rule field by field (the rule
//! form's inputs and pickers, and the dev remote, address fields by key).

use std::collections::HashMap;

use pm_model::{
    Action, BlockMode, GLOBAL_THROTTLE_RULE_ID, MapLocalSource, Matcher, Pattern, PatternKind, Phase, PresetKind, Rule,
    RuleGroup, RuleOrigin, RulePatch, ServeKey, ServeMiss, ServePick, StatusRange, WsDirection, WsOp,
};

pub use pm_model::{NO_CACHE_RULE_ID, NO_COOKIES_RULE_ID};

/// The toolbar's network conditions rule.
pub const THROTTLE_RULE_ID: &str = GLOBAL_THROTTLE_RULE_ID;

/// Built-in rules are managed elsewhere (the toolbar) and can't be deleted:
/// the Rules window hides them.
pub fn is_builtin(id: &str) -> bool {
    id.starts_with("builtin-")
}

/// A blank, unsaved user rule.
pub fn new_rule() -> Rule {
    Rule {
        id: String::new(),
        name: "New rule".into(),
        enabled: true,
        order: 0,
        origin: RuleOrigin::User,
        group: None,
        matcher: Matcher::default(),
        actions: Vec::new(),
    }
}

/// `newPattern`: a glob is case-insensitive by default.
pub fn new_pattern(kind: PatternKind, value: &str) -> Pattern {
    Pattern {
        kind,
        value: value.to_owned(),
        case_insensitive: kind == PatternKind::Glob,
    }
}

/// One entry of the "Add action" menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionType {
    /// The action's `type` tag.
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
}

const fn at(id: &'static str, label: &'static str, group: &'static str) -> ActionType {
    ActionType { id, label, group }
}

/// Every action, in the order the "Add action" menu shows them.
pub const ACTION_TYPES: [ActionType; 23] = [
    at("SetHeader", "Set header", "Headers"),
    at("AddHeader", "Add header value", "Headers"),
    at("RemoveHeader", "Remove header", "Headers"),
    at("ReplaceHeaderValue", "Replace in header", "Headers"),
    at("SetQuery", "Set query parameter", "Query"),
    at("RemoveQuery", "Remove query parameter", "Query"),
    at("SetBody", "Set body", "Body"),
    at("ReplaceBody", "Replace in body", "Body"),
    at("JsonSet", "JSON set", "Body"),
    at("JsonRemove", "JSON remove", "Body"),
    at("FormSet", "Form field set", "Body"),
    at("FormRemove", "Form field remove", "Body"),
    at("SetStatus", "Set status", "Status"),
    at("MapLocal", "Map Local", "Routing"),
    at("MapRemote", "Map Remote", "Routing"),
    at("ServeFrom", "Serve from recording", "Routing"),
    at("Breakpoint", "Breakpoint", "Flow control"),
    at("Delay", "Delay", "Flow control"),
    at("Throttle", "Throttle", "Flow control"),
    at("Block", "Block", "Flow control"),
    at("Script", "Run script", "Scripts"),
    at("WsMessage", "WebSocket message", "WebSocket"),
    at("Comment", "Comment on flow", "Annotate"),
];

/// The "Add action" menu's groups, in order.
pub fn action_groups() -> Vec<&'static str> {
    let mut groups: Vec<&'static str> = Vec::new();
    for a in ACTION_TYPES {
        if !groups.contains(&a.group) {
            groups.push(a.group);
        }
    }
    groups
}

/// An action's `type` tag.
pub fn action_type(a: &Action) -> &'static str {
    match a {
        Action::SetHeader { .. } => "SetHeader",
        Action::RemoveHeader { .. } => "RemoveHeader",
        Action::ReplaceHeaderValue { .. } => "ReplaceHeaderValue",
        Action::SetQuery { .. } => "SetQuery",
        Action::RemoveQuery { .. } => "RemoveQuery",
        Action::ReplaceBody { .. } => "ReplaceBody",
        Action::SetBody { .. } => "SetBody",
        Action::JsonSet { .. } => "JsonSet",
        Action::JsonRemove { .. } => "JsonRemove",
        Action::FormSet { .. } => "FormSet",
        Action::FormRemove { .. } => "FormRemove",
        Action::AddHeader { .. } => "AddHeader",
        Action::SetStatus { .. } => "SetStatus",
        Action::MapLocal { .. } => "MapLocal",
        Action::MapRemote { .. } => "MapRemote",
        Action::Breakpoint { .. } => "Breakpoint",
        Action::Delay { .. } => "Delay",
        Action::Throttle { .. } => "Throttle",
        Action::Block { .. } => "Block",
        Action::Script { .. } => "Script",
        Action::WsMessage { .. } => "WsMessage",
        Action::ServeFrom { .. } => "ServeFrom",
        Action::Comment { .. } => "Comment",
    }
}

pub fn action_label(type_id: &str) -> &str {
    ACTION_TYPES
        .iter()
        .find(|a| a.id == type_id)
        .map_or(type_id, |a| a.label)
}

/// A new action of `type_id` with sensible defaults (`defaultAction`).
pub fn default_action(type_id: &str) -> Option<Action> {
    use Phase::{Request, Response};
    let s = String::new;
    Some(match type_id {
        "SetHeader" => Action::SetHeader {
            phase: Response,
            name: "X-Proxymate".into(),
            value: "1".into(),
        },
        "AddHeader" => Action::AddHeader {
            phase: Response,
            name: s(),
            value: s(),
        },
        "RemoveHeader" => Action::RemoveHeader { phase: Request, name: s() },
        "ReplaceHeaderValue" => Action::ReplaceHeaderValue {
            phase: Request,
            name: s(),
            pattern: new_pattern(PatternKind::Exact, ""),
            replacement: s(),
        },
        "SetQuery" => Action::SetQuery { name: s(), value: s() },
        "RemoveQuery" => Action::RemoveQuery { name: s() },
        "SetBody" => Action::SetBody {
            phase: Response,
            body: s(),
            content_type: None,
        },
        "ReplaceBody" => Action::ReplaceBody {
            phase: Response,
            pattern: new_pattern(PatternKind::Exact, ""),
            replacement: s(),
        },
        "JsonSet" => Action::JsonSet {
            phase: Response,
            pointer: "/".into(),
            value: serde_json::Value::Null,
            raw: Some("null".into()),
        },
        "JsonRemove" => Action::JsonRemove {
            phase: Response,
            pointer: "/".into(),
        },
        "FormSet" => Action::FormSet {
            phase: Request,
            name: s(),
            value: s(),
        },
        "FormRemove" => Action::FormRemove { phase: Request, name: s() },
        "SetStatus" => Action::SetStatus { status: 200 },
        "MapLocal" => Action::MapLocal {
            source: MapLocalSource::Inline { body: s() },
            status: 200,
            headers: Vec::new(),
            content_type: Some("application/json".into()),
        },
        "MapRemote" => Action::MapRemote {
            scheme: None,
            host: None,
            port: None,
            path: None,
            preserve_host: false,
        },
        "Breakpoint" => Action::Breakpoint {
            request: true,
            response: false,
        },
        "Delay" => Action::Delay { phase: Request, ms: 1000 },
        "Throttle" => Action::Throttle {
            download_kbps: 780,
            upload_kbps: 330,
            latency_ms: 100,
        },
        "Block" => Action::Block {
            mode: BlockMode::Status { status: 403 },
        },
        "Script" => Action::Script { script_id: s() },
        "WsMessage" => Action::WsMessage {
            direction: None,
            filter: None,
            op: default_ws_op("replace")?,
        },
        "ServeFrom" => Action::ServeFrom {
            source: s(),
            key: ServeKey::default(),
            on_miss: ServeMiss::PassThrough,
            pick: ServePick::First,
        },
        "Comment" => Action::Comment { text: s() },
        _ => return None,
    })
}

/// The WebSocket message action's ops (`WS_OP_LABELS`), in menu order.
pub const WS_OPS: [(&str, &str); 6] = [
    ("replace", "Replace"),
    ("json_set", "JSON set"),
    ("json_remove", "JSON remove"),
    ("drop", "Drop"),
    ("delay", "Delay"),
    ("breakpoint", "Breakpoint"),
];

pub fn ws_op_kind(op: &WsOp) -> &'static str {
    match op {
        WsOp::Drop => "drop",
        WsOp::Replace { .. } => "replace",
        WsOp::JsonSet { .. } => "json_set",
        WsOp::JsonRemove { .. } => "json_remove",
        WsOp::Delay { .. } => "delay",
        WsOp::Breakpoint => "breakpoint",
    }
}

pub fn default_ws_op(kind: &str) -> Option<WsOp> {
    Some(match kind {
        "drop" => WsOp::Drop,
        "replace" => WsOp::Replace {
            pattern: new_pattern(PatternKind::Exact, ""),
            replacement: String::new(),
        },
        "json_set" => WsOp::JsonSet {
            pointer: "/".into(),
            value: serde_json::Value::Null,
            raw: Some("null".into()),
        },
        "json_remove" => WsOp::JsonRemove { pointer: "/".into() },
        "delay" => WsOp::Delay { ms: 1000 },
        "breakpoint" => WsOp::Breakpoint,
        _ => return None,
    })
}

pub const SERVE_MISS: [(ServeMiss, &str, &str); 3] = [
    (ServeMiss::PassThrough, "pass_through", "Pass through"),
    (ServeMiss::NotFound, "not_found", "Answer 404"),
    (ServeMiss::BadGateway, "bad_gateway", "Answer 502"),
];

pub const SERVE_PICK: [(ServePick, &str, &str); 3] = [
    (ServePick::First, "first", "The first recorded"),
    (ServePick::Last, "last", "The last recorded"),
    (ServePick::RoundRobin, "round_robin", "Each in turn"),
];

pub const PATTERN_KINDS: [(PatternKind, &str, &str); 3] = [
    (PatternKind::Exact, "exact", "Exact"),
    (PatternKind::Glob, "glob", "Glob"),
    (PatternKind::Regex, "regex", "Regex"),
];

pub fn pattern_kind_id(kind: PatternKind) -> &'static str {
    PATTERN_KINDS.iter().find(|k| k.0 == kind).map_or("glob", |k| k.1)
}

/// The methods the matcher offers as toggles.
pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

pub fn phase_id(phase: Phase) -> &'static str {
    match phase {
        Phase::Request => "request",
        Phase::Response => "response",
    }
}

fn direction_id(d: WsDirection) -> &'static str {
    match d {
        WsDirection::Sent => "sent",
        WsDirection::Received => "received",
    }
}

pub fn pattern_label(p: Option<&Pattern>) -> String {
    let Some(p) = p else { return "any".into() };
    let flag = if p.case_insensitive { " (Aa)" } else { "" };
    match p.kind {
        PatternKind::Exact => format!("= {}{flag}", p.value),
        PatternKind::Glob => format!("{}{flag}", p.value),
        PatternKind::Regex => format!("/{}/{}", p.value, if p.case_insensitive { "i" } else { "" }),
    }
}

/// One line describing what a rule matches, for the rules list.
pub fn matcher_summary(m: &Matcher) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !m.methods.is_empty() {
        parts.push(m.methods.join("|"));
    }
    if let Some(s) = m.scheme {
        parts.push(format!("{}://", s.as_str()));
    }
    parts.push(match &m.host {
        Some(h) => strip_exact(pattern_label(Some(h))),
        None => "*".into(),
    });
    if let Some(port) = m.port {
        parts.push(format!(":{port}"));
    }
    if let Some(p) = &m.path {
        parts.push(strip_exact(pattern_label(Some(p))));
    }
    let extras = m.query.len() + m.headers.len() + usize::from(m.client_app.is_some()) + usize::from(m.response.is_some());
    if extras > 0 {
        parts.push(format!("+{extras}"));
    }
    parts.join(" ")
}

fn strip_exact(label: String) -> String {
    label.strip_prefix("= ").map(str::to_owned).unwrap_or(label)
}

/// A `JsonSet`'s value as JSON text: its exact `raw`, else `value`.
pub fn json_set_text(value: &serde_json::Value, raw: Option<&str>) -> String {
    raw.map_or_else(|| value.to_string(), str::to_owned)
}

/// Names by id: scripts, or mock library entries.
pub type Names = HashMap<String, String>;

/// A short description of an action, for the rules list.
pub fn action_summary(a: &Action, scripts: &Names, mocks: &Names) -> String {
    match a {
        Action::SetHeader { phase, name, value } => format!("{} {name}: {value}", phase_id(*phase)),
        Action::AddHeader { phase, name, value } => format!("{} + {name}: {value}", phase_id(*phase)),
        Action::RemoveHeader { phase, name } => format!("remove {} {name}", phase_id(*phase)),
        Action::ReplaceHeaderValue {
            phase,
            name,
            pattern,
            replacement,
        } => format!(
            "{} {name} {} → {replacement}",
            phase_id(*phase),
            pattern_label(Some(pattern))
        ),
        Action::SetQuery { name, value } => format!("?{name}={value}"),
        Action::RemoveQuery { name } => format!("remove ?{name}"),
        Action::SetBody { phase, .. } => format!("{} body", phase_id(*phase)),
        Action::ReplaceBody {
            phase,
            pattern,
            replacement,
        } => format!(
            "{} body {} → {replacement}",
            phase_id(*phase),
            pattern_label(Some(pattern))
        ),
        Action::JsonSet {
            phase,
            pointer,
            value,
            raw,
        } => format!(
            "{} {pointer} = {}",
            phase_id(*phase),
            json_set_text(value, raw.as_deref())
        ),
        Action::JsonRemove { phase, pointer } => format!("{} remove {pointer}", phase_id(*phase)),
        Action::FormSet { phase, name, value } => format!("{} form {name}={value}", phase_id(*phase)),
        Action::FormRemove { phase, name } => format!("{} remove form {name}", phase_id(*phase)),
        Action::SetStatus { status } => format!("status {status}"),
        Action::MapLocal { source, .. } => format!(
            "Map Local {}",
            match source {
                MapLocalSource::Inline { .. } => "inline".to_owned(),
                MapLocalSource::File { path } => path.clone(),
                MapLocalSource::Directory { root } => format!("{root}/"),
            }
        ),
        Action::MapRemote {
            scheme,
            host,
            port,
            path,
            ..
        } => format!(
            "→ {}{}{}{}",
            scheme.map(|s| format!("{}://", s.as_str())).unwrap_or_default(),
            host.as_deref().unwrap_or("same host"),
            port.map(|p| format!(":{p}")).unwrap_or_default(),
            path.as_deref().unwrap_or("")
        ),
        Action::Breakpoint { request, response } => {
            let at: Vec<&str> = [request.then_some("request"), response.then_some("response")]
                .into_iter()
                .flatten()
                .collect();
            format!("breakpoint {}", if at.is_empty() { "off".into() } else { at.join(" + ") })
        }
        Action::Delay { phase, ms } => format!("delay {} {ms} ms", phase_id(*phase)),
        Action::Throttle {
            download_kbps,
            upload_kbps,
            latency_ms,
        } => format!("throttle {download_kbps}/{upload_kbps} kbps, {latency_ms} ms"),
        Action::Block { mode } => match mode {
            BlockMode::Status { status } => format!("block {status}"),
            BlockMode::Reset => "block (reset)".into(),
        },
        Action::Script { script_id } => format!(
            "script {}",
            scripts.get(script_id).cloned().unwrap_or_else(|| if script_id.is_empty() {
                "(none)".into()
            } else {
                script_id.clone()
            })
        ),
        Action::ServeFrom {
            source,
            on_miss,
            pick,
            ..
        } => {
            let name = mocks.get(source).cloned().unwrap_or_else(|| {
                if source.is_empty() {
                    "(none)".into()
                } else {
                    "(missing recording)".into()
                }
            });
            let pick = match pick {
                ServePick::First => String::new(),
                ServePick::Last => " (last)".into(),
                ServePick::RoundRobin => " (round-robin)".into(),
            };
            let miss = match on_miss {
                ServeMiss::PassThrough => "",
                ServeMiss::NotFound => ", else 404",
                ServeMiss::BadGateway => ", else 502",
            };
            format!("serve from {name}{pick}{miss}")
        }
        Action::Comment { text } => format!("comment “{}”", text.trim()),
        Action::WsMessage { direction, filter, op } => {
            let filter = filter
                .as_ref()
                .map(|f| format!(" {}", pattern_label(Some(f))))
                .unwrap_or_default();
            let at = format!("{}{filter}", direction.map_or("ws", direction_id));
            match op {
                WsOp::Replace { pattern, replacement } => {
                    format!("{at} {} → {replacement}", pattern_label(Some(pattern)))
                }
                WsOp::JsonSet { pointer, value, raw } => {
                    format!("{at} {pointer} = {}", json_set_text(value, raw.as_deref()))
                }
                WsOp::JsonRemove { pointer } => format!("{at} remove {pointer}"),
                WsOp::Delay { ms } => format!("{at} delay {ms} ms"),
                WsOp::Drop => format!("{at} drop"),
                WsOp::Breakpoint => format!("{at} breakpoint"),
            }
        }
    }
}

/// A rule's actions summarised, or "no actions".
pub fn actions_summary(rule: &Rule, scripts: &Names, mocks: &Names) -> String {
    if rule.actions.is_empty() {
        return "no actions".into();
    }
    rule.actions
        .iter()
        .map(|a| action_summary(a, scripts, mocks))
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn preset_label(preset: PresetKind) -> &'static str {
    match preset {
        PresetKind::MapLocal => "Map Local",
        PresetKind::Breakpoint => "Breakpoint",
        PresetKind::Rewrite => "Rewrite",
        PresetKind::MapRemote => "Map Remote",
        PresetKind::Block => "Block",
        PresetKind::Throttle => "Throttle",
        PresetKind::ServeFrom => "Serve from",
        PresetKind::OpenApi => "OpenAPI",
    }
}

/// The badge text for a rule's origin: none for the user's own.
pub fn origin_label(origin: &RuleOrigin) -> Option<String> {
    match origin {
        RuleOrigin::User => None,
        RuleOrigin::Preset { preset } => Some(preset_label(*preset).into()),
        RuleOrigin::Agent { client, .. } => Some(format!("AI · {client}")),
    }
}

/// The badge's tooltip.
pub fn origin_tip(origin: &RuleOrigin) -> Option<String> {
    match origin {
        RuleOrigin::User => None,
        RuleOrigin::Preset { preset } => Some(format!("Created from the {} preset", preset_label(*preset))),
        RuleOrigin::Agent { client, .. } => Some(format!("Created by the MCP client {client}")),
    }
}

/// Network conditions offered by the toolbar's throttle menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThrottlePreset {
    pub name: String,
    pub download_kbps: u32,
    pub upload_kbps: u32,
    pub latency_ms: u32,
}

/// `THROTTLE_PRESETS`: name, down and up (kbps), latency (ms).
pub const THROTTLE_PRESETS: [(&str, u32, u32, u32); 5] = [
    ("LTE", 12_000, 6_000, 40),
    ("Fast 3G", 1_600, 750, 150),
    ("3G", 780, 330, 100),
    ("Edge", 240, 200, 400),
    ("Very bad network", 50, 20, 800),
];

pub fn throttle_preset(name: &str) -> Option<ThrottlePreset> {
    THROTTLE_PRESETS
        .iter()
        .find(|p| p.0 == name)
        .map(|&(name, download_kbps, upload_kbps, latency_ms)| ThrottlePreset {
            name: name.into(),
            download_kbps,
            upload_kbps,
            latency_ms,
        })
}

/// `12 Mbps`, `780 kbps`.
pub fn kbps(n: u32) -> String {
    if n >= 1000 {
        let mbps = f64::from(n) / 1000.;
        format!("{mbps} Mbps")
    } else {
        format!("{n} kbps")
    }
}

/// The throttle menu's detail for a preset: "12 Mbps ↓ · 6 Mbps ↑ · 40 ms".
pub fn throttle_detail(p: &ThrottlePreset) -> String {
    format!(
        "{} ↓ · {} ↑ · {} ms",
        kbps(p.download_kbps),
        kbps(p.upload_kbps),
        p.latency_ms
    )
}

/// The preset the global throttle rule is set to, "Custom" for other
/// speeds, or `None` when it is off.
pub fn active_throttle(rule: Option<&Rule>) -> Option<ThrottlePreset> {
    let rule = rule.filter(|r| r.enabled)?;
    let (download_kbps, upload_kbps, latency_ms) = rule.actions.iter().find_map(|a| match a {
        Action::Throttle {
            download_kbps,
            upload_kbps,
            latency_ms,
        } => Some((*download_kbps, *upload_kbps, *latency_ms)),
        _ => None,
    })?;
    let name = THROTTLE_PRESETS
        .iter()
        .find(|p| (p.1, p.2, p.3) == (download_kbps, upload_kbps, latency_ms))
        .map_or("Custom", |p| p.0);
    Some(ThrottlePreset {
        name: name.into(),
        download_kbps,
        upload_kbps,
        latency_ms,
    })
}

/// The global throttle rule's patch for `preset` (off with `None`); the
/// actions are only replaced when a preset is picked (`withThrottle`).
pub fn throttle_patch(preset: Option<&ThrottlePreset>) -> RulePatch {
    match preset {
        None => RulePatch {
            enabled: Some(false),
            ..RulePatch::default()
        },
        Some(p) => RulePatch {
            enabled: Some(true),
            actions: Some(vec![Action::Throttle {
                download_kbps: p.download_kbps,
                upload_kbps: p.upload_kbps,
                latency_ms: p.latency_ms,
            }]),
            ..RulePatch::default()
        },
    }
}

/// Moves `from` to `to` (both indices into the same list).
pub fn move_item<T>(list: &mut Vec<T>, from: usize, to: usize) {
    if from >= list.len() {
        return;
    }
    let item = list.remove(from);
    list.insert(to.min(list.len()), item);
}

/// Parses "200-299, 404, 5xx" into status ranges; `None` if any part is
/// invalid.
pub fn parse_status_ranges(text: &str) -> Option<Vec<StatusRange>> {
    let mut out = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let three = |s: &str| (s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse::<u16>().ok())?;
        let lower = part.to_ascii_lowercase();
        if let Some(d) = lower.strip_suffix("xx") {
            let d: u16 = (d.len() == 1).then(|| d.parse().ok())??;
            out.push(StatusRange {
                from: d * 100,
                to: d * 100 + 99,
            });
            continue;
        }
        let (from, to) = match part.split_once(['-', '–']) {
            Some((a, b)) => (three(a.trim())?, three(b.trim())?),
            None => {
                let n = three(part)?;
                (n, n)
            }
        };
        if to < from {
            return None;
        }
        out.push(StatusRange { from, to });
    }
    Some(out)
}

pub fn format_status_ranges(ranges: &[StatusRange]) -> String {
    ranges
        .iter()
        .map(|r| {
            if r.from == r.to {
                r.from.to_string()
            } else if r.from % 100 == 0 && r.to == r.from + 99 {
                format!("{}xx", r.from / 100)
            } else {
                format!("{}-{}", r.from, r.to)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Why `text` isn't JSON, if it isn't. Only checked: the text itself is
/// kept (big numbers and `1.50` stay as typed).
pub fn json_error(text: &str) -> Option<String> {
    serde_json::from_str::<serde::de::IgnoredAny>(text)
        .err()
        .map(|e| e.to_string())
}

// ---- The list ----

/// The group a rule is listed under: its group, if that group exists.
pub fn section<'a>(rule: &'a Rule, groups: &[RuleGroup]) -> Option<&'a str> {
    rule.group.as_deref().filter(|g| groups.iter().any(|x| x.id == *g))
}

/// The rules of one section (`None`: the ungrouped ones), in order.
pub fn members<'a>(rules: &'a [Rule], groups: &[RuleGroup], group: Option<&str>) -> Vec<&'a Rule> {
    rules.iter().filter(|r| section(r, groups) == group).collect()
}

/// "3" when all of a group's rules are on, else "2/3".
pub fn group_count(members: &[&Rule]) -> String {
    let on = members.iter().filter(|r| r.enabled).count();
    if on == members.len() {
        members.len().to_string()
    } else {
        format!("{on}/{}", members.len())
    }
}

/// The ids in order after dropping `moving` on `target`: it takes the
/// target's place (`dropOnRule`).
pub fn order_after_drop(rules: &[Rule], moving: &str, target: &str) -> Option<Vec<String>> {
    if moving == target {
        return None;
    }
    let mut ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
    let to = ids.iter().position(|id| id == target)?;
    let from = ids.iter().position(|id| id == moving)?;
    let id = ids.remove(from);
    ids.insert(to, id);
    Some(ids)
}

/// The ids in order after moving `id` to the top or bottom (the engine
/// keeps each rule in its section, so that's the section's top or bottom).
pub fn order_to_edge(rules: &[Rule], id: &str, top: bool) -> Vec<String> {
    let mut ids: Vec<String> = rules.iter().map(|r| r.id.clone()).filter(|x| x != id).collect();
    if top {
        ids.insert(0, id.to_owned());
    } else {
        ids.push(id.to_owned());
    }
    ids
}

/// A copy of `rule` to save as a new user rule ("<name> copy").
pub fn duplicate(rule: &Rule) -> Rule {
    let name = if rule.name.is_empty() { "Untitled" } else { &rule.name };
    Rule {
        id: String::new(),
        name: format!("{name} copy"),
        origin: RuleOrigin::User,
        ..rule.clone()
    }
}

/// The group ids in order after moving `id` by one place.
pub fn group_order_after_move(groups: &[RuleGroup], id: &str, down: bool) -> Option<Vec<String>> {
    let index = groups.iter().position(|g| g.id == id)?;
    let to = if down {
        (index + 1 < groups.len()).then_some(index + 1)?
    } else {
        index.checked_sub(1)?
    };
    let mut ids: Vec<String> = groups.iter().map(|g| g.id.clone()).collect();
    move_item(&mut ids, index, to);
    Some(ids)
}

/// The group ids after dropping group `moving` on group `target` (it goes
/// before it).
pub fn group_order_after_drop(groups: &[RuleGroup], moving: &str, target: &str) -> Option<Vec<String>> {
    if moving == target {
        return None;
    }
    let mut ids: Vec<String> = groups.iter().map(|g| g.id.clone()).filter(|id| id != moving).collect();
    let to = ids.iter().position(|id| id == target)?;
    ids.insert(to, moving.to_owned());
    Some(ids)
}

/// The JSON a rule is copied as (pretty, as the Tauri UI's `JSON.stringify(rule, null, 2)`).
pub fn rule_json(rule: &Rule) -> String {
    serde_json::to_string_pretty(rule).unwrap_or_default()
}

pub mod edit;

#[cfg(test)]
mod tests;
