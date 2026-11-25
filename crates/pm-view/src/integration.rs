//! The toolbar's macOS integration surfaces (G13), as plain data: the
//! certificate wizard's checks and the Certificate popover's fields
//! (`CertWizard.tsx`, `CertificateMenu.tsx`), the system proxy popover's
//! texts (`SystemProxyMenu.tsx`), the MCP indicator's tone and tip
//! (`McpIndicator.tsx`), and the Intercept menu's targets with the toast a
//! launch shows (`lib/capture.ts`, `lib/actions.ts` `intercept`).

use pm_model::{InterceptSession, InterceptTarget, KeychainTrust, McpStatus, SystemProxyStatus, TrustStatus};

use crate::devices::{Check, CheckState};

// ---- Certificate ----

/// `ab:cd:…`, as Keychain Access shows a fingerprint.
pub fn colon_hex(hex: &str) -> String {
    hex.as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

/// The popover's "Try it" line.
pub fn curl_example(listening: Option<&str>, cert_path: &str) -> String {
    let proxy = listening.map_or_else(|| "http://127.0.0.1:9090".to_owned(), |a| format!("http://{a}"));
    format!("curl -x {proxy} --cacert \"{cert_path}\" https://example.com/")
}

/// The verifier's own wording, minus the rustls error enum, as a sentence.
pub fn trust_detail(detail: &str) -> String {
    // `/ \(.*\)$/`: from the first " (" when the text ends in ")".
    let text = match detail.find(" (") {
        Some(at) if detail.ends_with(')') => &detail[..at],
        _ => detail,
    };
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// The wizard's two checks: the keychain, then the real handshake.
pub fn wizard_checks(trust: Option<&TrustStatus>) -> [Check; 2] {
    let Some(trust) = trust else {
        return [
            Check::new(CheckState::Pending, "Trusted in the login keychain", None),
            Check::new(CheckState::Pending, "Verified with a real HTTPS handshake", None),
        ];
    };
    let state = |ok: bool| if ok { CheckState::Ok } else { CheckState::Warn };
    let keychain = Check::new(
        state(trust.keychain == KeychainTrust::Trusted),
        "Trusted in the login keychain",
        match trust.keychain {
            KeychainTrust::Denied => Some("Set to “Never Trust” in Keychain Access.".into()),
            KeychainTrust::Unsupported => Some("Not available on this platform yet.".into()),
            _ => None,
        },
    );
    let handshake = Check::new(
        state(trust.verified),
        "Verified with a real HTTPS handshake",
        if trust.verified {
            None
        } else {
            trust.detail.as_deref().map(trust_detail)
        },
    );
    [keychain, handshake]
}

/// What a failed trust change says: a cancelled macOS prompt changed nothing.
pub fn trust_error(cancelled: bool, message: &str) -> String {
    if cancelled {
        "Cancelled. Nothing was changed.".into()
    } else {
        message.to_owned()
    }
}

/// The toolbar's "Certificate not trusted": while listening, once the
/// check has run, the CA isn't verified and the keychain is supported.
pub fn trust_warning(listening: bool, trust: Option<&TrustStatus>) -> bool {
    listening && trust.is_some_and(|t| !t.verified && t.keychain != KeychainTrust::Unsupported)
}

// ---- System proxy ----

/// The popover's title, e.g. "System proxy on".
pub fn system_proxy_title(status: Option<&SystemProxyStatus>) -> String {
    let on = status.is_some_and(|s| s.enabled);
    format!("System proxy {}", if on { "on" } else { "off" })
}

pub fn system_proxy_description(listening: Option<&str>) -> String {
    format!(
        "Routes Safari, Chrome and other apps that follow the macOS proxy settings through {}. Your previous \
         settings come back on quit, even after a crash.",
        listening.unwrap_or("Proxymate")
    )
}

// ---- MCP ----

/// The indicator's colour, by state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpTone {
    /// Off or not serving: muted and faded.
    Off,
    /// It should serve and can't.
    Error,
    /// Serving, nobody connected.
    Serving,
    /// Agents connected: the agent colour, with a count.
    Clients,
}

pub fn mcp_tone(status: Option<&McpStatus>) -> McpTone {
    match status {
        Some(s) if s.url.is_none() && s.error.is_some() => McpTone::Error,
        Some(s) if s.url.is_some() && !s.clients.is_empty() => McpTone::Clients,
        Some(s) if s.url.is_some() => McpTone::Serving,
        _ => McpTone::Off,
    }
}

pub fn mcp_tip(status: Option<&McpStatus>) -> String {
    let Some(s) = status else {
        return "MCP server".into();
    };
    let read_only = if s.settings.read_only { " · read-only" } else { "" };
    if !s.settings.enabled {
        "MCP server off · open Settings to turn it on".into()
    } else if let Some(url) = &s.url {
        if s.clients.is_empty() {
            format!("MCP server on {url}{read_only} · no agents connected")
        } else {
            let names: Vec<&str> = s.clients.iter().map(|c| c.name.as_str()).collect();
            format!("MCP: {}{read_only}", names.join(", "))
        }
    } else {
        match &s.error {
            Some(err) => format!("MCP server not running: {err}"),
            None => "MCP server not running".into(),
        }
    }
}

pub fn mcp_label(status: Option<&McpStatus>) -> String {
    match status.map_or(0, |s| s.clients.len()) {
        0 => "MCP server".into(),
        n => format!("MCP server: {n} connected"),
    }
}

// ---- Intercept ----

/// One entry of the Intercept menu, in `InterceptTarget::ALL`'s order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterceptEntry {
    pub target: InterceptTarget,
    /// The engine's name for the app.
    pub name: &'static str,
    /// A command-line id (`intercept-<id>`, the dev remote).
    pub id: &'static str,
    pub terminal: bool,
}

