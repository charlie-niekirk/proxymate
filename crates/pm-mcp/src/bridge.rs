//! `mcp-stdio`: for MCP clients that only speak stdio. Reads the bearer
//! token from `app.sqlite`, then relays JSON-RPC between stdin/stdout and
//! the Streamable HTTP endpoint (rmcp's stdio transport on one side, its
//! HTTP client transport, which keeps the session, on the other). If
//! nothing listens yet, it starts the app once and retries for 10 s.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use rmcp::RoleServer;
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, Transport};
use tokio::net::TcpStream;

/// How long to wait for the app to come up.
const LAUNCH_WAIT: Duration = Duration::from_secs(10);
const RETRY_EVERY: Duration = Duration::from_millis(250);

pub struct BridgeConfig {
    /// The endpoint's address (`127.0.0.1:9095`).
    pub addr: SocketAddr,
    /// Where `app.sqlite` (and so the token) lives.
    pub data_dir: PathBuf,
    /// Starts the app when nothing listens, e.g. `open -b <bundle id>`.
    pub launch: Option<Box<dyn Fn() -> std::io::Result<()> + Send>>,
}

/// Relays until stdin closes or the server goes away.
pub async fn run(config: BridgeConfig) -> Result<(), String> {
    let token = wait_for_server(&config).await?;
    let url = format!("http://{}{}", config.addr, crate::http::PATH);
    crate::install_crypto_provider();
    let mut http = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
    );
    let mut stdio = AsyncRwTransport::<RoleServer, _, _>::new_server(tokio::io::stdin(), tokio::io::stdout());
    loop {
        tokio::select! {
            from_client = stdio.receive() => match from_client {
                Some(message) => http.send(message).await.map_err(|e| format!("MCP endpoint: {e}"))?,
                None => break,
            },
            from_server = http.receive() => match from_server {
                Some(message) => stdio.send(message).await.map_err(|e| format!("stdout: {e}"))?,
                None => return Err("the Proxymate MCP endpoint closed the session".into()),
            },
        }
    }
    let _ = http.close().await;
    Ok(())
}

/// Waits until the endpoint accepts connections and a token exists,
/// launching the app once if it doesn't. Returns the token.
async fn wait_for_server(config: &BridgeConfig) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + LAUNCH_WAIT;
    let mut launched = false;
    loop {
        let listening = TcpStream::connect(config.addr).await.is_ok();
        let token = pm_core::mcp::read_token(&config.data_dir);
        if listening && let Some(token) = token {
            return Ok(token);
        }
        if !listening
            && !launched
            && let Some(launch) = &config.launch
        {
            launched = true;
            launch().map_err(|e| format!("can't start Proxymate: {e}"))?;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(match (listening, launched || config.launch.is_none()) {
                (false, _) => format!(
                    "nothing is listening on {} — is Proxymate running with MCP enabled (Settings › MCP)?",
                    config.addr
                ),
                (true, _) => format!("no MCP token in {}", config.data_dir.display()),
            });
        }
        tokio::time::sleep(RETRY_EVERY).await;
    }
}
