//! Why a flow failed (R3): a stable [`ErrorKind`] recorded beside the
//! message, decided where the failure happens (the dialer, the upstream TLS
//! handshake, the client side) from typed errors rather than read back from
//! the text. The UI switches on the kind; the message is for people.
//!
//! [`legacy_kind`] is the one place that still reads messages: sessions
//! saved before format 8 and imported HAR and Charles files carry only the
//! text.

use std::error::Error as StdError;

use pm_model::ErrorKind;

use crate::proxy::DialError;

/// A flow failure: its kind and the message the inspector shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub kind: ErrorKind,
    pub message: String,
}

impl Failure {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Failure {
        Failure {
            kind,
            message: message.into(),
        }
    }

    /// `message` with the kind `err`'s source chain shows, else `fallback`.
    pub fn from_chain(err: &(dyn StdError + 'static), fallback: ErrorKind, message: impl Into<String>) -> Failure {
        Failure::new(kind_of(err).unwrap_or(fallback), message)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A failure of our own making with nothing more specific to say (a bad
/// URL, shutting down).
impl From<String> for Failure {
    fn from(message: String) -> Failure {
        Failure::new(ErrorKind::Other, message)
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Failure {
        Failure::new(ErrorKind::Other, message)
    }
}

impl From<DialError> for Failure {
    fn from(err: DialError) -> Failure {
        Failure::new(err.flow, err.message)
    }
}

/// The kind the typed errors in `err`'s source chain show: a dial error, a
/// TLS alert, an I/O or hyper timeout, a refused connection.
pub(crate) fn kind_of(err: &(dyn StdError + 'static)) -> Option<ErrorKind> {
    let mut next = Some(err);
    while let Some(err) = next {
        if let Some(dial) = err.downcast_ref::<DialError>() {
            return Some(dial.flow);
        }
        if let Some(tls) = err.downcast_ref::<rustls::Error>()
            && let Some(kind) = tls_kind(tls)
        {
            return Some(kind);
        }
        if let Some(hyper) = err.downcast_ref::<hyper::Error>()
            && hyper.is_timeout()
        {
            return Some(ErrorKind::Timeout);
        }
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            // `io::Error::source` skips the error it wraps.
            if let Some(kind) = io.get_ref().and_then(|inner| kind_of(inner)) {
                return Some(kind);
            }
            match io.kind() {
                std::io::ErrorKind::TimedOut => return Some(ErrorKind::Timeout),
                std::io::ErrorKind::ConnectionRefused => return Some(ErrorKind::Refused),
                _ => {}
            }
        }
        next = err.source();
    }
    None
}

/// TLS errors that say more than "the handshake failed".
fn tls_kind(err: &rustls::Error) -> Option<ErrorKind> {
    match err {
        rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRequired) => {
            Some(ErrorKind::ClientCertRequired)
        }
        _ => None,
    }
}

/// The kind of a failure known only by its message: what the inspector
/// used to infer from the text before format 8 recorded kinds. Only for
/// sessions saved before then and imported HAR and Charles files.
pub(crate) fn legacy_kind(message: &str) -> ErrorKind {
    let e = message.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| e.contains(n));
    if has(&["rejected the proxymate certificate"]) {
        ErrorKind::ClientRejectedCert
    } else if has(&["asks for a client certificate"]) {
        ErrorKind::ClientCertRequired
    } else if has(&["upstream proxy", "socks5 proxy"]) {
        if has(&["username and password"]) {
            ErrorKind::UpstreamProxyAuth
        } else if has(&["proxymate itself"]) {
            ErrorKind::UpstreamProxyLoop
        } else {
            ErrorKind::UpstreamProxy
        }
    } else if has(&["certificate isn't trusted", "unknownissuer", "self-signed"]) {
        ErrorKind::UpstreamCertUntrusted
    } else if has(&["certificate has expired", "isn't valid for that name"]) {
        ErrorKind::UpstreamCertInvalid
    } else if has(&["protocol version", "tls 1.0", "tlsv1.0", "handshake failure"]) {
        ErrorKind::UpstreamTlsVersion
    } else if has(&["tls handshake with the client"]) {
        ErrorKind::ClientTls
    } else if has(&["tls handshake with"]) {
        ErrorKind::UpstreamTls
    } else if has(&["can't resolve", "failed to lookup", "nodename nor servname"]) {
        ErrorKind::Dns
    } else if has(&["timed out"]) {
        ErrorKind::Timeout
    } else if has(&["connection refused", "can't reach"]) {
        ErrorKind::Refused
    } else if has(&["client went away", "client closed"]) {
        ErrorKind::ClientGone
    } else {
        ErrorKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_errors_give_their_kind() {
        let refused = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Connection refused");
        assert_eq!(kind_of(&refused), Some(ErrorKind::Refused));
        let wrapped = std::io::Error::other(std::io::Error::new(std::io::ErrorKind::TimedOut, "slow"));
        assert_eq!(kind_of(&wrapped), Some(ErrorKind::Timeout));
        let alert = std::io::Error::other(rustls::Error::AlertReceived(
            rustls::AlertDescription::CertificateRequired,
        ));
        assert_eq!(kind_of(&alert), Some(ErrorKind::ClientCertRequired));
        assert_eq!(kind_of(&std::io::Error::other("nothing typed")), None);
    }

    /// The messages Proxymate wrote before format 8, and what they meant.
    #[test]
    fn legacy_messages_map_to_kinds() {
        for (message, kind) in [
            (
                "The client rejected the Proxymate certificate. It sent an unknown_ca alert.",
                ErrorKind::ClientRejectedCert,
            ),
            (
                "api.test asks for a client certificate. Add one for api.test in Settings › Network › Client Certificates.",
                ErrorKind::ClientCertRequired,
            ),
            (
                "the upstream proxy corp:3128 asks for a username and password (407).",
                ErrorKind::UpstreamProxyAuth,
            ),
            (
                "the upstream proxy 127.0.0.1:9090 is Proxymate itself, which would loop.",
                ErrorKind::UpstreamProxyLoop,
            ),
            ("the SOCKS5 proxy s:1080 failed: reset", ErrorKind::UpstreamProxy),
            (
                "self.test's certificate isn't trusted: it is self-signed",
                ErrorKind::UpstreamCertUntrusted,
            ),
            ("old.test's certificate has expired.", ErrorKind::UpstreamCertInvalid),
            (
                "old.test only offers TLS versions or ciphers Proxymate doesn't support (TLS 1.0 and 1.1 are not supported)",
                ErrorKind::UpstreamTlsVersion,
            ),
            ("TLS handshake with the client failed: eof", ErrorKind::ClientTls),
            ("The TLS handshake with x.test timed out.", ErrorKind::UpstreamTls),
            ("can't resolve nope.test: failed to lookup address", ErrorKind::Dns),
            ("connecting to x.test:443 timed out", ErrorKind::Timeout),
            ("can't reach 127.0.0.1:1: Connection refused", ErrorKind::Refused),
            (
                "the client closed the connection before the response finished",
                ErrorKind::ClientGone,
            ),
            ("upstream reset the stream after the response", ErrorKind::Other),
            ("net::ERR_NAME_NOT_RESOLVED", ErrorKind::Other),
        ] {
            assert_eq!(legacy_kind(message), kind, "{message}");
        }
    }
}
