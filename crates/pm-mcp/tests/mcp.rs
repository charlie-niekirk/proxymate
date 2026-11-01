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

#[tokio::test(flavor = "multi_thread")]
async fn agents_serve_offline_from_a_recording() {
    let h = Harness::start().await;
    let upstream = Upstream::start().await;
    let url = upstream.url("/json");
    let (status, recorded) = client::get(h.proxy, &url).await;
    assert_eq!(status, 200);
    h.engine.flush();
    drop(upstream);

    let (client, _) = h.client().await;
    let made = ok(
        &client,
        "create_serve_from",
        json!({ "store": 0, "name": "capture", "on_miss": "not_found" }),
    )
    .await;
    assert_eq!(made["rule"]["origin"]["kind"], "agent");
    assert_eq!(made["rule"]["actions"][0]["type"], "ServeFrom");
    assert_eq!(made["rule"]["actions"][0]["on_miss"], "not_found");
    assert_eq!(made["source"]["name"], "capture");
    let (status, body) = client::get(h.proxy, &url).await;
    assert_eq!((status, body), (200, recorded));
    let missing = url.replace("/json", "/missing");
    assert_eq!(client::get(h.proxy, &missing).await.0, 404);
    let sources = ok(&client, "list_mock_sources", json!({})).await;
    assert_eq!(sources["sources"].as_array().unwrap().len(), 1);
    let both = call(&client, "create_serve_from", json!({ "store": 0, "source": "x" })).await;
    assert_eq!(both.is_error, Some(true));

    // `op:` filters bind to the imported specs over MCP too.
    h.engine
        .import_openapi(
            "local",
            r#"{"openapi":"3.1.0","info":{"title":"Local","version":"1"},"paths":{"/json":{"get":{"operationId":"getJson","summary":"The JSON"}}}}"#,
        )
        .unwrap();
    h.engine.flush();
    let flows = ok(&client, "list_flows", json!({ "filter": "op:getJson" })).await;
    let flows = flows["flows"].as_array().unwrap();
    assert!(!flows.is_empty());
    let flow = ok(&client, "get_flow", json!({ "id": flows[0]["id"] })).await;
    assert_eq!(flow["operation"]["operation_id"], "getJson");

    client.cancel().await.unwrap();
    h.stop().await;
}

