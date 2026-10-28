//! The body-viewer "zoo": one fixture per `ContentKind`, listed in
//! `fixtures/zoo/zoo.json` and served by [`crate::Upstream`] at
//! `/zoo/<name>`. The UI's mock backend serves the same files.

use std::net::SocketAddr;
use std::sync::OnceLock;

use bytes::Bytes;
use http::Request;
use http_body_util::{BodyExt, Full};
use serde::Deserialize;

const MANIFEST: &str = include_str!("../fixtures/zoo/zoo.json");

/// Size of the generated `big-json` body.
pub const BIG_JSON_LEN: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct Fixture {
    pub name: String,
    /// Response body file in `fixtures/zoo/`.
    #[serde(default)]
    pub file: Option<String>,
    /// Generated response body instead of a file (`big-json`).
    #[serde(default)]
    pub generate: Option<String>,
    /// Response `Content-Type`; none is sent if absent.
    #[serde(rename = "type", default)]
    pub content_type: Option<String>,
    /// Response `Content-Encoding` applied to the file.
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub request: Option<RequestFixture>,
    #[serde(default = "default_status")]
    pub status: u16,
    /// The flow's expected `ContentKind`, as serialized.
    pub kind: String,
    #[serde(default)]
    pub graphql_op: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RequestFixture {
    pub file: String,
    #[serde(rename = "type")]
    pub content_type: String,
}

#[derive(Deserialize)]
struct Manifest {
    fixtures: Vec<Fixture>,
}

fn default_method() -> String {
    "GET".into()
}

fn default_status() -> u16 {
    200
}

/// Every fixture, in manifest order.
pub fn fixtures() -> &'static [Fixture] {
    static FIXTURES: OnceLock<Vec<Fixture>> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        serde_json::from_str::<Manifest>(MANIFEST)
            .expect("zoo.json is valid")
            .fixtures
    })
}

pub fn fixture(name: &str) -> Option<&'static Fixture> {
    fixtures().iter().find(|f| f.name == name)
}

impl Fixture {
    /// The response body as sent (content-encoded if `encoding` is set).
    pub fn response_body(&self) -> Vec<u8> {
        let plain = match (&self.file, self.generate.as_deref()) {
            (Some(file), _) => file_bytes(file).to_vec(),
            (None, Some("big-json")) => big_json(BIG_JSON_LEN),
            _ => Vec::new(),
        };
        match &self.encoding {
            Some(coding) => crate::upstream::encode(coding, &plain),
            None => plain,
        }
    }

    pub fn request_body(&self) -> Option<&'static [u8]> {
        self.request.as_ref().map(|r| file_bytes(&r.file))
    }

    /// `/zoo/<name>`.
    pub fn path(&self) -> String {
        format!("/zoo/{}", self.name)
    }
}

/// The bytes of a file in `fixtures/zoo/`.
pub fn file_bytes(name: &str) -> &'static [u8] {
    macro_rules! files {
        ($($f:literal),* $(,)?) => {
            match name {
                $($f => include_bytes!(concat!("../fixtures/zoo/", $f)),)*
                other => panic!("no zoo file {other}"),
            }
        };
    }
    files!(
        "animated.gif",
        "app.min.js",
        "base64-field.json",
        "clip.mp4",
        "document.pdf",
        "favicon.ico",
        "feed.xml",
        "font.woff2",
        "form.txt",
        "graphql-request.json",
        "graphql-response.json",
        "grpc-web-request.bin",
        "grpc-web-response.bin",
        "image.avif",
        "image.bmp",
        "image.png",
        "image.webp",
        "logo.svg",
        "nested.json",
        "notes.txt",
        "ok.json",
        "order.pb",
        "page.html",
        "photo.jpg",
        "profile.cbor",
        "profile.msgpack",
        "random.bin",
        "styles.min.css",
        "tone.mp3",
        "upload.multipart",
    )
}

