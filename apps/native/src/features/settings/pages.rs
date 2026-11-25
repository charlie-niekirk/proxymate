//! The eleven pages (`ProxyPage.tsx` … `UpdatesPage.tsx`), each a list of
//! sections built from the window's caches and pushes. Reading a cache
//! here fetches it if it is missing or stale.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, FontWeight, Role, SharedString, Toggled, Window, div, px};
use pm_model::{ClientIdentity, DeviceDecision, McpCallOutcome, UpstreamProxy};
use pm_theme::kit::TokensExt as _;
use pm_theme::{Mode, Tokens};
use pm_view::settings::{self as logic, network};

use super::parts::{LABEL_W, Look, empty_rows, error, labelled, muted, nav, over, rows, saved, section, status_line};
use super::window::SettingsWindow;
use crate::bridge::resource::Resource;
use crate::prefs::Appearance;
use crate::workspace::Workspace;

/// A one-value cache's value, fetching it if needed.
fn load<V: Clone + Send + 'static>(entity: &Entity<Resource<(), V>>, cx: &mut App) -> Option<V> {
    entity.update(cx, |c, cx| c.get(&(), cx)).ready().cloned()
}

pub fn render(w: &mut SettingsWindow, window: &mut Window, cx: &mut Context<SettingsWindow>) -> AnyElement {
    let t = *cx.tokens();
    let page = w.page();
    let (header, body) = match page {
        "proxy" => (None, proxy(w, window, cx)),
        "network" => network_page(w, window, cx),
        "ssl" => (None, ssl(w, window, cx)),
        "clients" => (None, clients(w, window, cx)),
        "storage" => (None, storage(w, window, cx)),
        "protobuf" => (None, protobuf(w, cx)),
        "mocking" => (None, mocking(w, cx)),
        "appearance" => (None, appearance(w, cx)),
        "advanced" => (None, advanced(w, window, cx)),
        "mcp" => (None, mcp(w, cx)),
        _ => (None, updates(w, cx)),
    };
    let title = logic::page_title(page).unwrap_or_default();
    div()
        .id("settings")
        .key_context("SettingsWindow")
        .track_focus(&w.focus)
        .size_full()
        .flex()
        .bg(t.background.hsla())
        .text_color(t.foreground.hsla())
        .font_family(cx.theme().font_family.clone())
        .child(nav(page, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .children(header)
                .child(
                    div()
                        .id(SharedString::from(format!("settings-{page}")))
                        .role(Role::Group)
                        .aria_label(SharedString::from(format!("{title} settings")))
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&w.scroll)
                        .child(
                            div()
                                .p(px(16.))
                                .flex()
                                .flex_col()
                                .gap(px(20.))
                                .text_size(px(12.))
                                .children(body),
                        ),
                ),
        )
        .into_any_element()
}

fn dot_ok(ok: bool, t: &Tokens) -> pm_theme::Rgba {
    if ok { t.s_2xx } else { t.s_5xx }
}

// ---- Proxy ----

fn proxy(w: &mut SettingsWindow, window: &mut Window, cx: &mut Context<SettingsWindow>) -> Vec<AnyElement> {
    let t = *cx.tokens();
    let status = Workspace::global(cx)
        .map(|ws| ws.read(cx).proxy.clone())
        .unwrap_or_default();
    let settings = load(&w.caches.proxy, cx);
    let access = load(&w.integration.access, cx);
    let sysproxy = load(&w.integration.system_proxy, cx);
    load(&w.caches.terminal_env, cx);

    let (ok, line) = logic::listening_line(&status);
    let listening = w
        .with_menu(
            status_line("proxy-listening", dot_ok(ok, &t), line, &t),
            "listening".into(),
            cx,
        )
        .test_support()
        .into_any_element();
    let mut listening_body = vec![listening];
    if let Some(settings) = settings {
        let busy = w.is_busy("proxy");
        let parsed = logic::parse_number(&w.text("proxy.port", cx), logic::PORT_MIN, logic::PORT_MAX);
        let unchanged = parsed.as_ref().is_ok_and(|p| *p == u32::from(settings.port));
        let input = w.input("proxy.port", "", "Port", Some(112.), true, window, cx);
        let apply = w.button(
            "proxy.apply",
            if busy { "Restarting…" } else { "Apply" },
            Look::Outline,
            busy || unchanged || parsed.is_err(),
            cx,
        );
        listening_body.push(labelled("Port", vec![input, apply], None, &t));
    }
    if let Some(err) = w.error("proxy") {
        listening_body.push(error("proxy", err.to_owned(), &t));
    }

    let lan_on = access.as_ref().is_some_and(|a| a.enabled);
    let mut lan_body = Vec::new();
    if let Some(access) = &access
        && access.enabled
        && !access.interfaces.is_empty()
    {
        let options: Vec<(String, String)> = access
            .interfaces
            .iter()
            .map(|i| (i.name.clone(), format!("{} ({})", i.ip, i.name)))
            .collect();
        let current = access.interface.clone().unwrap_or_default();
        let picker = w.picker(
            "proxy.interface",
            &options,
            &current,
            "Address shown to devices",
            240.,
            cx,
        );
        let picker = w
            .with_menu(div().id("lan-interface").child(picker), "lan-interface".into(), cx)
            .into_any_element();
        lan_body.push(labelled("Address shown to devices", vec![picker], None, &t));
    }
    if let Some(err) = access.as_ref().and_then(|a| a.error.clone()) {
        lan_body.push(error("lan", err, &t));
    }

    let sys_on = sysproxy.as_ref().is_some_and(|s| s.enabled);
    let sys_disabled = sysproxy.as_ref().is_none_or(|s| !s.supported);
    let mut sys_body = Vec::new();
    if let Some(err) = sysproxy.as_ref().and_then(|s| s.error.clone()) {
        sys_body.push(error("sysproxy", err, &t));
    }

    vec![
        section(
            "Listening",
            Some(
                "Proxymate tries this port first, then the nine after it. Changing it restarts the listener; open \
                 connections finish on the old one.",
            ),
            None,
            listening_body,
            &t,
        ),
        section(
            "LAN access",
            Some(
                "Listen on every network interface so phones and other computers can use the proxy. Each new \
                 device has to be allowed first (Settings › Clients).",
            ),
            Some(w.switch("proxy.lan", lan_on, "LAN access", access.is_none(), cx)),
            lan_body,
            &t,
        ),
        section(
            "System proxy",
            Some(
                "Point this Mac's HTTP and HTTPS proxy at Proxymate, for every app that follows the system \
                 setting. It's put back when Proxymate quits, even after a crash.",
            ),
            Some(w.switch("proxy.sysproxy", sys_on, "System proxy", sys_disabled, cx)),
            sys_body,
            &t,
        ),
        section(
            "Proxied terminal",
            Some(
                "Paste into a shell to send curl, Node, Python and friends through Proxymate. The CA bundle holds \
                 this Mac's trusted roots plus Proxymate's CA.",
            ),
            None,
            w.copy_box("terminal-env", cx).into_iter().collect(),
            &t,
        ),
    ]
}

