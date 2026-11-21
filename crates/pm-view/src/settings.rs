//! Settings (G12): the logic of `components/Settings/*.tsx` that needs no
//! gpui, and `lib/mcp.ts` with `mcp.test.ts`. The pages' numbers, list
//! editors, status lines and copies; Settings › Network's draft edited by
//! key ([`network`]), as the rule form's fields are (`rules::edit`).

use pm_model::{
    AutoSaveStatus, DeviceClient, DeviceDecision, ListenerKind, ListenerStatus, McpAuditEntry, McpCallOutcome,
    McpStatus, MockSource, OpenApiSpec, ProtoMapping, ProtoSchema, ProxyStatus, ReverseProxy, StorageStatus,
};

use crate::commands::SETTINGS_PAGES;
use crate::flow::format_size;

/// The page the window opens on when none is asked for.
pub const DEFAULT_PAGE: &str = "proxy";

/// A page's title, by id (`None` for an unknown page).
pub fn page_title(id: &str) -> Option<&'static str> {
    SETTINGS_PAGES.iter().find(|(p, _)| *p == id).map(|(_, t)| *t)
}

/// The page to show for `id`: the default for an empty or unknown one.
pub fn page_or_default(id: &str) -> &'static str {
    SETTINGS_PAGES
        .iter()
        .find(|(p, _)| *p == id)
        .map_or(DEFAULT_PAGE, |(p, _)| *p)
}

// ---- Numbers ----

/// A number field's text as a value in `min..=max` (`NumberField`'s
/// `min`/`max`): the error says what it takes.
pub fn parse_number(text: &str, min: u32, max: u32) -> Result<u32, String> {
    let text = text.trim().replace([',', '_'], "");
    if text.is_empty() {
        return Err("Enter a number".into());
    }
    match text.parse::<u64>() {
        Ok(n) if n >= u64::from(min) && n <= u64::from(max) => Ok(n as u32),
        Ok(_) => Err(format!(
            "Enter {} to {}",
            group_digits(min.into()),
            group_digits(max.into())
        )),
        Err(_) => Err("Enter a whole number".into()),
    }
}

/// 250000 as "250,000" (`Intl.NumberFormat` in English).
pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "1 flow", "3 flows".
pub fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", group_digits(n), if n == 1 { one } else { many })
}

// ---- List editors (`ListEditor`) ----

/// The list with `draft` added: `None` when it is empty or already there.
pub fn add_item(items: &[String], draft: &str) -> Option<Vec<String>> {
    let value = draft.trim();
    if value.is_empty() || items.iter().any(|i| i == value) {
        return None;
    }
    let mut next = items.to_vec();
    next.push(value.to_owned());
    Some(next)
}

pub fn remove_item(items: &[String], item: &str) -> Vec<String> {
    items.iter().filter(|i| *i != item).cloned().collect()
}

// ---- Proxy ----

/// The Listening line: whether it is up, and what it says.
pub fn listening_line(status: &ProxyStatus) -> (bool, String) {
    match &status.listening {
        Some(addr) => (true, format!("Listening on {addr}")),
        None => (
            false,
            format!("Not running: {}", status.error.as_deref().unwrap_or("stopped")),
        ),
    }
}

/// The proxy's port, to start the Port field from (the status names the
/// port actually bound; the setting is what is tried first).
pub const PORT_MIN: u32 = 1024;
pub const PORT_MAX: u32 = 65535;

// ---- SSL Proxying ----

/// The five SSL lists: id (a `SslSettings` field), title, description,
/// placeholder and what an empty one says.
pub const SSL_LISTS: [(&str, &str, &str, &str, &str); 5] = [
    (
        "include",
        "Decrypt",
        "Hosts whose HTTPS is decrypted. * decrypts everything not excluded below.",
        "*.example.com",
        "Nothing is decrypted: every HTTPS connection is tunnelled",
    ),
    (
        "exclude",
        "Never decrypt",
        "Hosts that are tunnelled as is. Apple, iCloud and certificate checks are here by default: they pin, and \
         breaking them breaks the Mac.",
        "*.apple.com",
        "No exclusions",
    ),
    (
        "bypass",
        "SSL bypass",
        "Added with \u{201c}Add to SSL bypass\u{201d}, usually for apps that pin their certificates and refuse \
         Proxymate's.",
        "pinned.example.com",
        "Nothing bypassed",
    ),
    (
        "insecure_upstream",
        "Allow insecure upstream",
        "Hosts whose own certificate is accepted without verification, e.g. a dev server with a self-signed \
         certificate. Signatures are still checked.",
        "localhost",
        "Every upstream certificate is verified",
    ),
    (
        "exclude_apps",
        "Apps never decrypted",
        "Everything these apps send is tunnelled, whatever the host. Use a bundle id (com.example.App) or, for \
         command-line tools, their name. Right-click an app in the sidebar to add it.",
        "com.example.App",
        "Every app is decrypted",
    ),
];

