//! WebSocket rules (M19), against the pm-testkit echo server: rules on the
//! handshake, message rewrites, drops, delays and breakpoints, messages
//! sent from the engine, `onWsMessage` scripts, replays that reconnect, and
//! order under load.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{
    Action, BlockMode, BreakpointInfo, BreakpointResolution, FlowId, FlowState, Matcher, MessageEdits, Pattern, Phase,
    Rule, RuleOrigin, RuleSettings, Scheme, Script, ScriptKind, SendOpts, WsDirection, WsMessage, WsOp, WsOpcode,
    WsPage, WsQuery, WsSend,
};
use pm_testkit::Upstream;
use pm_testkit::client::{eventually, ws_absolute};
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const WAIT: Duration = Duration::from_secs(10);

struct Proxy {
    engine: Arc<Engine>,
    addr: SocketAddr,
    upstream: Upstream,
}

impl Proxy {
    async fn start() -> Proxy {
        let dir = std::env::temp_dir()
            .join("proxymate-tests")
            .join(uuid::Uuid::new_v4().to_string());
        let engine = Engine::with_config(EngineConfig {
            data_dir: Some(dir),
            ..Default::default()
        });
        let config = ProxyConfig {
            ports: 0..=0,
            ..Default::default()
        };
        let addr = engine.start_proxy(config).await.expect("proxy starts");
        Proxy {
            engine,
            addr,
            upstream: Upstream::start().await,
        }
    }

    fn rule(&self, actions: Vec<Action>) -> Rule {
        self.engine
            .create_rule(Rule {
                id: String::new(),
                name: "ws".into(),
                enabled: true,
                order: 0,
                origin: RuleOrigin::User,
                matcher: Matcher {
                    path: Some(Pattern::exact("/ws")),
                    ..Matcher::default()
                },
                actions,
            })
            .expect("rule is valid")
    }

    /// A WebSocket to the echo server through the proxy (absolute form).
    async fn connect(&self) -> (String, WebSocketStream<TcpStream>) {
        let (head, ws) = ws_absolute(self.addr, &self.upstream.url("/ws"), &[]).await;
        (head.clone(), ws.unwrap_or_else(|| panic!("handshake answered {head}")))
    }

    fn messages(&self, id: FlowId) -> WsPage {
        self.engine
            .websocket_messages(id, &WsQuery::default(), 0, 10_000)
            .expect("messages")
    }

    async fn wait_messages(&self, id: FlowId, n: u32) -> WsPage {
        eventually(WAIT, || Some(self.messages(id)).filter(|p| p.total >= n)).await
    }

    async fn breakpoint(&self) -> BreakpointInfo {
        eventually(WAIT, || self.engine.breakpoints().into_iter().next()).await
    }
}

fn ws(direction: Option<WsDirection>, op: WsOp) -> Action {
    Action::WsMessage {
        direction,
        filter: None,
        op,
    }
}

/// The next text or binary message.
async fn next(ws: &mut WebSocketStream<TcpStream>) -> Message {
    loop {
        let message = tokio::time::timeout(WAIT, ws.next())
            .await
            .expect("a message in time")
            .expect("socket open")
            .expect("valid message");
        if matches!(message, Message::Text(_) | Message::Binary(_)) {
            return message;
        }
    }
}

/// Nothing arrives within `wait`.
async fn silent(ws: &mut WebSocketStream<TcpStream>, wait: Duration) {
    if let Ok(Some(Ok(message))) = tokio::time::timeout(wait, ws.next()).await {
        panic!("expected nothing, got {message:?}");
    }
}

fn data(page: &WsPage) -> Vec<&WsMessage> {
    page.messages
        .iter()
        .filter(|m| matches!(m.opcode, WsOpcode::Text | WsOpcode::Binary))
        .collect()
}