// ---- Network ----

fn network_page(
    w: &mut SettingsWindow,
    window: &mut Window,
    cx: &mut Context<SettingsWindow>,
) -> (Option<AnyElement>, Vec<AnyElement>) {
    let t = *cx.tokens();
    load(&w.caches.network, cx);
    let Some(n) = w.network_draft().cloned() else {
        return (None, vec![muted("Loading…", &t)]);
    };
    let busy = w.is_busy("network");
    let dirty = n.dirty();
    let bar_text = match w.error("network") {
        Some(err) => error("network", err.to_owned(), &t),
        None => muted(
            if dirty {
                "Unsaved changes"
            } else {
                "New connections use these settings straight away."
            },
            &t,
        ),
    };
    let header = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .py(px(8.))
        .border_b_1()
        .border_color(t.border.hsla())
        .bg(t.background.hsla())
        .child(div().flex_1().min_w_0().child(bar_text))
        .child(w.button("net.revert", "Revert", Look::Ghost, !dirty || busy, cx))
        .child(w.button(
            "net.save",
            if busy { "Saving…" } else { "Save" },
            Look::Primary,
            !dirty || busy || !n.field_errors.is_empty(),
            cx,
        ))
        .into_any_element();

    let row_errors = |prefix: &str| -> Vec<AnyElement> {
        n.field_errors
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, e)| error(&format!("net-{k}"), e.clone(), &t))
            .collect()
    };

    // Upstream proxies.
    let mut up_rows = Vec::new();
    for (i, rule) in n.draft.upstream.rules.iter().enumerate() {
        let p = format!("net.up.{i}");
        let mut fields = vec![
            div()
                .w_full()
                .child(over(
                    "Hosts (comma separated)",
                    w.input(
                        &format!("{p}.hosts"),
                        "*.example.com, api.test",
                        "Hosts",
                        None,
                        true,
                        window,
                        cx,
                    ),
                    None,
                    &t,
                ))
                .into_any_element(),
        ];
        let kind = network::kind(&rule.proxy);
        let options: Vec<(String, String)> = [("http", "HTTP"), ("socks5", "SOCKS5"), ("direct", "Direct")]
            .map(|(v, l)| (v.into(), l.into()))
            .into();
        fields.push(over(
            "Through",
            w.picker(&format!("{p}.kind"), &options, kind, "Through", 80., cx),
            Some(112.),
            &t,
        ));
        if let UpstreamProxy::Http(s) | UpstreamProxy::Socks5(s) = &rule.proxy {
            let saved_pw = s.has_password;
            fields.push(over(
                "Proxy host",
                w.input(
                    &format!("{p}.host"),
                    "proxy.corp.example",
                    "Proxy host",
                    None,
                    true,
                    window,
                    cx,
                ),
                Some(176.),
                &t,
            ));
            fields.push(over(
                "Port",
                w.input(&format!("{p}.port"), "", "Proxy port", None, true, window, cx),
                Some(80.),
                &t,
            ));
            fields.push(over(
                "Username",
                w.input(&format!("{p}.username"), "None", "Username", None, false, window, cx),
                Some(112.),
                &t,
            ));
            fields.push(over(
                "Password",
                w.password(
                    &format!("{p}.password"),
                    if saved_pw { "Saved in the keychain" } else { "None" },
                    "Password",
                    144.,
                    window,
                    cx,
                ),
                Some(144.),
                &t,
            ));
        }
        let enabled = w.switch(&format!("{p}.enabled"), rule.enabled, "Enabled", false, cx);
        up_rows.push(network_row(
            &format!("network-upstream-{i}"),
            enabled,
            fields,
            row_errors(&format!("up.{i}.")),
            w.icon_button(&format!("net.remove.up.{i}"), IconName::Trash, "Remove", false, cx),
            &t,
        ));
    }
    let previous = div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(w.switch(
            "net.previous",
            n.draft.upstream.use_system_previous,
            "Use the proxy that was set before Proxymate",
            false,
            cx,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child("Use the proxy that was set before Proxymate for hosts no rule matches")
                .child(muted(
                    "While Proxymate is the system proxy, that's the one it replaced (with its bypass list); \
                     otherwise the system proxy itself.",
                    &t,
                )),
        )
        .into_any_element();
    let bypass = w.list_editor(
        "net.bypass",
        "Always direct",
        "*.internal.example",
        "Nothing bypasses the upstream rules",
        window,
        cx,
    );
    let mut upstream = vec![
        list_or_empty(
            "network-upstream",
            "Upstream rules",
            up_rows,
            "Every connection goes directly to its server",
            &t,
        ),
        w.icon_label_button(
            "net.add.up",
            IconName::Plus,
            "Add upstream rule",
            Look::Outline,
            false,
            cx,
        ),
    ];
    upstream.push(
        div()
            .flex()
            .items_start()
            .gap(px(12.))
            .child(div().w(px(LABEL_W)).flex_none().pt(px(4.)).child("Always direct"))
            .child(div().flex_1().min_w_0().child(bypass))
            .into_any_element(),
    );
    upstream.push(previous);

    // SOCKS5.
    let socks: Vec<AnyElement> = ["socks-address", "socks-curl"]
        .iter()
        .filter_map(|id| w.copy_box(id, cx))
        .collect();

    // Reverse proxies.
    let listeners = SettingsWindow::reverse_listeners(cx);
    let mut rev_rows = Vec::new();
    for (i, r) in n.draft.reverse_proxies.iter().enumerate() {
        let p = format!("net.rev.{i}");
        let mut fields = vec![
            over(
                "Local port",
                w.input(&format!("{p}.port"), "", "Local port", None, true, window, cx),
                Some(80.),
                &t,
            ),
            over(
                "Forwards to",
                w.input(
                    &format!("{p}.target"),
                    "https://api.example.com",
                    "Forwards to",
                    None,
                    true,
                    window,
                    cx,
                ),
                None,
                &t,
            ),
            labelled_switch(w.switch(&format!("{p}.tls"), r.tls, "HTTPS", false, cx), "HTTPS"),
            labelled_switch(
                w.switch(&format!("{p}.keep_host"), r.preserve_host, "Keep Host", false, cx),
                "Keep Host",
            ),
        ];
        if r.enabled {
            let (line, failed) = logic::reverse_status(r, &listeners);
            fields.push(
                div()
                    .w_full()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(11.))
                    .text_color(if failed { t.s_failed } else { t.muted_foreground }.hsla())
                    .child(line)
                    .into_any_element(),
            );
        }
        let enabled = w.switch(&format!("{p}.enabled"), r.enabled, "Enabled", false, cx);
        rev_rows.push(network_row(
            &format!("network-reverse-{i}"),
            enabled,
            fields,
            row_errors(&format!("rev.{i}.")),
            w.icon_button(&format!("net.remove.rev.{i}"), IconName::Trash, "Remove", false, cx),
            &t,
        ));
    }

    // Client certificates.
    let mut cert_rows = Vec::new();
    for (i, c) in n.draft.client_certs.iter().enumerate() {
        let p = format!("net.cert.{i}");
        let format = match c.identity {
            ClientIdentity::Pkcs12 { .. } => "pkcs12",
            ClientIdentity::Pem { .. } => "pem",
        };
        let options: Vec<(String, String)> = [("pkcs12", "PKCS#12"), ("pem", "PEM")]
            .map(|(v, l)| (v.into(), l.into()))
            .into();
        let mut fields = vec![
            over(
                "Hosts (comma separated)",
                w.input(
                    &format!("{p}.hosts"),
                    "mtls.example.com",
                    "Hosts",
                    None,
                    true,
                    window,
                    cx,
                ),
                Some(176.),
                &t,
            ),
            over(
                "Format",
                w.picker(&format!("{p}.format"), &options, format, "Format", 80., cx),
                Some(112.),
                &t,
            ),
        ];
        let mut path =
            |w: &mut SettingsWindow, field: &str, label: &str, placeholder: &str, cx: &mut Context<SettingsWindow>| {
                let input = w.input(&format!("{p}.{field}"), placeholder, label, None, true, window, cx);
                let choose = w.button(
                    &format!("net.choose.cert.{i}.{field}"),
                    "Choose…",
                    Look::Outline,
                    false,
                    cx,
                );
                over(
                    label,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(input)
                        .child(choose)
                        .into_any_element(),
                    None,
                    &t,
                )
            };
        match &c.identity {
            ClientIdentity::Pkcs12 { has_password, .. } => {
                fields.push(path(w, "path", ".p12 / .pfx file", "/Users/me/certs/client.p12", cx));
                fields.push(over(
                    "Password",
                    w.password(
                        &format!("{p}.password"),
                        if *has_password { "Saved in the keychain" } else { "None" },
                        "Certificate password",
                        144.,
                        window,
                        cx,
                    ),
                    Some(144.),
                    &t,
                ));
            }
            ClientIdentity::Pem { .. } => {
                fields.push(path(w, "cert", "Certificate (PEM)", "/Users/me/certs/client.pem", cx));
                fields.push(path(w, "key", "Key (PEM, optional)", "In the certificate file", cx));
            }
        }
        let enabled = w.switch(&format!("{p}.enabled"), c.enabled, "Enabled", false, cx);
        cert_rows.push(network_row(
            &format!("network-cert-{i}"),
            enabled,
            fields,
            row_errors(&format!("cert.{i}.")),
            w.icon_button(&format!("net.remove.cert.{i}"), IconName::Trash, "Remove", false, cx),
            &t,
        ));
    }

    // DNS overrides.
    let mut dns_rows = Vec::new();
    for (i, d) in n.draft.dns.iter().enumerate() {
        let p = format!("net.dns.{i}");
        let fields = vec![
            over(
                "Host",
                w.input(
                    &format!("{p}.host"),
                    "api.example.com or *.staging.test",
                    "Host",
                    None,
                    true,
                    window,
                    cx,
                ),
                None,
                &t,
            ),
            over(
                "Connect to",
                w.input(&format!("{p}.ip"), "127.0.0.1", "Connect to", None, true, window, cx),
                Some(160.),
                &t,
            ),
        ];
        let enabled = w.switch(&format!("{p}.enabled"), d.enabled, "Enabled", false, cx);
        dns_rows.push(network_row(
            &format!("network-dns-{i}"),
            enabled,
            fields,
            row_errors(&format!("dns.{i}.")),
            w.icon_button(&format!("net.remove.dns.{i}"), IconName::Trash, "Remove", false, cx),
            &t,
        ));
    }

    let body = vec![
        section(
            "Upstream proxies",
            Some(
                "Send Proxymate's own connections through another proxy, e.g. a corporate one. The first enabled \
                 rule whose hosts match wins; HTTPS and WebSockets go through CONNECT, plain HTTP in absolute form, \
                 and SOCKS5 proxies resolve names themselves.",
            ),
            None,
            upstream,
            &t,
        ),
        section(
            "SOCKS5",
            Some(
                "The proxy port also speaks SOCKS5 (no authentication). Connections take the same path as CONNECT: \
                 the SSL lists decide what is decrypted, plain HTTP is recorded as HTTP and anything else as a tunnel.",
            ),
            None,
            socks,
            &t,
        ),
        section(
            "Reverse proxies",
            Some(
                "Listen on a local port and forward everything to one server, recorded like proxied traffic. Point \
                 an app at http://localhost:<port> when it can't use a proxy. With HTTPS the port serves a Proxymate \
                 certificate for localhost.",
            ),
            None,
            vec![
                list_or_empty("network-reverse", "Reverse proxies", rev_rows, "No reverse proxies", &t),
                w.icon_label_button(
                    "net.add.rev",
                    IconName::Plus,
                    "Add reverse proxy",
                    Look::Outline,
                    false,
                    cx,
                ),
            ],
            &t,
        ),
        section(
            "Client certificates",
            Some(
                "Presented to servers that ask for one (mutual TLS). A PKCS#12 file's password is kept in the \
                 keychain; a PEM key must be unencrypted.",
            ),
            None,
            vec![
                list_or_empty(
                    "network-certs",
                    "Client certificates",
                    cert_rows,
                    "No client certificates",
                    &t,
                ),
                w.icon_label_button(
                    "net.add.cert",
                    IconName::Plus,
                    "Add client certificate",
                    Look::Outline,
                    false,
                    cx,
                ),
            ],
            &t,
        ),
        section(
            "DNS overrides",
            Some(
                "Connect to an address of your choosing for matching hosts, before any lookup, like /etc/hosts for \
                 Proxymate alone. The server name (SNI) and Host header don't change.",
            ),
            None,
            vec![
                list_or_empty("network-dns", "DNS overrides", dns_rows, "Names resolve normally", &t),
                w.icon_label_button(
                    "net.add.dns",
                    IconName::Plus,
                    "Add DNS override",
                    Look::Outline,
                    false,
                    cx,
                ),
            ],
            &t,
        ),
    ];
    (Some(header), body)
}

