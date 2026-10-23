//! [`Engine::test_script`]: a script (saved, or unsaved source) run against
//! a recorded flow in any store. Nothing is sent. Test runs go to a worker
//! of their own and evaluate a fresh module each time, so they never touch
//! the `context.shared` of the scripts running on traffic.

use std::time::Duration;

use pm_model::{
    BodyPart, ConsoleEntry, FlowId, HttpVersion, RequestHead, ResponseHead, ScriptKind, ScriptTestResult,
    ScriptTestSpec, StoreId,
};

use super::host::{RunResult, ScriptHost};
use super::traffic::{Changed, apply_request, apply_response, mock_answer};
use super::views::{messages, script_views};
use super::{Call, CallInfo, Code, JsRequest, JsResponse, Output, ScriptError};
use crate::rules::eval::{RequestDraft, ResponseDraft, header_map};
use crate::{Engine, Error};

/// Bodies are shown up to this size.
const TEST_BODY_MAX: usize = 256 * 1024;

fn text(body: Option<&Vec<u8>>) -> Option<String> {
    body.map(|b| String::from_utf8_lossy(&b[..b.len().min(TEST_BODY_MAX)]).into_owned())
}

fn request_head(draft: &RequestDraft, version: HttpVersion) -> RequestHead {
    RequestHead {
        method: draft.method.clone(),
        url: draft.url(),
        version,
        headers: crate::proxy::headers_of(&draft.headers),
    }
}

fn response_head(draft: &ResponseDraft, version: HttpVersion) -> ResponseHead {
    ResponseHead {
        status: draft.status,
        reason: http::StatusCode::from_u16(draft.status)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or_default()
            .to_owned(),
        version,
        headers: crate::proxy::headers_of(&draft.headers),
    }
}

fn draft_of(request: &JsRequest, scheme: pm_model::Scheme) -> RequestDraft {
    RequestDraft {
        method: request.method.clone(),
        scheme,
        host: request.host.clone(),
        port: request.port,
        path: request.path.clone(),
        query: request.query.clone(),
        headers: header_map(
            &request
                .headers
                .iter()
                .map(|(n, v)| pm_model::Header::new(n, v))
                .collect::<Vec<_>>(),
        ),
        body: request.body.as_deref().cloned(),
    }
}

fn response_draft(response: &JsResponse) -> ResponseDraft {
    ResponseDraft {
        status: response.status,
        headers: header_map(
            &response
                .headers
                .iter()
                .map(|(n, v)| pm_model::Header::new(n, v))
                .collect::<Vec<_>>(),
        ),
        body: response.body.as_deref().cloned(),
    }
}

struct Runner<'a> {
    host: &'a ScriptHost,
    code: Code,
    timeout: Duration,
    info: CallInfo,
    console: Vec<ConsoleEntry>,
    duration: Duration,
}

impl Runner<'_> {
    async fn run(&mut self, engine: &Engine, call: Call) -> Result<Output, String> {
        let result: RunResult = self
            .host
            .run(self.code.clone(), call, self.info.clone(), self.timeout, true)
            .await;
        self.duration += result.duration;
        engine.scripts.log_run(&self.code, &result, self.info.flow_id);
        let error = result.output.as_ref().err().cloned();
        for (level, message) in result
            .console
            .into_iter()
            .chain(error.map(|e| (pm_model::ConsoleLevel::Error, e)))
        {
            self.console.push(ConsoleEntry {
                seq: 0,
                time_ms: super::now_ms(),
                script_id: self.code.id.clone(),
                script_name: self.code.name.clone(),
                level,
                message,
                flow_id: self.info.flow_id,
            });
        }
        result.output
    }
}

