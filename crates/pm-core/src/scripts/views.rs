//! View scripts in the inspector: [`Engine::run_view_scripts`] finds the
//! enabled view scripts whose [`ViewConfig`] matches a flow's body (on its
//! summary and detail alone), and only then reads the bodies and runs
//! them. Results for finished flows are cached by (store, flow, part,
//! script source), so reopening a flow, or a saved session, is instant.
//!
//! Flow ids are never reused within a store, and store ids never repeat,
//! so the key needs no epoch.

use std::sync::Arc;

use base64::Engine as _;
use moka::sync::Cache;
use pm_model::{
    BodyPart, ContentKind, FlowDetail, FlowId, FlowState, FlowSummary, LIVE_STORE, ScriptView, ScriptViewKind, StoreId,
    ViewResult,
};

use super::host::ViewOut;
use super::{Call, CallInfo, Code, JsRequest, JsResponse, Output, SCRIPT_BODY_MAX, ViewScript};
use crate::rules::compile::RequestInfo;
use crate::rules::eval::header_map;
use crate::{Engine, Error};

/// Bytes of cached view results.
const CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// Each view's text or bytes are cut here.
pub(crate) const VIEW_MAX: usize = 8 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq, Hash)]
struct ViewKey {
    store: StoreId,
    flow: FlowId,
    part: BodyPart,
    script: String,
    hash: u64,
}

pub(crate) struct ViewCache(Cache<ViewKey, Arc<ViewResult>>);

impl ViewCache {
    pub fn new() -> ViewCache {
        ViewCache(
            Cache::builder()
                .max_capacity(CACHE_BYTES)
                .weigher(|_, result: &Arc<ViewResult>| {
                    let bytes: usize = result
                        .views
                        .iter()
                        .map(|v| {
                            v.title.len()
                                + v.text.as_ref().map_or(0, String::len)
                                + v.base64.as_ref().map_or(0, String::len)
                        })
                        .sum();
                    u32::try_from(bytes + 256).unwrap_or(u32::MAX)
                })
                .build(),
        )
    }

    pub fn clear(&self) {
        self.0.invalidate_all();
    }
}

/// Whether `view` applies to `part` of this flow.
fn applies(view: &ViewScript, info: &RequestInfo<'_>, detail: &FlowDetail, part: BodyPart, kind: ContentKind) -> bool {
    let config = &view.config;
    if !config.parts.is_empty() && !config.parts.contains(&part) {
        return false;
    }
    if !config.kinds.is_empty() && !config.kinds.contains(&kind) {
        return false;
    }
    if view.matcher.mismatch(info, true).is_some() {
        return false;
    }
    match (&view.matcher.response, &detail.response) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(matcher), Some(response)) => matcher.matches(response.status, &header_map(&response.headers)),
    }
}

fn pairs(headers: &[pm_model::Header]) -> Vec<(String, String)> {
    headers.iter().map(|h| (h.name.clone(), h.value.clone())).collect()
}

/// The request and response a view script (or a dry run) gets.
pub(crate) fn messages(
    summary: &FlowSummary,
    detail: &FlowDetail,
    request_body: Option<Vec<u8>>,
    response_body: Option<Vec<u8>>,
) -> (JsRequest, Option<JsResponse>) {
    let request = JsRequest {
        method: detail.request.method.clone(),
        url: detail.request.url.clone(),
        scheme: summary.scheme.as_str().to_owned(),
        host: summary.host.to_string(),
        port: summary.port,
        path: summary.path.clone(),
        query: summary.query.clone(),
        headers: pairs(&detail.request.headers),
        body: request_body.map(Arc::new),
    };
    let response = detail.response.as_ref().map(|r| JsResponse {
        status: r.status,
        headers: pairs(&r.headers),
        body: response_body.map(Arc::new),
    });
    (request, response)
}

pub(crate) fn script_views(views: Vec<ViewOut>) -> Vec<ScriptView> {
    views
        .into_iter()
        .map(|v| {
            let mut truncated = false;
            let text = v.text.map(|mut t| {
                if t.len() > VIEW_MAX {
                    let mut cut = VIEW_MAX;
                    while !t.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    t.truncate(cut);
                    truncated = true;
                }
                t
            });
            let base64 = v.bytes.map(|b| {
                truncated |= b.len() > VIEW_MAX;
                base64::engine::general_purpose::STANDARD.encode(&b[..b.len().min(VIEW_MAX)])
            });
            let mime = match v.kind {
                ScriptViewKind::Image => v.mime.or_else(|| Some("image/png".into())),
                _ => None,
            };
            ScriptView {
                title: v.title,
                kind: v.kind,
                text,
                base64,
                mime,
                truncated,
            }
        })
        .collect()
}