fn list_or_empty(id: &str, label: &str, items: Vec<AnyElement>, empty: &str, t: &Tokens) -> AnyElement {
    if items.is_empty() {
        empty_rows(empty, t)
    } else {
        rows(id, label, items, t)
    }
}

fn labelled_switch(switch: AnyElement, label: &str) -> AnyElement {
    div()
        .h(px(28.))
        .mt(px(14.))
        .flex()
        .items_center()
        .gap(px(6.))
        .child(switch)
        .child(label.to_owned())
        .into_any_element()
}

/// A row of the network page: its switch, its fields (wrapping), any
/// field errors, then Remove.
fn network_row(
    id: &str,
    enabled: AnyElement,
    fields: Vec<AnyElement>,
    errors: Vec<AnyElement>,
    remove: AnyElement,
    _t: &Tokens,
) -> AnyElement {
    div()
        .id(SharedString::from(id.to_owned()))
        .role(Role::ListItem)
        .flex()
        .items_start()
        .gap(px(8.))
        .child(div().pt(px(18.)).child(enabled))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().flex().flex_wrap().items_end().gap(px(8.)).children(fields))
                .children(errors),
        )
        .child(div().pt(px(16.)).child(remove))
        .into_any_element()
}

// ---- SSL Proxying ----

fn ssl(w: &mut SettingsWindow, window: &mut Window, cx: &mut Context<SettingsWindow>) -> Vec<AnyElement> {
    let t = *cx.tokens();
    let mut out = Vec::new();
    if let Some(err) = w.error("ssl") {
        out.push(error("ssl", err.to_owned(), &t));
    }
    for (id, title, description, placeholder, empty) in logic::SSL_LISTS {
        let editor = w.list_editor(&format!("ssl.{id}"), title, placeholder, empty, window, cx);
        out.push(section(title, Some(description), None, vec![editor], &t));
    }
    out
}

