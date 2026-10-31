//! Replay, Edit & Replay, Compose and Copy as cURL. The MCP tools
//! `replay_flow` and `send_request` (M7) wrap the same methods.

use std::sync::Arc;

use base64::Engine as _;
use pm_model::{BodyPart, FlowId, Header, RequestSpec, SendOpts, SentBy, StoreId};

use crate::io::{curl, decode, session};
use crate::{Engine, Error, proxy};

/// Headers `Engine::send` derives itself, left out of a flow's spec.
const DERIVED: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authorization",
    "upgrade",
    "te",
    "trailer",
];

impl Engine {
    /// Sends a request Proxymate makes itself (Replay, Compose, an agent)
    /// and records it as a new live flow, marked with `opts.origin`.
    ///
    /// - With `opts.apply_rules` it goes through the rules exactly as a
    ///   proxied request would (rewrites, Map Local, breakpoints, the
    ///   throttle); without, straight to the upstream.
    /// - With `opts.wait` it resolves once the response has finished (or
    ///   the flow failed), and the flow's summary is up to date; without,
    ///   as soon as the flow is in the store (`get_flow` finds it).
    /// - Plain HTTP uses the pooled client; HTTPS dials a one-off HTTP/1.1
    ///   TLS connection (verified like any upstream).
    /// - Sends are recorded even while recording is paused.
    ///
    /// Errors are about the spec itself (a bad method, URL or header); an
    /// unreachable upstream is a failed flow, not an error.
    pub async fn send(self: &Arc<Self>, spec: RequestSpec, opts: SendOpts) -> Result<FlowId, Error> {
        let sent_by = opts.origin.sent_by();
        self.send_marked(spec, opts, sent_by).await
    }

    /// [`Engine::send`], with the flow marked `sent_by` (Repeat's flows).
    pub(crate) async fn send_marked(
        self: &Arc<Self>,
        spec: RequestSpec,
        opts: SendOpts,
        sent_by: SentBy,
    ) -> Result<FlowId, Error> {
        let sender = self.sender.get_or_init(|| proxy::Sender::new(Arc::downgrade(self)));
        let (id, task) = sender.send(self, spec, &opts, sent_by)?;
        if opts.wait {
            let _ = task.await;
        }
        // Callers read the flow straight away (the UI selects it), so it must
        // be in the store, not just queued for ingest.
        let engine = self.clone();
        let _ = tokio::task::spawn_blocking(move || engine.flush()).await;
        Ok(id)
    }

    /// Sends a recorded request again, as it was sent (body still
    /// content-encoded). See [`Engine::send`].
    pub async fn replay(self: &Arc<Self>, store: StoreId, id: FlowId, opts: SendOpts) -> Result<FlowId, Error> {
        let spec = self.request_spec(store, id, false)?;
        self.send(spec, opts).await
    }

    /// A recorded request as a [`RequestSpec`], for Edit & Replay. Host,
    /// Content-Length and hop-by-hop headers are left out (`send` derives
    /// them). With `editable`, a content-encoded body is decoded and its
    /// `Content-Encoding` dropped, so it can be edited as text.
    pub fn request_spec(&self, store: StoreId, id: FlowId, editable: bool) -> Result<RequestSpec, Error> {
        let detail = match self.flow_detail_in(store, id)? {
            Some(detail) => detail,
            None => session::fallback_detail(&self.get_flow_in(store, id).ok_or(Error::UnknownFlow(id))?),
        };
        if detail.request_body.truncated {
            return Err(Error::Send(
                "the request body was larger than the capture limit and wasn't recorded whole".into(),
            ));
        }
        let mut body = self
            .body_head_in(store, id, BodyPart::Request, false, None)
            .map(|b| b.bytes)
            .unwrap_or_default();
        let mut decoded = false;
        if editable
            && let Some(encoding) = &detail.request_body.encoding
            && !body.is_empty()
        {
            body = decode::decode(encoding, &body)?;
            decoded = true;
        }
        // A WebSocket sent in absolute form was recorded with its http URL.
        let websocket = detail
            .request
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("upgrade") && h.value.eq_ignore_ascii_case("websocket"));
        let headers: Vec<Header> = detail
            .request
            .headers
            .into_iter()
            .filter(|h| {
                let name = h.name.to_ascii_lowercase();
                !DERIVED.contains(&name.as_str()) && !(decoded && name == "content-encoding")
            })
            .collect();
        let (body, body_base64) = match String::from_utf8(body) {
            Ok(text) if text.is_empty() => (None, false),
            Ok(text) => (Some(text), false),
            Err(err) => (
                Some(base64::engine::general_purpose::STANDARD.encode(err.as_bytes())),
                true,
            ),
        };
        let mut url = detail.request.url;
        if websocket {
            for (from, to) in [("http://", "ws://"), ("https://", "wss://")] {
                if url.len() > from.len() && url[..from.len()].eq_ignore_ascii_case(from) {
                    url = format!("{to}{}", &url[from.len()..]);
                }
            }
        }
        Ok(RequestSpec {
            method: detail.request.method,
            url,
            headers,
            body,
            body_base64,
        })
    }

    /// A shell command that sends the recorded request again with curl.
    pub fn curl_command(&self, store: StoreId, id: FlowId) -> Result<String, Error> {
        let detail = match self.flow_detail_in(store, id)? {
            Some(detail) => detail,
            None => session::fallback_detail(&self.get_flow_in(store, id).ok_or(Error::UnknownFlow(id))?),
        };
        let body = self
            .body_head_in(store, id, BodyPart::Request, false, None)
            .map(|b| b.bytes)
            .unwrap_or_default();
        Ok(curl::command(&detail, &body))
    }
}
