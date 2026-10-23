//! One [`ProxymateMcp`] per MCP session. It records who the client is
//! (from `initialize`), filters the tool list in read-only mode, writes
//! every tool call to the audit log, and forwards engine changes as
//! notifications: `tools/list_changed` when read-only flips, and
//! `resources/updated` for subscribed `proxymate://breakpoints` (every hit)
//! and `proxymate://rules`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use pm_core::Engine;
use pm_core::mcp::McpCall;
use pm_model::{McpCallOutcome, McpClient};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, GetPromptRequestParams, GetPromptResponse,
    Implementation, InitializeRequestParams, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams,
    ReadResourceResponse, ResourceUpdatedNotificationParam, ServerCapabilities, ServerConfig, SubscribeRequestParams,
    UnsubscribeRequestParams,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::resources::{BREAKPOINTS_URI, RULES_URI, SCRIPTS_URI};
use crate::{prompts, resources, tools};

const INSTRUCTIONS: &str = "Proxymate is an HTTP(S) debugging proxy. These tools read the traffic it \
records (list_flows, search_flows, get_flow, get_flow_body, get_websocket_messages, wait_for_flow) and \
change what it does to traffic: rules (create_map_local, create_rewrite, create_breakpoint, create_rule) \
and paused flows (list_breakpoints, resolve_breakpoint). Filters use the Proxymate DSL, e.g. \
`host:*.example.com method:POST status:4xx ct:json dur>500 -path:/health`. Rules you create are tagged \
as yours and shown with an AI badge; delete_agent_rules removes them all.";

/// State shared by every session of one endpoint.
pub(crate) struct Shared {
    pub engine: Arc<Engine>,
    sessions: Mutex<HashMap<String, Session>>,
    ct: CancellationToken,
}

struct Session {
    peer: Peer<RoleServer>,
    subscriptions: HashSet<String>,
}

impl Shared {
    pub fn new(engine: Arc<Engine>, ct: CancellationToken) -> Arc<Shared> {
        Arc::new(Shared {
            engine,
            sessions: Mutex::new(HashMap::new()),
            ct,
        })
    }

    /// Forwards engine changes to the sessions until the server stops.
    pub fn spawn_watchers(self: &Arc<Self>) {
        // Read-only on or off: every client re-reads the tool list.
        let shared = self.clone();
        let mut settings = self.engine.watch_mcp_settings();
        tokio::spawn(async move {
            let mut read_only = settings.borrow_and_update().read_only;
            loop {
                tokio::select! {
                    _ = shared.ct.cancelled() => return,
                    changed = settings.changed() => if changed.is_err() { return },
                }
                let now = settings.borrow_and_update().read_only;
                if now != read_only {
                    read_only = now;
                    for peer in shared.peers(None) {
                        let _ = peer.notify_tool_list_changed().await;
                    }
                }
            }
        });

        // Breakpoint hits and rule edits, as resource updates.
        let (tx, mut rx) = mpsc::unbounded_channel::<&'static str>();
        let breakpoints = tx.clone();
        self.engine
            .subscribe_breakpoints(Box::new(move |_| breakpoints.send(BREAKPOINTS_URI).is_ok()));
        let scripts = tx.clone();
        self.engine
            .subscribe_scripts(Box::new(move |_| scripts.send(SCRIPTS_URI).is_ok()));
        self.engine
            .subscribe_rules(Box::new(move |_| tx.send(RULES_URI).is_ok()));
        let shared = self.clone();
        tokio::spawn(async move {
            loop {
                let uri = tokio::select! {
                    _ = shared.ct.cancelled() => return,
                    uri = rx.recv() => match uri { Some(uri) => uri, None => return },
                };
                for peer in shared.peers(Some(uri)) {
                    let _ = peer
                        .notify_resource_updated(ResourceUpdatedNotificationParam::new(uri))
                        .await;
                }
            }
        });
    }

    /// Peers of every session, or of those subscribed to `uri`.
    fn peers(&self, uri: Option<&str>) -> Vec<Peer<RoleServer>> {
        self.sessions
            .lock()
            .values()
            .filter(|s| uri.is_none_or(|u| s.subscriptions.contains(u)))
            .map(|s| s.peer.clone())
            .collect()
    }
}

/// The MCP service for one session.
pub struct ProxymateMcp {
    shared: Arc<Shared>,
    /// Our id for the session: tags agent rules and audit entries.
    id: String,
    client: Mutex<Option<Implementation>>,
}