#[tokio::test]
async fn rules_apply_to_the_handshake_and_its_101() {
    let proxy = Proxy::start().await;
    proxy.rule(vec![
        // The proxy strips extensions before rules run, so the echo
        // server reporting this one proves the rewrite reached it.
        Action::SetHeader {
            phase: Phase::Request,
            name: "sec-websocket-extensions".into(),
            value: "x-proof".into(),
        },
        Action::SetQuery {
            name: "room".into(),
            value: "7".into(),
        },
        Action::SetHeader {
            phase: Phase::Response,
            name: "x-rule".into(),
            value: "yes".into(),
        },
    ]);
    let (head, mut ws) = proxy.connect().await;
    let head = head.to_ascii_lowercase();
    assert!(head.contains("x-seen-extensions: x-proof"), "{head}");
    assert!(head.contains("x-rule: yes"), "{head}");
    ws.send(Message::text("still works")).await.unwrap();
    assert_eq!(next(&mut ws).await, Message::text("still works"));

    proxy.engine.flush();
    let flow = proxy.engine.get_flow(0).unwrap();
    assert_eq!((flow.scheme, flow.status, flow.edited), (Scheme::Ws, Some(101), true));
    let detail = proxy.engine.flow_detail(0).unwrap().unwrap();
    assert!(detail.request.url.ends_with("/ws?room=7"), "{}", detail.request.url);
    let phases: Vec<Phase> = detail.rules.applied.iter().map(|a| a.phase).collect();
    assert_eq!(phases, [Phase::Request, Phase::Response]);
    let response = detail.response.unwrap();
    assert!(response.headers.iter().any(|h| h.name == "x-rule"));
}

#[tokio::test]
async fn a_blocked_handshake_never_reaches_the_server() {
    let proxy = Proxy::start().await;
    proxy.rule(vec![Action::Block {
        mode: BlockMode::Status { status: 403 },
    }]);
    let (head, ws) = ws_absolute(proxy.addr, &proxy.upstream.url("/ws"), &[]).await;
    assert!(ws.is_none());
    assert!(head.starts_with("HTTP/1.1 403 "), "{head}");
    let flow = eventually(WAIT, || {
        proxy.engine.flush();
        proxy.engine.get_flow(0).filter(|f| f.state != FlowState::Active)
    })
    .await;
    assert_eq!(flow.state, FlowState::Blocked);
}

#[tokio::test]
async fn a_rule_rewrites_a_json_field_and_keeps_the_original() {
    let proxy = Proxy::start().await;
    proxy.rule(vec![ws(
        Some(WsDirection::Received),
        WsOp::JsonSet {
            pointer: "/price".into(),
            value: json!(0),
            raw: Some("12345678901234567890".into()),
        },
    )]);
    let (_, mut ws) = proxy.connect().await;
    let sent = r#"{"item": "tea", "price": 1.50}"#;
    ws.send(Message::text(sent)).await.unwrap();
    let rewritten = r#"{"item": "tea", "price": 12345678901234567890}"#;
    assert_eq!(next(&mut ws).await, Message::text(rewritten));
    // Binary isn't JSON: it goes as it came.
    ws.send(Message::binary(vec![1u8, 2, 3])).await.unwrap();
    assert_eq!(next(&mut ws).await, Message::binary(vec![1u8, 2, 3]));

    let page = proxy.wait_messages(0, 4).await;
    let messages = data(&page);
    assert_eq!(messages.len(), 4);
    // The client's message is untouched; the echo is edited.
    assert!(!messages[0].edited);
    let echo = messages[1];
    assert_eq!((echo.direction, echo.edited), (WsDirection::Received, true));
    assert_eq!(
        proxy.engine.websocket_payload(0, echo.index).unwrap(),
        rewritten.as_bytes()
    );
    assert_eq!(
        proxy.engine.websocket_original_in(0, 0, echo.index).unwrap().unwrap(),
        sent.as_bytes()
    );
    assert_eq!(
        proxy.engine.websocket_original_in(0, 0, messages[0].index).unwrap(),
        None
    );
    proxy.engine.flush();
    assert!(proxy.engine.get_flow(0).unwrap().edited);
    let detail = proxy.engine.flow_detail(0).unwrap().unwrap();
    assert!(
        detail.rules.rewrite_skipped.unwrap().contains("binary"),
        "the skipped binary message is noted"
    );
}

