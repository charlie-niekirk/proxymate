//! Scripts on the wire: [`ScriptRun`] runs a rule's `Action::Script` on a
//! request or response draft (`rules::eval`), and applies what came back.
//! Whatever a script returns is checked in full before any of it is
//! applied, so a script that returns something unusable leaves the
//! message exactly as it was.

use std::sync::Arc;

use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use pm_model::{FlowId, Scheme, ScriptIssue, ScriptPhase};

use super::host::{MockOut, RequestOut, ResponseOut};
use super::{Call, CallInfo, CompiledScript, JsRequest, JsResponse, Output, SCRIPT_BODY_MAX, Scripts};
use crate::rules::compile::CompiledRule;
use crate::rules::eval::{RequestDraft, ResponseDraft};

/// A mock script's answer.
pub(crate) type MockAnswer = ResponseDraft;

/// Runs scripts for one flow. Without scripts (unit tests of the
/// evaluator) a script action is reported and skipped.
#[derive(Clone, Copy)]
pub(crate) struct ScriptRun<'a> {
    scripts: Option<&'a Scripts>,
    flow_id: Option<FlowId>,
}

impl<'a> ScriptRun<'a> {
    pub fn new(scripts: &'a Scripts, flow_id: Option<FlowId>) -> ScriptRun<'a> {
        ScriptRun {
            scripts: Some(scripts),
            flow_id,
        }
    }

    #[cfg(test)]
    pub fn none() -> ScriptRun<'static> {
        ScriptRun {
            scripts: None,
            flow_id: None,
        }
    }

    async fn call(&self, script: &CompiledScript, rule: &CompiledRule, call: Call) -> Result<Output, ScriptIssue> {
        let phase = call.phase().unwrap_or(ScriptPhase::Request);
        let issue = |message: String| ScriptIssue {
            script_id: script.code.id.clone(),
            script_name: script.code.name.clone(),
            rule_id: Some(rule.applied.rule_id.clone()),
            phase,
            message,
        };
        let Some(scripts) = self.scripts else {
            return Err(issue("scripts aren't available here".into()));
        };
        let info = CallInfo {
            flow_id: self.flow_id,
            rule: Some((rule.applied.rule_id.clone(), rule.applied.name.clone())),
            part: None,
        };
        let result = scripts
            .host
            .run(script.code.clone(), call, info, scripts.traffic_timeout(), false)
            .await;
        scripts.log_run(&script.code, &result, self.flow_id);
        result.output.map_err(issue)
    }

    /// `onRequest`. `None` when the script left the request as it was.
    pub async fn request(
        &self,
        script: &CompiledScript,
        rule: &CompiledRule,
        draft: &RequestDraft,
    ) -> Result<Option<RequestOut>, ScriptIssue> {
        match self.call(script, rule, Call::Request(js_request(draft))).await? {
            Output::Request(out) => Ok(Some(out)),
            _ => Ok(None),
        }
    }

    /// `onResponse`, given the request as it was sent.
    pub async fn response(
        &self,
        script: &CompiledScript,
        rule: &CompiledRule,
        request: &JsRequest,
        draft: &ResponseDraft,
    ) -> Result<Option<ResponseOut>, ScriptIssue> {
        match self
            .call(script, rule, Call::Response(request.clone(), js_response(draft)))
            .await?
        {
            Output::Response(out) => Ok(Some(out)),
            _ => Ok(None),
        }
    }

    /// `onMock`. `None` passes the request on.
    pub async fn mock(
        &self,
        script: &CompiledScript,
        rule: &CompiledRule,
        draft: &RequestDraft,
    ) -> Result<Option<MockOut>, ScriptIssue> {
        match self.call(script, rule, Call::Mock(js_request(draft))).await? {
            Output::Mock(answer) => Ok(answer),
            _ => Ok(None),
        }
    }

    /// The issue for a script result that couldn't be applied.
    pub fn issue(
        &self,
        script: &CompiledScript,
        rule: &CompiledRule,
        phase: ScriptPhase,
        message: String,
    ) -> ScriptIssue {
        if let Some(scripts) = self.scripts {
            scripts.log_error(&script.code, &message, self.flow_id);
        }
        ScriptIssue {
            script_id: script.code.id.clone(),
            script_name: script.code.name.clone(),
            rule_id: Some(rule.applied.rule_id.clone()),
            phase,
            message,
        }
    }
}

