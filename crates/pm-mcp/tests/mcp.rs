//! The MCP server end to end: an rmcp client over Streamable HTTP against a
//! headless engine with a live proxy and pm-testkit upstreams.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_mcp::McpServer;
use pm_model::{McpCallOutcome, McpSettings};
use pm_testkit::Upstream;
use pm_testkit::client::{self, eventually};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientConfig, Implementation, ReadResourceRequestParams,
    ResourceUpdatedNotificationParam, SubscribeRequestParams,
};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const WAIT: Duration = Duration::from_secs(10);
const CLIENT_NAME: &str = "pm-mcp-test";

struct Harness {
    engine: Arc<Engine>,
    proxy: SocketAddr,
    server: McpServer,
    dir: PathBuf,
}

impl Harness {
    async fn start() -> Harness {
        let dir = std::env::temp_dir()
            .join("proxymate-tests")
            .join(uuid::Uuid::new_v4().to_string());
        let engine = Engine::with_config(EngineConfig {
            data_dir: Some(dir.clone()),
            ..Default::default()
        });
        let proxy = engine
            .start_proxy(ProxyConfig {
                ports: 0..=0,
                ..Default::default()
            })
            .await
            .expect("proxy starts");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server = pm_mcp::serve(engine.clone(), listener).await.unwrap();
        Harness {
            engine,
            proxy,
            server,
            dir,
        }
    }

    async fn client(&self) -> (RunningService<RoleClient, Recorder>, mpsc::UnboundedReceiver<Event>) {
        let (tx, rx) = mpsc::unbounded_channel();
        pm_mcp::install_crypto_provider();
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(self.server.url()).auth_header(self.engine.mcp_token()),
        );
        let client = Recorder { tx }.serve(transport).await.expect("client connects");
        (client, rx)
    }

    async fn stop(self) {
        self.server.stop().await;
        self.engine.shutdown();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[derive(Debug, PartialEq)]
enum Event {
    ToolListChanged,
    ResourceUpdated(String),
}

/// A client that reports the notifications it gets.
struct Recorder {
    tx: mpsc::UnboundedSender<Event>,
}

impl ClientHandler for Recorder {
    fn get_info(&self) -> ClientConfig {
        let mut config = ClientConfig::default();
        config.client_info = Implementation::new(CLIENT_NAME, "1.0");
        config
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        let _ = self.tx.send(Event::ToolListChanged);
    }

    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let _ = self.tx.send(Event::ResourceUpdated(params.uri));
    }
}

async fn call(client: &RunningService<RoleClient, Recorder>, tool: &str, args: Value) -> CallToolResult {
    let Value::Object(args) = args else {
        panic!("arguments must be an object")
    };
    client
        .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(args))
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e}"))
}

/// A successful call's structured result.
async fn ok(client: &RunningService<RoleClient, Recorder>, tool: &str, args: Value) -> Value {
    let result = call(client, tool, args).await;
    assert_ne!(result.is_error, Some(true), "{tool} failed: {:?}", result.content);
    result
        .structured_content
        .unwrap_or_else(|| panic!("{tool}: no structured content"))
}

async fn next_event(rx: &mut mpsc::UnboundedReceiver<Event>, want: impl Fn(&Event) -> bool) {
    tokio::time::timeout(WAIT, async {
        loop {
            let event = rx.recv().await.expect("client still connected");
            if want(&event) {
                return;
            }
        }
    })
    .await
    .expect("notification arrives");
}

