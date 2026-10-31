//! Requests Proxymate sends itself (`Engine::send`): Replay, Edit & Replay,
//! Compose and the MCP send tools. They run through the same [`Exchange`]
//! as proxied traffic, so rules apply exactly as they would on the wire
//! (unless the caller turns them off), and the flow is recorded the same
//! way, marked with who sent it.
//!
//! Plain HTTP goes out on the engine's pooled client. HTTPS dials a one-off
//! HTTP/1.1 TLS connection, as a Map Remote to another origin does: there
//! is no pooled HTTPS client. Sends are recorded even while recording is
//! paused, since someone asked for them.
//!
//! A `ws://` or `wss://` spec is a WebSocket handshake (a replay of a
//! WebSocket flow): it gets a fresh `Sec-WebSocket-Key`, and on `101` the
//! connection stays open with no client. Received messages are recorded;
//! `Engine::ws_send` sends to the server.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Weak};

use base64::Engine as _;
use bytes::Bytes;
use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use http::{Method, Request, Uri};
use http_body_util::BodyExt;
use pm_model::{
    BodyInfo, FlowDetail, FlowId, FlowState, FlowSummary, Header, HttpVersion, RequestHead, RequestSpec, Scheme,
    SendOpts, SendOrigin, Timing, TlsStatus,
};
use tokio::task::JoinHandle;

use super::body::Inbound;
use super::exchange::{Exchange, Route};
use super::{ProxyCtx, content_kind, now_ms};
use crate::client::PeerClient;
use crate::recorder::content_encoding;
use crate::{Engine, Error};

/// The client name of flows sent from the app (Replay, Compose).
pub const APP_CLIENT: &str = "Proxymate";

/// What sends share: a context with no listener of its own.
pub(crate) struct Sender {
    ctx: Arc<ProxyCtx>,
}

impl Sender {
    pub fn new(engine: Weak<Engine>) -> Sender {
        Sender {
            ctx: Arc::new(ProxyCtx {
                engine,
                // Only the listener's loop check reads this; sends never loop
                // through it.
                addr: SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            }),
        }
    }

    /// Records the request as a new flow and starts the exchange. Returns
    /// the flow id and the task that finishes it.
    pub fn send(
        &self,
        engine: &Arc<Engine>,
        spec: RequestSpec,
        opts: &SendOpts,
        sent_by: pm_model::SentBy,
    ) -> Result<(FlowId, JoinHandle<()>), Error> {
        let request = Prepared::new(spec)?;
        let client = match &opts.origin {
            SendOrigin::User => APP_CLIENT.to_owned(),
            SendOrigin::Agent { client } => client.clone(),
        };
        let summary = FlowSummary {
            id: 0,
            state: FlowState::Active,
            scheme: request.scheme,
            method: request.method.as_str().into(),
            host: request.host.as_str().into(),
            port: request.port,
            path: request.uri.path().to_owned(),
            query: request.uri.query().unwrap_or_default().to_owned(),
            status: None,
            content_kind: content_kind(&request.headers),
            client: Some(client.into()),
            started_at_ms: now_ms(),
            duration_ms: None,
            req_size: 0,
            resp_size: 0,
            edited: false,
            http_version: HttpVersion::Http11,
            tls: if request.scheme.is_tls() {
                TlsStatus::Intercepted
            } else {
                TlsStatus::None
            },
            graphql_op: None,
            sent_by: Some(sent_by),
            pinned: false,
            grpc_status: None,
            label: None,
            comment: None,
            extra: Vec::new(),
        };
        let headers: Vec<Header> = super::headers_of(&request.headers);
        let detail = FlowDetail {
            id: 0,
            request: RequestHead {
                method: request.method.to_string(),
                url: request.uri.to_string(),
                version: HttpVersion::Http11,
                headers: headers.clone(),
            },
            response: None,
            request_body: BodyInfo {
                encoding: content_encoding(&headers),
                ..Default::default()
            },
            response_body: BodyInfo::default(),
            error: None,
            tls: None,
            timing: Timing::default(),
            rules: Default::default(),
            trailers: None,
            via: None,
        };
        let recorder = engine.record_flow(summary, detail);
        let id = recorder.id();
        let ctx = self.ctx.clone();
        let apply_rules = opts.apply_rules;
        let task = tokio::spawn(async move {
            let peer = PeerClient::known(None);
            let Prepared {
                method,
                uri,
                headers,
                body,
                scheme,
                host,
                port,
            } = request;
            let mut req = Request::new(Inbound::local(body));
            *req.method_mut() = method;
            *req.uri_mut() = uri;
            *req.headers_mut() = headers;
            let exchange = Exchange {
                ctx: &ctx,
                route: Route::Forward,
                peer: &peer,
                recorder: Some(recorder),
                scheme,
                host,
                port,
                apply_rules,
                websocket: scheme.is_websocket().then_some(super::ws::ClientSide::Headless),
            };
            // A Reset (Block, Abort) has already finished the flow.
            if let Ok(response) = exchange.run(req).await {
                // Reading the body to its end records it and finishes the flow.
                let _ = response.into_body().collect().await;
            }
        });
        Ok((id, task))
    }
}

