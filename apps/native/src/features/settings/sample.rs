//! Sample settings for offscreen renders and tests with no engine: every
//! page with something on it (a row of each Network section, a device of
//! each decision, a schema, a recording, a spec, MCP clients and calls).

use gpui_kit::App;
use pm_model::{
    AutoSaveSettings, AutoSaveStatus, ClientCert, ClientIdentity, DeviceAccess, DeviceClient, DeviceDecision,
    DevicesSnapshot, DnsOverride, LanInterface, ListenerKind, ListenerStatus, LocalActivity, Matcher, McpAuditEntry,
    McpAuditPage, McpCallOutcome, McpClient, McpSettings, McpStatus, MockSource, NetworkSettings, OpenApiSpec, Pattern,
    ProtoMapping, ProtoSchema, ProxyServer, ProxySettings, ProxyStatus, RecordingSettings, ReverseProxy, RuleSettings,
    ScriptSettings, SslSettings, StorageSettings, StorageStatus, SystemProxyStatus, UpdateSettings, UpstreamProxy,
    UpstreamRule, UpstreamSettings,
};

use crate::bridge::{self, keys};
use crate::workspace::Workspace;

pub const TOKEN: &str = "0123456789abcdef0123456789abcdef";
pub const NOW_MS: f64 = 1_795_000_000_000.;

pub fn proxy_status() -> ProxyStatus {
    ProxyStatus {
        listening: Some("127.0.0.1:9090".into()),
        error: None,
        recording: true,
        listeners: vec![
            ListenerStatus {
                kind: ListenerKind::Socks5,
                addr: Some("127.0.0.1:9090".into()),
                target: None,
                error: None,
            },
            ListenerStatus {
                kind: ListenerKind::Reverse,
                addr: Some("127.0.0.1:8080".into()),
                target: Some("http://localhost:3000".into()),
                error: None,
            },
        ],
    }
}

pub fn network() -> NetworkSettings {
    NetworkSettings {
        upstream: UpstreamSettings {
            rules: vec![UpstreamRule {
                enabled: true,
                hosts: vec!["*.corp.example".into()],
                proxy: UpstreamProxy::Http(ProxyServer {
                    host: "proxy.corp.example".into(),
                    port: 3128,
                    username: Some("me".into()),
                    password: None,
                    has_password: true,
                }),
            }],
            ..Default::default()
        },
        reverse_proxies: vec![ReverseProxy {
            enabled: true,
            listen_port: 8080,
            target_url: "http://localhost:3000".into(),
            preserve_host: false,
            tls: false,
        }],
        client_certs: vec![ClientCert {
            enabled: true,
            hosts: vec!["mtls.example.com".into()],
            identity: ClientIdentity::Pkcs12 {
                path: "/Users/me/certs/client.p12".into(),
                password: None,
                has_password: true,
            },
        }],
        dns: vec![DnsOverride {
            enabled: true,
            host: "api.staging.test".into(),
            ip: "10.0.0.7".into(),
        }],
    }
}

pub fn ssl() -> SslSettings {
    SslSettings {
        include: vec!["*".into()],
        exclude: vec!["*.apple.com".into(), "*.icloud.com".into()],
        bypass: vec!["pinned.example.com".into()],
        insecure_upstream: vec!["localhost".into()],
        exclude_apps: vec![],
    }
}

fn device(ip: &str, nickname: Option<&str>, decision: DeviceDecision, flows: u32) -> DeviceClient {
    DeviceClient {
        ip: ip.into(),
        nickname: nickname.map(Into::into),
        decision,
        label: nickname.unwrap_or(ip).into(),
        flows,
        https_decrypted: flows / 2,
        https_refused: if decision == DeviceDecision::Allowed { 1 } else { 0 },
        onboarding: false,
        last_seen_ms: Some(NOW_MS),
        waiting: 0,
        held_until_ms: None,
    }
}

