//! The Settings window's menus, built from state by key, so tests and the
//! dev remote run them as the user would (`SettingsWindow::menu_for`):
//!
//! - `listening`, `lan-interface`, `storage-status`, `mcp-serving`;
//! - `list:<list id>:<item>` (a list editor's row: Copy, Remove);
//! - `device:<ip>` (`DeviceMenu`), `mcp-client:<session>`, `audit:<id>`;
//! - `copy:<field>` (a copyable field: its copy and the extra copies);
//! - `picker:<key>` (a dropdown's options).

use gpui_kit::App;
use pm_model::{DeviceDecision, ListenerKind};
use pm_view::settings::{self as logic, network};

use super::window::SettingsWindow;
use super::{SettingsChoice, SettingsPress};
use crate::features::context_menu::Menu;
use crate::workspace::Workspace;

/// A copyable field (`CopyField`): what it copies and shows.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyField {
    pub value: String,
    /// Shown in place of `value` (the masked token).
    pub display: Option<String>,
    /// The Copy button's and the menu's first entry.
    pub label: String,
    pub what: String,
    pub multiline: bool,
    /// More copies in its menu: (label, text, what).
    pub more: Vec<(String, String, String)>,
}

fn press(id: impl Into<String>) -> SettingsPress {
    SettingsPress { id: id.into().into() }
}

impl SettingsWindow {
    fn proxy_status(cx: &App) -> pm_model::ProxyStatus {
        Workspace::global(cx)
            .map(|w| w.read(cx).proxy.clone())
            .unwrap_or_default()
    }

    /// The MCP endpoint's URL (the default port until the server says).
    pub fn mcp_url(&self, cx: &App) -> String {
        self.mcp
            .read(cx)
            .value
            .as_ref()
            .and_then(|s| s.url.clone())
            .unwrap_or_else(|| logic::DEFAULT_MCP_URL.into())
    }

    /// The copyable field `id`: `terminal-env`, `socks-address`,
    /// `socks-curl`, `mcp-claude`, `mcp-json`.
    pub fn copy_field(&self, id: &str, cx: &App) -> Option<CopyField> {
        let simple = |value: String, label: &str, what: &str, multiline: bool| CopyField {
            value,
            display: None,
            label: label.into(),
            what: what.into(),
            multiline,
            more: Vec::new(),
        };
        match id {
            "terminal-env" => {
                let env = self.caches.terminal_env.read(cx).peek(&())?.clone();
                Some(simple(env, "Copy", "environment", true))
            }
            "socks-address" => {
                let addr = logic::socks_addr(&Self::proxy_status(cx))?;
                Some(simple(
                    format!("socks5h://{addr}"),
                    "Copy SOCKS5 address",
                    "SOCKS5 address",
                    false,
                ))
            }
            "socks-curl" => {
                let addr = logic::socks_addr(&Self::proxy_status(cx))?;
                Some(simple(
                    format!("curl --socks5-hostname {addr} https://example.com/"),
                    "Copy curl example",
                    "curl command",
                    false,
                ))
            }
            "mcp-claude" | "mcp-json" => {
                let token = self.caches.mcp_token.read(cx).peek(&())?.clone();
                let url = self.mcp_url(cx);
                let masked = logic::mask_token(&token);
                let more = vec![
                    ("Copy URL".into(), url.clone(), "URL".into()),
                    ("Copy bearer token".into(), token.clone(), "token".into()),
                ];
                Some(if id == "mcp-claude" {
                    CopyField {
                        value: logic::claude_add_command(&url, &token),
                        display: Some(logic::claude_add_command(&url, &masked)),
                        label: "Copy the claude mcp add command".into(),
                        what: "command".into(),
                        multiline: false,
                        more,
                    }
                } else {
                    CopyField {
                        value: logic::mcp_json_config(&url, &token),
                        display: Some(logic::mcp_json_config(&url, &masked)),
                        label: "Copy the JSON config".into(),
                        what: "JSON config".into(),
                        multiline: true,
                        more,
                    }
                })
            }
            _ => None,
        }
    }

