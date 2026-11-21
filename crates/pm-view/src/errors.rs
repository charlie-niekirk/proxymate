//! Why a flow failed, in a line, with the fixes that apply: the port of
//! `lib/errors.ts`. It goes by `FlowDetail.error_kind` (R3: the engine
//! records the kind where the failure happens, and infers it for sessions
//! saved before format 8), never by the message's wording; the message is
//! shown as it is.

use pm_model::{ErrorKind, FlowDetail, FlowSummary, TlsStatus};

/// Something the user can do about a failed flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FixAction {
    /// The certificate wizard: the device may not trust the CA yet.
    CertWizard,
    /// Add the host to the SSL bypass list.
    Bypass,
    /// Accept (or stop accepting) the host's upstream certificate unverified.
    InsecureUpstream,
    /// Settings › Advanced (the timeouts).
    Timeouts,
    /// Settings › SSL Proxying.
    TlsSettings,
    /// Settings › Network (upstream proxy, client certificates).
    NetworkSettings,
}

impl FixAction {
    /// The Settings page a fix opens, if it opens one.
    pub fn settings_page(self) -> Option<&'static str> {
        match self {
            FixAction::Timeouts => Some("advanced"),
            FixAction::TlsSettings => Some("ssl"),
            FixAction::NetworkSettings => Some("network"),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureHelp {
    /// One line on what went wrong.
    pub title: String,
    /// The engine's own message, which names the host and the detail.
    pub detail: String,
    pub actions: Vec<FixAction>,
}

/// The headline and fixes for a failed flow; `None` when it didn't fail.
pub fn explain_failure(flow: &FlowSummary, detail: Option<&FlowDetail>) -> Option<FailureHelp> {
    let detail = detail?;
    explain(
        flow.tls,
        &flow.host,
        flow.port,
        detail.error.as_deref()?,
        detail.error_kind,
    )
}

/// [`explain_failure`] from its parts.
pub fn explain(tls: TlsStatus, host: &str, port: u16, error: &str, kind: Option<ErrorKind>) -> Option<FailureHelp> {
    use FixAction::*;
    let help = |title: String, actions: &[FixAction]| FailureHelp {
        title,
        detail: error.to_owned(),
        actions: actions.to_vec(),
    };
    let with_text = |title: String, actions: &[FixAction], text: String| FailureHelp {
        title,
        detail: text,
        actions: actions.to_vec(),
    };
    let refused_cert = || {
        help(
            "The client refused Proxymate's certificate".into(),
            &[CertWizard, Bypass],
        )
    };
    if tls == TlsStatus::LikelyPinned {
        return Some(refused_cert());
    }
    Some(match kind.unwrap_or(ErrorKind::Other) {
        ErrorKind::ClientRejectedCert => refused_cert(),
        ErrorKind::ClientTls => help("The TLS handshake with the client failed".into(), &[CertWizard, Bypass]),
        ErrorKind::ClientCertRequired => help("The server wants a client certificate".into(), &[NetworkSettings]),
        ErrorKind::UpstreamProxyAuth => help("The upstream proxy wants other credentials".into(), &[NetworkSettings]),
        ErrorKind::UpstreamProxyLoop => help("The upstream proxy is Proxymate itself".into(), &[NetworkSettings]),
        ErrorKind::UpstreamProxy => help("The upstream proxy couldn't connect".into(), &[NetworkSettings]),
        ErrorKind::UpstreamCertUntrusted => help(
            "The server's certificate isn't trusted".into(),
            &[InsecureUpstream, Bypass],
        ),
        ErrorKind::UpstreamCertInvalid => help(
            "The server's certificate is invalid".into(),
            &[InsecureUpstream, Bypass],
        ),
        ErrorKind::UpstreamTlsVersion => with_text(
            "The server only speaks an old TLS version".into(),
            &[Bypass],
            format!(
                "{error} Proxymate supports TLS 1.2 and 1.3; add the host to the SSL bypass list to tunnel it instead."
            ),
        ),
        ErrorKind::UpstreamTls => help(
            "The TLS handshake with the server failed".into(),
            &[Bypass, TlsSettings],
        ),
        ErrorKind::Dns => with_text(
            format!("Couldn't find {host}"),
            &[],
            format!("{error} Check the host name and this Mac's DNS."),
        ),
        ErrorKind::Timeout => help("The server didn't answer in time".into(), &[Timeouts]),
        ErrorKind::Refused => with_text(
            format!("Nothing answered at {host}:{port}"),
            &[],
            format!("{error} Is the server running?"),
        ),
        ErrorKind::ClientGone => help("The client gave up".into(), &[]),
        ErrorKind::Upstream | ErrorKind::Other => help("The flow failed".into(), &[]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use FixAction::*;

    const HOST: &str = "api.example.com";

    fn explain_as(tls: TlsStatus, error: Option<&str>, kind: Option<ErrorKind>) -> Option<FailureHelp> {
        explain(tls, HOST, 443, error?, kind)
    }

    #[test]
    fn offers_the_wizard_and_the_bypass_for_a_pinning_client() {
        let help = explain_as(
            TlsStatus::LikelyPinned,
            Some("The client rejected the Proxymate certificate. It sent an unknown_ca alert."),
            Some(ErrorKind::ClientRejectedCert),
        )
        .unwrap();
        assert_eq!(help.actions, [CertWizard, Bypass]);
        // A pinned flow is pinned whatever kind the engine gave it.
        let other = explain_as(TlsStatus::LikelyPinned, Some("x"), Some(ErrorKind::Timeout)).unwrap();
        assert_eq!(other.title, "The client refused Proxymate's certificate");
    }

    #[test]
    fn offers_insecure_upstream_for_an_untrusted_server_certificate() {
        let help = explain_as(
            TlsStatus::Intercepted,
            Some("api.example.com's certificate isn't trusted: it is self-signed"),
            Some(ErrorKind::UpstreamCertUntrusted),
        )
        .unwrap();
        assert!(help.title.contains("isn't trusted"));
        assert!(help.actions.contains(&InsecureUpstream));
    }

    #[test]
    fn points_client_certificate_and_upstream_proxy_failures_at_settings_network() {
        let cert = explain_as(
            TlsStatus::Intercepted,
            Some("api.example.com asks for a client certificate. Add one…"),
            Some(ErrorKind::ClientCertRequired),
        )
        .unwrap();
        assert_eq!(cert.title, "The server wants a client certificate");
        assert_eq!(cert.actions, [NetworkSettings]);
        let auth = explain_as(
            TlsStatus::Intercepted,
            Some("the upstream proxy http://127.0.0.1:3128 rejected the username and password (407)."),
            Some(ErrorKind::UpstreamProxyAuth),
        )
        .unwrap();
        assert_eq!(auth.title, "The upstream proxy wants other credentials");
        let unreachable = explain_as(
            TlsStatus::None,
            Some("the upstream proxy http://p:1: can't reach p:1: Connection refused"),
            Some(ErrorKind::UpstreamProxy),
        )
        .unwrap();
        assert_eq!(unreachable.title, "The upstream proxy couldn't connect");
        assert_eq!(unreachable.actions, [NetworkSettings]);
        assert_eq!(NetworkSettings.settings_page(), Some("network"));
    }

    #[test]
    fn names_dns_refused_connections_and_timeouts() {
        let dns = explain_as(
            TlsStatus::None,
            Some("can't resolve nope.test: failed to lookup address"),
            Some(ErrorKind::Dns),
        )
        .unwrap();
        assert_eq!(dns.title, "Couldn't find api.example.com");
        assert!(dns.detail.ends_with("Check the host name and this Mac's DNS."));
        let refused = explain_as(
            TlsStatus::None,
            Some("can't reach api.example.com:443: Connection refused"),
            Some(ErrorKind::Refused),
        )
        .unwrap();
        assert_eq!(refused.title, "Nothing answered at api.example.com:443");
        let timeout = explain_as(
            TlsStatus::None,
            Some("connecting to api.example.com:443 timed out"),
            Some(ErrorKind::Timeout),
        )
        .unwrap();
        assert_eq!(timeout.actions, [Timeouts]);
        assert_eq!(Timeouts.settings_page(), Some("advanced"));
        assert_eq!(explain_as(TlsStatus::None, None, None), None);
    }

    #[test]
    fn goes_by_the_kind_not_the_wording() {
        // A message that reads like a timeout but isn't one.
        let help = explain_as(
            TlsStatus::None,
            Some("the upstream said: request timed out"),
            Some(ErrorKind::Upstream),
        )
        .unwrap();
        assert_eq!(help.title, "The flow failed");
        // No kind (a detail from before the engine recorded kinds): generic.
        let old = explain_as(TlsStatus::None, Some("connecting to x timed out"), None).unwrap();
        assert_eq!(old.title, "The flow failed");
    }

    #[test]
    fn every_kind_has_a_headline() {
        let kinds = [
            (
                ErrorKind::ClientRejectedCert,
                "The client refused Proxymate's certificate",
            ),
            (ErrorKind::ClientTls, "The TLS handshake with the client failed"),
            (ErrorKind::ClientGone, "The client gave up"),
            (ErrorKind::UpstreamProxyLoop, "The upstream proxy is Proxymate itself"),
            (ErrorKind::UpstreamCertInvalid, "The server's certificate is invalid"),
            (
                ErrorKind::UpstreamTlsVersion,
                "The server only speaks an old TLS version",
            ),
            (ErrorKind::UpstreamTls, "The TLS handshake with the server failed"),
        ];
        for (kind, title) in kinds {
            let help = explain_as(TlsStatus::Intercepted, Some("boom."), Some(kind)).unwrap();
            assert_eq!(help.title, title, "{kind:?}");
        }
        let old_tls = explain_as(
            TlsStatus::Intercepted,
            Some("boom."),
            Some(ErrorKind::UpstreamTlsVersion),
        )
        .unwrap();
        assert!(old_tls.detail.starts_with("boom. Proxymate supports TLS 1.2 and 1.3"));
        assert_eq!(
            explain_as(TlsStatus::Intercepted, Some("x"), Some(ErrorKind::UpstreamTls))
                .unwrap()
                .actions,
            [Bypass, TlsSettings]
        );
    }
}