/// A [`RequestSpec`] checked and turned into request parts.
struct Prepared {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    scheme: Scheme,
    host: String,
    port: u16,
}

impl Prepared {
    fn new(spec: RequestSpec) -> Result<Prepared, Error> {
        let bad = |message: String| Error::Send(message);
        let method = spec.method.trim().to_ascii_uppercase();
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| bad(format!("{method:?} isn't a method")))?;
        if method == Method::CONNECT {
            return Err(bad("CONNECT requests can't be sent".into()));
        }
        let uri: Uri = spec
            .url
            .trim()
            .parse()
            .map_err(|e| bad(format!("{:?} isn't a URL: {e}", spec.url)))?;
        let scheme = match uri.scheme_str().map(str::to_ascii_lowercase).as_deref() {
            Some("http") => Scheme::Http,
            Some("https") => Scheme::Https,
            Some("ws") => Scheme::Ws,
            Some("wss") => Scheme::Wss,
            _ => {
                return Err(bad(format!(
                    "{:?} must start with http://, https://, ws:// or wss://",
                    spec.url
                )));
            }
        };
        let host = uri
            .host()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| bad(format!("{:?} has no host", spec.url)))?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let port = uri.port_u16().unwrap_or(if scheme.is_tls() { 443 } else { 80 });
        let body = match (spec.body, spec.body_base64) {
            (None, _) => Bytes::new(),
            (Some(text), false) => Bytes::from(text),
            (Some(data), true) => base64::engine::general_purpose::STANDARD
                .decode(data.trim())
                .map(Bytes::from)
                .map_err(|e| bad(format!("the body isn't valid base64: {e}")))?,
        };
        let mut headers = HeaderMap::new();
        for h in &spec.headers {
            let name = HeaderName::from_bytes(h.name.trim().as_bytes())
                .map_err(|_| bad(format!("{:?} isn't a header name", h.name)))?;
            let value = HeaderValue::from_str(h.value.trim())
                .map_err(|_| bad(format!("the value of {:?} isn't a valid header value", h.name)))?;
            headers.append(name, value);
        }
        // The body is sent whole, so its length is known.
        headers.remove(header::CONTENT_LENGTH);
        headers.remove(header::TRANSFER_ENCODING);
        if scheme.is_websocket() {
            if method != Method::GET || !body.is_empty() {
                return Err(bad("a WebSocket handshake is a GET without a body".into()));
            }
            handshake_headers(&mut headers);
        } else if !body.is_empty() || matches!(method, Method::POST | Method::PUT | Method::PATCH) {
            headers.insert(header::CONTENT_LENGTH, HeaderValue::from(body.len()));
        }
        // The exchange addresses the upstream by origin; the request line
        // carries only the path.
        let pq = uri.path_and_query().map_or("/", |pq| pq.as_str());
        let path_uri: Uri = format!("{}://{}{pq}", scheme.as_str(), authority(&host, port, scheme))
            .parse()
            .map_err(|e| bad(format!("{:?} isn't a URL: {e}", spec.url)))?;
        Ok(Prepared {
            method,
            uri: path_uri,
            headers,
            body,
            scheme,
            host,
            port,
        })
    }
}

/// What a WebSocket handshake needs, whatever the spec carried: the
/// upgrade headers, version 13, and a key of its own (a recorded key is
/// the old connection's).
fn handshake_headers(headers: &mut HeaderMap) {
    headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
    headers.insert(header::SEC_WEBSOCKET_VERSION, HeaderValue::from_static("13"));
    let key = base64::engine::general_purpose::STANDARD.encode(uuid::Uuid::new_v4().as_bytes());
    if let Ok(key) = HeaderValue::from_str(&key) {
        headers.insert(header::SEC_WEBSOCKET_KEY, key);
    }
    headers.remove(header::SEC_WEBSOCKET_ACCEPT);
}

fn authority(host: &str, port: u16, scheme: Scheme) -> String {
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let default = if scheme.is_tls() { 443 } else { 80 };
    if port == default {
        host
    } else {
        format!("{host}:{port}")
    }
}