// ---- Clients ----

fn clients(w: &mut SettingsWindow, window: &mut Window, cx: &mut Context<SettingsWindow>) -> Vec<AnyElement> {
    let t = *cx.tokens();
    let access = load(&w.integration.access, cx);
    let devices = w.devices.read(cx).value.clone().map(|d| d.devices).unwrap_or_default();
    let lan_on = access.as_ref().is_some_and(|a| a.enabled);
    let mut body = Vec::new();
    if !lan_on {
        body.push(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(muted("LAN access is off, so only this Mac can use the proxy.", &t))
                .child(w.button("page:proxy", "Turn it on in Proxy", Look::Ghost, false, cx))
                .into_any_element(),
        );
    }
    if let Some(err) = w.error("clients") {
        body.push(error("clients", err.to_owned(), &t));
    }
    if devices.is_empty() {
        body.push(empty_rows("No devices have connected yet", &t));
    } else {
        let mono = cx.theme().mono_font_family.clone();
        let mut items = Vec::new();
        for d in &devices {
            let tone = match d.decision {
                DeviceDecision::Allowed => t.s_2xx,
                DeviceDecision::Denied => t.s_failed,
                DeviceDecision::Pending => t.s_4xx,
            };
            let field = format!("client.{}", d.ip);
            let nickname = w.text(&field, cx);
            let input = w.input(
                &field,
                "Nickname",
                &format!("Nickname for {}", d.ip),
                Some(160.),
                false,
                window,
                cx,
            );
            let buttons: Vec<AnyElement> = logic::client_buttons(d, &nickname)
                .into_iter()
                .map(|(label, allow)| {
                    let (id, look) = match allow {
                        Some(true) => (format!("client.allow:{}", d.ip), Look::Primary),
                        Some(false) => (format!("client.deny:{}", d.ip), Look::Outline),
                        None => (format!("client.forget:{}", d.ip), Look::Ghost),
                    };
                    w.button(&id, label, look, false, cx)
                })
                .collect();
            let row = div()
                .id(SharedString::from(format!("client-{}", d.ip)))
                .role(Role::ListItem)
                .aria_label(SharedString::from(format!(
                    "{}, {}",
                    d.ip,
                    logic::decision_label(d.decision)
                )))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(pm_widgets::icon(IconName::Smartphone, t.muted_foreground.hsla(), 14.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap(px(8.))
                                .child(div().font_family(mono.clone()).text_size(px(11.)).child(d.ip.clone()))
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(tone.hsla())
                                        .child(logic::decision_label(d.decision)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(t.muted_foreground.hsla())
                                .child(logic::device_activity(d)),
                        ),
                )
                .child(input)
                .children(buttons);
            items.push(
                w.with_menu(row, format!("device:{}", d.ip), cx)
                    .test_support()
                    .into_any_element(),
            );
        }
        body.push(rows("clients-list", "LAN clients", items, &t));
    }
    vec![section(
        "LAN clients",
        lan_on.then_some(
            "Devices on your network that connected to Proxymate. Loopback and this Mac's own addresses are always \
             allowed.",
        ),
        None,
        body,
        &t,
    )]
}

// ---- Storage ----

fn storage(w: &mut SettingsWindow, window: &mut Window, cx: &mut Context<SettingsWindow>) -> Vec<AnyElement> {
    let t = *cx.tokens();
    let status = w.storage.read(cx).value.clone();
    let limits = load(&w.caches.storage, cx);
    let recording = load(&w.caches.recording, cx);
    let autosave = load(&w.caches.autosave, cx);

    // Live capture.
    let mut live = Vec::new();
    if let Some(status) = &status {
        let mono = cx.theme().mono_font_family.clone();
        let list = div()
            .id("storage-status")
            .role(Role::List)
            .aria_label("Live capture")
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(logic::storage_rows(status).into_iter().map(|(label, value)| {
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .w(px(LABEL_W))
                            .flex_none()
                            .text_color(t.muted_foreground.hsla())
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .when(label == "Location", |d| d.font_family(mono.clone()).text_size(px(11.)))
                            .child(value),
                    )
            }));
        live.push(
            w.with_menu(list, "storage-status".into(), cx)
                .test_support()
                .into_any_element(),
        );
        if let Some(err) = &status.error {
            live.push(error(
                "storage-write",
                format!("Bodies aren't being recorded: {err}."),
                &t,
            ));
        }
    }
    live.push(w.button("storage.clear", "Clear all flows", Look::Outline, false, cx));

    // Limits.
    let mut limit_body = Vec::new();
    if let Some(limits) = limits {
        for (id, label, unit, hint) in logic::LIMITS {
            limit_body.push(w.number(&format!("storage.{id}"), label, Some(unit), Some(hint), window, cx));
        }
        let parsed = w.numbers(
            &[
                ("storage.max_flows", 0, u32::MAX),
                ("storage.max_disk_mb", 0, u32::MAX),
                ("storage.max_ws_messages", 0, u32::MAX),
            ],
            cx,
        );
        let changed = parsed.as_ref().map_or(true, |n| {
            *n != [limits.max_flows, limits.max_disk_mb, limits.max_ws_messages]
        });
        let mut buttons = vec![w.button(
            "storage.save",
            "Save limits",
            Look::Primary,
            !changed || parsed.is_err(),
            cx,
        )];
        if changed {
            buttons.push(w.button("storage.revert", "Revert", Look::Ghost, false, cx));
        }
        limit_body.push(div().flex().gap(px(8.)).children(buttons).into_any_element());
        if let Some(err) = w.error("storage") {
            limit_body.push(error("storage", err.to_owned(), &t));
        } else if let Some(note) = w.note("storage") {
            limit_body.push(saved(note.to_owned(), &t));
        }
    }

    let mut out = vec![
        section(
            "Live capture",
            Some("Bodies and details are kept in files that are deleted on Clear and on quit."),
            None,
            live,
            &t,
        ),
        section(
            "Limits",
            Some(
                "Past a limit the oldest flows are evicted, a twentieth at a time, and their files deleted. Pinned \
                 flows and flows still in flight are kept. 0 means no limit.",
            ),
            None,
            limit_body,
            &t,
        ),
    ];

    // Recording lists (M21).
    if recording.is_some() {
        let never = w.list_editor(
            "rec.never",
            "Never record",
            "*.apple.com",
            "Every host is recorded",
            window,
            cx,
        );
        out.push(section(
            "Never record",
            Some(
                "Traffic to these hosts is still proxied, decrypted and rewritten by rules, but leaves no flow. \
                 *.example.com also covers example.com. Right-click a flow for \u{201c}Never record this host\u{201d}.",
            ),
            None,
            vec![never],
            &t,
        ));
        let mut only = vec![w.list_editor(
            "rec.only",
            "Only record",
            "api.example.com",
            "No allow-list: every host not above is recorded",
            window,
            cx,
        )];
        if let Some(err) = w.error("recording") {
            only.push(error("recording", err.to_owned(), &t));
        }
        out.push(section(
            "Only record",
            Some("When this list has hosts, flows to any other host aren't recorded. Never record wins over it."),
            None,
            only,
            &t,
        ));
    }

    // Auto-save (M21).
    if let Some(settings) = autosave {
        let folder = w.input(
            "autosave.folder",
            "/Users/you/Documents/Proxymate",
            "Folder",
            None,
            true,
            window,
            cx,
        );
        let choose = w.button("autosave.choose", "Choose…", Look::Outline, false, cx);
        let mut body = vec![
            labelled("Folder", vec![folder, choose], None, &t),
            w.number("autosave.interval", "Every", Some("minutes"), None, window, cx),
            w.number(
                "autosave.keep",
                "Keep",
                Some("sessions"),
                Some("Older Proxymate Autosave files in the folder are deleted; nothing else there is touched."),
                window,
                cx,
            ),
        ];
        let status_now = status.as_ref().map(|s| s.autosave.clone());
        let saving = status_now.as_ref().is_some_and(|s| s.saving);
        let draft = w.numbers(&[("autosave.interval", 1, 1440), ("autosave.keep", 1, 1000)], cx);
        let changed = match &draft {
            Ok(n) => {
                w.text("autosave.folder", cx).trim() != settings.folder
                    || n[0] != settings.interval_minutes
                    || n[1] != settings.keep
            }
            Err(_) => true,
        };
        let line = logic::autosave_line(status_now.as_ref(), settings.enabled);
        body.push(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(w.button("autosave.save", "Save", Look::Primary, !changed || draft.is_err(), cx))
                .child(w.button(
                    "autosave.now",
                    "Save now",
                    Look::Outline,
                    settings.folder.is_empty() || saving,
                    cx,
                ))
                .child(
                    div()
                        .id("autosave-status")
                        .role(Role::Status)
                        .aria_label(SharedString::from(line.clone()))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(t.muted_foreground.hsla())
                        .child(line),
                )
                .into_any_element(),
        );
        if let Some(err) = w.error("autosave").map(str::to_owned).or_else(|| {
            status_now
                .and_then(|s| s.error)
                .map(|e| format!("The last auto-save failed: {e}"))
        }) {
            body.push(error("autosave", err, &t));
        }
        out.push(section(
            "Auto-save",
            Some(
                "Saves the live capture as a session in a folder, keeping the newest few. Nothing is written while \
                 the capture hasn't changed.",
            ),
            Some(w.switch(
                "autosave.enabled",
                settings.enabled,
                "Auto-save the live capture",
                false,
                cx,
            )),
            body,
            &t,
        ));
    }
    out
}