#[tokio::test]
async fn agents_write_test_and_run_scripts() {
    let h = Harness::start().await;
    let upstream = Upstream::start().await;
    let (client, mut events) = h.client().await;
    #[allow(deprecated)] // resources/subscribe is how 2025-11-25 sessions subscribe.
    let subscribe = client
        .subscribe(SubscribeRequestParams::new("proxymate://scripts"))
        .await;
    assert!(subscribe.is_ok());

    // A mock script, run by a rule, serves a body with a base64 JSON field.
    let mock = ok(
        &client,
        "create_script",
        json!({
            "name": "Profile",
            "kind": "mock",
            "source": "export function onMock() { return { body: { payload: btoa(JSON.stringify({ user: 'ada' })) } }; }",
        }),
    )
    .await;
    assert_eq!(mock["origin"]["kind"], "agent");
    next_event(&mut events, |e| {
        *e == Event::ResourceUpdated("proxymate://scripts".into())
    })
    .await;
    let rule = ok(
        &client,
        "create_rule",
        json!({
            "name": "mock profile",
            "matcher": { "path": { "kind": "exact", "value": "/profile" } },
            "actions": [{ "type": "Script", "script_id": mock["id"] }],
        }),
    )
    .await;
    // A view script decodes it.
    let view = ok(
        &client,
        "create_script",
        json!({
            "name": "Decode payload",
            "kind": "view",
            "source": "export function onView(c, flow) { console.log('viewing', flow.id); \
                       return { title: 'Payload', body: Proxymate.base64DecodeJson(flow.response.body.payload) }; }",
            "view": { "parts": ["response"], "kinds": ["json"] },
        }),
    )
    .await;
    let (status, body) = client::get(h.proxy, &upstream.url("/profile")).await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("payload"));
    let flow = eventually(WAIT, || {
        h.engine.flush();
        h.engine.get_flow(0).filter(|f| f.status.is_some())
    })
    .await;

    let views = ok(&client, "get_flow_views", json!({ "id": flow.id })).await;
    let result = &views["results"][0];
    assert_eq!(result["script_id"], view["id"]);
    assert_eq!(result["views"][0]["text"], "{\n  \"user\": \"ada\"\n}");
    let console = ok(&client, "get_script_console", json!({})).await;
    assert_eq!(
        console["entries"].as_array().unwrap().last().unwrap()["message"],
        "viewing 0"
    );

    let test = ok(
        &client,
        "test_script",
        json!({ "flow_id": flow.id, "kind": "rewrite", "source": "export function onResponse(c, u, q, r) { r.status = 418; }" }),
    )
    .await;
    assert_eq!(test["response"]["status"], 418);
    assert_eq!(test["response_before"]["status"], 200);

    let listed = ok(&client, "list_scripts", json!({ "kind": "view" })).await;
    assert_eq!(listed["total"], 1);
    let api = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new("proxymate://scripts/api"))
        .await
        .unwrap();
    let rmcp::model::ResourceContents::TextResourceContents { text, .. } = &api.contents[0] else {
        panic!("text")
    };
    assert!(text.contains("declare function onView"));

    // The mock script is in use, so only the view script goes.
    let deleted = ok(&client, "delete_agent_scripts", json!({})).await;
    assert_eq!(deleted["deleted"], 1);
    let refused = call(&client, "delete_script", json!({ "id": mock["id"] })).await;
    assert_eq!(refused.is_error, Some(true));
    ok(&client, "delete_rule", json!({ "id": rule["id"] })).await;
    ok(&client, "delete_script", json!({ "id": mock["id"] })).await;
    assert!(h.engine.scripts().is_empty());

    // Read-only mode hides every tool that saves or runs agent code.
    h.engine
        .set_mcp_settings(McpSettings {
            enabled: true,
            read_only: true,
        })
        .unwrap();
    next_event(&mut events, |e| *e == Event::ToolListChanged).await;
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for hidden in [
        "create_script",
        "update_script",
        "delete_script",
        "test_script",
        "set_script_enabled",
    ] {
        assert!(!names.iter().any(|n| n == hidden), "{hidden}");
    }
    for shown in ["list_scripts", "get_script", "get_flow_views", "get_script_console"] {
        assert!(names.iter().any(|n| n == shown), "{shown}");
    }

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

    let python = ok(&client, "export_code", json!({ "id": echo, "language": "python" })).await;
    let code = python["code"].as_str().unwrap();
    assert!(
        code.contains("requests.post(") && code.contains("hello agent"),
        "{code}"
    );

    let repeated = ok(
        &client,
        "repeat_flow",
        json!({ "id": id, "iterations": 20, "concurrency": 4, "apply_rules": false }),
    )
    .await;
    assert_eq!(
        (repeated["sent"].as_u64(), repeated["failed"].as_u64()),
        (Some(20), Some(0))
    );
    assert_eq!(repeated["statuses"], json!([{ "status": 200, "count": 20 }]));
    let last = repeated["last_flow"].as_u64().unwrap() as u32;
    let flow = h.engine.get_flow(last).unwrap();
    assert_eq!(flow.sent_by, Some(pm_model::SentBy::Repeat));
    assert!(flow.client.as_deref().is_some_and(|c| c != "Proxymate"));

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