impl Engine {
    /// A decoded body for a script, or `None` when it is over
    /// [`SCRIPT_BODY_MAX`] or can't be decoded.
    pub(crate) fn script_body(&self, store: StoreId, flow: FlowId, part: BodyPart) -> Option<Vec<u8>> {
        self.body_head_in(store, flow, part, true, Some(SCRIPT_BODY_MAX))
            .ok()
            .filter(|b| !b.cut)
            .map(|b| b.bytes)
    }

    /// Runs the enabled view scripts that apply to `part` of a flow, in any
    /// store, and returns what each made of it (views, or an error). Empty
    /// when none applies.
    pub async fn run_view_scripts(
        &self,
        store: StoreId,
        flow: FlowId,
        part: BodyPart,
    ) -> Result<Vec<ViewResult>, Error> {
        let view_scripts = self.scripts.view_scripts();
        if view_scripts.is_empty() {
            return Ok(Vec::new());
        }
        let summary = self.get_flow_in(store, flow).ok_or(Error::UnknownFlow(flow))?;
        let detail = self.flow_detail_in(store, flow)?.ok_or(Error::UnknownFlow(flow))?;
        let headers = header_map(&detail.request.headers);
        let info = RequestInfo {
            method: &detail.request.method,
            scheme: summary.scheme,
            host: &summary.host,
            port: summary.port,
            path: &summary.path,
            query: &summary.query,
            headers: &headers,
            client_app: summary.client.as_deref(),
        };
        let kind = match part {
            BodyPart::Request => detail.request_body.kind,
            BodyPart::Response => detail.response_body.kind,
        };
        let candidates: Vec<&Arc<ViewScript>> = view_scripts
            .iter()
            .filter(|v| applies(v, &info, &detail, part, kind))
            .collect();
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let finished = !matches!(summary.state, FlowState::Active | FlowState::Paused);
        let mut messages_for_run = None;
        let mut results = Vec::with_capacity(candidates.len());
        for view in candidates {
            let code = &view.script.code;
            let key = ViewKey {
                store,
                flow,
                part,
                script: code.id.clone(),
                hash: code.hash,
            };
            if finished && let Some(hit) = self.scripts.views.0.get(&key) {
                results.push(ViewResult::clone(&hit));
                continue;
            }
            let (request, response) = messages_for_run
                .get_or_insert_with(|| {
                    messages(
                        &summary,
                        &detail,
                        self.script_body(store, flow, BodyPart::Request),
                        detail
                            .response
                            .as_ref()
                            .and_then(|_| self.script_body(store, flow, BodyPart::Response)),
                    )
                })
                .clone();
            let result = self.run_view(code, request, response, store, flow, part).await;
            if finished {
                self.scripts.views.0.insert(key, Arc::new(result.clone()));
            }
            results.push(result);
        }
        Ok(results)
    }

    async fn run_view(
        &self,
        code: &Code,
        request: JsRequest,
        response: Option<JsResponse>,
        store: StoreId,
        flow: FlowId,
        part: BodyPart,
    ) -> ViewResult {
        // The console names live flows only: a document's ids are its own.
        let console_flow = (store == LIVE_STORE).then_some(flow);
        let info = CallInfo {
            flow_id: Some(flow),
            rule: None,
            part: Some(part),
        };
        let result = self
            .scripts
            .host
            .run(
                code.clone(),
                Call::View(request, response),
                info,
                self.scripts.view_timeout(),
                false,
            )
            .await;
        self.scripts.log_run(code, &result, console_flow);
        let duration_ms = result.duration.as_secs_f64() * 1000.0;
        let (views, error) = match result.output {
            Ok(Output::Views(views)) => (script_views(views), None),
            Ok(_) => (Vec::new(), Some("the script doesn't export onView".into())),
            Err(message) => (Vec::new(), Some(message)),
        };
        ViewResult {
            script_id: code.id.clone(),
            script_name: code.name.clone(),
            views,
            error,
            duration_ms,
        }
    }
}
