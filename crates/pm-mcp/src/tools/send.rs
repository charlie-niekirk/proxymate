//! Tools that send requests: compose, replay and Repeat (M8, M17).

use super::*;

/// Changes to a recorded request before it is sent again. Unset fields
/// stay as recorded.
#[derive(Deserialize, JsonSchema, Default)]
pub(super) struct RequestEdits {
    #[serde(default)]
    pub(super) method: Option<String>,
    /// An absolute URL.
    #[serde(default)]
    pub(super) url: Option<String>,
    /// Headers to set (replacing any of the same name).
    #[serde(default)]
    pub(super) set_headers: Vec<Header>,
    #[serde(default)]
    pub(super) remove_headers: Vec<String>,
    /// Replaces the body (the recorded one is decoded first, so edits are text).
    #[serde(default)]
    pub(super) body: Option<String>,
    /// `body` is base64.
    #[serde(default)]
    pub(super) body_base64: bool,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct ReplayFlow {
    pub(super) id: FlowId,
    #[serde(default)]
    pub(super) modify: Option<RequestEdits>,
    /// Run it through the rules like proxied traffic (default true).
    #[serde(default = "yes")]
    pub(super) apply_rules: bool,
    /// Wait for the response (default true).
    #[serde(default = "yes")]
    pub(super) wait: bool,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct RepeatFlow {
    pub(super) id: FlowId,
    /// How many times to send it, 1 to 10,000.
    pub(super) iterations: u32,
    /// Requests in flight at once, 1 to 64 (default 1).
    #[serde(default = "one")]
    pub(super) concurrency: u32,
    /// Pause before each request, in milliseconds (default 0).
    #[serde(default)]
    pub(super) delay_ms: u32,
    /// Run each request through the rules like proxied traffic (default true).
    #[serde(default = "yes")]
    pub(super) apply_rules: bool,
}

pub(super) fn one() -> u32 {
    1
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct SendRequest {
    pub(super) request: RequestSpec,
    /// Run it through the rules like proxied traffic (default true).
    #[serde(default = "yes")]
    pub(super) apply_rules: bool,
    /// Wait for the response (default true).
    #[serde(default = "yes")]
    pub(super) wait: bool,
}

/// Sends `spec` as this agent and describes the new flow.
pub(super) async fn send(ctx: &Ctx<'_>, spec: RequestSpec, apply_rules: bool, wait: bool) -> ToolResult {
    let opts = SendOpts {
        apply_rules,
        wait,
        origin: SendOrigin::Agent {
            client: ctx.client.to_owned(),
        },
    };
    let id = ctx.engine.send(spec, opts).await?;
    let flow = ctx.engine.get_flow(id).map(|f| brief(&f));
    let error = ctx.engine.flow_detail(id).ok().flatten().and_then(|d| d.error);
    Ok(json!({ "id": id, "flow": flow, "error": error }))
}

pub(super) async fn repeat_flow(ctx: &Ctx<'_>, p: RepeatFlow) -> ToolResult {
    let id = p.id;
    let spec = blocking(ctx, move |e| e.request_spec(LIVE_STORE, id, false)).await?;
    let opts = RepeatOpts {
        iterations: p.iterations,
        concurrency: p.concurrency,
        delay_ms: p.delay_ms,
        apply_rules: p.apply_rules,
    };
    let origin = SendOrigin::Agent {
        client: ctx.client.to_owned(),
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut tx = Some(tx);
    let id = ctx.engine.repeat_as(
        spec,
        opts,
        origin,
        Box::new(move |progress| {
            if progress.finished
                && let Some(tx) = tx.take()
            {
                let _ = tx.send(progress);
            }
        }),
    )?;
    let done = tokio::select! {
        done = rx => done.map_err(|_| ToolError("the repeat stopped without a result".into()))?,
        () = ctx.ct.cancelled() => {
            ctx.engine.cancel_repeat(id);
            return Err(ToolError("cancelled".into()));
        }
    };
    if let Some(error) = &done.error
        && done.done == 0
    {
        return Err(ToolError(error.clone()));
    }
    Ok(json!({
        "repeated": p.id,
        "sent": done.done,
        "failed": done.failed,
        "statuses": done.statuses,
        "min_ms": done.min_ms,
        "p50_ms": done.p50_ms,
        "p95_ms": done.p95_ms,
        "max_ms": done.max_ms,
        "first_flow": done.first_flow,
        "last_flow": done.last_flow,
        "error": done.error,
    }))
}

pub(super) async fn replay_flow(ctx: &Ctx<'_>, p: ReplayFlow) -> ToolResult {
    let Some(edits) = p.modify else {
        let opts = SendOpts {
            apply_rules: p.apply_rules,
            wait: p.wait,
            origin: SendOrigin::Agent {
                client: ctx.client.to_owned(),
            },
        };
        let id = ctx.engine.replay(LIVE_STORE, p.id, opts).await?;
        let flow = ctx.engine.get_flow(id).map(|f| brief(&f));
        let error = ctx.engine.flow_detail(id).ok().flatten().and_then(|d| d.error);
        return Ok(json!({ "id": id, "replayed": p.id, "flow": flow, "error": error }));
    };
    let id = p.id;
    let mut spec = blocking(ctx, move |e| e.request_spec(LIVE_STORE, id, true)).await?;
    if let Some(method) = edits.method {
        spec.method = method;
    }
    if let Some(url) = edits.url {
        spec.url = url;
    }
    let drop: Vec<String> = edits
        .remove_headers
        .iter()
        .chain(edits.set_headers.iter().map(|h| &h.name))
        .map(|n| n.to_ascii_lowercase())
        .collect();
    spec.headers.retain(|h| !drop.contains(&h.name.to_ascii_lowercase()));
    spec.headers.extend(edits.set_headers);
    if let Some(body) = edits.body {
        spec.body = Some(body);
        spec.body_base64 = edits.body_base64;
    }
    let mut out = send(ctx, spec, p.apply_rules, p.wait).await?;
    out["replayed"] = json!(p.id);
    Ok(out)
}