/// A list of `SslSettings` by id.
pub fn ssl_list<'a>(ssl: &'a pm_model::SslSettings, id: &str) -> Option<&'a Vec<String>> {
    Some(match id {
        "include" => &ssl.include,
        "exclude" => &ssl.exclude,
        "bypass" => &ssl.bypass,
        "insecure_upstream" => &ssl.insecure_upstream,
        "exclude_apps" => &ssl.exclude_apps,
        _ => return None,
    })
}

/// `ssl` with list `id` replaced.
pub fn with_ssl_list(ssl: &pm_model::SslSettings, id: &str, items: Vec<String>) -> Option<pm_model::SslSettings> {
    let mut next = ssl.clone();
    *match id {
        "include" => &mut next.include,
        "exclude" => &mut next.exclude,
        "bypass" => &mut next.bypass,
        "insecure_upstream" => &mut next.insecure_upstream,
        "exclude_apps" => &mut next.exclude_apps,
        _ => return None,
    } = items;
    Some(next)
}

// ---- Clients ----

pub fn decision_label(decision: DeviceDecision) -> &'static str {
    match decision {
        DeviceDecision::Pending => "Waiting for an answer",
        DeviceDecision::Allowed => "Allowed",
        DeviceDecision::Denied => "Denied",
    }
}

/// "12 flows · 9 decrypted · 3 refused (CA not trusted yet?)".
pub fn device_activity(device: &DeviceClient) -> String {
    let mut line = format!("{} flows · {} decrypted", device.flows, device.https_decrypted);
    if device.https_refused > 0 {
        line.push_str(&format!(" · {} refused (CA not trusted yet?)", device.https_refused));
    }
    line
}

/// A client row's buttons, in order: (label, allow?) where `allow` is
/// `Some(true)` for Allow/Save, `Some(false)` for Deny, and the last is
/// always Forget (`None`). `nickname` is what the field holds.
pub fn client_buttons(device: &DeviceClient, nickname: &str) -> Vec<(&'static str, Option<bool>)> {
    let changed = normalized(nickname) != device.nickname.as_deref().and_then(normalized);
    let mut out = Vec::new();
    if device.decision != DeviceDecision::Allowed || changed {
        out.push((
            if device.decision == DeviceDecision::Allowed {
                "Save"
            } else {
                "Allow"
            },
            Some(true),
        ));
    } else {
        out.push(("Deny", Some(false)));
    }
    if device.decision == DeviceDecision::Pending {
        out.push(("Deny", Some(false)));
    }
    out.push(("Forget", None));
    out
}

/// A nickname as saved: trimmed, `None` when empty.
pub fn normalized(nickname: &str) -> Option<&str> {
    let n = nickname.trim();
    (!n.is_empty()).then_some(n)
}

// ---- Storage ----

/// The Live capture rows (`StoragePage`'s list).
pub fn storage_rows(status: &StorageStatus) -> Vec<(&'static str, String)> {
    let mut flows = group_digits(status.flows.into());
    if status.pinned > 0 {
        flows.push_str(&format!(" ({} pinned)", group_digits(status.pinned.into())));
    }
    vec![
        ("Flows", flows),
        (
            "On disk",
            if status.disk_bytes > 0 {
                format_size(status.disk_bytes)
            } else {
                "Nothing yet".into()
            },
        ),
        (
            "Evicted since launch",
            format!("{} flows", group_digits(status.evicted)),
        ),
        ("Not recorded since launch", plural(status.unrecorded, "flow", "flows")),
        ("Location", status.dir.clone()),
    ]
}

/// The limits: field id, label, unit, hint (`Limits`).
pub const LIMITS: [(&str, &str, &str, &str); 3] = [
    (
        "max_flows",
        "Flows kept",
        "flows",
        "250,000 by default. About 200 bytes of memory each, plus their bodies on disk.",
    ),
    (
        "max_disk_mb",
        "Disk used",
        "MB",
        "Bodies over 8 KB and every flow's details live in 64 MB files; whole files are freed.",
    ),
    (
        "max_ws_messages",
        "WebSocket messages",
        "per connection",
        "A long-lived connection keeps its newest messages; older ones are dropped.",
    ),
];

/// The auto-save status line (`AutoSaveForm`).
pub fn autosave_line(status: Option<&AutoSaveStatus>, enabled: bool) -> String {
    match status {
        Some(s) if s.saving => "Saving…".into(),
        Some(AutoSaveStatus {
            last_at_ms: Some(at),
            last_path,
            ..
        }) => {
            let name = last_path
                .as_deref()
                .and_then(|p| p.rsplit('/').next())
                .unwrap_or_default();
            format!("Last saved {} · {name}", format_date_time(*at as f64))
        }
        _ if enabled => "Not saved yet".into(),
        _ => String::new(),
    }
}

