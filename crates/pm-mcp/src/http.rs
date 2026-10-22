//! The Streamable HTTP endpoint: rmcp's session service mounted at `/mcp`
//! in axum, behind a bearer-token check. rmcp validates `Host` (loopback
//! names only) and `Origin` (none, or a loopback page), which is what stops
//! DNS rebinding: a page on another site that resolves to 127.0.0.1 still
//! sends its own `Host` and `Origin`.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use http::{StatusCode, header};
use pm_core::Engine;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::handler::{ProxymateMcp, Shared};

/// The endpoint's path.
pub const PATH: &str = "/mcp";
/// Sessions idle this long are closed: one-shot clients rarely send the
/// closing `DELETE`, and the toolbar counts open sessions. A client that
/// comes back gets 404 and re-initializes, as the spec has it.
const SESSION_IDLE: Duration = Duration::from_secs(5 * 60);
/// How often a port that was taken is tried again.
const REBIND_EVERY: Duration = Duration::from_secs(5);

/// A running endpoint.
pub struct McpServer {
    pub addr: SocketAddr,
    ct: CancellationToken,
    task: JoinHandle<()>,
}

impl McpServer {
    pub fn url(&self) -> String {
        format!("http://{}{PATH}", self.addr)
    }

    /// Closes every session and stops listening.
    pub async fn stop(self) {
        self.ct.cancel();
        let _ = self.task.await;
    }
}

/// Serves MCP on `listener` until [`McpServer::stop`].
pub async fn serve(engine: Arc<Engine>, listener: TcpListener) -> std::io::Result<McpServer> {
    let addr = listener.local_addr()?;
    let ct = CancellationToken::new();
    let shared = Shared::new(engine.clone(), ct.clone());
    shared.spawn_watchers();

    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive = Some(SESSION_IDLE);
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(["127.0.0.1", "localhost", "::1", "[::1]"])
        .with_allowed_origins(["http://127.0.0.1:*", "http://localhost:*"])
        .enforce_origin_validation()
        .with_cancellation_token(ct.child_token());
    let factory_shared = shared.clone();
    let service = StreamableHttpService::new(
        move || Ok(ProxymateMcp::new(factory_shared.clone())),
        Arc::new(sessions),
        config,
    );
    let app = Router::new()
        .route_service(PATH, service)
        .layer(middleware::from_fn_with_state(engine, require_token));
    let shutdown = ct.clone();
    let task = tokio::spawn(async move {
        let served = axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await;
        if let Err(err) = served {
            eprintln!("proxymate: MCP server stopped: {err}");
        }
    });
    Ok(McpServer { addr, ct, task })
}

/// Every request needs `Authorization: Bearer <token>` (the per-install
/// token from `app.sqlite`, compared in constant time).
async fn require_token(State(engine): State<Arc<Engine>>, request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(str::trim);
    let token = engine.mcp_token();
    match presented {
        Some(presented) if constant_time_eq(presented.as_bytes(), token.as_bytes()) => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer realm=\"proxymate\"")],
            "Proxymate MCP: missing or wrong bearer token. Copy the command from Settings › MCP.\n",
        )
            .into_response(),
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Runs the endpoint on `127.0.0.1:<port>` whenever the MCP settings have
/// it enabled, and reports where it serves (or why it can't) to the
/// engine for the toolbar. A taken port is retried every few seconds.
pub fn spawn_managed(engine: Arc<Engine>, port: u16) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut settings = engine.watch_mcp_settings();
        let mut server: Option<McpServer> = None;
        loop {
            let enabled = settings.borrow_and_update().enabled;
            match (enabled, server.is_some()) {
                (true, false) => match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
                    Ok(listener) => match serve(engine.clone(), listener).await {
                        Ok(running) => {
                            engine.set_mcp_serving(Some(running.url()), None);
                            server = Some(running);
                        }
                        Err(err) => engine.set_mcp_serving(None, Some(err.to_string())),
                    },
                    Err(err) => engine.set_mcp_serving(None, Some(format!("can't listen on 127.0.0.1:{port}: {err}"))),
                },
                (false, true) => {
                    if let Some(running) = server.take() {
                        running.stop().await;
                    }
                    engine.set_mcp_serving(None, None);
                }
                _ => {}
            }
            let retry = enabled && server.is_none();
            tokio::select! {
                changed = settings.changed() => if changed.is_err() { break },
                _ = tokio::time::sleep(REBIND_EVERY), if retry => {}
            }
        }
        if let Some(running) = server {
            running.stop().await;
        }
    })
}