#[tokio::test]
async fn dropped_messages_are_recorded_but_never_forwarded() {
    let proxy = Proxy::start().await;
    proxy.rule(vec![Action::WsMessage {
        direction: Some(WsDirection::Sent),
        filter: Some(Pattern::exact("secret")),
        op: WsOp::Drop,
    }]);
    let (_, mut ws) = proxy.connect().await;
    ws.send(Message::text("my secret")).await.unwrap();
    ws.send(Message::text("public")).await.unwrap();
    // Only the second one reaches the echo server.
    assert_eq!(next(&mut ws).await, Message::text("public"));
    silent(&mut ws, Duration::from_millis(200)).await;
    let page = proxy.wait_messages(0, 3).await;
    let flags: Vec<(&str, bool)> = data(&page).iter().map(|m| (m.preview.as_str(), m.dropped)).collect();
    assert_eq!(flags, [("my secret", true), ("public", false), ("public", false)]);
}

#[tokio::test]
async fn a_delay_holds_the_messages_behind_it() {
    let proxy = Proxy::start().await;
    proxy.rule(vec![Action::WsMessage {
        direction: Some(WsDirection::Sent),
        filter: Some(Pattern::exact("slow")),
        op: WsOp::Delay { ms: 400 },
    }]);
    let (_, mut ws) = proxy.connect().await;
    let started = Instant::now();
    ws.send(Message::text("slow")).await.unwrap();
    ws.send(Message::text("fast")).await.unwrap();
    assert_eq!(next(&mut ws).await, Message::text("slow"));
    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "{:?}",
        started.elapsed()
    );
    // "fast" waited behind it, and kept its place.
    assert_eq!(next(&mut ws).await, Message::text("fast"));
}