// ---- Protobuf ----

fn list_row(
    id: String,
    icon: IconName,
    title: AnyElement,
    lines: Vec<String>,
    actions: Vec<AnyElement>,
    label: String,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let t = *cx.tokens();
    let mono = cx.theme().mono_font_family.clone();
    let n = lines.len();
    div()
        .id(SharedString::from(id))
        .role(Role::ListItem)
        .aria_label(SharedString::from(label))
        .flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .pt(px(2.))
                .child(pm_widgets::icon(icon, t.muted_foreground.hsla(), 14.)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(title)
                .children(lines.into_iter().enumerate().map(|(i, line)| {
                    div()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(t.muted_foreground.hsla())
                        .when(i == 0 && n > 1, |d| d.font_family(mono.clone()))
                        .child(line)
                })),
        )
        .children(actions)
        .into_any_element()
}

fn strong(text: impl Into<SharedString>) -> AnyElement {
    div()
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
        .into_any_element()
}

fn protobuf(w: &mut SettingsWindow, cx: &mut Context<SettingsWindow>) -> Vec<AnyElement> {
    let t = *cx.tokens();
    let schemas = load(&w.caches.proto_schemas, cx).unwrap_or_default();
    let mappings = load(&w.caches.proto_mappings, cx).unwrap_or_default();
    let busy = w.is_busy("proto");
    let mut body = Vec::new();
    if let Some(err) = w.error("proto") {
        body.push(error("proto", err.to_owned(), &t));
    }
    if schemas.is_empty() {
        body.push(muted("No schemas yet.", &t));
    } else {
        let items = schemas
            .iter()
            .map(|s| {
                let (files, types) = logic::schema_lines(s);
                let remove = w.icon_button(
                    &format!("proto.remove:{}", s.id),
                    IconName::Trash,
                    &format!("Remove {}", s.name),
                    busy,
                    cx,
                );
                list_row(
                    format!("proto-schema-{}", s.id),
                    IconName::FileCode,
                    strong(s.name.clone()),
                    vec![files, types],
                    vec![remove],
                    s.name.clone(),
                    cx,
                )
            })
            .collect();
        body.push(rows("proto-schemas", "Schemas", items, &t));
    }
    let mut map_body = Vec::new();
    if mappings.is_empty() {
        map_body.push(muted("None.", &t));
    } else {
        let mono = cx.theme().mono_font_family.clone();
        let items = mappings
            .iter()
            .map(|m| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .font_family(mono.clone())
                    .text_size(px(11.))
                    .child(div().flex_1().min_w_0().truncate().child(logic::mapping_line(m)))
                    .child(m.message_type.clone())
                    .child(w.icon_button(
                        &format!("proto.unmap:{}", m.id),
                        IconName::Trash,
                        "Remove mapping",
                        busy,
                        cx,
                    ))
                    .into_any_element()
            })
            .collect();
        map_body.push(rows("proto-mappings", "Decode as mappings", items, &t));
    }
    vec![
        section(
            "Schemas",
            Some(
                ".proto files (compiled here, imports resolved from their folder) or a descriptor set from buf \
                 build -o or protoc --descriptor_set_out. gRPC calls find their message types from the path; \
                 without a schema, bodies decode schema-less.",
            ),
            Some(w.button("proto.import", "Import…", Look::Outline, busy, cx)),
            body,
            &t,
        ),
        section(
            "Decode as",
            Some(
                "Protobuf bodies outside gRPC decode as the message type picked in the Body tab's \u{201c}Decode \
                 as\u{201d} menu, for that host and path.",
            ),
            None,
            map_body,
            &t,
        ),
    ]
}

// ---- Mocking ----

