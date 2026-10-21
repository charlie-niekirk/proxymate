//! The `pmbody://` URI scheme: `pmbody://localhost/<store>/<flow>/<part>`
//! (`http://pmbody.localhost/…` on Windows) serves a body's bytes with
//! their real Content-Type, so images, media, PDFs and fonts load straight
//! into the webview with no base64 and no IPC JSON.
//!
//! Query: `decoded=1` undoes the Content-Encoding; `max=N` serves at most
//! the first N bytes. Single `Range` requests are answered with 206, which
//! WebKit needs to play media. Every response carries a restrictive CSP and
//! `nosniff`, so a body opened as a document can't run or load anything.

use std::sync::Arc;

use parking_lot::Mutex;
use pm_core::Engine;
use pm_model::{BodyPart, ContentKind, FlowId};
use tauri::http::{Request, Response, StatusCode, header};

pub const SCHEME: &str = "pmbody";

/// Bodies kept in memory for repeated (range) requests.
const CACHE_ENTRIES: usize = 4;

/// What a body served as a document may do: nothing beyond inline styles
/// and data: images. PDFs can't be sandboxed or WebKit won't render them.
const CSP: &str = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; media-src 'self'";

#[derive(Clone, PartialEq, Eq)]
struct Key {
    flow: FlowId,
    part: BodyPart,
    decoded: bool,
    max: Option<usize>,
}

struct Body {
    bytes: Vec<u8>,
    mime: String,
    kind: ContentKind,
    cut: bool,
}

/// A small cache of recently served bodies.
#[derive(Default)]
pub struct BodyCache(Mutex<Vec<(Key, Arc<Body>)>>);

pub fn handle(engine: &Engine, cache: &BodyCache, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    match serve(engine, cache, request) {
        Ok(response) => response,
        Err((status, message)) => Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(message.into_bytes())
            .expect("valid error response"),
    }
}

type Failure = (StatusCode, String);

fn serve(engine: &Engine, cache: &BodyCache, request: &Request<Vec<u8>>) -> Result<Response<Vec<u8>>, Failure> {
    let key = parse(request.uri())?;
    let body = load(engine, cache, &key)?;
    let total = body.bytes.len();
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| byte_range(v, total));
    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, &body.mime)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "x-pm-cut, content-range")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-pm-cut", if body.cut { "1" } else { "0" });
    response = if body.kind == ContentKind::Pdf {
        response.header(header::CONTENT_SECURITY_POLICY, CSP)
    } else {
        response.header(header::CONTENT_SECURITY_POLICY, format!("{CSP}; sandbox"))
    };
    let response = match range {
        Some((start, end)) => response
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
            .body(body.bytes[start..=end].to_vec()),
        None => response.status(StatusCode::OK).body(body.bytes.clone()),
    };
    response.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// `/<store>/<flow>/<part>?decoded=1&max=N`.
fn parse(uri: &tauri::http::Uri) -> Result<Key, Failure> {
    let bad = |what: &str| (StatusCode::BAD_REQUEST, format!("pmbody: {what} in {uri}"));
    let segments: Vec<&str> = uri.path().trim_matches('/').split('/').collect();
    let [store, flow, part] = segments.as_slice() else {
        return Err(bad("expected /<store>/<flow>/<part>"));
    };
    // Saved sessions and HAR imports (M8) add stores beside the live one.
    if *store != "live" {
        return Err((StatusCode::NOT_FOUND, format!("pmbody: unknown store {store:?}")));
    }
    let flow = flow.parse().map_err(|_| bad("bad flow id"))?;
    let part = match *part {
        "request" => BodyPart::Request,
        "response" => BodyPart::Response,
        _ => return Err(bad("part must be request or response")),
    };
    let mut decoded = false;
    let mut max = None;
    for pair in uri.query().unwrap_or_default().split('&') {
        match pair.split_once('=') {
            Some(("decoded", v)) => decoded = v == "1" || v == "true",
            Some(("max", v)) => max = Some(v.parse().map_err(|_| bad("bad max"))?),
            _ => {}
        }
    }
    Ok(Key {
        flow,
        part,
        decoded,
        max,
    })
}

fn load(engine: &Engine, cache: &BodyCache, key: &Key) -> Result<Arc<Body>, Failure> {
    if let Some((_, body)) = cache.0.lock().iter().find(|(k, _)| k == key) {
        return Ok(body.clone());
    }
    let not_found = || (StatusCode::NOT_FOUND, format!("pmbody: no flow {}", key.flow));
    let detail = engine
        .flow_detail(key.flow)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(not_found)?;
    let info = match key.part {
        BodyPart::Request => detail.request_body,
        BodyPart::Response => detail.response_body,
    };
    let bytes = engine
        .body_head(key.flow, key.part, key.decoded, key.max)
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    // Still-encoded bytes are not what the MIME type describes.
    let encoded = info.encoding.is_some() && bytes.decoded_from.is_none();
    let mime = match info.mime {
        Some(mime) if !encoded => mime,
        _ => "application/octet-stream".to_owned(),
    };
    let body = Arc::new(Body {
        bytes: bytes.bytes,
        mime,
        kind: info.kind,
        cut: bytes.cut,
    });
    let mut entries = cache.0.lock();
    entries.retain(|(k, _)| k != key);
    entries.insert(0, (key.clone(), body.clone()));
    entries.truncate(CACHE_ENTRIES);
    Ok(body)
}

