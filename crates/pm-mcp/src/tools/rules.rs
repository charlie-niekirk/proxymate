//! Rule and breakpoint tools, and the Map Local, Serve From, breakpoint and rewrite shortcuts.

use super::*;

#[derive(Deserialize, JsonSchema)]
pub(super) struct ListRules {
    #[serde(default)]
    pub(super) origin: Option<OriginFilter>,
    /// 1–200, default 50.
    #[serde(default)]
    pub(super) limit: Option<u32>,
    #[serde(default)]
    pub(super) cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct RuleIdParam {
    pub(super) id: String,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct ListBreakpoints {
    /// 1–200, default 50.
    #[serde(default)]
    pub(super) limit: Option<u32>,
    #[serde(default)]
    pub(super) cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct GetBreakpoint {
    pub(super) id: BreakpointId,
    /// Body bytes to return as text (default 65536, at most 1048576; 0 leaves it out).
    #[serde(default)]
    pub(super) max_bytes: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct CreateRule {
    pub(super) name: String,
    #[serde(default = "yes")]
    pub(super) enabled: bool,
    pub(super) matcher: Matcher,
    pub(super) actions: Vec<Action>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct UpdateRule {
    pub(super) id: String,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default)]
    pub(super) matcher: Option<Matcher>,
    #[serde(default)]
    pub(super) actions: Option<Vec<Action>>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct SetRuleEnabled {
    pub(super) id: String,
    pub(super) enabled: bool,
}

/// Which requests a sugar tool's rule matches: a URL pattern, or a
/// recorded flow's method, host and path.
#[derive(Deserialize, JsonSchema)]
pub(super) struct Target {
    /// URL pattern: `https://api.example.com/v1/users/*`, `api.example.com/login`,
    /// `*.example.com/*`. `*` is a wildcard (it also crosses `/`); without a
    /// path every path matches. The query string is ignored.
    #[serde(default)]
    pub(super) url: Option<String>,
    /// Match the same method, host and path as this recorded flow.
    #[serde(default)]
    pub(super) flow_id: Option<FlowId>,
    /// Only this method (e.g. `GET`); default any (with `url`) or the flow's (with `flow_id`).
    #[serde(default)]
    pub(super) method: Option<String>,
    /// Rule name; default describes the target.
    #[serde(default)]
    pub(super) name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct CreateMapLocal {
    #[serde(flatten)]
    pub(super) target: Target,
    /// Default 200.
    #[serde(default)]
    pub(super) status: Option<u16>,
    /// The response body. With `flow_id` and no body, the flow's recorded response body.
    #[serde(default)]
    pub(super) body: Option<String>,
    /// Serve this file instead of `body`.
    #[serde(default)]
    pub(super) file: Option<String>,
    /// e.g. `application/json`; guessed from the body when unset.
    #[serde(default)]
    pub(super) content_type: Option<String>,
    #[serde(default)]
    pub(super) headers: Vec<Header>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct CreateServeFrom {
    /// A `.proxymate` session, HAR, or Charles `.chlsj`/`.chlsx` file to copy into the mock library.
    #[serde(default)]
    pub(super) path: Option<String>,
    /// Or an open document's store id (0 is the live capture).
    #[serde(default)]
    pub(super) store: Option<StoreId>,
    /// Or the id of a mock library entry (list_mock_sources).
    #[serde(default)]
    pub(super) source: Option<String>,
    /// The library entry's name (default: the file or document name).
    #[serde(default)]
    pub(super) name: Option<String>,
    /// What a request must share with a recorded one (default: the query counts, the body doesn't).
    #[serde(default)]
    pub(super) key: Option<ServeKey>,
    /// pass_through (default), not_found (404) or bad_gateway (502).
    #[serde(default)]
    pub(super) on_miss: Option<ServeMiss>,
    /// first (default), last or round_robin, when several recorded responses match.
    #[serde(default)]
    pub(super) pick: Option<ServePick>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct CreateBreakpoint {
    #[serde(flatten)]
    pub(super) target: Target,
    /// Pause the request before it is sent (default true).
    #[serde(default = "yes")]
    pub(super) request: bool,
    /// Pause the response before the client gets it (default false).
    #[serde(default)]
    pub(super) response: bool,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct Replace {
    /// Literal text to find (or a regex with `regex: true`).
    pub(super) find: String,
    pub(super) replace: String,
    #[serde(default)]
    pub(super) regex: bool,
    /// Default response.
    #[serde(default = "response_phase")]
    pub(super) phase: Phase,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct JsonEdit {
    /// RFC 6901 pointer, e.g. `/user/name`; `-` appends to an array.
    pub(super) pointer: String,
    /// The new value (default null).
    #[serde(default)]
    pub(super) value: Value,
    /// The new value as exact JSON text instead, e.g. `12345678901234567890` or `1.50`.
    #[serde(default)]
    pub(super) raw: Option<String>,
    /// Default response.
    #[serde(default = "response_phase")]
    pub(super) phase: Phase,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct JsonRemoveEdit {
    /// RFC 6901 pointer, e.g. `/user/token`.
    pub(super) pointer: String,
    /// Default response.
    #[serde(default = "response_phase")]
    pub(super) phase: Phase,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct NameValue {
    pub(super) name: String,
    pub(super) value: String,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct FormField {
    pub(super) name: String,
    pub(super) value: String,
    /// Default request.
    #[serde(default = "request_phase")]
    pub(super) phase: Phase,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct FormFieldName {
    pub(super) name: String,
    /// Default request.
    #[serde(default = "request_phase")]
    pub(super) phase: Phase,
}

pub(super) fn response_phase() -> Phase {
    Phase::Response
}

pub(super) fn request_phase() -> Phase {
    Phase::Request
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct CreateRewrite {
    #[serde(flatten)]
    pub(super) target: Target,
    /// Headers to set on the request.
    #[serde(default)]
    pub(super) set_request_headers: Vec<Header>,
    /// Headers to set on the response.
    #[serde(default)]
    pub(super) set_response_headers: Vec<Header>,
    #[serde(default)]
    pub(super) remove_request_headers: Vec<String>,
    #[serde(default)]
    pub(super) remove_response_headers: Vec<String>,
    /// Header values to add to the request, keeping existing ones (after
    /// the removals: remove a header, then add each of its new values).
    #[serde(default)]
    pub(super) add_request_headers: Vec<Header>,
    #[serde(default)]
    pub(super) add_response_headers: Vec<Header>,
    /// Change the response status.
    #[serde(default)]
    pub(super) status: Option<u16>,
    /// Query parameters to set (every occurrence replaced, or added).
    #[serde(default)]
    pub(super) set_query: Vec<NameValue>,
    #[serde(default)]
    pub(super) remove_query: Vec<String>,
    #[serde(default)]
    pub(super) replace_body: Vec<Replace>,
    /// JSON values to remove (before `json_set`).
    #[serde(default)]
    pub(super) json_remove: Vec<JsonRemoveEdit>,
    #[serde(default)]
    pub(super) json_set: Vec<JsonEdit>,
    /// Fields of a form-urlencoded body to set.
    #[serde(default)]
    pub(super) form_set: Vec<FormField>,
    #[serde(default)]
    pub(super) form_remove: Vec<FormFieldName>,
    /// Any further actions, run after the ones above.
    #[serde(default)]
    pub(super) actions: Vec<Action>,
}

/// A rule to try without saving it.
#[derive(Deserialize, JsonSchema)]
pub(super) struct DraftRule {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) matcher: Matcher,
    pub(super) actions: Vec<Action>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct TestRule {
    /// A saved rule to try. Give this or `rule`.
    #[serde(default)]
    pub(super) rule_id: Option<String>,
    /// An unsaved rule to try.
    #[serde(default)]
    pub(super) rule: Option<DraftRule>,
    /// The recorded flow to try it on.
    pub(super) flow_id: FlowId,
    /// The flow's store: 0, the live capture (default), or an opened session's.
    #[serde(default)]
    pub(super) store: Option<StoreId>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct ResolveBreakpoint {
    pub(super) id: BreakpointId,
    pub(super) resolution: BreakpointResolution,
}

pub(super) fn list_rules(ctx: &Ctx<'_>, p: ListRules) -> ToolResult {
    let rules: Vec<Rule> = ctx
        .engine
        .rules()
        .into_iter()
        .filter(|r| match p.origin {
            None => true,
            Some(OriginFilter::User) => matches!(r.origin, RuleOrigin::User),
            Some(OriginFilter::Preset) => matches!(r.origin, RuleOrigin::Preset { .. }),
            Some(OriginFilter::Agent) => matches!(r.origin, RuleOrigin::Agent { .. }),
        })
        .collect();
    let (page, next) = window(&rules, &p.cursor, limit(p.limit, 50, 200))?;
    Ok(json!({ "rules": page, "total": rules.len(), "next_cursor": next }))
}

pub(super) fn list_breakpoints(ctx: &Ctx<'_>, p: ListBreakpoints) -> ToolResult {
    let queue = ctx.engine.breakpoints();
    let (page, next) = window(&queue, &p.cursor, limit(p.limit, 50, 200))?;
    Ok(json!({ "breakpoints": page, "total": queue.len(), "next_cursor": next }))
}

pub(super) fn get_breakpoint(ctx: &Ctx<'_>, p: GetBreakpoint) -> ToolResult {
    let info = ctx.engine.breakpoint(p.id).ok_or_else(|| {
        ToolError(format!(
            "no paused flow with breakpoint id {} (resolved or timed out?)",
            p.id
        ))
    })?;
    let max = p.max_bytes.map_or(BODY_DEFAULT, |m| (m as usize).min(BODY_MAX));
    let mut out = json!(info);
    if max > 0 {
        let body = ctx.engine.breakpoint_body(p.id)?;
        let cut = &body[..body.len().min(max)];
        out["body_text"] = json!(String::from_utf8_lossy(cut));
        out["body_cut"] = json!(body.len() > max);
        if info
            .message
            .as_ref()
            .is_some_and(|m| m.opcode == pm_model::WsOpcode::Binary)
        {
            out["body_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(cut));
        }
    }
    Ok(out)
}

/// Saves a new rule as the agent's.
pub(super) fn save(ctx: &Ctx<'_>, name: String, matcher: Matcher, actions: Vec<Action>) -> ToolResult {
    if actions.is_empty() {
        return Err(ToolError("a rule needs at least one action".into()));
    }
    let rule = ctx.engine.create_rule(Rule {
        id: String::new(),
        name,
        enabled: true,
        order: 0,
        origin: agent_origin(ctx),
        matcher,
        actions,
    })?;
    Ok(json!(rule))
}

pub(super) fn create_rule(ctx: &Ctx<'_>, p: CreateRule) -> ToolResult {
    let mut rule = save(ctx, p.name, p.matcher, p.actions)?;
    if !p.enabled
        && let Some(id) = rule["id"].as_str()
    {
        rule = json!(ctx.engine.set_rule_enabled(id, false)?);
    }
    Ok(rule)
}

pub(super) fn update_rule(ctx: &Ctx<'_>, p: UpdateRule) -> ToolResult {
    let patch = RulePatch {
        name: p.name,
        enabled: p.enabled,
        matcher: p.matcher,
        actions: p.actions,
    };
    Ok(json!(ctx.engine.patch_rule(&p.id, patch)?))
}

/// Turns a URL pattern into a matcher: scheme, host (exact or glob) and
/// path (exact or glob; none matches every path).
pub(super) fn url_matcher(url: &str) -> Result<Matcher, ToolError> {
    let url = url.trim();
    if url.is_empty() {
        return Err(ToolError("url is empty".into()));
    }
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => {
            let scheme = match scheme.to_ascii_lowercase().as_str() {
                "http" => Scheme::Http,
                "https" => Scheme::Https,
                "ws" => Scheme::Ws,
                "wss" => Scheme::Wss,
                "*" => return url_matcher(rest),
                other => return Err(ToolError(format!("unsupported scheme {other:?}"))),
            };
            (Some(scheme), rest)
        }
        None => (None, url),
    };
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], Some(&rest[at..])),
        None => (rest, None),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(']') || host.ends_with(']') => match port.parse::<u16>() {
            Ok(port) => (host, Some(port)),
            Err(_) if port == "*" => (host, None),
            Err(_) => return Err(ToolError(format!("invalid port in {url:?}"))),
        },
        _ => (authority, None),
    };
    if host.is_empty() {
        return Err(ToolError(format!("no host in {url:?}")));
    }
    let wild = |s: &str| s.contains(['*', '?']);
    let host = if host == "*" {
        None
    } else if wild(host) {
        Some(Pattern::glob(host))
    } else {
        Some(Pattern {
            case_insensitive: true,
            ..Pattern::exact(host)
        })
    };
    let path = match path {
        None | Some("/*") | Some("*") => None,
        Some(p) if wild(p) => Some(Pattern {
            case_insensitive: false,
            ..Pattern::glob(p)
        }),
        Some(p) => Some(Pattern::exact(p)),
    };
    Ok(Matcher {
        scheme,
        host,
        port,
        path,
        ..Matcher::default()
    })
}

/// The matcher (and a default name) for a sugar tool's target.
pub(super) fn target_matcher(ctx: &Ctx<'_>, target: &Target, kind: &str) -> Result<(Matcher, String), ToolError> {
    let (mut matcher, described) = match (&target.url, target.flow_id) {
        (Some(url), None) => (url_matcher(url)?, url.clone()),
        (None, Some(id)) => {
            let draft = ctx.engine.draft_rule(id, PresetKind::Breakpoint)?;
            let flow = ctx
                .engine
                .get_flow(id)
                .ok_or_else(|| ToolError(format!("no flow {id}")))?;
            (draft.matcher, format!("{} {}{}", flow.method, flow.host, flow.path))
        }
        (Some(_), Some(_)) => return Err(ToolError("give either url or flow_id, not both".into())),
        (None, None) => {
            return Err(ToolError(
                "give url (a URL pattern) or flow_id (a recorded flow)".into(),
            ));
        }
    };
    if let Some(method) = &target.method {
        matcher.methods = vec![method.to_ascii_uppercase()];
    }
    let method = matcher.methods.first().map(|m| format!("{m} ")).unwrap_or_default();
    let name = target.name.clone().unwrap_or_else(|| match &target.url {
        Some(_) => format!("{kind} · {method}{described}"),
        None => format!("{kind} · {described}"),
    });
    Ok((matcher, name))
}

pub(super) fn create_map_local(ctx: &Ctx<'_>, p: CreateMapLocal) -> ToolResult {
    let (matcher, name) = target_matcher(ctx, &p.target, "Map Local")?;
    let source = match (p.file, p.body) {
        (Some(_), Some(_)) => return Err(ToolError("give either body or file, not both".into())),
        (Some(path), None) => MapLocalSource::File { path },
        (None, Some(body)) => MapLocalSource::Inline { body },
        (None, None) => match p.target.flow_id {
            // Start from what the flow answered.
            Some(id) => match ctx
                .engine
                .draft_rule(id, PresetKind::MapLocal)?
                .actions
                .into_iter()
                .next()
            {
                Some(Action::MapLocal { source, .. }) => source,
                _ => MapLocalSource::Inline { body: String::new() },
            },
            None => MapLocalSource::Inline { body: String::new() },
        },
    };
    let content_type = p.content_type.or_else(|| match &source {
        MapLocalSource::Inline { body } => {
            let t = body.trim_start();
            if t.starts_with('{') || t.starts_with('[') {
                Some("application/json".into())
            } else if t.starts_with('<') {
                Some("text/html; charset=utf-8".into())
            } else {
                Some("text/plain; charset=utf-8".into())
            }
        }
        _ => None,
    });
    let action = Action::MapLocal {
        source,
        status: p.status.unwrap_or(200),
        headers: p.headers,
        content_type,
    };
    save(ctx, name, matcher, vec![action])
}

pub(super) fn create_serve_from(ctx: &Ctx<'_>, p: CreateServeFrom) -> ToolResult {
    let source = match (p.path, p.store, p.source) {
        (Some(path), None, None) => ctx
            .engine
            .create_mock_source_from_file(std::path::Path::new(&path), p.name)?,
        (None, Some(store), None) => ctx.engine.create_mock_source(store, p.name)?,
        (None, None, Some(id)) => ctx
            .engine
            .mock_sources()
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| ToolError(format!("there is no mock source {id}; see list_mock_sources")))?,
        _ => return Err(ToolError("give exactly one of path, store or source".into())),
    };
    let mut draft = ctx.engine.draft_serve_from_rule(&source.id)?;
    draft.actions = vec![Action::ServeFrom {
        source: source.id.clone(),
        key: p.key.unwrap_or_default(),
        on_miss: p.on_miss.unwrap_or_default(),
        pick: p.pick.unwrap_or_default(),
    }];
    let rule = save(ctx, draft.name, draft.matcher, draft.actions)?;
    Ok(json!({ "rule": rule, "source": source }))
}

pub(super) fn create_breakpoint(ctx: &Ctx<'_>, p: CreateBreakpoint) -> ToolResult {
    if !p.request && !p.response {
        return Err(ToolError("pause the request, the response or both".into()));
    }
    let (matcher, name) = target_matcher(ctx, &p.target, "Breakpoint")?;
    save(
        ctx,
        name,
        matcher,
        vec![Action::Breakpoint {
            request: p.request,
            response: p.response,
        }],
    )
}

pub(super) fn create_rewrite(ctx: &Ctx<'_>, p: CreateRewrite) -> ToolResult {
    let (matcher, name) = target_matcher(ctx, &p.target, "Rewrite")?;
    let mut actions = Vec::new();
    for (phase, headers) in [
        (Phase::Request, p.set_request_headers),
        (Phase::Response, p.set_response_headers),
    ] {
        actions.extend(headers.into_iter().map(|h| Action::SetHeader {
            phase,
            name: h.name,
            value: h.value,
        }));
    }
    for (phase, names) in [
        (Phase::Request, p.remove_request_headers),
        (Phase::Response, p.remove_response_headers),
    ] {
        actions.extend(names.into_iter().map(|name| Action::RemoveHeader { phase, name }));
    }
    for (phase, headers) in [
        (Phase::Request, p.add_request_headers),
        (Phase::Response, p.add_response_headers),
    ] {
        actions.extend(headers.into_iter().map(|h| Action::AddHeader {
            phase,
            name: h.name,
            value: h.value,
        }));
    }
    if let Some(status) = p.status {
        actions.push(Action::SetStatus { status });
    }
    actions.extend(p.set_query.into_iter().map(|q| Action::SetQuery {
        name: q.name,
        value: q.value,
    }));
    actions.extend(p.remove_query.into_iter().map(|name| Action::RemoveQuery { name }));
    actions.extend(p.replace_body.into_iter().map(|r| Action::ReplaceBody {
        phase: r.phase,
        pattern: if r.regex {
            Pattern::regex(r.find)
        } else {
            Pattern::exact(r.find)
        },
        replacement: r.replace,
    }));
    actions.extend(p.json_remove.into_iter().map(|j| Action::JsonRemove {
        phase: j.phase,
        pointer: j.pointer,
    }));
    actions.extend(p.json_set.into_iter().map(|j| Action::JsonSet {
        phase: j.phase,
        pointer: j.pointer,
        value: j.value,
        raw: j.raw,
    }));
    actions.extend(p.form_set.into_iter().map(|f| Action::FormSet {
        phase: f.phase,
        name: f.name,
        value: f.value,
    }));
    actions.extend(p.form_remove.into_iter().map(|f| Action::FormRemove {
        phase: f.phase,
        name: f.name,
    }));
    actions.extend(p.actions);
    save(ctx, name, matcher, actions)
}

pub(super) async fn test_rule(ctx: &Ctx<'_>, p: TestRule) -> ToolResult {
    let rule = match (p.rule_id, p.rule) {
        (Some(id), None) => ctx
            .engine
            .rule(&id)
            .ok_or_else(|| ToolError(format!("no rule with id {id}")))?,
        (None, Some(draft)) => Rule {
            id: String::new(),
            name: draft.name.unwrap_or_else(|| "Dry run".into()),
            enabled: true,
            order: 0,
            origin: agent_origin(ctx),
            matcher: draft.matcher,
            actions: draft.actions,
        },
        _ => {
            return Err(ToolError(
                "give either rule_id (a saved rule) or rule (an unsaved one)".into(),
            ));
        }
    };
    let store = p.store.unwrap_or(LIVE_STORE);
    Ok(json!(ctx.engine.test_rule_in(store, &rule, p.flow_id, None).await?))
}