/// A deterministic JSON document of at least `min_len` bytes: an object
/// with an `items` array of small records.
pub fn big_json(min_len: usize) -> Vec<u8> {
    let mut out = String::with_capacity(min_len + 256);
    out.push_str(r#"{"generated":"proxymate zoo","items":["#);
    let mut i: u64 = 0;
    while out.len() < min_len {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            r#"{{"id":{i},"name":"item-{i}","active":{},"price":{}.{:02},"tags":["t{}","t{}"],"owner":{{"id":{},"login":"user{}"}},"note":null}}"#,
            i.is_multiple_of(3),
            i % 1000,
            i % 100,
            i % 7,
            i % 11,
            i % 5000,
            i % 5000,
        ));
        i += 1;
    }
    out.push_str(&format!(r#"],"count":{i}}}"#));
    out.into_bytes()
}

/// Sends every fixture's request through the proxy at `proxy` to the
/// upstream at `upstream`; returns `(name, status)` per fixture.
pub async fn send_all(proxy: SocketAddr, upstream: SocketAddr) -> Vec<(String, u16)> {
    let mut results = Vec::new();
    for fixture in fixtures() {
        let url = format!("http://{upstream}{}", fixture.path());
        let mut builder = Request::builder().method(fixture.method.as_str()).uri(&url);
        if let Some(request) = &fixture.request {
            builder = builder.header(http::header::CONTENT_TYPE, &request.content_type);
        }
        let body = fixture.request_body().map(Bytes::from_static).unwrap_or_default();
        let req = builder.body(Full::new(body)).expect("valid zoo request");
        let response = crate::client::send(proxy, req).await;
        let status = response.status().as_u16();
        let _ = response.into_body().collect().await;
        results.push((fixture.name.clone(), status));
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixture_has_a_body() {
        for f in fixtures() {
            assert!(!f.response_body().is_empty(), "{}", f.name);
            if let Some(request) = &f.request {
                assert!(!file_bytes(&request.file).is_empty(), "{}", f.name);
            }
        }
    }

    #[test]
    fn big_json_is_valid_and_large() {
        let body = big_json(BIG_JSON_LEN);
        assert!(body.len() >= BIG_JSON_LEN);
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(value["items"].as_array().unwrap().len() > 10_000);
    }
}

/// The binary fixtures (M16) are generated: `UPDATE_ZOO=1 cargo test -p
/// pm-testkit zoo` rewrites them, otherwise the test checks they're current.
#[cfg(test)]
mod binary_fixtures {
    use prost::Message;

    use crate::grpc::{self, order, shop};

    fn order_pb() -> Vec<u8> {
        order("o-1001").encode_to_vec()
    }

    /// A gRPC-web exchange: the response's last frame holds the trailers.
    fn grpc_web() -> (Vec<u8>, Vec<u8>) {
        let request = grpc::frame(&shop::GetOrderRequest { id: "o-1001".into() }).to_vec();
        let mut response = grpc::frame(&order("o-1001")).to_vec();
        let trailers = b"grpc-status:0\r\ngrpc-message:\r\n";
        response.push(0x80);
        response.extend_from_slice(&(trailers.len() as u32).to_be_bytes());
        response.extend_from_slice(trailers);
        (request, response)
    }

    fn msgpack() -> Vec<u8> {
        use rmpv::Value as V;
        let value = V::Map(vec![
            ("id".into(), V::from(4012)),
            ("user".into(), "ada".into()),
            ("roles".into(), V::Array(vec!["admin".into(), "ops".into()])),
            ("active".into(), V::Boolean(true)),
            ("score".into(), V::F64(98.5)),
            ("big".into(), V::from(u64::MAX)),
            (
                "avatar".into(),
                V::Binary(vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
            ),
            ("seen_at".into(), V::Ext(-1, 1_700_000_000u32.to_be_bytes().to_vec())),
            (V::from(7), "an int key".into()),
            ("meta".into(), V::Nil),
        ]);
        let mut out = Vec::new();
        rmpv::encode::write_value(&mut out, &value).expect("encode");
        out
    }

    fn cbor() -> Vec<u8> {
        use ciborium::Value as V;
        let value = V::Map(vec![
            ("id".into(), V::Integer(4012.into())),
            ("name".into(), V::Text("ada".into())),
            ("created".into(), V::Tag(1, Box::new(V::Integer(1_700_000_000.into())))),
            ("uuid".into(), V::Tag(37, Box::new(V::Bytes((0u8..16).collect())))),
            ("blob".into(), V::Bytes(vec![0xca, 0xfe])),
            (
                "nested".into(),
                V::Map(vec![("ok".into(), V::Bool(true)), ("ratio".into(), V::Float(0.25))]),
            ),
            (V::Integer(1.into()), V::Text("an int key".into())),
        ]);
        let mut out = Vec::new();
        ciborium::ser::into_writer(&value, &mut out).expect("encode");
        out
    }

    #[test]
    fn are_current() {
        let (request, response) = grpc_web();
        let files = [
            ("order.pb", order_pb()),
            ("grpc-web-request.bin", request),
            ("grpc-web-response.bin", response),
            ("profile.msgpack", msgpack()),
            ("profile.cbor", cbor()),
        ];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/zoo");
        for (name, bytes) in files {
            if std::env::var_os("UPDATE_ZOO").is_some() {
                std::fs::write(dir.join(name), &bytes).expect("write fixture");
            } else {
                assert_eq!(
                    super::file_bytes(name),
                    bytes.as_slice(),
                    "{name} is stale: UPDATE_ZOO=1"
                );
            }
        }
    }
}