impl ProxymateMcp {
    pub(crate) fn new(shared: Arc<Shared>) -> ProxymateMcp {
        let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
        ProxymateMcp {
            shared,
            id,
            client: Mutex::new(None),
        }
    }

    fn client_name(&self, context: &RequestContext<RoleServer>) -> String {
        self.client
            .lock()
            .as_ref()
            .map(|c| c.name.clone())
            .or_else(|| context.peer.peer_info().map(|i| i.client_info.name.clone()))
            .unwrap_or_else(|| "unknown client".into())
    }

    fn read_only(&self) -> bool {
        self.shared.engine.mcp_settings().read_only
    }
}

impl Drop for ProxymateMcp {
    fn drop(&mut self) {
        self.shared.sessions.lock().remove(&self.id);
        if self.client.lock().is_some() {
            self.shared.engine.mcp_client_disconnected(&self.id);
        }
    }
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

impl ServerHandler for ProxymateMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_resources()
                .enable_resources_subscribe()
                .enable_prompts()
                .build(),
        )
        .with_server_info(
            Implementation::new("proxymate", env!("CARGO_PKG_VERSION"))
                .with_title("Proxymate")
                .with_description("HTTP(S) debugging proxy: recorded traffic, rules and breakpoints"),
        )
        .with_instructions(INSTRUCTIONS)
    }

    /// Sessions carry the client's identity, the tool list changes and
    /// resources are subscribed, so we speak the protocol versions that
    /// still have `initialize` and sessions.
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::LATEST_WITH_INITIALIZE))
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        let info = request.client_info.clone();
        *self.client.lock() = Some(info.clone());
        self.shared.sessions.lock().insert(
            self.id.clone(),
            Session {
                peer: context.peer.clone(),
                subscriptions: HashSet::new(),
            },
        );
        self.shared.engine.mcp_client_connected(McpClient {
            session: self.id.clone(),
            name: info.name,
            version: info.version,
            connected_at_ms: now_ms(),
        });
        self.negotiate_initialize(&request)
    }

    async fn on_initialized(&self, _context: NotificationContext<RoleServer>) {}

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::list(self.read_only())))
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        tools::list(self.read_only()).into_iter().find(|t| t.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let started = Instant::now();
        let name = request.name.to_string();
        let arguments = request.arguments.unwrap_or_default();
        let client = self.client_name(&context);
        let ctx = tools::Ctx {
            engine: &self.shared.engine,
            client: &client,
            session: &self.id,
            ct: &context.ct,
        };
        let (result, outcome, error) = match tools::kind(&name) {
            None => {
                let message = format!("unknown tool {name:?}");
                (
                    CallToolResult::error(vec![ContentBlock::text(&message)]),
                    McpCallOutcome::Error,
                    Some(message),
                )
            }
            Some(kind) if kind.mutates() && self.read_only() => {
                let message = format!(
                    "{name} changes Proxymate's state, and its MCP server is read-only. The user can turn \
                     read-only mode off in Settings › MCP."
                );
                (
                    CallToolResult::error(vec![ContentBlock::text(&message)]),
                    McpCallOutcome::Refused,
                    Some(message),
                )
            }
            Some(_) => match tools::call(&ctx, &name, arguments.clone()).await {
                Ok(value) => (CallToolResult::structured(value), McpCallOutcome::Ok, None),
                Err(err) => (
                    CallToolResult::error(vec![ContentBlock::text(&err.0)]),
                    McpCallOutcome::Error,
                    Some(err.0),
                ),
            },
        };
        self.shared.engine.record_mcp_call(McpCall {
            client,
            session: Some(self.id.clone()),
            tool: name,
            arguments: serde_json::to_string(&arguments).unwrap_or_default(),
            outcome,
            error,
            duration_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        });
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(resources::list()))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(resources::templates()))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        resources::read(&self.shared.engine, &request.uri).map(Into::into)
    }

    #[allow(deprecated)]
    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if let Some(session) = self.shared.sessions.lock().get_mut(&self.id) {
            session.subscriptions.insert(request.uri);
        }
        Ok(())
    }

    #[allow(deprecated)]
    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if let Some(session) = self.shared.sessions.lock().get_mut(&self.id) {
            session.subscriptions.remove(&request.uri);
        }
        Ok(())
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(prompts::list()))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        prompts::get(&request.name, request.arguments.unwrap_or_default()).map(Into::into)
    }
}