pub const INTERCEPT_TARGETS: [InterceptEntry; 7] = [
    entry(InterceptTarget::Terminal, "Terminal", "terminal", true),
    entry(InterceptTarget::Iterm, "iTerm", "iterm", true),
    entry(InterceptTarget::Chrome, "Google Chrome", "chrome", false),
    entry(InterceptTarget::Edge, "Microsoft Edge", "edge", false),
    entry(InterceptTarget::Brave, "Brave Browser", "brave", false),
    entry(InterceptTarget::Arc, "Arc", "arc", false),
    entry(InterceptTarget::Firefox, "Firefox", "firefox", false),
];

const fn entry(target: InterceptTarget, name: &'static str, id: &'static str, terminal: bool) -> InterceptEntry {
    InterceptEntry {
        target,
        name,
        id,
        terminal,
    }
}

pub fn intercept_entry(id: &str) -> Option<InterceptEntry> {
    INTERCEPT_TARGETS.iter().copied().find(|e| e.id == id)
}

pub const INTERCEPT_TIP: &str = "Intercept: a terminal or browser that goes through Proxymate";

/// The toast after a launch: "Started <name>" and what to know.
pub fn intercept_started(session: &InterceptSession) -> (String, String) {
    let description = if !session.notes.is_empty() {
        session.notes.join(" ")
    } else if session.target.is_terminal() {
        "Commands run in the new window go through Proxymate and trust its CA.".into()
    } else {
        format!(
            "Its traffic shows as {} (Intercepted), decrypted without the keychain.",
            session.name
        )
    };
    (format!("Started {}", session.name), description)
}

/// A running launch's right-hand text.
pub fn intercept_pid(session: &InterceptSession) -> String {
    match session.pid {
        Some(pid) => format!("pid {pid}"),
        None => "starting…".into(),
    }
}

#[cfg(test)]
mod tests {
    use pm_model::{McpClient, McpSettings};

    use super::*;

    #[test]
    fn fingerprints_and_the_curl_line() {
        assert_eq!(colon_hex("abcdef01"), "ab:cd:ef:01");
        assert_eq!(colon_hex("abc"), "ab:c");
        assert_eq!(
            curl_example(Some("127.0.0.1:9091"), "/ca.pem"),
            "curl -x http://127.0.0.1:9091 --cacert \"/ca.pem\" https://example.com/"
        );
        assert!(curl_example(None, "/ca.pem").starts_with("curl -x http://127.0.0.1:9090 "));
    }