/// "21 Nov 2026, 14:03" in local time (`dateStyle: medium, timeStyle: short`).
pub fn format_date_time(ms: f64) -> String {
    use chrono::{Local, TimeZone as _};
    match Local.timestamp_millis_opt(ms as i64).single() {
        Some(t) => t.format("%-d %b %Y, %H:%M").to_string(),
        None => "—".into(),
    }
}

/// "14:03:07" in local time (`toLocaleTimeString`).
pub fn format_clock(ms: f64) -> String {
    use chrono::{Local, TimeZone as _};
    match Local.timestamp_millis_opt(ms as i64).single() {
        Some(t) => t.format("%H:%M:%S").to_string(),
        None => "—".into(),
    }
}

// ---- Protobuf ----

/// A schema's second and third lines: its files, then its types and services.
pub fn schema_lines(s: &ProtoSchema) -> (String, String) {
    let mut types = plural(s.messages.into(), "message type", "message types");
    if !s.services.is_empty() {
        types.push_str(&format!(" · {}", s.services.join(", ")));
    }
    (s.files.join(", "), types)
}

/// A "Decode as" mapping: where it applies, and the message type.
pub fn mapping_line(m: &ProtoMapping) -> String {
    let host = m.matcher.host.as_ref().map_or("*", |p| p.value.as_str());
    let path = m.matcher.path.as_ref().map_or("", |p| p.value.as_str());
    let part = match m.part {
        Some(pm_model::BodyPart::Request) => " (request)",
        Some(pm_model::BodyPart::Response) => " (response)",
        None => "",
    };
    format!("{host}{path}{part}")
}

// ---- Mocking ----

/// A recording's last line: flows, size and hosts.
pub fn source_line(s: &MockSource) -> String {
    let mut line = format!("{} · {}", plural(s.flows.into(), "flow", "flows"), format_size(s.size));
    if !s.hosts.is_empty() {
        line.push_str(&format!(" · {}", s.hosts.join(", ")));
    }
    line
}

/// A spec's second line: version, operations and servers.
pub fn spec_line(s: &OpenApiSpec) -> String {
    let mut line = format!(
        "OpenAPI {} · {}",
        s.openapi,
        plural(s.operations.into(), "operation", "operations")
    );
    if !s.servers.is_empty() {
        line.push_str(&format!(" · {}", s.servers.join(", ")));
    }
    line
}

/// Generate mocks' toast.
pub fn generated_toast(n: usize) -> String {
    format!(
        "{n} mock rule{} added, turned off until you review them",
        if n == 1 { "" } else { "s" }
    )
}

// ---- MCP (`lib/mcp.ts`) ----

pub const DEFAULT_MCP_URL: &str = "http://127.0.0.1:9095/mcp";

/// `claude mcp add` for Claude Code.
pub fn claude_add_command(url: &str, token: &str) -> String {
    format!("claude mcp add --transport http proxymate {url} --header \"Authorization: Bearer {token}\"")
}

/// The `mcpServers` entry most clients (Cursor, Claude Desktop, `.mcp.json`)
/// read, laid out as `JSON.stringify(config, null, 2)` lays it out.
pub fn mcp_json_config(url: &str, token: &str) -> String {
    let s = |v: &str| serde_json::to_string(v).unwrap_or_default();
    format!(
        "{{\n  \"mcpServers\": {{\n    \"proxymate\": {{\n      \"type\": \"http\",\n      \"url\": {},\n      \
         \"headers\": {{\n        \"Authorization\": {}\n      }}\n    }}\n  }}\n}}",
        s(url),
        s(&format!("Bearer {token}"))
    )
}

/// The token with all but its last four characters hidden, for display.
pub fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 4 {
        return "••••".into();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{}{tail}", "•".repeat(8))
}

/// How the server is doing: a tone (`ok`, `error`, `idle`) and a line.
pub fn serving_line(status: &McpStatus) -> (&'static str, String) {
    if let Some(url) = &status.url {
        ("ok", format!("Serving on {url}"))
    } else if let Some(err) = &status.error {
        ("error", format!("Not serving: {err}"))
    } else if status.settings.enabled {
        ("idle", "Starting…".into())
    } else {
        ("idle", "Off".into())
    }
}

pub fn outcome_label(outcome: McpCallOutcome) -> &'static str {
    match outcome {
        McpCallOutcome::Ok => "ok",
        McpCallOutcome::Error => "error",
        McpCallOutcome::Refused => "refused",
    }
}

/// The audit log's first page and each "Load older calls".
pub const AUDIT_PAGE: u32 = 50;
pub const AUDIT_MORE: u32 = 100;
pub const AUDIT_MAX: u32 = 1000;

/// Copy entry as JSON: the entry as the Tauri menu copies it
/// (`JSON.stringify(e, null, 2)`, in the type's field order).
pub fn audit_entry_json(e: &McpAuditEntry) -> String {
    serde_json::to_string_pretty(e).unwrap_or_default()
}

// ---- Network listeners ----