#[tokio::test]
async fn tools_list_call_and_paginate() {
    let h = Harness::start().await;
    let upstream = Upstream::start().await;
    let (status, _) = client::get(h.proxy, &upstream.url("/json")).await;
    assert_eq!(status, 200);
    pm_core::synth::seed(&h.engine, 25, 1);
    h.engine.flush();

    let (client, _) = h.client().await;
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for name in [
        "get_status",
        "list_flows",
        "get_flow_body",
        "create_map_local",
        "resolve_breakpoint",
    ] {
        assert!(names.contains(&name), "{name} missing from {names:?}");
    }
    let status_tool = tools.iter().find(|t| t.name == "get_status").unwrap();
    assert_eq!(status_tool.annotations.as_ref().unwrap().read_only_hint, Some(true));
    let delete = tools.iter().find(|t| t.name == "delete_rule").unwrap();
    assert_eq!(delete.annotations.as_ref().unwrap().destructive_hint, Some(true));

    let status = ok(&client, "get_status", json!({})).await;
    assert_eq!(status["flows"], 26);
    assert_eq!(status["mcp"]["client"], CLIENT_NAME);

    // Oldest first, ten at a time, until the cursor runs out.
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = ok(
            &client,
            "list_flows",
            json!({ "limit": 10, "sort": "oldest_first", "cursor": cursor }),
        )
        .await;
        seen.extend(
            page["flows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["id"].as_u64().unwrap()),
        );
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    assert_eq!(seen, (0..26).collect::<Vec<u64>>());

    // The real flow, by filter, then its detail and body.
    let port = upstream.addr().port();
    let found = ok(
        &client,
        "list_flows",
        json!({ "filter": "host:127.0.0.1 path:/json status:2xx" }),
    )
    .await;
    let flows = found["flows"].as_array().unwrap();
    assert_eq!(flows.len(), 1, "{found}");
    let id = flows[0]["id"].as_u64().unwrap();
    assert_eq!(flows[0]["url"], format!("http://127.0.0.1:{port}/json"));
    let flow = ok(&client, "get_flow", json!({ "id": id })).await;
    assert_eq!(flow["response"]["status"], 200);
    let body = ok(&client, "get_flow_body", json!({ "id": id, "json_pointer": "" })).await;
    assert_eq!(body["encoding"], "utf8");
    assert!(body["body"].as_str().unwrap().contains('\n'), "pretty-printed: {body}");
    let cut = ok(
        &client,
        "get_flow_body",
        json!({ "id": id, "format": "text", "max_bytes": 5 }),
    )
    .await;
    assert_eq!(
        (cut["returned_bytes"].as_u64(), cut["more"].as_bool()),
        (Some(5), Some(true))
    );

    let bad = call(&client, "list_flows", json!({ "filter": "hots:x" })).await;
    assert_eq!(bad.is_error, Some(true));

    let search = ok(&client, "search_flows", json!({ "query": "/json", "scopes": ["url"] })).await;
    assert!(search["hits"].as_array().unwrap().iter().any(|h| h["flow"]["id"] == id));

    // Every call is in the audit log, with the client's name.
    let audit = h.engine.mcp_audit(None, 100).unwrap();
    assert!(audit.entries.len() >= 8);
    assert!(audit.entries.iter().all(|e| e.client == CLIENT_NAME));
    assert!(
        audit
            .entries
            .iter()
            .any(|e| e.tool == "list_flows" && e.outcome == McpCallOutcome::Error)
    );
    assert_eq!(h.engine.mcp_status().clients.len(), 1);

    client.cancel().await.unwrap();
    h.stop().await;
}

#[tokio::test]
async fn agents_mock_endpoints_with_tagged_rules() {
    let h = Harness::start().await;
    let upstream = Upstream::start().await;
    let (client, _) = h.client().await;

    let rule = ok(
        &client,
        "create_map_local",
        json!({ "url": format!("http://127.0.0.1:{}/users/*", upstream.addr().port()), "body": "{\"mocked\":true}" }),
    )
    .await;
    assert_eq!(rule["origin"]["kind"], "agent");
    assert_eq!(rule["origin"]["client"], CLIENT_NAME);
    let (status, body) = client::get(h.proxy, &upstream.url("/users/42")).await;
    assert_eq!((status, &body[..]), (200, &b"{\"mocked\":true}"[..]));
    let (status, _) = client::get(h.proxy, &upstream.url("/json")).await;
    assert_eq!(status, 200);

    let rules = ok(&client, "list_rules", json!({ "origin": "agent" })).await;
    assert_eq!(rules["total"], 1);
    let deleted = ok(&client, "delete_agent_rules", json!({})).await;
    assert_eq!(deleted["deleted"], 1);
    let (_, body) = client::get(h.proxy, &upstream.url("/users/42")).await;
    assert_ne!(&body[..], b"{\"mocked\":true}");

    client.cancel().await.unwrap();
    h.stop().await;
}

#[tokio::test]
async fn agents_replay_send_and_export() {
    let h = Harness::start().await;
    let upstream = Upstream::start().await;
    let (status, _) = client::get(h.proxy, &upstream.url("/json")).await;
    assert_eq!(status, 200);
    h.engine.flush();
    let (client, _) = h.client().await;
    let id = ok(&client, "list_flows", json!({ "filter": "path:/json" })).await["flows"][0]["id"]
        .as_u64()
        .unwrap();

    let replay = ok(&client, "replay_flow", json!({ "id": id })).await;
    assert_eq!(replay["flow"]["status"], 200);
    assert_eq!(replay["flow"]["url"], upstream.url("/json"));

    // Modified: another path, a body; /echo sends the body back.
    let edited = ok(
        &client,
        "replay_flow",
        json!({ "id": id, "modify": { "method": "POST", "url": upstream.url("/echo"), "body": "hello agent",
                "set_headers": [{ "name": "content-type", "value": "text/plain" }] } }),
    )
    .await;
    let echo = edited["id"].as_u64().unwrap();
    let body = ok(&client, "get_flow_body", json!({ "id": echo, "format": "text" })).await;
    assert_eq!(body["body"], "hello agent");

    let sent = ok(
        &client,
        "send_request",
        json!({ "request": { "method": "GET", "url": upstream.url("/status/418") } }),
    )
    .await;
    assert_eq!(sent["flow"]["status"], 418);
    let summary = h.engine.get_flow(sent["id"].as_u64().unwrap() as u32).unwrap();
    assert_eq!(summary.sent_by, Some(pm_model::SentBy::Agent));

    let curl = ok(&client, "export_curl", json!({ "id": echo })).await;
    let command = curl["command"].as_str().unwrap();
    assert!(command.starts_with("curl") && command.contains("/echo"), "{command}");

    let har = ok(&client, "export_har", json!({ "filter": "status:2xx" })).await;
    assert_eq!(har["har"]["log"]["version"], "1.2");
    assert_eq!(
        har["har"]["log"]["entries"].as_array().unwrap().len(),
        har["flows"].as_u64().unwrap() as usize
    );

    client.cancel().await.unwrap();
    h.stop().await;
}

#[tokio::test]
async fn read_only_mode_changes_the_tool_list_live() {
    let h = Harness::start().await;
    let (client, mut events) = h.client().await;
    let all = client.list_all_tools().await.unwrap().len();

    h.engine
        .set_mcp_settings(McpSettings {
            enabled: true,
            read_only: true,
        })
        .unwrap();
    next_event(&mut events, |e| *e == Event::ToolListChanged).await;
    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.len() < all);
    assert!(
        tools
            .iter()
            .all(|t| t.annotations.as_ref().unwrap().read_only_hint == Some(true))
    );
    assert!(!tools.iter().any(|t| t.name == "create_rule"));

    let refused = call(&client, "clear_flows", json!({})).await;
    assert_eq!(refused.is_error, Some(true));
    let audit = h.engine.mcp_audit(None, 1).unwrap();
    assert_eq!(audit.entries[0].outcome, McpCallOutcome::Refused);

    h.engine.set_mcp_settings(McpSettings::default()).unwrap();
    next_event(&mut events, |e| *e == Event::ToolListChanged).await;
    assert_eq!(client.list_all_tools().await.unwrap().len(), all);

    client.cancel().await.unwrap();
    h.stop().await;
}