    #[test]
    fn the_wizard_checks_the_keychain_then_a_handshake() {
        assert_eq!(wizard_checks(None)[0].state, CheckState::Pending);
        let untrusted = TrustStatus {
            keychain: KeychainTrust::Denied,
            verified: false,
            detail: Some("invalid peer certificate: UnknownIssuer (InvalidCertificate(UnknownIssuer))".into()),
        };
        let [keychain, handshake] = wizard_checks(Some(&untrusted));
        assert_eq!(keychain.state, CheckState::Warn);
        assert_eq!(
            keychain.detail.as_deref(),
            Some("Set to “Never Trust” in Keychain Access.")
        );
        assert_eq!(
            handshake.detail.as_deref(),
            Some("Invalid peer certificate: UnknownIssuer.")
        );
        let trusted = TrustStatus {
            keychain: KeychainTrust::Trusted,
            verified: true,
            detail: None,
        };
        assert!(wizard_checks(Some(&trusted)).iter().all(|c| c.state == CheckState::Ok));
        assert_eq!(
            trust_detail(
                "the system doesn't trust the Proxymate CA yet (Other(OtherError(\"“Proxymate CA (Mac, 2026)” is \
                 not trusted: -67843\")))"
            ),
            "The system doesn't trust the Proxymate CA yet."
        );
        assert_eq!(trust_error(true, "x"), "Cancelled. Nothing was changed.");
        assert!(trust_warning(true, Some(&untrusted)));
        assert!(!trust_warning(false, Some(&untrusted)));
        assert!(!trust_warning(true, None));
        assert!(!trust_warning(true, Some(&trusted)));
    }

    fn mcp(url: Option<&str>, clients: &[&str], error: Option<&str>) -> McpStatus {
        McpStatus {
            settings: McpSettings::default(),
            url: url.map(Into::into),
            error: error.map(Into::into),
            clients: clients
                .iter()
                .map(|n| McpClient {
                    session: (*n).into(),
                    name: (*n).into(),
                    version: "1".into(),
                    connected_at_ms: 0.0,
                })
                .collect(),
        }
    }

    #[test]
    fn the_mcp_indicator_says_its_state() {
        assert_eq!(mcp_tip(None), "MCP server");
        let url = "http://127.0.0.1:9095/mcp";
        assert_eq!(mcp_tone(Some(&mcp(Some(url), &[], None))), McpTone::Serving);
        assert_eq!(
            mcp_tip(Some(&mcp(Some(url), &[], None))),
            "MCP server on http://127.0.0.1:9095/mcp · no agents connected"
        );
        let two = mcp(Some(url), &["claude-code", "cursor"], None);
        assert_eq!(mcp_tone(Some(&two)), McpTone::Clients);
        assert_eq!(mcp_tip(Some(&two)), "MCP: claude-code, cursor");
        assert_eq!(mcp_label(Some(&two)), "MCP server: 2 connected");
        let failed = mcp(None, &[], Some("address in use"));
        assert_eq!(mcp_tone(Some(&failed)), McpTone::Error);
        assert_eq!(mcp_tip(Some(&failed)), "MCP server not running: address in use");
        let mut off = mcp(None, &[], None);
        off.settings.enabled = false;
        assert_eq!(mcp_tone(Some(&off)), McpTone::Off);
        assert_eq!(mcp_tip(Some(&off)), "MCP server off · open Settings to turn it on");
        let mut read_only = mcp(Some(url), &[], None);
        read_only.settings.read_only = true;
        assert!(mcp_tip(Some(&read_only)).contains(" · read-only · "));
    }

    #[test]
    fn intercept_targets_follow_the_engine() {
        let order: Vec<InterceptTarget> = INTERCEPT_TARGETS.iter().map(|e| e.target).collect();
        assert_eq!(order, InterceptTarget::ALL);
        for e in INTERCEPT_TARGETS {
            assert_eq!(e.terminal, e.target.is_terminal());
            // The command list's ids are `intercept-<id>`.
            assert!(
                crate::commands::get(&format!("intercept-{}", e.id)).is_some(),
                "{}",
                e.id
            );
        }
        let session = |target, notes: Vec<String>| InterceptSession {
            id: 1,
            target,
            name: "Google Chrome".into(),
            pid: None,
            started_at_ms: 0.0,
            notes,
        };
        assert_eq!(
            intercept_started(&session(InterceptTarget::Chrome, vec![])),
            (
                "Started Google Chrome".into(),
                "Its traffic shows as Google Chrome (Intercepted), decrypted without the keychain.".into()
            )
        );
        assert_eq!(
            intercept_started(&session(InterceptTarget::Firefox, vec!["A".into(), "B".into()])).1,
            "A B"
        );
        assert_eq!(intercept_pid(&session(InterceptTarget::Chrome, vec![])), "starting…");
    }
}