pub fn devices() -> DevicesSnapshot {
    DevicesSnapshot {
        devices: vec![
            device("192.168.1.42", Some("Pixel 9"), DeviceDecision::Allowed, 128),
            device("192.168.1.57", None, DeviceDecision::Pending, 0),
            device("192.168.1.80", None, DeviceDecision::Denied, 0),
        ],
        simulator: LocalActivity::default(),
        emulator: LocalActivity::default(),
    }
}

pub fn mcp() -> McpStatus {
    McpStatus {
        settings: McpSettings {
            enabled: true,
            read_only: false,
        },
        url: Some("http://127.0.0.1:9095/mcp".into()),
        error: None,
        clients: vec![McpClient {
            session: "a1b2c3".into(),
            name: "claude-code".into(),
            version: "2.4.0".into(),
            connected_at_ms: NOW_MS,
        }],
    }
}

pub fn audit() -> McpAuditPage {
    let entry = |id: i64, tool: &str, outcome: McpCallOutcome, error: Option<&str>| McpAuditEntry {
        id,
        time_ms: NOW_MS - id as f64 * 1000.,
        client: "claude-code".into(),
        session: Some("a1b2c3".into()),
        tool: tool.into(),
        arguments: r#"{"filter":"host:api.example.com","limit":20}"#.into(),
        outcome,
        error: error.map(Into::into),
        duration_ms: 3 + id as u32,
    };
    McpAuditPage {
        entries: vec![
            entry(3, "find_flows", McpCallOutcome::Ok, None),
            entry(2, "create_rule", McpCallOutcome::Refused, Some("read-only mode")),
            entry(1, "get_flow_body", McpCallOutcome::Error, Some("no flow 99")),
        ],
        next: Some(0),
    }
}