/// An agent holds a WebSocket message at a breakpoint, edits it, sends
/// its own messages both ways and closes the connection.
#[tokio::test]
async fn agents_hold_edit_and_send_websocket_messages() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let h = Harness::start().await;
    let upstream = Upstream::start().await;
    let (client, _events) = h.client().await;
    ok(
        &client,
        "create_rule",
        json!({
            "name": "hold sent messages",
            "matcher": { "path": { "kind": "exact", "value": "/ws" } },
            "actions": [{ "type": "WsMessage", "direction": "sent", "op": { "kind": "breakpoint" } }]
        }),
    )
    .await;
    let (head, ws) = client::ws_absolute(h.proxy, &upstream.url("/ws"), &[]).await;
    let mut ws = ws.unwrap_or_else(|| panic!("handshake answered {head}"));
    async fn next(ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> String {
        let text = async {
            loop {
                if let Message::Text(text) = ws.next().await.expect("open").expect("valid") {
                    return text.to_string();
                }
            }
        };
        tokio::time::timeout(WAIT, text).await.expect("a message in time")
    }
    ws.send(Message::text(r#"{"price":1}"#)).await.unwrap();

    let id = eventually(WAIT, || h.engine.breakpoints().first().map(|b| b.id)).await;
    let paused = ok(&client, "get_breakpoint", json!({ "id": id })).await;
    assert_eq!(paused["message"]["direction"], "sent");
    assert_eq!(paused["body_text"], r#"{"price":1}"#);
    let refused = call(
        &client,
        "resolve_breakpoint",
        json!({ "id": id, "resolution": { "action": "respond", "status": 200 } }),
    )
    .await;
    assert_eq!(refused.is_error, Some(true));
    ok(
        &client,
        "resolve_breakpoint",
        json!({ "id": id, "resolution": { "action": "continue", "edits": { "body": "{\"price\":0}" } } }),
    )
    .await;
    assert_eq!(next(&mut ws).await, r#"{"price":0}"#);

    ok(
        &client,
        "send_websocket_message",
        json!({ "id": 0, "direction": "received", "payload": "hello from the agent" }),
    )
    .await;
    assert_eq!(next(&mut ws).await, "hello from the agent");

    let page = ok(&client, "get_websocket_messages", json!({ "id": 0 })).await;
    let messages = page["messages"].as_array().unwrap();
    let edited = messages
        .iter()
        .find(|m| m["edited"] == true)
        .expect("the edited message");
    assert_eq!(
        (edited["payload"].as_str(), edited["original"].as_str()),
        (Some(r#"{"price":0}"#), Some(r#"{"price":1}"#))
    );
    assert!(
        messages
            .iter()
            .any(|m| m["injected"] == true && m["payload"] == "hello from the agent")
    );
    assert_eq!(page["open"], true);

    ok(&client, "close_websocket", json!({ "id": 0 })).await;
    while let Ok(Some(Ok(_))) = tokio::time::timeout(WAIT, ws.next()).await {}
    eventually(WAIT, || (!h.engine.websocket_is_open(0)).then_some(())).await;
    let late = call(
        &client,
        "send_websocket_message",
        json!({ "id": 0, "direction": "sent", "payload": "too late" }),
    )
    .await;
    assert_eq!(late.is_error, Some(true));

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

#[tokio::test]
async fn agents_dry_run_and_create_exact_rewrites() {
    let h = Harness::start().await;
    let upstream = Upstream::start().await;
    let (status, _) = client::get(h.proxy, &upstream.url("/json")).await;
    assert_eq!(status, 200);
    h.engine.flush();
    let (client, _) = h.client().await;
    let id = ok(&client, "list_flows", json!({ "filter": "path:/json" })).await["flows"][0]["id"]
        .as_u64()
        .unwrap();

    // An unsaved rule, tried on the recorded flow.
    let dry = ok(
        &client,
        "test_rule",
        json!({ "flow_id": id, "rule": { "matcher": { "path": { "kind": "exact", "value": "/json" } },
                "actions": [{ "type": "JsonSet", "phase": "response", "pointer": "/big", "value": null,
                              "raw": "12345678901234567890" }] } }),
    )
    .await;
    assert_eq!(dry["matched"], true);
    assert!(
        dry["response_body"]
            .as_str()
            .unwrap()
            .contains("\"big\":12345678901234567890"),
        "{dry}"
    );
    let miss = ok(
        &client,
        "test_rule",
        json!({ "flow_id": id, "rule": { "matcher": { "path": { "kind": "exact", "value": "/other" } }, "actions": [] } }),
    )
    .await;
    assert_eq!(miss["matched"], false);

    let rule = ok(
        &client,
        "create_rewrite",
        json!({ "flow_id": id, "json_set": [{ "pointer": "/id", "raw": "9007199254740993" }],
                "json_remove": [{ "pointer": "/nested" }], "set_query": [{ "name": "page", "value": "2" }],
                "form_set": [{ "name": "a", "value": "b" }],
                "add_response_headers": [{ "name": "x-a", "value": "1" }, { "name": "x-a", "value": "2" }] }),
    )
    .await;
    let types: Vec<&str> = rule["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        ["AddHeader", "AddHeader", "SetQuery", "JsonRemove", "JsonSet", "FormSet"]
    );
    let saved = ok(&client, "test_rule", json!({ "flow_id": id, "rule_id": rule["id"] })).await;
    let body = saved["response_body"].as_str().unwrap();
    assert!(
        body.contains("9007199254740993") && !body.contains("\"nested\""),
        "{body}"
    );
    assert!(
        saved["request"]["url"].as_str().unwrap().ends_with("?page=2"),
        "{saved}"
    );

    client.cancel().await.unwrap();
    h.stop().await;
}

/// M16: `format: "decoded"` gives agents the inspector's decoding, and
/// `list_proto_schemas` what was imported.
#[tokio::test]
async fn decoded_bodies_and_proto_schemas() {
    let h = Harness::start().await;
    let upstream = Upstream::start().await;
    for name in ["msgpack", "grpc-web"] {
        let fixture = pm_testkit::zoo::fixture(name).unwrap();
        let mut req = http::Request::builder()
            .method(fixture.method.as_str())
            .uri(upstream.url(&fixture.path()));
        if let Some(r) = &fixture.request {
            req = req.header("content-type", &r.content_type);
        }
        let body = fixture.request_body().map(Bytes::from_static).unwrap_or_default();
        let response = client::send(h.proxy, req.body(Full::new(body)).unwrap()).await;
        let _ = response.into_body().collect().await;
    }
    eventually(WAIT, || {
        h.engine.flush();
        (0..2)
            .all(|id| {
                h.engine
                    .get_flow(id)
                    .is_some_and(|f| f.state != pm_model::FlowState::Active)
            })
            .then_some(())
    })
    .await;
    h.engine
        .import_proto_schema(
            "shop",
            pm_core::ProtoImport::Sources(vec![("shop.proto".into(), pm_testkit::grpc::SHOP_PROTO.into())]),
        )
        .unwrap();

    let (client, _) = h.client().await;
    let msgpack = ok(&client, "get_flow_body", json!({ "id": 0, "format": "decoded" })).await;
    assert_eq!(msgpack["format"], "messagepack");
    assert_eq!(msgpack["messages"][0]["value"]["user"], "ada");
    assert_eq!(msgpack["messages"][0]["value"]["big"], json!(u64::MAX));
    assert!(
        msgpack["messages"][0]["badges"]["/seen_at"]
            .as_str()
            .unwrap()
            .starts_with("ext -1")
    );

    // The zoo serves gRPC-web at /zoo/grpc-web, not a service path: schema-less.
    let grpc = ok(&client, "get_flow_body", json!({ "id": 1, "format": "decoded" })).await;
    assert_eq!(grpc["format"], "grpcweb");
    assert_eq!(grpc["status"]["name"], "OK");
    assert_eq!(grpc["messages"][0]["schema_less"], true);
    assert_eq!(grpc["messages"][0]["value"]["1"], "o-1001");
    let pointer = ok(
        &client,
        "get_flow_body",
        json!({ "id": 1, "format": "decoded", "json_pointer": "/3/1/1" }),
    )
    .await;
    assert_eq!(pointer["messages"][0]["value"], "mug");

    let schemas = ok(&client, "list_proto_schemas", json!({})).await;
    assert_eq!(schemas["schemas"][0]["services"][0], "shop.v1.Orders");
    assert_eq!(schemas["mappings"], json!([]));

    let json_body = call(&client, "get_flow_body", json!({ "id": 999, "format": "decoded" })).await;
    assert_eq!(json_body.is_error, Some(true));
}