fn mocking(w: &mut SettingsWindow, cx: &mut Context<SettingsWindow>) -> Vec<AnyElement> {
    let t = *cx.tokens();
    let sources = load(&w.rules.mock_sources, cx).unwrap_or_default();
    let specs = load(&w.caches.openapi_specs, cx).unwrap_or_default();
    let mut rec = Vec::new();
    if let Some(err) = w.error("mocking") {
        rec.push(error("mocking", err.to_owned(), &t));
    }
    if sources.is_empty() {
        rec.push(muted("No recordings yet.", &t));
    } else {
        let items = sources
            .iter()
            .map(|s| {
                let remove = w.icon_button(
                    &format!("mock.remove:{}", s.id),
                    IconName::Trash,
                    &format!("Remove {}", s.name),
                    false,
                    cx,
                );
                list_row(
                    format!("mock-source-{}", s.id),
                    IconName::FileBox,
                    strong(s.name.clone()),
                    vec![s.origin.clone(), logic::source_line(s)],
                    vec![remove],
                    s.name.clone(),
                    cx,
                )
            })
            .collect();
        rec.push(rows("mock-sources", "Recordings", items, &t));
    }
    let mut spec_body = Vec::new();
    if specs.is_empty() {
        spec_body.push(muted("No specs yet.", &t));
    } else {
        let items = specs
            .iter()
            .map(|s| {
                let generate = w.icon_label_button(
                    &format!("openapi.generate:{}", s.id),
                    IconName::WandSparkles,
                    "Generate mocks",
                    Look::Outline,
                    s.operations == 0,
                    cx,
                );
                let remove = w.icon_button(
                    &format!("openapi.remove:{}", s.id),
                    IconName::Trash,
                    &format!("Remove {}", s.name),
                    false,
                    cx,
                );
                let title = div()
                    .flex()
                    .gap(px(6.))
                    .child(strong(s.name.clone()))
                    .child(div().text_color(t.muted_foreground.hsla()).child(s.version.clone()))
                    .into_any_element();
                list_row(
                    format!("openapi-spec-{}", s.id),
                    IconName::FileCode,
                    title,
                    vec![logic::spec_line(s)],
                    vec![generate, remove],
                    s.name.clone(),
                    cx,
                )
            })
            .collect();
        spec_body.push(rows("openapi-specs", "OpenAPI specs", items, &t));
    }
    vec![
        section(
            "Recordings",
            Some(
                "Sessions copied in with \u{201c}Mock from session…\u{201d} (a tab's menu) or from a file: a \
                 .proxymate session, a HAR, or a Charles export (.chlsj, .chlsx). A \u{201c}Serve from \
                 recording\u{201d} rule answers matching requests with what was recorded, without contacting the \
                 server.",
            ),
            Some(w.button("mock.file", "Add from file…", Look::Outline, false, cx)),
            rec,
            &t,
        ),
        section(
            "OpenAPI specs",
            Some(
                "OpenAPI 3.0 and 3.1, JSON or YAML. Flows whose method and path match an operation show its \
                 operationId in the inspector, and op: in the search box finds them. \u{201c}Generate mocks\u{201d} \
                 adds one Map Local rule per operation, from its example or a minimal value of its response schema, \
                 turned off until you review it.",
            ),
            Some(w.button("openapi.import", "Import…", Look::Outline, false, cx)),
            spec_body,
            &t,
        ),
    ]
}

// ---- Appearance ----

fn appearance(w: &mut SettingsWindow, cx: &mut Context<SettingsWindow>) -> Vec<AnyElement> {
    let t = *cx.tokens();
    let current = crate::prefs::get(cx).appearance;
    let cards = [
        (Appearance::System, "system", "System", IconName::Monitor),
        (Appearance::Light, "light", "Light", IconName::Sun),
        (Appearance::Dark, "dark", "Dark", IconName::Moon),
    ]
    .map(|(value, id, label, icon)| {
        let on = current == value;
        div()
            .id(SharedString::from(format!("appearance-{id}")))
            .role(Role::RadioButton)
            .aria_label(SharedString::from(label))
            .aria_toggled(if on { Toggled::True } else { Toggled::False })
            .w(px(144.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(if on { t.primary } else { t.border }.hsla())
            .when(on, |d| d.border_2())
            .cursor_pointer()
            .hover(|d| d.bg(t.accent.hsla().opacity(0.5)))
            .child(swatch(value, &t))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(pm_widgets::icon(icon, t.foreground.hsla(), 14.))
                    .child(label),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.choose("appearance", id, window, cx)))
            .test_support()
    });
    vec![
        section(
            "Theme",
            Some("System follows macOS, switching with it between light and dark."),
            None,
            vec![
                div()
                    .id("appearance-theme")
                    .role(Role::RadioGroup)
                    .aria_label("Theme")
                    .flex()
                    .gap(px(12.))
                    .children(cards)
                    .into_any_element(),
            ],
            &t,
        ),
        section(
            "Flow table columns",
            Some(
                "Drag a column's edge to resize it, drag its header to move it, and right-click the header to show \
                 or hide columns. Widths, order, visibility and sort are remembered.",
            ),
            None,
            vec![w.button("columns.reset", "Reset columns", Look::Outline, false, cx)],
            &t,
        ),
    ]
}

/// A tiny window in that theme, drawn with its own tokens (System shows
/// light beside dark).
fn swatch(appearance: Appearance, t: &Tokens) -> AnyElement {
    let mini = |mode: Mode| {
        let t = mode.tokens();
        div()
            .flex_1()
            .flex()
            .bg(t.pane_table.hsla())
            .child(div().w(px(18.)).bg(t.muted.hsla()))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .p(px(5.))
                    .child(div().h(px(5.)).rounded_full().bg(t.border.hsla()))
                    .children([t.m_get, t.m_post, t.selection_bg, t.m_delete].map(|c| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .child(div().h(px(5.)).w(px(10.)).rounded(px(1.)).bg(c.hsla()))
                            .child(
                                div()
                                    .h(px(3.))
                                    .flex_1()
                                    .rounded_full()
                                    .bg(t.muted_foreground.hsla().opacity(0.4)),
                            )
                    })),
            )
    };
    let body = match appearance {
        Appearance::System => div().flex_1().flex().child(mini(Mode::Light)).child(mini(Mode::Dark)),
        Appearance::Light => div().flex_1().flex().child(mini(Mode::Light)),
        Appearance::Dark => div().flex_1().flex().child(mini(Mode::Dark)),
    };
    div()
        .h(px(64.))
        .flex()
        .overflow_hidden()
        .rounded(px(6.))
        .border_1()
        .border_color(t.border.hsla())
        .child(body)
        .into_any_element()
}

// ---- Advanced ----