#[tokio::test]
async fn a_breakpoint_is_resolved_over_mcp_while_the_request_waits() {
    let h = Harness::start().await;
    let upstream = Upstream::start().await;
    let (client, mut events) = h.client().await;
    #[allow(deprecated)] // resources/subscribe is how 2025-11-25 sessions subscribe.
    client
        .subscribe(SubscribeRequestParams::new("proxymate://breakpoints"))
        .await
        .unwrap();
    ok(
        &client,
        "create_breakpoint",
        json!({ "url": format!("127.0.0.1:{}/json", upstream.addr().port()) }),
    )
    .await;

    let proxy = h.proxy;
    let url = upstream.url("/json");
    let request = tokio::spawn(async move { client::get(proxy, &url).await });

    next_event(&mut events, |e| {
        *e == Event::ResourceUpdated("proxymate://breakpoints".into())
    })
    .await;
    let id = eventually(WAIT, || h.engine.breakpoints().first().map(|b| b.id)).await;
    let listed = ok(&client, "list_breakpoints", json!({})).await;
    assert_eq!(listed["breakpoints"][0]["id"], id);
    let resource = client
        .read_resource(ReadResourceRequestParams::new("proxymate://breakpoints"))
        .await
        .unwrap();
    assert!(format!("{resource:?}").contains("/json"));
    let paused = ok(&client, "get_breakpoint", json!({ "id": id })).await;
    assert_eq!(paused["phase"], "request");

    ok(
        &client,
        "resolve_breakpoint",
        json!({ "id": id, "resolution": { "action": "respond", "status": 299, "body": "from the agent" } }),
    )
    .await;
    let (status, body) = tokio::time::timeout(WAIT, request).await.unwrap().unwrap();
    assert_eq!((status, &body[..]), (299, &b"from the agent"[..]));

    let again = call(
        &client,
        "resolve_breakpoint",
        json!({ "id": id, "resolution": { "action": "abort" } }),
    )
    .await;
    assert_eq!(again.is_error, Some(true));

    client.cancel().await.unwrap();
    h.stop().await;
}

