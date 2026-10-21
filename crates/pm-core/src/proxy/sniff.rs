//! First-bytes sniffing: peek at a new connection to decide how to serve
//! it before any byte is consumed.

use std::time::Duration;

use tokio::net::TcpStream;

/// Enough for a method, a space and `https://`.
const PEEK_LEN: usize = 24;
/// A client that connects and says nothing is dropped after this long.
const FIRST_BYTES_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    /// `CONNECT host:port`: a tunnel request.
    Connect,
    /// `GET http://host/path`: a forward-proxy request.
    AbsoluteForm,
    /// `GET /path`: a request addressed to the proxy itself.
    OriginForm,
    /// A TLS ClientHello: a client talking HTTPS straight at the proxy port.
    Tls,
    /// Not HTTP.
    Garbage,
}

impl Sniffed {
    pub fn is_http(self) -> bool {
        matches!(self, Sniffed::Connect | Sniffed::AbsoluteForm | Sniffed::OriginForm)
    }
}

/// Waits for the first bytes of `stream` and classifies them without
/// consuming anything. `None` if the client closed or stayed silent.
pub async fn sniff(stream: &TcpStream) -> Option<Sniffed> {
    let mut buf = [0u8; PEEK_LEN];
    let n = tokio::time::timeout(FIRST_BYTES_TIMEOUT, stream.peek(&mut buf))
        .await
        .ok()?
        .ok()?;
    (n > 0).then(|| classify(&buf[..n]))
}

/// Classifies the start of a connection. A request line split across
/// packets may be cut short; anything that still looks like a method is
/// treated as HTTP and left for the parser to judge.
pub fn classify(bytes: &[u8]) -> Sniffed {
    if bytes.first() == Some(&0x16) {
        return Sniffed::Tls;
    }
    let method_len = bytes.iter().take_while(|b| is_token(**b)).count();
    if method_len == 0 {
        return Sniffed::Garbage;
    }
    let Some(rest) = bytes[method_len..].strip_prefix(b" ") else {
        // Either the whole peek is a method-like token, or a token followed
        // by something other than a space.
        return if method_len == bytes.len() {
            Sniffed::OriginForm
        } else {
            Sniffed::Garbage
        };
    };
    let method = &bytes[..method_len];
    if method == b"CONNECT" {
        Sniffed::Connect
    } else if starts_with_ignore_case(rest, b"http://") || starts_with_ignore_case(rest, b"https://") {
        Sniffed::AbsoluteForm
    } else {
        Sniffed::OriginForm
    }
}

/// RFC 9110 token characters, which is what a method is made of.
fn is_token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

fn starts_with_ignore_case(bytes: &[u8], prefix: &[u8]) -> bool {
    bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_request_lines() {
        assert_eq!(classify(b"GET http://example.com/ HTTP/1.1\r\n"), Sniffed::AbsoluteForm);
        assert_eq!(classify(b"POST HTTPS://example.com/"), Sniffed::AbsoluteForm);
        assert_eq!(classify(b"CONNECT example.com:443 HTTP/1.1"), Sniffed::Connect);
        assert_eq!(classify(b"GET / HTTP/1.1\r\n"), Sniffed::OriginForm);
        assert_eq!(classify(b"OPTIONS * HTTP/1.1"), Sniffed::OriginForm);
        assert_eq!(classify(b"GE"), Sniffed::OriginForm);
    }

    #[test]
    fn rejects_tls_and_noise() {
        assert_eq!(classify(&[0x16, 0x03, 0x01, 0x02, 0x00]), Sniffed::Tls);
        assert_eq!(classify(b"\x00\x01binary"), Sniffed::Garbage);
        assert_eq!(classify(b"GET\r\n"), Sniffed::Garbage);
        assert!(!Sniffed::Tls.is_http());
    }
}