#[tokio::test]
async fn a_breakpoint_edits_a_message_and_holds_its_direction() {
    let proxy = Proxy::start().await;
    proxy.rule(vec![Action::WsMessage {
        direction: Some(WsDirection::Sent),
        filter: Some(Pattern::exact("hold")),
        op: WsOp::Breakpoint,
    }]);
    let (_, mut ws) = proxy.connect().await;
    ws.send(Message::text(r#"{"hold":true,"n":1}"#)).await.unwrap();
    ws.send(Message::text("after")).await.unwrap();
    let paused = proxy.breakpoint().await;
    let held = paused.message.clone().expect("a held message");
    assert_eq!((held.direction, held.opcode), (WsDirection::Sent, WsOpcode::Text));
    assert_eq!(paused.phase, Phase::Request);
    assert!(paused.request.url.ends_with("/ws"), "{}", paused.request.url);
    assert_eq!(
        proxy.engine.breakpoint_body(paused.id).unwrap(),
        br#"{"hold":true,"n":1}"#
    );
    // Later messages in that direction wait behind it.
    silent(&mut ws, Duration::from_millis(200)).await;
    // A held message can't be answered like a request.
    let respond = BreakpointResolution::Respond {
        status: 200,
        headers: Vec::new(),
        body: String::new(),
    };
    assert!(proxy.engine.resolve_breakpoint(paused.id, respond).is_err());
    let edits = MessageEdits {
        body: Some(r#"{"hold":false,"n":2}"#.into()),
        ..Default::default()
    };
    proxy
        .engine
        .resolve_breakpoint(paused.id, BreakpointResolution::Continue { edits: Some(edits) })
        .unwrap();
    assert_eq!(next(&mut ws).await, Message::text(r#"{"hold":false,"n":2}"#));
    assert_eq!(next(&mut ws).await, Message::text("after"));

    // Abort drops the held message; binary edits go as bytes.
    ws.send(Message::text("hold me")).await.unwrap();
    let paused = proxy.breakpoint().await;
    proxy
        .engine
        .resolve_breakpoint(paused.id, BreakpointResolution::Abort)
        .unwrap();
    ws.send(Message::binary(b"hold".to_vec())).await.unwrap();
    let paused = proxy.breakpoint().await;
    assert_eq!(paused.message.unwrap().opcode, WsOpcode::Binary);
    let edits = MessageEdits {
        body_base64: Some("AAEC".into()),
        ..Default::default()
    };
    proxy
        .engine
        .resolve_breakpoint(paused.id, BreakpointResolution::Continue { edits: Some(edits) })
        .unwrap();
    assert_eq!(next(&mut ws).await, Message::binary(vec![0u8, 1, 2]));

    let page = proxy.wait_messages(0, 7).await;
    let sent: Vec<(bool, bool)> = data(&page)
        .iter()
        .filter(|m| m.direction == WsDirection::Sent)
        .map(|m| (m.edited, m.dropped))
        .collect();
    assert_eq!(sent, [(true, false), (false, false), (false, true), (true, false)]);
}

#[tokio::test]
async fn an_unanswered_message_breakpoint_times_out_unchanged() {
    let proxy = Proxy::start().await;
    proxy
        .engine
        .set_rule_settings(RuleSettings {
            breakpoint_timeout_secs: 1,
        })
        .unwrap();
    proxy.rule(vec![ws(Some(WsDirection::Received), WsOp::Breakpoint)]);
    let (_, mut ws) = proxy.connect().await;
    ws.send(Message::text("echo me")).await.unwrap();
    let paused = proxy.breakpoint().await;
    assert_eq!(paused.phase, Phase::Response);
    assert!(paused.deadline_ms.is_some());
    assert_eq!(next(&mut ws).await, Message::text("echo me"));
    assert!(proxy.engine.breakpoints().is_empty());
    drop(ws);
    eventually(WAIT, || {
        proxy.engine.flush();
        proxy.engine.get_flow(0).filter(|f| f.state != FlowState::Active)
    })
    .await;
    assert!(proxy.engine.flow_detail(0).unwrap().unwrap().rules.breakpoint_timeout);
}

#[tokio::test]
async fn the_engine_injects_messages_both_ways() {
    let proxy = Proxy::start().await;
    let (_, mut ws) = proxy.connect().await;
    eventually(WAIT, || proxy.engine.websocket_is_open(0).then_some(())).await;
    let to_client = WsSend {
        direction: WsDirection::Received,
        payload: "from proxymate".into(),
        binary: false,
    };
    proxy.engine.ws_send(0, &to_client).unwrap();
    assert_eq!(next(&mut ws).await, Message::text("from proxymate"));
    // To the server, which echoes it back to the client.
    let to_server = WsSend {
        direction: WsDirection::Sent,
        payload: "AAEC".into(),
        binary: true,
    };
    proxy.engine.ws_send(0, &to_server).unwrap();
    assert_eq!(next(&mut ws).await, Message::binary(vec![0u8, 1, 2]));

    let page = proxy.wait_messages(0, 3).await;
    let flags: Vec<(WsDirection, bool)> = data(&page).iter().map(|m| (m.direction, m.injected)).collect();
    assert_eq!(
        flags,
        [
            (WsDirection::Received, true),
            (WsDirection::Sent, true),
            (WsDirection::Received, false)
        ]
    );
    let bad = WsSend {
        direction: WsDirection::Sent,
        payload: "not base64!".into(),
        binary: true,
    };
    assert!(proxy.engine.ws_send(0, &bad).is_err());
    proxy.engine.ws_close(0).unwrap();
    while let Ok(Some(Ok(_))) = tokio::time::timeout(WAIT, ws.next()).await {}
    eventually(WAIT, || (!proxy.engine.websocket_is_open(0)).then_some(())).await;
    assert!(proxy.engine.ws_send(0, &to_client).is_err());
}

#[tokio::test]
async fn a_rule_added_while_open_applies_to_the_next_message() {
    let proxy = Proxy::start().await;
    let (_, mut ws) = proxy.connect().await;
    ws.send(Message::text("before")).await.unwrap();
    assert_eq!(next(&mut ws).await, Message::text("before"));
    proxy.rule(vec![ws_replace("before|after", "changed")]);
    ws.send(Message::text("after")).await.unwrap();
    // Changed on the way out; the echo of "changed" doesn't match.
    assert_eq!(next(&mut ws).await, Message::text("changed"));
}

fn ws_replace(regex: &str, replacement: &str) -> Action {
    ws(
        None,
        WsOp::Replace {
            pattern: Pattern::regex(regex),
            replacement: replacement.into(),
        },
    )
}

#[tokio::test]
async fn on_ws_message_scripts_rewrite_and_drop() {
    let proxy = Proxy::start().await;
    let script = proxy
        .engine
        .create_script(Script {
            id: String::new(),
            name: "ws".into(),
            kind: ScriptKind::Rewrite,
            enabled: true,
            source: r#"
                export function onWsMessage(context, message) {
                  if (message.text === "drop") return null;
                  if (message.direction === "received" && typeof message.body === "object") {
                    message.body.seen = context.phase;
                  }
                  return message;
                }
            "#
            .into(),
            origin: RuleOrigin::User,
            view: None,
            updated_at_ms: 0.0,
        })
        .expect("script is valid");
    proxy.rule(vec![Action::Script { script_id: script.id }]);
    let (_, mut ws) = proxy.connect().await;
    ws.send(Message::text("drop")).await.unwrap();
    ws.send(Message::text(r#"{"a":1}"#)).await.unwrap();
    assert_eq!(next(&mut ws).await, Message::text(r#"{"a":1,"seen":"websocket"}"#));
    let page = proxy.wait_messages(0, 3).await;
    assert!(data(&page)[0].dropped);
}

#[tokio::test]
async fn order_is_kept_under_load() {
    const N: usize = 500;
    let proxy = Proxy::start().await;
    proxy.rule(vec![ws(
        Some(WsDirection::Received),
        WsOp::JsonSet {
            pointer: "/seen".into(),
            value: json!(true),
            raw: None,
        },
    )]);
    let (_, ws) = proxy.connect().await;
    let (mut tx, mut rx) = ws.split();
    let writer = tokio::spawn(async move {
        for i in 0..N {
            tx.send(Message::text(format!(r#"{{"n":{i}}}"#))).await.unwrap();
        }
        tx
    });
    for i in 0..N {
        let message = loop {
            match tokio::time::timeout(WAIT, rx.next()).await.unwrap().unwrap().unwrap() {
                Message::Text(text) => break text,
                _ => continue,
            }
        };
        assert_eq!(message.as_str(), format!(r#"{{"n":{i},"seen":true}}"#));
    }
    let _tx = writer.await.unwrap();
    let page = proxy.wait_messages(0, (N * 2) as u32).await;
    let received: Vec<&WsMessage> = data(&page)
        .into_iter()
        .filter(|m| m.direction == WsDirection::Received)
        .collect();
    assert_eq!(received.len(), N);
    assert!(received.iter().all(|m| m.edited));
}

#[tokio::test]
async fn a_replay_reconnects_with_the_recorded_handshake() {
    let proxy = Proxy::start().await;
    let (_, mut ws) = proxy.connect().await;
    ws.send(Message::text("original")).await.unwrap();
    next(&mut ws).await;
    drop(ws);

    let id = proxy
        .engine
        .replay(
            0,
            0,
            SendOpts {
                wait: true,
                ..SendOpts::default()
            },
        )
        .await
        .expect("replay");
    proxy.engine.flush();
    let flow = proxy.engine.get_flow(id).unwrap();
    assert_eq!(
        (flow.scheme, flow.status, flow.state),
        (Scheme::Ws, Some(101), FlowState::Active)
    );
    eventually(WAIT, || proxy.engine.websocket_is_open(id).then_some(())).await;
    // No client: only the server side can be sent to.
    let to_client = WsSend {
        direction: WsDirection::Received,
        payload: "x".into(),
        binary: false,
    };
    assert!(proxy.engine.ws_send(id, &to_client).is_err());
    let to_server = WsSend {
        direction: WsDirection::Sent,
        payload: "again".into(),
        binary: false,
    };
    proxy.engine.ws_send(id, &to_server).unwrap();
    let page = proxy.wait_messages(id, 2).await;
    let echo = data(&page)[1];
    assert_eq!(
        (echo.direction, echo.preview.as_str()),
        (WsDirection::Received, "again")
    );

    proxy.engine.ws_close(id).unwrap();
    let flow = eventually(WAIT, || {
        proxy.engine.flush();
        proxy.engine.get_flow(id).filter(|f| f.state != FlowState::Active)
    })
    .await;
    assert_eq!(flow.state, FlowState::Completed);
    let close = proxy.messages(id).close.expect("close recorded");
    // Proxymate closed it, speaking for the (absent) client.
    assert_eq!((close.code, close.by), (1000, WsDirection::Sent));
}