fn advanced(w: &mut SettingsWindow, window: &mut Window, cx: &mut Context<SettingsWindow>) -> Vec<AnyElement> {
    let t = *cx.tokens();
    let rules = load(&w.rules.settings, cx);
    let proxy = load(&w.caches.proxy, cx);
    let scripts = load(&w.caches.scripts, cx);
    let mut out = Vec::new();
    if let Some(rules) = rules {
        let parsed = w.numbers(&[("adv.bp", 0, u32::MAX)], cx);
        let same = parsed.as_ref().is_ok_and(|n| n[0] == rules.breakpoint_timeout_secs);
        let input = w.input("adv.bp", "", "Breakpoint timeout", Some(112.), true, window, cx);
        let save = w.button("adv.bp.save", "Save", Look::Outline, same || parsed.is_err(), cx);
        let mut body = vec![labelled("Timeout", vec![input, muted("seconds", &t), save], None, &t)];
        if let Some(err) = w.error("bp") {
            body.push(error("bp", err.to_owned(), &t));
        }
        out.push(section(
            "Breakpoints",
            Some(
                "A paused flow waits this long for you, then continues unmodified (and is marked as timed out). 0 \
                 waits forever.",
            ),
            None,
            body,
            &t,
        ));
    }
    if let Some(scripts) = scripts {
        let keys = [("adv.traffic_ms", 10, 60_000), ("adv.view_ms", 10, 60_000)];
        let parsed = w.numbers(&keys, cx);
        let changed = parsed
            .as_ref()
            .map_or(true, |n| *n != [scripts.traffic_timeout_ms, scripts.view_timeout_ms]);
        let mut body = vec![
            w.number(
                "adv.traffic_ms",
                "Rewrite and mock scripts",
                Some("ms"),
                None,
                window,
                cx,
            ),
            w.number("adv.view_ms", "View scripts", Some("ms"), None, window, cx),
            w.button(
                "adv.scripts.save",
                "Save",
                Look::Primary,
                !changed || parsed.is_err(),
                cx,
            ),
        ];
        if let Some(err) = w.error("scripts") {
            body.push(error("scripts", err.to_owned(), &t));
        }
        out.push(section(
            "Scripts",
            Some("How long a script may run before it is stopped; the flow then goes on without it. 10 ms to 60 s."),
            None,
            body,
            &t,
        ));
    }
    if let Some(proxy) = proxy {
        let keys = [
            ("adv.pool_idle", 0, u32::MAX),
            ("adv.pool_max", 0, 1024),
            ("adv.connect", 1, 300),
        ];
        let parsed = w.numbers(&keys, cx);
        let changed = parsed.as_ref().map_or(true, |n| {
            *n != [
                proxy.pool_idle_secs,
                proxy.pool_max_idle_per_host,
                proxy.connect_timeout_secs,
            ]
        });
        let busy = w.is_busy("pool");
        let mut body = vec![
            w.number(
                "adv.pool_idle",
                "Keep idle connections for",
                Some("seconds"),
                None,
                window,
                cx,
            ),
            w.number(
                "adv.pool_max",
                "Idle connections per host",
                None,
                Some("0 closes every connection after its request."),
                window,
                cx,
            ),
            w.number("adv.connect", "Connect timeout", Some("seconds"), None, window, cx),
            w.button(
                "adv.pool.save",
                if busy { "Restarting…" } else { "Save" },
                Look::Primary,
                busy || !changed || parsed.is_err(),
                cx,
            ),
        ];
        if let Some(err) = w.error("pool") {
            body.push(error("pool", err.to_owned(), &t));
        }
        out.push(section(
            "Upstream connections",
            Some(
                "Plain HTTP requests share a pool of connections to each server. Decrypted HTTPS opens one upstream \
                 connection per client connection. Saving restarts the listener.",
            ),
            None,
            body,
            &t,
        ));
    }
    out
}

// ---- MCP ----

fn small_label(text: &str, t: &Tokens) -> AnyElement {
    div()
        .mt(px(4.))
        .text_size(px(10.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.muted_foreground.hsla())
        .child(text.to_uppercase())
        .into_any_element()
}

fn mcp(w: &mut SettingsWindow, cx: &mut Context<SettingsWindow>) -> Vec<AnyElement> {
    let t = *cx.tokens();
    let Some(status) = w.mcp.read(cx).value.clone() else {
        return vec![muted("Loading…", &t)];
    };
    let token = load(&w.caches.mcp_token, cx);
    let limit = w.audit_limit();
    let audit = w
        .caches
        .mcp_audit
        .update(cx, |c, cx| c.get(&limit, cx))
        .ready()
        .cloned();

    let (tone, line) = logic::serving_line(&status);
    let dot = match tone {
        "ok" => t.s_2xx,
        "error" => t.s_5xx,
        _ => t.muted_foreground,
    };
    let mut serving = status_line("mcp-serving", dot, line, &t);
    if status.settings.read_only {
        serving = serving.child(muted("read-only", &t));
    }
    let serving = w
        .with_menu(serving, "mcp-serving".into(), cx)
        .test_support()
        .into_any_element();

    let mut out = vec![
        section(
            "MCP server",
            Some(
                "Lets AI agents (Claude Code, Cursor, the MCP Inspector…) read recorded traffic, manage rules and \
                 resolve breakpoints.",
            ),
            Some(w.switch(
                "mcp.enabled",
                status.settings.enabled,
                "Enable the MCP server",
                false,
                cx,
            )),
            vec![serving],
            &t,
        ),
        section(
            "Read-only",
            Some(
                "Agents can still inspect traffic and rules, but can't create or change rules, resolve breakpoints, \
                 pause recording or clear flows. Connected clients see their tool list change straight away.",
            ),
            Some(w.switch("mcp.read_only", status.settings.read_only, "Read-only mode", false, cx)),
            w.error("mcp")
                .map(|e| error("mcp", e.to_owned(), &t))
                .into_iter()
                .collect(),
            &t,
        ),
    ];

    if token.is_some() {
        let mut body = vec![small_label("Claude Code", &t)];
        body.extend(w.copy_box("mcp-claude", cx));
        body.push(small_label("JSON config (Cursor, Claude Desktop, .mcp.json…)", &t));
        body.extend(w.copy_box("mcp-json", cx));
        body.push(muted(
            "Clients that only speak stdio can run proxymate mcp-stdio (the app binary), which reads the token \
             itself and starts Proxymate if it isn't running.",
            &t,
        ));
        let regenerate = if w.confirming_regenerate() {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .text_color(t.s_4xx.hsla())
                        .child("Connected agents will need the new token."),
                )
                .child(w.button("mcp.regenerate.confirm", "Regenerate", Look::Danger, false, cx))
                .child(w.button("mcp.regenerate.cancel", "Cancel", Look::Ghost, false, cx))
                .into_any_element()
        } else {
            w.icon_label_button(
                "mcp.regenerate",
                IconName::KeyRound,
                "Regenerate token",
                Look::Outline,
                false,
                cx,
            )
        };
        body.push(regenerate);
        out.push(section(
            "Connect an agent",
            Some(
                "Each request needs this install's bearer token. Only loopback clients without a foreign Origin are \
                 accepted.",
            ),
            None,
            body,
            &t,
        ));
    }

    // Connected clients.
    let clients_body = if status.clients.is_empty() {
        vec![muted("No agent is connected.", &t)]
    } else {
        let items = status
            .clients
            .iter()
            .map(|c| {
                let row = div()
                    .id(SharedString::from(format!("mcp-client-{}", c.session)))
                    .role(Role::ListItem)
                    .aria_label(SharedString::from(format!("{} {}", c.name, c.version)))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(pm_widgets::icon(IconName::Bot, t.rule_agent.hsla(), 14.))
                    .child(strong(c.name.clone()))
                    .child(muted(c.version.clone(), &t))
                    .child(
                        div()
                            .ml_auto()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(px(10.))
                            .text_color(t.muted_foreground.hsla())
                            .child(format!(
                                "session {} · since {}",
                                c.session,
                                logic::format_clock(c.connected_at_ms)
                            )),
                    );
                w.with_menu(row, format!("mcp-client:{}", c.session), cx)
                    .test_support()
                    .into_any_element()
            })
            .collect();
        vec![rows("mcp-clients", "Connected clients", items, &t)]
    };
    out.push(section(
        format!("Connected clients ({})", status.clients.len()),
        None,
        None,
        clients_body,
        &t,
    ));

    // Audit log.
    let entries = audit.as_ref().map(|a| a.entries.clone()).unwrap_or_default();
    let aside = div()
        .flex()
        .gap(px(4.))
        .child(w.icon_button("mcp.audit.refresh", IconName::RefreshCw, "Refresh", false, cx))
        .child(w.icon_label_button(
            "mcp.audit.clear",
            IconName::Trash,
            "Clear",
            Look::Ghost,
            entries.is_empty(),
            cx,
        ))
        .into_any_element();
    let mut audit_body = Vec::new();
    if entries.is_empty() {
        audit_body.push(muted("No calls yet.", &t));
    } else {
        audit_body.push(audit_table(w, &entries, cx));
    }
    if audit.as_ref().is_some_and(|a| a.next.is_some()) && limit < logic::AUDIT_MAX {
        audit_body.push(w.button("mcp.audit.more", "Load older calls", Look::Ghost, false, cx));
    }
    out.push(section(
        "Audit log",
        Some("Every tool call, newest first, with the client that made it."),
        Some(aside),
        audit_body,
        &t,
    ));
    out
}