    /// The menu `key` names (see the module docs); `None` if it has none now.
    pub fn menu_for(&self, key: &str, cx: &App) -> Option<Menu> {
        let (kind, arg) = key.split_once(':').unwrap_or((key, ""));
        let menu = match kind {
            "listening" => {
                let status = Self::proxy_status(cx);
                Menu::new()
                    .when_some_copy(status.listening.as_deref(), "Copy address", "proxy address")
                    .when_some_copy(status.error.as_deref(), "Copy error", "error")
            }
            "lan-interface" => {
                let access = self.integration.access.read(cx).peek(&())?;
                let ip = access
                    .interfaces
                    .iter()
                    .find(|i| Some(&i.name) == access.interface.as_ref())
                    .map(|i| i.ip.as_str());
                Menu::new()
                    .when_some_copy(ip, "Copy IP address", "IP address")
                    .when_some_copy(access.proxy.as_deref(), "Copy address and port", "proxy address")
            }
            "storage-status" => {
                let status = self.storage.read(cx).value.as_ref()?;
                Menu::new().copy("Copy location", status.dir.clone(), "path")
            }
            "list" => {
                let (list_id, item) = arg.split_once(':')?;
                if !self.list_items(list_id, cx).iter().any(|i| i == item) {
                    return None;
                }
                Menu::new()
                    .copy(format!("Copy {item}"), item.to_owned(), "entry")
                    .item("Remove", press(format!("list.{list_id}.remove:{item}")))
            }
            "device" => {
                let snapshot = self.devices.read(cx).value.as_ref()?;
                let d = snapshot.devices.iter().find(|d| d.ip == arg)?;
                Menu::new()
                    .copy("Copy IP address", d.ip.clone(), "IP address")
                    .when_some_copy(d.nickname.as_deref(), "Copy nickname", "nickname")
                    .separator()
                    .when(d.decision != DeviceDecision::Allowed, |m| {
                        m.item("Allow", press(format!("client.allow:{}", d.ip)))
                    })
                    .when(d.decision != DeviceDecision::Denied, |m| {
                        m.item("Deny", press(format!("client.deny:{}", d.ip)))
                    })
                    .item("Forget", press(format!("client.forget:{}", d.ip)))
            }
            "copy" => {
                let field = self.copy_field(arg, cx)?;
                field.more.iter().fold(
                    Menu::new().copy(field.label.clone(), field.value.clone(), field.what.clone()),
                    |m, (label, text, what)| m.copy(label.clone(), text.clone(), what.clone()),
                )
            }
            "mcp-serving" => {
                let status = self.mcp.read(cx).value.as_ref()?;
                Menu::new()
                    .when_some_copy(status.url.as_deref(), "Copy URL", "URL")
                    .when_some_copy(status.error.as_deref(), "Copy error", "error")
            }
            "mcp-client" => {
                let status = self.mcp.read(cx).value.as_ref()?;
                let c = status.clients.iter().find(|c| c.session == arg)?;
                Menu::new()
                    .copy("Copy name", c.name.clone(), "client name")
                    .copy("Copy name and version", format!("{} {}", c.name, c.version), "client")
                    .copy("Copy session id", c.session.clone(), "session id")
            }
            "audit" => {
                let page = self.caches.mcp_audit.read(cx).peek(&self.audit_limit())?;
                let e = page.entries.iter().find(|e| e.id.to_string() == arg)?;
                Menu::new()
                    .copy("Copy tool", e.tool.clone(), "tool name")
                    .copy("Copy arguments", e.arguments.clone(), "arguments")
                    .when_some_copy(e.error.as_deref(), "Copy error", "error")
                    .copy("Copy client", e.client.clone(), "client")
                    .separator()
                    .copy("Copy entry as JSON", logic::audit_entry_json(e), "entry")
            }
            "picker" => {
                let (options, current) = self.picker_options(arg, cx)?;
                options.into_iter().fold(Menu::new(), |m, (value, label)| {
                    let on = value == current;
                    m.check(
                        label,
                        on,
                        SettingsChoice {
                            key: arg.to_owned().into(),
                            value: value.into(),
                        },
                    )
                })
            }
            _ => return None,
        };
        (!menu.is_empty()).then_some(menu)
    }

    /// A picker's options (value, label) and its current value.
    pub fn picker_options(&self, key: &str, cx: &App) -> Option<(Vec<(String, String)>, String)> {
        let pairs = |list: &[(&str, &str)]| list.iter().map(|(v, l)| ((*v).into(), (*l).into())).collect();
        if key == "proxy.interface" {
            let access = self.integration.access.read(cx).peek(&())?;
            let options = access
                .interfaces
                .iter()
                .map(|i| (i.name.clone(), format!("{} ({})", i.ip, i.name)))
                .collect();
            return Some((options, access.interface.clone().unwrap_or_default()));
        }
        let rest = key.strip_prefix("net.")?;
        let draft = &self.network.as_ref()?.draft;
        let current = network::chosen(draft, rest)?;
        let options = if rest.ends_with(".kind") {
            pairs(&[("http", "HTTP"), ("socks5", "SOCKS5"), ("direct", "Direct")])
        } else if rest.ends_with(".format") {
            pairs(&[("pkcs12", "PKCS#12"), ("pem", "PEM")])
        } else {
            return None;
        };
        Some((options, current))
    }

    /// The reverse proxies' listeners, for their rows' status lines.
    pub fn reverse_listeners(cx: &App) -> Vec<pm_model::ListenerStatus> {
        Self::proxy_status(cx)
            .listeners
            .into_iter()
            .filter(|l| l.kind != ListenerKind::Socks5)
            .collect()
    }
}

trait MenuExt {
    fn when_some_copy(self, text: Option<&str>, label: &str, what: &str) -> Self;
}

impl MenuExt for Menu {
    fn when_some_copy(self, text: Option<&str>, label: &str, what: &str) -> Menu {
        match text {
            Some(text) => self.copy(label.to_owned(), text.to_owned(), what.to_owned()),
            None => self,
        }
    }
}