/// The SOCKS5 address: its own listener's, else the proxy's.
pub fn socks_addr(status: &ProxyStatus) -> Option<String> {
    status
        .listeners
        .iter()
        .find(|l| l.kind == ListenerKind::Socks5)
        .and_then(|l| l.addr.clone())
        .or_else(|| status.listening.clone())
}

/// A reverse proxy's line under its row: where it listens, why it failed,
/// or that it starts when saved; `true` for an error.
pub fn reverse_status(r: &ReverseProxy, listeners: &[ListenerStatus]) -> (String, bool) {
    let reverse = |l: &&ListenerStatus| l.kind != ListenerKind::Socks5;
    let port = format!(":{}", r.listen_port);
    if let Some(addr) = listeners
        .iter()
        .filter(reverse)
        .find_map(|l| l.addr.as_ref().filter(|a| a.ends_with(&port)))
    {
        let scheme = if r.tls { "https" } else { "http" };
        return (format!("{scheme}://{addr} → {}", r.target_url), false);
    }
    if let Some(err) = listeners
        .iter()
        .filter(reverse)
        .find(|l| l.addr.is_none() && l.target.as_deref() == Some(r.target_url.as_str()))
        .and_then(|l| l.error.clone())
    {
        return (err, true);
    }
    ("Starts when saved".into(), false)
}

/// Settings › Network's draft, edited by key, as the rule form edits a
/// rule (`rules::edit`). Keys (under the window's `net.` prefix):
///
/// - `up.N.enabled|kind` (`http`, `socks5`, `direct`) and
///   `up.N.hosts|host|port|username|password`;
/// - `previous` (use the proxy set before Proxymate), with the bypass
///   list edited as a list;
/// - `rev.N.enabled|tls|keep_host` and `rev.N.port|target`;
/// - `cert.N.enabled|format` (`pkcs12`, `pem`) and
///   `cert.N.hosts|path|password|cert|key`;
/// - `dns.N.enabled` and `dns.N.host|ip`.
///
/// A password's text is what was typed (empty keeps the saved one).
pub mod network {
    use pm_model::{
        ClientCert, ClientIdentity, DnsOverride, NetworkSettings, ProxyServer, ReverseProxy, UpstreamProxy,
        UpstreamRule,
    };

    /// The sections rows are added to: `up`, `rev`, `cert`, `dns`.
    pub const SECTIONS: [&str; 4] = ["up", "rev", "cert", "dns"];

    pub fn hosts_text(hosts: &[String]) -> String {
        hosts.join(", ")
    }

    pub fn parse_hosts(text: &str) -> Vec<String> {
        text.split(',').map(|h| h.trim().to_owned()).collect()
    }

    fn split(key: &str) -> Option<(&str, usize, &str)> {
        let mut parts = key.splitn(3, '.');
        let section = parts.next()?;
        let index = parts.next()?.parse().ok()?;
        Some((section, index, parts.next()?))
    }

    fn server(proxy: &UpstreamProxy) -> Option<&ProxyServer> {
        match proxy {
            UpstreamProxy::Direct => None,
            UpstreamProxy::Http(s) | UpstreamProxy::Socks5(s) => Some(s),
        }
    }

    fn server_mut(proxy: &mut UpstreamProxy) -> Option<&mut ProxyServer> {
        match proxy {
            UpstreamProxy::Direct => None,
            UpstreamProxy::Http(s) | UpstreamProxy::Socks5(s) => Some(s),
        }
    }