const AUDIT_COLUMNS: [(&str, Option<f32>); 6] = [
    ("Time", Some(72.)),
    ("Client", Some(96.)),
    ("Tool", Some(140.)),
    ("Arguments", None),
    ("Result", Some(64.)),
    ("ms", Some(52.)),
];

fn audit_table(
    w: &SettingsWindow,
    entries: &[pm_model::McpAuditEntry],
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let t = *cx.tokens();
    let mono = cx.theme().mono_font_family.clone();
    let cell = |text: SharedString, width: Option<f32>, i: usize| {
        div()
            .px(px(8.))
            .py(px(4.))
            .truncate()
            .map(|d| match width {
                Some(w) => d.w(px(w)).flex_none(),
                None => d.flex_1().min_w_0(),
            })
            .when(i == 5, |d| d.text_right())
            .child(text)
    };
    let header = div()
        .id("audit-header")
        .role(Role::Row)
        .flex()
        .bg(t.muted.hsla().opacity(0.5))
        .text_color(t.muted_foreground.hsla())
        .font_weight(FontWeight::MEDIUM)
        .children(AUDIT_COLUMNS.iter().enumerate().map(|(i, (title, width))| {
            cell((*title).into(), *width, i)
                .id(SharedString::from(format!("audit-column-{i}")))
                .role(Role::ColumnHeader)
                .aria_label(SharedString::from(*title))
        }));
    let rows_el = entries.iter().map(|e| {
        let (fg, bg) = match e.outcome {
            McpCallOutcome::Ok => (t.s_2xx, t.s_2xx_bg),
            McpCallOutcome::Error => (t.s_5xx, t.s_5xx_bg),
            McpCallOutcome::Refused => (t.s_4xx, t.s_4xx_bg),
        };
        let args = match &e.error {
            Some(err) => div().text_color(t.s_5xx.hsla()).child(err.clone()),
            None => div().text_color(t.muted_foreground.hsla()).child(e.arguments.clone()),
        };
        let row = div()
            .id(SharedString::from(format!("audit-{}", e.id)))
            .role(Role::Row)
            .aria_label(SharedString::from(format!(
                "{} by {}: {}",
                e.tool,
                e.client,
                logic::outcome_label(e.outcome)
            )))
            .flex()
            .border_t_1()
            .border_color(t.border.hsla())
            .child(
                cell(logic::format_clock(e.time_ms).into(), Some(72.), 0)
                    .font_family(mono.clone())
                    .text_color(t.muted_foreground.hsla()),
            )
            .child(cell(e.client.clone().into(), Some(96.), 1))
            .child(cell(e.tool.clone().into(), Some(140.), 2).font_family(mono.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px(px(8.))
                    .py(px(4.))
                    .truncate()
                    .font_family(mono.clone())
                    .child(args),
            )
            .child(
                div().w(px(64.)).flex_none().px(px(8.)).py(px(4.)).child(
                    div()
                        .px(px(4.))
                        .rounded(px(3.))
                        .text_size(px(10.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg.hsla())
                        .bg(bg.hsla())
                        .child(logic::outcome_label(e.outcome)),
                ),
            )
            .child(
                cell(e.duration_ms.to_string().into(), Some(52.), 5)
                    .font_family(mono.clone())
                    .text_color(t.muted_foreground.hsla()),
            );
        w.with_menu(row, format!("audit:{}", e.id), cx).test_support()
    });
    div()
        .id("mcp-audit")
        .role(Role::Table)
        .aria_label("Audit log")
        .rounded(px(6.))
        .border_1()
        .border_color(t.border.hsla())
        .overflow_hidden()
        .text_size(px(11.))
        .child(header)
        .children(rows_el.collect::<Vec<_>>())
        .into_any_element()
}

// ---- Updates ----

fn updates(w: &mut SettingsWindow, cx: &mut Context<SettingsWindow>) -> Vec<AnyElement> {
    let t = *cx.tokens();
    let settings = load(&w.caches.updates, cx);
    let loaded = settings.is_some();
    let mut out = vec![
        section(
            format!("Proxymate {}", crate::updater::VERSION),
            Some(
                "Updates come from Proxymate's GitHub releases and are verified against its signing key before \
                 they're installed. Proxymate asks before installing and restarts afterwards.",
            ),
            Some(w.button("updates.check", "Check Now", Look::Outline, false, cx)),
            vec![],
            &t,
        ),
        section(
            "Check for updates at launch",
            Some(
                "A quiet check a few seconds after Proxymate starts. You only hear about it when an update is available.",
            ),
            Some(w.switch(
                "updates.launch",
                settings.is_none_or(|s| s.check_at_launch),
                "Check for updates at launch",
                !loaded,
                cx,
            )),
            vec![],
            &t,
        ),
    ];
    if let Some(err) = w.error("updates") {
        out.push(error("updates", err.to_owned(), &t));
    }
    out
}