impl Engine {
    /// Dry run: what a script would do to a recorded flow. A rewrite
    /// script runs `onRequest` then `onResponse`; a mock script `onMock`;
    /// a view script `onView` on `spec.part` (the response by default).
    pub async fn test_script(
        &self,
        store: StoreId,
        flow: FlowId,
        spec: ScriptTestSpec,
    ) -> Result<ScriptTestResult, Error> {
        let (code, kind) = match (spec.source, &spec.script_id) {
            (Some(source), id) => {
                let saved = id.as_deref().and_then(|id| self.script(id));
                let kind = spec
                    .kind
                    .or(saved.as_ref().map(|s| s.kind))
                    .ok_or_else(|| Error::Invalid("say which kind of script the source is".into()))?;
                let name = saved.map_or_else(|| "Unsaved script".to_owned(), |s| s.name);
                (
                    Code::new(id.clone().unwrap_or_else(|| "test".into()), name, source.into()),
                    kind,
                )
            }
            (None, Some(id)) => {
                let script = self.script(id).ok_or_else(|| ScriptError::Unknown(id.clone()))?;
                (
                    Code::new(script.id, script.name, script.source.into()),
                    spec.kind.unwrap_or(script.kind),
                )
            }
            (None, None) => return Err(Error::Invalid("give a script id or source to test".into())),
        };
        let summary = self.get_flow_in(store, flow).ok_or(Error::UnknownFlow(flow))?;
        let detail = self.flow_detail_in(store, flow)?.ok_or(Error::UnknownFlow(flow))?;
        let (request, response) = messages(
            &summary,
            &detail,
            self.script_body(store, flow, BodyPart::Request),
            detail
                .response
                .as_ref()
                .and_then(|_| self.script_body(store, flow, BodyPart::Response)),
        );
        let version = detail.request.version;
        let timeout = match kind {
            ScriptKind::View => self.scripts.view_timeout(),
            _ => self.scripts.traffic_timeout(),
        };
        let part = spec.part.unwrap_or(BodyPart::Response);
        let mut runner = Runner {
            host: &self.scripts.host,
            code,
            timeout,
            info: CallInfo {
                flow_id: Some(flow),
                rule: None,
                part: (kind == ScriptKind::View).then_some(part),
            },
            console: Vec::new(),
            duration: Duration::ZERO,
        };
        let mut result = ScriptTestResult {
            kind,
            error: None,
            request_before: None,
            request_body_before: None,
            request: None,
            request_body: None,
            response_before: None,
            response_body_before: None,
            response: None,
            response_body: None,
            views: Vec::new(),
            console: Vec::new(),
            duration_ms: 0.0,
            notes: Vec::new(),
        };
        match kind {
            ScriptKind::Rewrite => {
                let mut draft = draft_of(&request, summary.scheme);
                result.request_before = Some(request_head(&draft, version));
                result.request_body_before = text(draft.body.as_ref());
                match runner.run(self, Call::Request(request.clone())).await {
                    Ok(Output::Request(out)) => match apply_request(&mut draft, out) {
                        Ok(Changed { head, body }) if !head && !body => {
                            result.notes.push("onRequest left the request unchanged.".into())
                        }
                        Ok(_) => {}
                        Err(message) => result.error = Some(format!("onRequest returned {message}")),
                    },
                    Ok(_) => result.notes.push("The script doesn't export onRequest.".into()),
                    Err(message) => result.error = Some(message),
                }
                result.request = Some(request_head(&draft, version));
                result.request_body = text(draft.body.as_ref());
                if let Some(response) = &response {
                    let sent = super::traffic::js_request(&draft);
                    let mut resp = response_draft(response);
                    result.response_before = Some(response_head(&resp, version));
                    result.response_body_before = text(resp.body.as_ref());
                    if result.error.is_none() {
                        match runner.run(self, Call::Response(sent, response.clone())).await {
                            Ok(Output::Response(out)) => match apply_response(&mut resp, out) {
                                Ok(Changed { head, body }) if !head && !body => {
                                    result.notes.push("onResponse left the response unchanged.".into())
                                }
                                Ok(_) => {}
                                Err(message) => result.error = Some(format!("onResponse returned {message}")),
                            },
                            Ok(_) => result.notes.push("The script doesn't export onResponse.".into()),
                            Err(message) => result.error = Some(message),
                        }
                    }
                    result.response = Some(response_head(&resp, version));
                    result.response_body = text(resp.body.as_ref());
                } else {
                    result
                        .notes
                        .push("The flow has no response, so onResponse didn't run.".into());
                }
            }
            ScriptKind::Mock => {
                let draft = draft_of(&request, summary.scheme);
                result.request_before = Some(request_head(&draft, version));
                result.request_body_before = text(draft.body.as_ref());
                match runner.run(self, Call::Mock(request)).await {
                    Ok(Output::Mock(Some(out))) => match mock_answer(out) {
                        Ok(answer) => {
                            result
                                .notes
                                .push("Answered by the mock; the upstream isn't contacted.".into());
                            result.response = Some(response_head(&answer, version));
                            result.response_body = text(answer.body.as_ref());
                        }
                        Err(message) => result.error = Some(format!("onMock returned {message}")),
                    },
                    Ok(Output::Mock(None)) => result
                        .notes
                        .push("onMock returned nothing, so the request goes on to the upstream.".into()),
                    Ok(_) => result.notes.push("The script doesn't export onMock.".into()),
                    Err(message) => result.error = Some(message),
                }
            }
            ScriptKind::View => match runner.run(self, Call::View(request, response)).await {
                Ok(Output::Views(views)) => {
                    if views.is_empty() {
                        result.notes.push("onView returned no views for this body.".into());
                    }
                    result.views = script_views(views);
                }
                Ok(_) => result.notes.push("The script doesn't export onView.".into()),
                Err(message) => result.error = Some(message),
            },
        }
        result.console = runner.console;
        result.duration_ms = runner.duration.as_secs_f64() * 1000.0;
        Ok(result)
    }
}
