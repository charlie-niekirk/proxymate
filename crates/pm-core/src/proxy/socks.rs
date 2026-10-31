//! The SOCKS5 listener (M18), on the proxy's own port: `sniff.rs` sends a
//! leading `0x05` here. No authentication, CONNECT only, IPv4, IPv6 and
//! domain targets (RFC 1928). The connection was admitted like any other
//! (the LAN allow-list applies), and after the handshake it takes the
//! CONNECT path: the SSL lists decide between the MITM and a passthrough
//! tunnel, plain HTTP is served as HTTP, and anything else is relayed as a
//! recorded tunnel.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use pm_model::{HttpVersion, RequestHead};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::ProxyCtx;
use super::mitm::{self, Decision, Target};
use crate::client::PeerClient;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

const VERSION: u8 = 0x05;
const NO_AUTH: u8 = 0x00;
const NO_ACCEPTABLE_METHOD: u8 = 0xFF;
const CONNECT: u8 = 0x01;
const SUCCEEDED: u8 = 0x00;
const NOT_ALLOWED: u8 = 0x02;
const COMMAND_NOT_SUPPORTED: u8 = 0x07;
const ADDRESS_NOT_SUPPORTED: u8 = 0x08;

/// A LAN client the user didn't allow: no method is acceptable.
pub(super) async fn refuse(mut stream: TcpStream) {
    let mut greeting = [0u8; 2];
    let _ = tokio::time::timeout(Duration::from_millis(200), stream.read_exact(&mut greeting)).await;
    let _ = stream.write_all(&[VERSION, NO_ACCEPTABLE_METHOD]).await;
}

pub(super) async fn serve(ctx: Arc<ProxyCtx>, peer: Arc<PeerClient>, mut stream: TcpStream) {
    let Ok(Ok(Some((host, port)))) = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&mut stream)).await else {
        return;
    };
    let _ = stream.set_nodelay(true);
    let Some(engine) = ctx.engine.upgrade() else {
        return;
    };
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let target = Target {
        host,
        port,
        head: RequestHead {
            method: "CONNECT".into(),
            url: authority,
            version: HttpVersion::Http11,
            headers: Vec::new(),
        },
        peer,
    };
    match mitm::decide(&engine, &target).await {
        Decision::Probe => {
            if reply(&mut stream, SUCCEEDED).await {
                mitm::trust_probe(&engine.tls(), stream).await;
            }
        }
        Decision::Onboarding => {
            reply(&mut stream, NOT_ALLOWED).await;
        }
        Decision::Passthrough => {
            let recorder = mitm::record_connect(&engine, &target, pm_model::TlsStatus::Passthrough, None);
            match mitm::open_tunnel(&engine, &target, recorder.as_ref()).await {
                Ok(upstream) => {
                    drop(engine);
                    if reply(&mut stream, SUCCEEDED).await {
                        mitm::relay(recorder, stream, upstream).await;
                    } else if let Some(recorder) = &recorder {
                        recorder.fail("the client went away before the tunnel opened");
                    }
                }
                Err(err) => {
                    reply(&mut stream, err.socks_reply()).await;
                }
            }
        }
        Decision::Intercept => {
            drop(engine);
            if reply(&mut stream, SUCCEEDED).await {
                mitm::intercept(ctx, stream, target, true).await;
            }
        }
    }
}

/// Reads the greeting and the request; answers what it can't serve.
/// `Ok(None)`: refused, already answered.
async fn handshake(stream: &mut TcpStream) -> std::io::Result<Option<(String, u16)>> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await?;
    let mut methods = vec![0u8; usize::from(head[1])];
    stream.read_exact(&mut methods).await?;
    if head[0] != VERSION || !methods.contains(&NO_AUTH) {
        stream.write_all(&[VERSION, NO_ACCEPTABLE_METHOD]).await?;
        return Ok(None);
    }
    stream.write_all(&[VERSION, NO_AUTH]).await?;
    let mut request = [0u8; 4];
    stream.read_exact(&mut request).await?;
    if request[0] != VERSION {
        return Ok(None);
    }
    let host = match request[3] {
        0x01 => {
            let mut ip = [0u8; 4];
            stream.read_exact(&mut ip).await?;
            Ipv4Addr::from(ip).to_string()
        }
        0x04 => {
            let mut ip = [0u8; 16];
            stream.read_exact(&mut ip).await?;
            Ipv6Addr::from(ip).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut name = vec![0u8; usize::from(len[0])];
            stream.read_exact(&mut name).await?;
            match String::from_utf8(name) {
                Ok(name) if !name.is_empty() => name,
                _ => {
                    reply(stream, ADDRESS_NOT_SUPPORTED).await;
                    return Ok(None);
                }
            }
        }
        _ => {
            reply(stream, ADDRESS_NOT_SUPPORTED).await;
            return Ok(None);
        }
    };
    let mut port = [0u8; 2];
    stream.read_exact(&mut port).await?;
    if request[1] != CONNECT {
        reply(stream, COMMAND_NOT_SUPPORTED).await;
        return Ok(None);
    }
    Ok(Some((host, u16::from_be_bytes(port))))
}

/// Sends a reply with an all-zero bound address; false if the client is gone.
async fn reply(stream: &mut TcpStream, code: u8) -> bool {
    stream
        .write_all(&[VERSION, code, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
        .is_ok()
}