    pub fn kind(proxy: &UpstreamProxy) -> &'static str {
        match proxy {
            UpstreamProxy::Direct => "direct",
            UpstreamProxy::Http(_) => "http",
            UpstreamProxy::Socks5(_) => "socks5",
        }
    }

    fn port_text(port: u16) -> String {
        if port > 0 { port.to_string() } else { String::new() }
    }

    fn parse_port(text: &str) -> Result<u16, String> {
        match text.trim().parse::<u16>() {
            Ok(p) if p > 0 => Ok(p),
            _ => Err("Enter a port, 1 to 65535".into()),
        }
    }

    /// A text field's value (`None`: no such field).
    pub fn text(n: &NetworkSettings, key: &str) -> Option<String> {
        let (section, i, field) = split(key)?;
        Some(match section {
            "up" => {
                let r = n.upstream.rules.get(i)?;
                match field {
                    "hosts" => hosts_text(&r.hosts),
                    _ => {
                        let s = server(&r.proxy)?;
                        match field {
                            "host" => s.host.clone(),
                            "port" => port_text(s.port),
                            "username" => s.username.clone().unwrap_or_default(),
                            "password" => s.password.clone().unwrap_or_default(),
                            _ => return None,
                        }
                    }
                }
            }
            "rev" => {
                let r = n.reverse_proxies.get(i)?;
                match field {
                    "port" => port_text(r.listen_port),
                    "target" => r.target_url.clone(),
                    _ => return None,
                }
            }
            "cert" => {
                let c = n.client_certs.get(i)?;
                match (field, &c.identity) {
                    ("hosts", _) => hosts_text(&c.hosts),
                    ("path", ClientIdentity::Pkcs12 { path, .. }) => path.clone(),
                    ("password", ClientIdentity::Pkcs12 { password, .. }) => password.clone().unwrap_or_default(),
                    ("cert", ClientIdentity::Pem { cert_path, .. }) => cert_path.clone(),
                    ("key", ClientIdentity::Pem { key_path, .. }) => key_path.clone(),
                    _ => return None,
                }
            }
            "dns" => {
                let d = n.dns.get(i)?;
                match field {
                    "host" => d.host.clone(),
                    "ip" => d.ip.clone(),
                    _ => return None,
                }
            }
            _ => return None,
        })
    }

    /// Writes a text field. A port that isn't one is refused (the draft
    /// keeps its last good value; the window shows the error).
    pub fn set_text(n: &mut NetworkSettings, key: &str, value: &str) -> Result<(), String> {
        let missing = || format!("no field {key}");
        let (section, i, field) = split(key).ok_or_else(missing)?;
        let opt = |v: &str| (!v.is_empty()).then(|| v.to_owned());
        match section {
            "up" => {
                let r = n.upstream.rules.get_mut(i).ok_or_else(missing)?;
                if field == "hosts" {
                    r.hosts = parse_hosts(value);
                    return Ok(());
                }
                let s = server_mut(&mut r.proxy).ok_or_else(missing)?;
                match field {
                    "host" => s.host = value.to_owned(),
                    "port" => s.port = parse_port(value)?,
                    "username" => s.username = opt(value),
                    "password" => s.password = opt(value),
                    _ => return Err(missing()),
                }
            }
            "rev" => {
                let r = n.reverse_proxies.get_mut(i).ok_or_else(missing)?;
                match field {
                    "port" => r.listen_port = parse_port(value)?,
                    "target" => r.target_url = value.to_owned(),
                    _ => return Err(missing()),
                }
            }
            "cert" => {
                let c = n.client_certs.get_mut(i).ok_or_else(missing)?;
                match (field, &mut c.identity) {
                    ("hosts", _) => c.hosts = parse_hosts(value),
                    ("path", ClientIdentity::Pkcs12 { path, .. }) => *path = value.to_owned(),
                    ("password", ClientIdentity::Pkcs12 { password, .. }) => *password = opt(value),
                    ("cert", ClientIdentity::Pem { cert_path, .. }) => *cert_path = value.to_owned(),
                    ("key", ClientIdentity::Pem { key_path, .. }) => *key_path = value.to_owned(),
                    _ => return Err(missing()),
                }
            }
            "dns" => {
                let d = n.dns.get_mut(i).ok_or_else(missing)?;
                match field {
                    "host" => d.host = value.to_owned(),
                    "ip" => d.ip = value.to_owned(),
                    _ => return Err(missing()),
                }
            }
            _ => return Err(missing()),
        }
        Ok(())
    }

    fn flag(b: bool) -> String {
        b.to_string()
    }

    /// A switch's or picker's value.
    pub fn chosen(n: &NetworkSettings, key: &str) -> Option<String> {
        if key == "previous" {
            return Some(flag(n.upstream.use_system_previous));
        }
        let (section, i, field) = split(key)?;
        Some(match (section, field) {
            ("up", "enabled") => flag(n.upstream.rules.get(i)?.enabled),
            ("up", "kind") => kind(&n.upstream.rules.get(i)?.proxy).into(),
            ("rev", "enabled") => flag(n.reverse_proxies.get(i)?.enabled),
            ("rev", "tls") => flag(n.reverse_proxies.get(i)?.tls),
            ("rev", "keep_host") => flag(n.reverse_proxies.get(i)?.preserve_host),
            ("cert", "enabled") => flag(n.client_certs.get(i)?.enabled),
            ("cert", "format") => match n.client_certs.get(i)?.identity {
                ClientIdentity::Pkcs12 { .. } => "pkcs12".into(),
                ClientIdentity::Pem { .. } => "pem".into(),
            },
            ("dns", "enabled") => flag(n.dns.get(i)?.enabled),
            _ => return None,
        })
    }

    /// Sets a switch or picker; false if `key` or `value` isn't one.
    pub fn choose(n: &mut NetworkSettings, key: &str, value: &str) -> bool {
        let on = match value {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        if key == "previous" {
            let Some(on) = on else { return false };
            n.upstream.use_system_previous = on;
            return true;
        }
        let Some((section, i, field)) = split(key) else {
            return false;
        };
        let set = |slot: &mut bool| on.map(|on| *slot = on).is_some();
        match (section, field) {
            ("up", "enabled") => n.upstream.rules.get_mut(i).is_some_and(|r| set(&mut r.enabled)),
            ("up", "kind") => n.upstream.rules.get_mut(i).is_some_and(|r| set_kind(r, value)),
            ("rev", "enabled") => n.reverse_proxies.get_mut(i).is_some_and(|r| set(&mut r.enabled)),
            ("rev", "tls") => n.reverse_proxies.get_mut(i).is_some_and(|r| set(&mut r.tls)),
            ("rev", "keep_host") => n.reverse_proxies.get_mut(i).is_some_and(|r| set(&mut r.preserve_host)),
            ("cert", "enabled") => n.client_certs.get_mut(i).is_some_and(|c| set(&mut c.enabled)),
            ("cert", "format") => n.client_certs.get_mut(i).is_some_and(|c| {
                c.identity = match value {
                    "pem" => ClientIdentity::Pem {
                        cert_path: String::new(),
                        key_path: String::new(),
                    },
                    "pkcs12" => ClientIdentity::Pkcs12 {
                        path: String::new(),
                        password: None,
                        has_password: false,
                    },
                    _ => return false,
                };
                true
            }),
            ("dns", "enabled") => n.dns.get_mut(i).is_some_and(|d| set(&mut d.enabled)),
            _ => false,
        }
    }

    /// "Through": a server keeps its fields across HTTP and SOCKS5; one
    /// without a port gets the kind's usual one.
    fn set_kind(rule: &mut UpstreamRule, kind: &str) -> bool {
        let old = server(&rule.proxy).cloned();
        let default_port = if kind == "socks5" { 1080 } else { 3128 };
        let make = || {
            let mut s = old.clone().unwrap_or(ProxyServer {
                host: String::new(),
                port: 0,
                username: None,
                password: None,
                has_password: false,
            });
            if s.port == 0 {
                s.port = default_port;
            }
            s
        };
        rule.proxy = match kind {
            "direct" => UpstreamProxy::Direct,
            "http" => UpstreamProxy::Http(make()),
            "socks5" => UpstreamProxy::Socks5(make()),
            _ => return false,
        };
        true
    }

    /// Adds a row to `section` with the Tauri page's defaults.
    pub fn add(n: &mut NetworkSettings, section: &str) -> bool {
        match section {
            "up" => n.upstream.rules.push(UpstreamRule {
                enabled: true,
                hosts: vec!["*".into()],
                proxy: UpstreamProxy::Http(ProxyServer {
                    host: String::new(),
                    port: 3128,
                    username: None,
                    password: None,
                    has_password: false,
                }),
            }),
            "rev" => n.reverse_proxies.push(ReverseProxy {
                enabled: true,
                listen_port: 8080,
                target_url: "http://localhost:3000".into(),
                preserve_host: false,
                tls: false,
            }),
            "cert" => n.client_certs.push(ClientCert {
                enabled: true,
                hosts: vec![String::new()],
                identity: ClientIdentity::Pkcs12 {
                    path: String::new(),
                    password: None,
                    has_password: false,
                },
            }),
            "dns" => n.dns.push(DnsOverride {
                enabled: true,
                host: String::new(),
                ip: "127.0.0.1".into(),
            }),
            _ => return false,
        }
        true
    }

    pub fn remove(n: &mut NetworkSettings, section: &str, i: usize) -> bool {
        fn take<T>(v: &mut Vec<T>, i: usize) -> bool {
            (i < v.len()).then(|| v.remove(i)).is_some()
        }
        match section {
            "up" => take(&mut n.upstream.rules, i),
            "rev" => take(&mut n.reverse_proxies, i),
            "cert" => take(&mut n.client_certs, i),
            "dns" => take(&mut n.dns, i),
            _ => false,
        }
    }

    pub fn rows(n: &NetworkSettings, section: &str) -> usize {
        match section {
            "up" => n.upstream.rules.len(),
            "rev" => n.reverse_proxies.len(),
            "cert" => n.client_certs.len(),
            "dns" => n.dns.len(),
            _ => 0,
        }
    }

    /// Whether a saved password stands behind an empty password field.
    pub fn has_saved_password(n: &NetworkSettings, key: &str) -> bool {
        let Some((section, i, _)) = split(key) else {
            return false;
        };
        match section {
            "up" => n
                .upstream
                .rules
                .get(i)
                .and_then(|r| server(&r.proxy))
                .is_some_and(|s| s.has_password),
            "cert" => matches!(
                n.client_certs.get(i).map(|c| &c.identity),
                Some(ClientIdentity::Pkcs12 { has_password: true, .. })
            ),
            _ => false,
        }
    }

    /// Every text key of the draft, for syncing inputs with it.
    pub fn text_keys(n: &NetworkSettings) -> Vec<String> {
        let mut keys = Vec::new();
        for (i, r) in n.upstream.rules.iter().enumerate() {
            keys.push(format!("up.{i}.hosts"));
            if server(&r.proxy).is_some() {
                keys.extend(["host", "port", "username", "password"].map(|f| format!("up.{i}.{f}")));
            }
        }
        for i in 0..n.reverse_proxies.len() {
            keys.extend(["port", "target"].map(|f| format!("rev.{i}.{f}")));
        }
        for (i, c) in n.client_certs.iter().enumerate() {
            keys.push(format!("cert.{i}.hosts"));
            match c.identity {
                ClientIdentity::Pkcs12 { .. } => keys.extend(["path", "password"].map(|f| format!("cert.{i}.{f}"))),
                ClientIdentity::Pem { .. } => keys.extend(["cert", "key"].map(|f| format!("cert.{i}.{f}"))),
            }
        }
        for i in 0..n.dns.len() {
            keys.extend(["host", "ip"].map(|f| format!("dns.{i}.{f}")));
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use pm_model::{McpSettings, NetworkSettings, UpstreamProxy};

    use super::*;

    const URL: &str = "http://127.0.0.1:9095/mcp";

    // `mcp.test.ts`

    #[test]
    fn builds_the_claude_mcp_add_command_with_the_bearer_header() {
        assert_eq!(
            claude_add_command(URL, "abc"),
            r#"claude mcp add --transport http proxymate http://127.0.0.1:9095/mcp --header "Authorization: Bearer abc""#
        );
    }

    #[test]
    fn builds_an_mcp_servers_json_entry() {
        let text = mcp_json_config(URL, "abc");
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed["mcpServers"]["proxymate"],
            serde_json::json!({"type": "http", "url": URL, "headers": {"Authorization": "Bearer abc"}})
        );
        assert!(text.starts_with("{\n  \"mcpServers\": {\n    \"proxymate\": {\n      \"type\""));
    }

    #[test]
    fn masks_all_but_the_end_of_the_token() {
        assert_eq!(mask_token("0123456789abcdef"), "••••••••cdef");
        assert_eq!(mask_token("ab"), "••••");
    }

    #[test]
    fn numbers_parse_within_their_range() {
        assert_eq!(parse_number(" 9090 ", 1024, 65535), Ok(9090));
        assert_eq!(parse_number("250,000", 0, u32::MAX), Ok(250_000));
        assert_eq!(parse_number("80", 1024, 65535), Err("Enter 1,024 to 65,535".into()));
        assert_eq!(parse_number("", 0, 10), Err("Enter a number".into()));
        assert_eq!(parse_number("1.5", 0, 10), Err("Enter a whole number".into()));
        assert_eq!(group_digits(1_234_567), "1,234,567");
        assert_eq!(group_digits(999), "999");
    }

    #[test]
    fn list_editors_add_once_and_remove() {
        let items = vec!["a".to_owned()];
        assert_eq!(add_item(&items, " b "), Some(vec!["a".into(), "b".into()]));
        assert_eq!(add_item(&items, "a"), None);
        assert_eq!(add_item(&items, "  "), None);
        assert!(remove_item(&items, "a").is_empty());
    }

    #[test]
    fn pages_have_titles_and_fall_back_to_proxy() {
        assert_eq!(SETTINGS_PAGES.len(), 11);
        assert_eq!(page_title("protobuf"), Some("Protobuf"));
        assert_eq!(page_title("updates"), Some("Updates"));
        assert_eq!(page_or_default(""), "proxy");
        assert_eq!(page_or_default("nope"), "proxy");
        assert_eq!(page_or_default("mcp"), "mcp");
    }

    #[test]
    fn client_buttons_follow_the_decision_and_the_nickname() {
        let mut d = DeviceClient {
            ip: "192.168.1.9".into(),
            nickname: Some("Pixel".into()),
            decision: DeviceDecision::Allowed,
            label: String::new(),
            flows: 3,
            https_decrypted: 2,
            https_refused: 1,
            onboarding: false,
            last_seen_ms: None,
            waiting: 0,
            held_until_ms: None,
        };
        let labels = |d: &DeviceClient, n: &str| client_buttons(d, n).into_iter().map(|b| b.0).collect::<Vec<_>>();
        assert_eq!(labels(&d, "Pixel "), ["Deny", "Forget"]);
        assert_eq!(labels(&d, "Phone"), ["Save", "Forget"]);
        d.decision = DeviceDecision::Pending;
        assert_eq!(labels(&d, ""), ["Allow", "Deny", "Forget"]);
        d.decision = DeviceDecision::Denied;
        assert_eq!(labels(&d, ""), ["Allow", "Forget"]);
        assert_eq!(
            device_activity(&d),
            "3 flows · 2 decrypted · 1 refused (CA not trusted yet?)"
        );
    }

    #[test]
    fn mcp_serving_line_says_where_or_why() {
        let mut s = McpStatus {
            settings: McpSettings {
                enabled: true,
                read_only: false,
            },
            url: None,
            error: None,
            clients: vec![],
        };
        assert_eq!(serving_line(&s), ("idle", "Starting…".into()));
        s.error = Some("port in use".into());
        assert_eq!(serving_line(&s).1, "Not serving: port in use");
        s.url = Some(URL.into());
        assert_eq!(serving_line(&s), ("ok", format!("Serving on {URL}")));
        s.settings.enabled = false;
        s.url = None;
        s.error = None;
        assert_eq!(serving_line(&s).1, "Off");
    }

    #[test]
    fn network_fields_edit_by_key() {
        let mut n = NetworkSettings::default();
        for s in network::SECTIONS {
            assert!(network::add(&mut n, s));
        }
        assert_eq!(network::text(&n, "up.0.hosts").as_deref(), Some("*"));
        assert_eq!(network::text(&n, "up.0.port").as_deref(), Some("3128"));
        network::set_text(&mut n, "up.0.hosts", "a.test, *.b.test").unwrap();
        assert_eq!(n.upstream.rules[0].hosts, ["a.test", "*.b.test"]);
        network::set_text(&mut n, "up.0.host", "proxy.corp").unwrap();
        network::set_text(&mut n, "up.0.password", "s3cret").unwrap();
        assert!(network::set_text(&mut n, "up.0.port", "99999").is_err());
        assert_eq!(
            network::text(&n, "up.0.port").as_deref(),
            Some("3128"),
            "the last good port stays"
        );

        // SOCKS5 keeps the server; Direct drops it, and back gives the default port.
        assert!(network::choose(&mut n, "up.0.kind", "socks5"));
        let UpstreamProxy::Socks5(s) = &n.upstream.rules[0].proxy else {
            panic!()
        };
        assert_eq!(
            (s.host.as_str(), s.port, s.password.as_deref()),
            ("proxy.corp", 3128, Some("s3cret"))
        );
        assert!(network::choose(&mut n, "up.0.kind", "direct"));
        assert_eq!(network::text(&n, "up.0.host"), None);
        assert!(network::choose(&mut n, "up.0.kind", "socks5"));
        assert_eq!(network::text(&n, "up.0.port").as_deref(), Some("1080"));

        assert!(network::choose(&mut n, "rev.0.tls", "true"));
        assert_eq!(network::chosen(&n, "rev.0.tls").as_deref(), Some("true"));
        assert!(network::choose(&mut n, "cert.0.format", "pem"));
        assert_eq!(network::text(&n, "cert.0.path"), None);
        network::set_text(&mut n, "cert.0.cert", "/c.pem").unwrap();
        assert!(network::choose(&mut n, "previous", "true"));
        assert!(n.upstream.use_system_previous);
        assert!(!network::choose(&mut n, "dns.0.enabled", "maybe"));

        let keys = network::text_keys(&n);
        assert!(keys.contains(&"cert.0.cert".to_owned()) && keys.contains(&"dns.0.ip".to_owned()));
        assert!(keys.iter().all(|k| network::text(&n, k).is_some()), "{keys:?}");

        assert!(network::remove(&mut n, "dns", 0));
        assert!(!network::remove(&mut n, "dns", 0));
        assert_eq!(network::rows(&n, "dns"), 0);
    }

    #[test]
    fn reverse_rows_say_where_they_listen_or_why_not() {
        let r = ReverseProxy {
            enabled: true,
            listen_port: 8080,
            target_url: "http://localhost:3000".into(),
            preserve_host: false,
            tls: true,
        };
        let up = ListenerStatus {
            kind: ListenerKind::ReverseTls,
            addr: Some("127.0.0.1:8080".into()),
            target: Some(r.target_url.clone()),
            error: None,
        };
        assert_eq!(
            reverse_status(&r, std::slice::from_ref(&up)),
            ("https://127.0.0.1:8080 → http://localhost:3000".into(), false)
        );
        let down = ListenerStatus {
            addr: None,
            error: Some("port 8080 is in use".into()),
            ..up
        };
        assert_eq!(reverse_status(&r, &[down]), ("port 8080 is in use".into(), true));
        assert_eq!(reverse_status(&r, &[]).0, "Starts when saved");
    }

    #[test]
    fn storage_and_autosave_lines() {
        let status = StorageStatus {
            flows: 12_345,
            pinned: 2,
            unrecorded: 1,
            dir: "/tmp/live".into(),
            ..Default::default()
        };
        let rows = storage_rows(&status);
        assert_eq!(rows[0].1, "12,345 (2 pinned)");
        assert_eq!(rows[1].1, "Nothing yet");
        assert_eq!(rows[3].1, "1 flow");
        assert_eq!(autosave_line(None, true), "Not saved yet");
        assert_eq!(autosave_line(None, false), "");
        let saved = AutoSaveStatus {
            last_at_ms: Some(0),
            last_path: Some("/a/Proxymate Autosave 1.proxymate".into()),
            ..Default::default()
        };
        assert!(autosave_line(Some(&saved), true).ends_with(" · Proxymate Autosave 1.proxymate"));
        let saving = AutoSaveStatus { saving: true, ..saved };
        assert_eq!(autosave_line(Some(&saving), true), "Saving…");
        assert_eq!(
            generated_toast(1),
            "1 mock rule added, turned off until you review them"
        );
    }
}
