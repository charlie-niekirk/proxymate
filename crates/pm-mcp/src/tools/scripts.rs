//! Script tools (M14).

use super::*;

#[derive(Deserialize, JsonSchema)]
pub(super) struct ListScripts {
    #[serde(default)]
    pub(super) kind: Option<ScriptKind>,
    #[serde(default)]
    pub(super) origin: Option<OriginFilter>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct ScriptIdParam {
    pub(super) id: String,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct CreateScript {
    pub(super) name: String,
    pub(super) kind: ScriptKind,
    /// An ES module exporting the kind's entry points.
    pub(super) source: String,
    #[serde(default = "yes")]
    pub(super) enabled: bool,
    /// View scripts: which bodies get the view (default: every response and request body).
    #[serde(default)]
    pub(super) view: Option<ViewConfig>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct UpdateScript {
    pub(super) id: String,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) source: Option<String>,
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default)]
    pub(super) view: Option<ViewConfig>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct SetScriptEnabled {
    pub(super) id: String,
    pub(super) enabled: bool,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct TestScript {
    /// The recorded flow to run it against.
    pub(super) flow_id: FlowId,
    /// A saved script to test.
    #[serde(default)]
    pub(super) script_id: Option<String>,
    /// Unsaved source (with `kind`), tested instead of the saved script.
    #[serde(default)]
    pub(super) source: Option<String>,
    #[serde(default)]
    pub(super) kind: Option<ScriptKind>,
    /// View scripts: which body to view (default response).
    #[serde(default)]
    pub(super) part: Option<BodyPart>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct GetFlowViews {
    pub(super) id: FlowId,
    /// Default response.
    #[serde(default)]
    pub(super) part: Option<BodyPart>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct GetScriptConsole {
    /// Only entries with a larger seq.
    #[serde(default)]
    pub(super) since: Option<u64>,
    /// 1–2000, default 200 (the newest ones).
    #[serde(default)]
    pub(super) limit: Option<u32>,
}

pub(super) fn list_scripts(ctx: &Ctx<'_>, p: ListScripts) -> ToolResult {
    let scripts: Vec<Script> = ctx
        .engine
        .scripts()
        .into_iter()
        .filter(|s| p.kind.is_none_or(|k| k == s.kind))
        .filter(|s| match p.origin {
            None => true,
            Some(OriginFilter::User) => matches!(s.origin, RuleOrigin::User),
            Some(OriginFilter::Preset) => matches!(s.origin, RuleOrigin::Preset { .. }),
            Some(OriginFilter::Agent) => matches!(s.origin, RuleOrigin::Agent { .. }),
        })
        .collect();
    Ok(json!({ "scripts": scripts, "total": scripts.len() }))
}

pub(super) async fn create_script(ctx: &Ctx<'_>, p: CreateScript) -> ToolResult {
    let script = Script {
        id: String::new(),
        name: p.name,
        kind: p.kind,
        enabled: p.enabled,
        source: p.source,
        origin: agent_origin(ctx),
        view: match p.kind {
            ScriptKind::View => Some(p.view.unwrap_or_default()),
            _ => None,
        },
        updated_at_ms: 0.0,
    };
    Ok(json!(blocking(ctx, move |e| e.create_script(script)).await?))
}

pub(super) async fn update_script(ctx: &Ctx<'_>, p: UpdateScript) -> ToolResult {
    let mut script = ctx
        .engine
        .script(&p.id)
        .ok_or_else(|| ToolError(format!("no script with id {}", p.id)))?;
    if let Some(name) = p.name {
        script.name = name;
    }
    if let Some(source) = p.source {
        script.source = source;
    }
    if let Some(enabled) = p.enabled {
        script.enabled = enabled;
    }
    if p.view.is_some() {
        script.view = p.view;
    }
    Ok(json!(blocking(ctx, move |e| e.update_script(script)).await?))
}

pub(super) async fn test_script(ctx: &Ctx<'_>, p: TestScript) -> ToolResult {
    let spec = ScriptTestSpec {
        script_id: p.script_id,
        source: p.source,
        kind: p.kind,
        part: p.part,
    };
    Ok(json!(ctx.engine.test_script(LIVE_STORE, p.flow_id, spec).await?))
}
