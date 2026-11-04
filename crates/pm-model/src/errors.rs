//! Stable error kinds (R3): why a flow failed, and why a command failed.
//! The UI switches on these kinds; the messages are for people and may
//! change wording at any time.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Why a flow failed, as recorded with its error message
/// (`FlowDetail.error_kind`). Stored in sessions by position: add variants
/// at the end only (and pin them in `store/sqlite/v7_fixture.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Anything without a more specific kind.
    Other,
    /// The client refused Proxymate's certificate: it doesn't trust the CA
    /// yet, or it pins its certificates.
    ClientRejectedCert,
    /// The TLS handshake with the client failed for another reason.
    ClientTls,
    /// The client closed the connection or went away before the flow ended.
    ClientGone,
    /// The upstream's host name didn't resolve.
    Dns,
    /// Nothing answered at the upstream's address (refused, unreachable).
    Refused,
    /// Connecting, resolving or a TLS handshake took too long.
    Timeout,
    /// The upstream server asks for a client certificate.
    ClientCertRequired,
    /// The upstream's certificate isn't trusted (self-signed, unknown CA).
    UpstreamCertUntrusted,
    /// The upstream's certificate is expired, for another name, or rejected.
    UpstreamCertInvalid,
    /// The upstream only speaks TLS versions or ciphers we don't.
    UpstreamTlsVersion,
    /// The TLS handshake with the upstream failed for another reason.
    UpstreamTls,
    /// An upstream proxy refused or failed.
    UpstreamProxy,
    /// An upstream proxy wants (other) credentials.
    UpstreamProxyAuth,
    /// The upstream proxy is Proxymate itself, which would loop.
    UpstreamProxyLoop,
    /// The upstream failed after connecting: a reset, a bad response, a body
    /// that broke off.
    Upstream,
}

/// Why a command failed: a stable `kind` to switch on and a `message` to
/// show. Every Tauri command's error is one of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct IpcError {
    pub kind: IpcErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum IpcErrorKind {
    /// No such view, flow, message or open document.
    NotFound,
    /// A setting, rule or argument that isn't valid, with what is allowed.
    Invalid,
    /// A filter query or search pattern that doesn't parse.
    Query,
    /// A rule that doesn't compile, or a rule store failure.
    Rule,
    /// A script that doesn't evaluate, or a script store failure.
    Script,
    /// A protobuf schema or mapping that doesn't load.
    Proto,
    /// A body that doesn't decode.
    Decode,
    /// Reading or writing flow storage or a file.
    Storage,
    /// Certificate authority and TLS failures.
    Tls,
    /// The proxy couldn't start or isn't running.
    Proxy,
    /// An OS integration failed (keychain, system proxy, a system tool).
    Platform,
    /// The user dismissed a system prompt.
    Cancelled,
    /// Not supported on this platform yet.
    Unsupported,
    /// Device setup: a bad address, a missing tool.
    Device,
    /// A request that can't be sent as given.
    Send,
    /// Saving, opening or importing a session, HAR or Charles file.
    Document,
    /// A WebSocket that can't take a message or a close.
    WebSocket,
    /// Anything else: a dialog that failed, a command that panicked.
    Internal,
}

impl IpcError {
    pub fn new(kind: IpcErrorKind, message: impl Into<String>) -> IpcError {
        IpcError {
            kind,
            message: message.into(),
        }
    }

    /// A failure outside the engine (a dialog, a panicked task).
    pub fn internal(message: impl std::fmt::Display) -> IpcError {
        IpcError::new(IpcErrorKind::Internal, message.to_string())
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for IpcError {}