/// Fills every settings cache and push with the samples (and the
/// workspace's SSL lists and proxy status, when there is one).
pub fn seed(cx: &mut App) {
    let c = keys::settings_caches(cx);
    let r = keys::rules_caches(cx);
    c.proxy.update(cx, |c, cx| c.set((), ProxySettings::default(), cx));
    c.network.update(cx, |c, cx| c.set((), network(), cx));
    c.storage.update(cx, |c, cx| {
        c.set(
            (),
            StorageSettings {
                max_flows: 250_000,
                max_disk_mb: 4096,
                max_ws_messages: 10_000,
            },
            cx,
        )
    });
    c.recording.update(cx, |c, cx| {
        c.set(
            (),
            RecordingSettings {
                only: vec![],
                never: vec!["*.apple.com".into(), "telemetry.example.com".into()],
            },
            cx,
        )
    });
    c.autosave.update(cx, |c, cx| {
        c.set(
            (),
            AutoSaveSettings {
                enabled: true,
                interval_minutes: 10,
                folder: "/Users/me/Documents/Proxymate".into(),
                keep: 5,
            },
            cx,
        )
    });
    c.proto_schemas.update(cx, |c, cx| {
        c.set(
            (),
            vec![ProtoSchema {
                id: 1,
                name: "shop.proto".into(),
                files: vec!["shop.proto".into(), "money.proto".into()],
                services: vec!["shop.v1.Orders".into()],
                messages: 6,
                added_at_ms: NOW_MS as u64,
            }],
            cx,
        )
    });
    c.proto_mappings.update(cx, |c, cx| {
        c.set(
            (),
            vec![ProtoMapping {
                id: "m1".into(),
                matcher: Matcher {
                    host: Some(Pattern::exact("api.example.com")),
                    path: Some(Pattern::exact("/v1/feed")),
                    ..Default::default()
                },
                part: Some(pm_model::BodyPart::Response),
                message_type: "feed.v1.Feed".into(),
            }],
            cx,
        )
    });
    c.openapi_specs.update(cx, |c, cx| {
        c.set(
            (),
            vec![OpenApiSpec {
                id: 1,
                name: "Petstore".into(),
                version: "1.0.0".into(),
                openapi: "3.1.0".into(),
                operations: 3,
                servers: vec!["https://petstore.example.com/v1".into()],
                added_at_ms: NOW_MS,
            }],
            cx,
        )
    });
    c.scripts.update(cx, |c, cx| {
        c.set(
            (),
            ScriptSettings {
                traffic_timeout_ms: 250,
                view_timeout_ms: 1000,
            },
            cx,
        )
    });
    c.mcp_token.update(cx, |c, cx| c.set((), TOKEN.into(), cx));
    c.mcp_audit.update(cx, |c, cx| {
        c.set(pm_view::settings::AUDIT_PAGE, std::sync::Arc::new(audit()), cx)
    });
    c.updates
        .update(cx, |c, cx| c.set((), UpdateSettings { check_at_launch: true }, cx));
    c.device_access.update(cx, |c, cx| {
        c.set(
            (),
            DeviceAccess {
                enabled: true,
                interface: Some("en0".into()),
                interfaces: vec![
                    LanInterface {
                        name: "en0".into(),
                        ip: "192.168.1.10".into(),
                    },
                    LanInterface {
                        name: "en1".into(),
                        ip: "10.0.0.4".into(),
                    },
                ],
                port: Some(9090),
                proxy: Some("192.168.1.10:9090".into()),
                setup_url: Some("http://192.168.1.10:9090/".into()),
                qr_svg: None,
                error: None,
            },
            cx,
        )
    });
    c.system_proxy.update(cx, |c, cx| {
        c.set(
            (),
            SystemProxyStatus {
                enabled: false,
                services: vec![],
                supported: true,
                error: None,
            },
            cx,
        )
    });
    c.terminal_env.update(cx, |c, cx| {
        c.set(
            (),
            "export HTTP_PROXY=http://127.0.0.1:9090\nexport HTTPS_PROXY=http://127.0.0.1:9090\n\
             export SSL_CERT_FILE=\"$HOME/Library/Application Support/Proxymate/ca/bundle.pem\""
                .into(),
            cx,
        )
    });
    r.settings.update(cx, |c, cx| {
        c.set(
            (),
            RuleSettings {
                breakpoint_timeout_secs: 300,
            },
            cx,
        )
    });
    r.mock_sources.update(cx, |c, cx| {
        c.set(
            (),
            vec![MockSource {
                id: "src-1".into(),
                name: "Login flow".into(),
                origin: "/Users/me/Sessions/Login.proxymate".into(),
                flows: 42,
                hosts: vec!["api.example.com".into(), "auth.example.com".into()],
                created_at_ms: NOW_MS,
                size: 812_000,
            }],
            cx,
        )
    });
    bridge::storage(cx).update(cx, |s, cx| {
        s.value = Some(StorageStatus {
            flows: 12_345,
            pinned: 3,
            disk_bytes: 48_000_000,
            evicted: 0,
            error: None,
            dir: "/Users/me/Library/Application Support/Proxymate/live/3f2a".into(),
            sent_bytes: 0,
            received_bytes: 0,
            unrecorded: 17,
            autosave: AutoSaveStatus {
                saving: false,
                last_at_ms: Some(NOW_MS as u64),
                last_path: Some("/Users/me/Documents/Proxymate/Proxymate Autosave 2026-11-21 14.00.proxymate".into()),
                error: None,
            },
        });
        cx.notify();
    });
    bridge::devices(cx).update(cx, |d, cx| {
        d.value = Some(devices());
        cx.notify();
    });
    bridge::mcp(cx).update(cx, |m, cx| {
        m.value = Some(mcp());
        cx.notify();
    });
    if let Some(ws) = Workspace::global(cx) {
        ws.update(cx, |w, cx| {
            w.ssl = ssl();
            w.proxy = proxy_status();
            cx.notify();
        });
    }
}