/// The inclusive byte range of a single-range `Range` header, clamped to
/// the body. Multiple ranges and unsatisfiable ones get the whole body.
fn byte_range(value: &str, total: usize) -> Option<(usize, usize)> {
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') || total == 0 {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = match (start.trim(), end.trim()) {
        ("", suffix) => {
            let n: usize = suffix.parse().ok()?;
            (total.saturating_sub(n), total - 1)
        }
        (start, "") => (start.parse().ok()?, total - 1),
        (start, end) => (start.parse().ok()?, end.parse::<usize>().ok()?.min(total - 1)),
    };
    (start <= end && start < total).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_clamped_and_suffixes_supported() {
        assert_eq!(byte_range("bytes=0-1", 10), Some((0, 1)));
        assert_eq!(byte_range("bytes=5-", 10), Some((5, 9)));
        assert_eq!(byte_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(byte_range("bytes=2-100", 10), Some((2, 9)));
        assert_eq!(byte_range("bytes=20-30", 10), None);
        assert_eq!(byte_range("bytes=0-1,4-5", 10), None);
        assert_eq!(byte_range("items=0-1", 10), None);
    }

    #[test]
    fn urls_parse_into_keys() {
        let key = parse(&"pmbody://localhost/live/42/response?decoded=1&max=100".parse().unwrap()).unwrap();
        assert_eq!(
            (key.flow, key.part, key.decoded, key.max),
            (42, BodyPart::Response, true, Some(100))
        );
        let key = parse(&"http://pmbody.localhost/live/7/request".parse().unwrap()).unwrap();
        assert_eq!(
            (key.flow, key.part, key.decoded, key.max),
            (7, BodyPart::Request, false, None)
        );
        assert!(parse(&"pmbody://localhost/live/x/response".parse().unwrap()).is_err());
        assert!(parse(&"pmbody://localhost/session/1/response".parse().unwrap()).is_err());
    }

    fn get(engine: &Engine, cache: &BodyCache, uri: &str, range: Option<&str>) -> Response<Vec<u8>> {
        let mut request = Request::builder().uri(uri);
        if let Some(range) = range {
            request = request.header(header::RANGE, range);
        }
        handle(engine, cache, &request.body(Vec::new()).unwrap())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bodies_are_served_with_their_type_and_a_csp() {
        let engine = Engine::with_config(pm_core::EngineConfig {
            data_dir: Some(std::env::temp_dir().join("proxymate-tests").join(uuid_like())),
            ..Default::default()
        });
        let proxy = engine
            .start_proxy(pm_core::ProxyConfig {
                ports: 0..=0,
                ..Default::default()
            })
            .await
            .unwrap();
        let upstream = pm_testkit::Upstream::start().await;
        pm_testkit::client::get(proxy, &upstream.url("/zoo/png-mislabelled")).await;
        pm_testkit::client::get(proxy, &upstream.url("/zoo/json-gzip")).await;
        pm_testkit::client::eventually(std::time::Duration::from_secs(10), || {
            engine.flush();
            engine.get_flow(1).filter(|f| f.state == pm_model::FlowState::Completed)
        })
        .await;
        let cache = BodyCache::default();

        let png = pm_testkit::zoo::file_bytes("image.png");
        let response = get(&engine, &cache, "pmbody://localhost/live/0/response?decoded=1", None);
        assert_eq!(response.status(), StatusCode::OK);
        let headers = response.headers();
        // Served as what the bytes are, not what the header claimed.
        assert_eq!(headers[header::CONTENT_TYPE], "image/png");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(
            csp.starts_with("default-src 'none'") && csp.ends_with("sandbox"),
            "{csp}"
        );
        assert_eq!(response.body(), png);

        let response = get(&engine, &cache, "pmbody://localhost/live/0/response", Some("bytes=1-3"));
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response.headers()[header::CONTENT_RANGE],
            format!("bytes 1-3/{}", png.len())
        );
        assert_eq!(response.body(), &png[1..=3]);

        // Encoded bodies: decoded on request, else opaque bytes.
        let nested = pm_testkit::zoo::file_bytes("nested.json");
        let decoded = get(
            &engine,
            &cache,
            "http://pmbody.localhost/live/1/response?decoded=1&max=5",
            None,
        );
        assert_eq!(decoded.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(decoded.headers()["x-pm-cut"], "1");
        assert_eq!(decoded.body(), &nested[..5]);
        let raw = get(&engine, &cache, "pmbody://localhost/live/1/response", None);
        assert_eq!(raw.headers()[header::CONTENT_TYPE], "application/octet-stream");

        assert_eq!(
            get(&engine, &cache, "pmbody://localhost/live/9/response", None).status(),
            StatusCode::NOT_FOUND
        );
    }

    fn uuid_like() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("scheme-{}-{nanos}", std::process::id())
    }
}