/// A raw POST of `initialize` with the given headers.
async fn post(addr: SocketAddr, headers: &[(&str, &str)]) -> u16 {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(conn);
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "raw", "version": "1" } }
    });
    let mut request = http::Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        request = request.header("host", addr.to_string());
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request.body(Full::new(Bytes::from(body.to_string()))).unwrap();
    let response = sender.send_request(request).await.unwrap();
    let status = response.status().as_u16();
    let _ = response.into_body().collect().await;
    status
}

#[tokio::test]
async fn tokens_hosts_and_origins_are_checked() {
    let h = Harness::start().await;
    let addr = h.server.addr;
    let bearer = format!("Bearer {}", h.engine.mcp_token());

    assert_eq!(post(addr, &[]).await, 401);
    assert_eq!(post(addr, &[("authorization", "Bearer nope")]).await, 401);
    assert_eq!(post(addr, &[("authorization", &bearer)]).await, 200);

    // DNS rebinding: a page on another site that resolves to 127.0.0.1.
    assert_eq!(
        post(addr, &[("authorization", &bearer), ("host", "evil.example:9095")]).await,
        403
    );
    assert_eq!(
        post(addr, &[("authorization", &bearer), ("origin", "http://evil.example")]).await,
        403
    );
    assert_eq!(
        post(addr, &[("authorization", &bearer), ("origin", "http://localhost:6274")]).await,
        200
    );

    // A new token locks out the old one.
    h.engine.regenerate_mcp_token().unwrap();
    assert_eq!(post(addr, &[("authorization", &bearer)]).await, 401);

    h.stop().await;
}