pub(crate) fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(n, v)| {
            (
                n.as_str().to_owned(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect()
}

fn script_body(body: &Option<Vec<u8>>) -> Option<Arc<Vec<u8>>> {
    body.as_ref()
        .filter(|b| b.len() <= SCRIPT_BODY_MAX)
        .map(|b| Arc::new(b.clone()))
}

/// The request as a script sees it.
pub(crate) fn js_request(draft: &RequestDraft) -> JsRequest {
    JsRequest {
        method: draft.method.clone(),
        url: draft.url(),
        scheme: draft.scheme.as_str().to_owned(),
        host: draft.host.clone(),
        port: draft.port,
        path: draft.path.clone(),
        query: draft.query.clone(),
        headers: header_pairs(&draft.headers),
        body: script_body(&draft.body),
    }
}

pub(crate) fn js_response(draft: &ResponseDraft) -> JsResponse {
    JsResponse {
        status: draft.status,
        headers: header_pairs(&draft.headers),
        body: script_body(&draft.body),
    }
}

fn header_map(pairs: &[(String, String)]) -> Result<HeaderMap, String> {
    let mut map = HeaderMap::with_capacity(pairs.len());
    for (name, value) in pairs {
        let name = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| format!("{name:?} isn't a valid header name"))?;
        let value =
            HeaderValue::from_str(value).map_err(|_| format!("{name}: {value:?} isn't a valid header value"))?;
        map.append(name, value);
    }
    Ok(map)
}

/// What applying a script's result changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Changed {
    pub head: bool,
    pub body: bool,
}

/// Applies `onRequest`'s result to the draft.
pub(crate) fn apply_request(draft: &mut RequestDraft, out: RequestOut) -> Result<Changed, String> {
    let method = out.method.trim().to_ascii_uppercase();
    if http::Method::from_bytes(method.as_bytes()).is_err() || method.is_empty() {
        return Err(format!("{:?} isn't an HTTP method", out.method));
    }
    let scheme = match out.scheme.to_ascii_lowercase().as_str() {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        "ws" => Scheme::Ws,
        "wss" => Scheme::Wss,
        other => return Err(format!("{other:?} isn't a scheme proxymate can send to")),
    };
    if out.host.is_empty() || out.host.contains(['/', ' ', '?', '#']) {
        return Err(format!("{:?} isn't a host", out.host));
    }
    let path = if out.path.starts_with('/') {
        out.path
    } else {
        format!("/{}", out.path)
    };
    let pq = if out.query.is_empty() {
        path.clone()
    } else {
        format!("{path}?{}", out.query)
    };
    if pq.parse::<http::uri::PathAndQuery>().is_err() {
        return Err(format!("{pq:?} isn't a valid path and query"));
    }
    let headers = out.headers.as_deref().map(header_map).transpose()?;

    let mut changed = Changed::default();
    let old_authority = draft.authority();
    if method != draft.method {
        draft.method = method;
        changed.head = true;
    }
    if (scheme, out.host.as_str(), out.port, path.as_str(), out.query.as_str())
        != (
            draft.scheme,
            draft.host.as_str(),
            draft.port,
            draft.path.as_str(),
            draft.query.as_str(),
        )
    {
        draft.scheme = scheme;
        draft.host = out.host;
        draft.port = out.port;
        draft.path = path;
        draft.query = out.query;
        changed.head = true;
    }
    if let Some(headers) = headers
        && headers != draft.headers
    {
        draft.headers = headers;
        changed.head = true;
    }
    // A new origin gets its own Host, unless the script set one itself.
    if draft.authority() != old_authority
        && draft.headers.get(header::HOST).and_then(|h| h.to_str().ok()) == Some(old_authority.as_str())
        && let Ok(host) = HeaderValue::from_str(&draft.authority())
    {
        draft.headers.insert(header::HOST, host);
    }
    if let Some(body) = out.body
        && draft.body.as_ref() != Some(&body)
    {
        draft.body = Some(body);
        changed.body = true;
    }
    Ok(changed)
}

/// Applies `onResponse`'s result to the draft.
pub(crate) fn apply_response(draft: &mut ResponseDraft, out: ResponseOut) -> Result<Changed, String> {
    let headers = out.headers.as_deref().map(header_map).transpose()?;
    let mut changed = Changed::default();
    if out.status != draft.status {
        draft.status = out.status;
        changed.head = true;
    }
    if let Some(headers) = headers
        && headers != draft.headers
    {
        draft.headers = headers;
        changed.head = true;
    }
    if let Some(body) = out.body
        && draft.body.as_ref() != Some(&body)
    {
        draft.body = Some(body);
        changed.body = true;
    }
    Ok(changed)
}

/// `onMock`'s answer as a response draft.
pub(crate) fn mock_answer(out: MockOut) -> Result<MockAnswer, String> {
    Ok(ResponseDraft {
        status: out.status,
        headers: header_map(&out.headers)?,
        body: Some(out.body),
    })
}
