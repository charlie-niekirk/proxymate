//! The script runtime: messages, dirty detection, mocks, views, limits,
//! `shared`, the built-ins (against known vectors) and the API surface.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pm_model::ScriptViewKind;

use super::host::{MockOut, RequestOut, ResponseOut, ViewOut};
use super::*;

fn code(id: &str, source: &str) -> Code {
    Code::new(id.into(), "test".into(), source.into())
}

fn request(content_type: &str, body: &[u8]) -> JsRequest {
    JsRequest {
        method: "POST".into(),
        url: "https://api.test:8443/v1/items?b=2&a=1&a=3".into(),
        scheme: "https".into(),
        host: "api.test".into(),
        port: 8443,
        path: "/v1/items".into(),
        query: "b=2&a=1&a=3".into(),
        headers: vec![
            ("host".into(), "api.test:8443".into()),
            ("content-type".into(), content_type.into()),
            ("x-tag".into(), "one".into()),
            ("x-tag".into(), "two".into()),
        ],
        body: Some(Arc::new(body.to_vec())),
    }
}

fn response(content_type: &str, body: &[u8]) -> JsResponse {
    JsResponse {
        status: 200,
        headers: vec![("content-type".into(), content_type.into())],
        body: Some(Arc::new(body.to_vec())),
    }
}

async fn run_with(host: &ScriptHost, code: Code, call: Call, timeout: Duration) -> RunResult {
    host.run(code, call, CallInfo::default(), timeout, false).await
}

async fn run(source: &str, call: Call) -> RunResult {
    run_with(&ScriptHost::new(), code("t", source), call, Duration::from_secs(2)).await
}

async fn on_request(source: &str, req: JsRequest) -> RequestOut {
    match run(source, Call::Request(req)).await.output {
        Ok(Output::Request(out)) => out,
        other => panic!("{other:?}"),
    }
}

async fn on_response(source: &str, resp: JsResponse) -> ResponseOut {
    match run(source, Call::Response(request("text/plain", b""), resp))
        .await
        .output
    {
        Ok(Output::Response(out)) => out,
        other => panic!("{other:?}"),
    }
}

async fn on_mock(source: &str) -> Option<MockOut> {
    match run(source, Call::Mock(request("text/plain", b""))).await.output {
        Ok(Output::Mock(out)) => out,
        other => panic!("{other:?}"),
    }
}

async fn on_view(source: &str, resp: JsResponse) -> Vec<ViewOut> {
    match run(source, Call::View(request("text/plain", b""), Some(resp)))
        .await
        .output
    {
        Ok(Output::Views(views)) => views,
        other => panic!("{other:?}"),
    }
}

/// Evaluates `expr` in a view and returns it as text.
async fn eval(expr: &str) -> String {
    let source =
        format!("export async function onView() {{ return [{{ title: 't', body: String(await ({expr})) }}]; }}");
    let views = on_view(&source, response("text/plain", b"")).await;
    views[0].text.clone().unwrap()
}

#[tokio::test]
async fn an_untouched_message_is_byte_identical() {
    let body = br#"{ "b" : 1.50, "a":[1,2] ,"id": 12345678901234567890 }"#;
    for source in [
        "export function onRequest(c, url, r) { return r; }",
        "export function onRequest(c, url, r) {}",
        // Reading everything, changing nothing.
        "export function onRequest(c, url, r) { r.body.a; r.text; r.headers['x-tag']; r.query.a; return r; }",
        "export function onRequest(c, url, r) { r.rawBody; return r; }",
    ] {
        let out = on_request(source, request("application/json", body)).await;
        assert_eq!(out.headers, None, "{source}");
        assert_eq!(out.body, None, "{source}");
        assert_eq!(out.query, "b=2&a=1&a=3", "{source}");
        assert_eq!(
            (out.method.as_str(), out.host.as_str(), out.port),
            ("POST", "api.test", 8443)
        );
    }
}

#[tokio::test]
async fn repeated_headers_come_back_in_order() {
    let out = on_request(
        "export function onRequest(c, url, r) { \
           if (r.headers['x-tag'].join() !== 'one,two') throw new Error('not an array'); \
           r.headers['x-new'] = 'yes'; return r; }",
        request("text/plain", b"hi"),
    )
    .await;
    let headers = out.headers.unwrap();
    let tags: Vec<&str> = headers
        .iter()
        .filter(|(n, _)| n == "x-tag")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(tags, ["one", "two"]);
    assert!(headers.iter().any(|(n, v)| n == "x-new" && v == "yes"));
    assert_eq!(out.body, None);
}

#[tokio::test]
async fn edits_are_detected_and_bigints_round_trip() {
    let body = br#"{"id": 12345678901234567890, "n": 1}"#;
    let out = on_request(
        "export function onRequest(c, url, r) { \
           if (typeof r.body.id !== 'bigint') throw new Error(typeof r.body.id); \
           r.body.n = 2; return r; }",
        request("application/json", body),
    )
    .await;
    assert_eq!(out.body.unwrap(), br#"{"id":12345678901234567890,"n":2}"#);

    let out = on_request(
        "export function onRequest(c, url, r) { r.body = { replaced: true }; }",
        request("application/json", body),
    )
    .await;
    assert_eq!(out.body.unwrap(), br#"{"replaced":true}"#);

    let out = on_request(
        "export function onRequest(c, url, r) { r.text = r.text.toUpperCase(); }",
        request("text/plain", b"shout"),
    )
    .await;
    assert_eq!(out.body.unwrap(), b"SHOUT");

    let out = on_request(
        "export function onRequest(c, url, r) { r.rawBody[0] = 0x4a; }",
        request("application/octet-stream", b"\x00ello"),
    )
    .await;
    assert_eq!(out.body.unwrap(), b"Jello");

    let out = on_request(
        "export function onRequest(c, url, r) { r.body.x = 'a b'; }",
        request("application/x-www-form-urlencoded", b"x=1&y=2"),
    )
    .await;
    assert_eq!(out.body.unwrap(), b"x=a+b&y=2");
}

#[tokio::test]
async fn the_url_and_query_can_change() {
    let out = on_request(
        "export function onRequest(c, url, r) { r.query.page = '3'; r.path = '/v2/items'; r.method = 'put'; }",
        request("text/plain", b""),
    )
    .await;
    assert_eq!((out.method.as_str(), out.path.as_str()), ("put", "/v2/items"));
    assert_eq!(out.query, "b=2&a=1&a=3&page=3");

    let out = on_request(
        "export function onRequest(c, url, r) { r.url = 'http://other.test/x?y=1'; }",
        request("text/plain", b""),
    )
    .await;
    assert_eq!(
        (
            out.scheme.as_str(),
            out.host.as_str(),
            out.port,
            out.path.as_str(),
            out.query.as_str()
        ),
        ("http", "other.test", 80, "/x", "y=1")
    );
}

#[tokio::test]
async fn responses_change_status_headers_and_body() {
    let out = on_response(
        "export function onResponse(c, url, req, res) { res.status = 201; res.body.ok = false; return res; }",
        response("application/json", br#"{"ok":true}"#),
    )
    .await;
    assert_eq!(out.status, 201);
    assert_eq!(out.headers, None);
    assert_eq!(out.body.unwrap(), br#"{"ok":false}"#);

    let result = run(
        "export function onResponse(c, url, req, res) { req.method = 'X'; return res; }",
        Call::Response(request("text/plain", b""), response("text/plain", b"x")),
    )
    .await;
    let message = result.output.unwrap_err();
    assert!(
        message.contains("read-only"),
        "the request is read-only here: {message}"
    );
}

#[tokio::test]
async fn mocks_take_objects_strings_bytes_or_null() {
    let out = on_mock("export function onMock() { return { status: 418, body: { tea: true } }; }")
        .await
        .unwrap();
    assert_eq!(out.status, 418);
    assert_eq!(out.body, br#"{"tea":true}"#);
    assert!(
        out.headers
            .contains(&("content-type".into(), "application/json".into()))
    );

    let out = on_mock("export function onMock() { return 'hello'; }").await.unwrap();
    assert_eq!((out.status, out.body.as_slice()), (200, &b"hello"[..]));
    assert!(out.headers[0].1.starts_with("text/plain"));

    let out = on_mock("export function onMock() { return new Uint8Array([1, 2, 3]); }")
        .await
        .unwrap();
    assert_eq!(out.body, [1, 2, 3]);

    let out = on_mock("export function onMock() { return { headers: { 'X-A': ['1', '2'] }, text: 'ok' }; }")
        .await
        .unwrap();
    assert!(out.headers.contains(&("x-a".into(), "2".into())));

    assert!(
        on_mock("export function onMock(c, url, r) { return null; }")
            .await
            .is_none()
    );
    assert!(on_mock("export function onMock(c, url, r) {}").await.is_none());
}

#[tokio::test]
async fn views_render_json_text_and_bytes() {
    let views = on_view(
        "export function onView(c, flow) { return [ \
           { title: 'Decoded', body: Proxymate.base64DecodeJson(flow.response.body.data) }, \
           { title: 'Raw', kind: 'hex', body: flow.response.rawBody }, \
           { title: 'Part', body: flow.part ?? 'none' } ]; }",
        response(
            "application/json",
            br#"{"data":"eyJ1c2VyIjoiYWRhIiwiaWQiOjEyMzQ1Njc4OTAxMjM0NTY3ODkwfQ"}"#,
        ),
    )
    .await;
    assert_eq!(views.len(), 3);
    assert_eq!(views[0].kind, ScriptViewKind::Json);
    assert_eq!(
        views[0].text.as_deref().unwrap(),
        "{\n  \"user\": \"ada\",\n  \"id\": 12345678901234567890\n}"
    );
    assert_eq!(views[1].kind, ScriptViewKind::Hex);
    assert!(views[1].bytes.as_ref().unwrap().starts_with(b"{\"data\""));
    assert_eq!(views[2].text.as_deref(), Some("none"));
}

#[tokio::test]
async fn a_throw_is_reported_with_its_message() {
    let result = run(
        "export function onRequest() { throw new TypeError('bad token'); }",
        Call::Request(request("text/plain", b"")),
    )
    .await;
    let message = result.output.unwrap_err();
    assert!(message.starts_with("TypeError: bad token"), "{message}");

    let result = run(
        "export function onRequest() { throw 'plain'; }",
        Call::Request(request("text/plain", b"")),
    )
    .await;
    assert!(result.output.unwrap_err().contains("plain"));

    let result = run(
        "export function onRequest( {",
        Call::Request(request("text/plain", b"")),
    )
    .await;
    assert!(result.output.unwrap_err().contains("SyntaxError"));
}

#[tokio::test]
async fn an_infinite_loop_stops_at_the_timeout_and_the_worker_recovers() {
    let host = ScriptHost::new();
    let timeout = Duration::from_millis(200);
    let started = Instant::now();
    let result = run_with(
        &host,
        code("loop", "export function onRequest() { for (;;) {} }"),
        Call::Request(request("text/plain", b"")),
        timeout,
    )
    .await;
    let elapsed = started.elapsed();
    assert!(result.output.unwrap_err().contains("timed out"));
    assert!(elapsed < timeout + Duration::from_millis(50), "{elapsed:?}");

    let result = run_with(
        &host,
        code(
            "loop",
            "export async function onRequest() { await new Promise(() => {}); }",
        ),
        Call::Request(request("text/plain", b"")),
        timeout,
    )
    .await;
    assert!(result.output.unwrap_err().contains("never settled"));

    let result = run_with(
        &host,
        code("loop", "export function onRequest(c, u, r) { return r; }"),
        Call::Request(request("text/plain", b"")),
        timeout,
    )
    .await;
    assert!(matches!(result.output, Ok(Output::Request(_))));
}

#[tokio::test]
async fn a_memory_bomb_fails_and_the_worker_recovers() {
    let host = ScriptHost::new();
    let result = run_with(
        &host,
        code(
            "bomb",
            "export function onRequest() { const a = []; for (;;) a.push(new Array(1e6).fill(1.5)); }",
        ),
        Call::Request(request("text/plain", b"")),
        Duration::from_secs(10),
    )
    .await;
    assert!(result.output.unwrap_err().contains("out of memory"));
    let result = run_with(
        &host,
        code("bomb", "export function onRequest(c, u, r) { return r; }"),
        Call::Request(request("text/plain", b"")),
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(result.output, Ok(Output::Request(_))));
}

#[tokio::test]
async fn async_entry_points_work() {
    let out = on_request(
        "async function tag() { await null; return 'later'; } \
         export async function onRequest(c, url, r) { r.headers['x-async'] = await tag(); return r; }",
        request("text/plain", b""),
    )
    .await;
    assert!(out.headers.unwrap().contains(&("x-async".into(), "later".into())));
}

#[tokio::test]
async fn shared_persists_between_calls_and_resets_on_edit() {
    let host = ScriptHost::new();
    let counter = "export function onMock(c) { c.shared.n = (c.shared.n ?? 0) + 1; return String(c.shared.n); }";
    let count = |source: &'static str| {
        let host = &host;
        async move {
            match run_with(
                host,
                code("counter", source),
                Call::Mock(request("text/plain", b"")),
                Duration::from_secs(1),
            )
            .await
            .output
            {
                Ok(Output::Mock(Some(out))) => String::from_utf8(out.body).unwrap(),
                other => panic!("{other:?}"),
            }
        }
    };
    assert_eq!(count(counter).await, "1");
    assert_eq!(count(counter).await, "2");
    // Saved again with new source: a fresh `shared`.
    let edited = "export function onMock(c) { c.shared.n = (c.shared.n ?? 10) + 1; return String(c.shared.n); }";
    assert_eq!(count(edited).await, "11");
}

#[tokio::test]
async fn console_lines_are_collected() {
    let result = run(
        "export function onRequest(c, u, r) { console.log('id', 1, { a: 2n }); console.warn('careful'); }",
        Call::Request(request("text/plain", b"")),
    )
    .await;
    assert_eq!(
        result.console,
        [
            (pm_model::ConsoleLevel::Log, "id 1 {\n  \"a\": 2\n}".to_owned()),
            (pm_model::ConsoleLevel::Warn, "careful".to_owned())
        ]
    );
}

#[tokio::test]
async fn the_built_ins_match_known_vectors() {
    let cases = [
        // FIPS 180 and RFC 4231 (test case 2).
        (
            "crypto.sha256('abc')",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        ("crypto.sha1('abc')", "a9993e364706816aba3e25717850c26c9cd0d89d"),
        ("crypto.sha512('abc').slice(0, 16)", "ddaf35a193617aba"),
        (
            "crypto.hmac('sha256', 'Jefe', 'what do ya want for nothing?')",
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        ),
        (
            "crypto.sha256('abc', 'base64')",
            "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=",
        ),
        ("/^[0-9a-f-]{36}$/.test(crypto.randomUUID())", "true"),
        ("btoa('hello')", "aGVsbG8="),
        ("atob('aGVsbG8=')", "hello"),
        ("Proxymate.base64DecodeText('aGk_Pz8')", "hi???"),
        ("Proxymate.base64DecodeText('aGk-')", "hi>"),
        ("Proxymate.base64Encode('hi???', { urlSafe: true })", "aGk_Pz8"),
        ("Proxymate.base64DecodeJson('eyJhIjoxfQ').a", "1"),
        (
            "new TextDecoder().decode(new TextEncoder().encode('héllo €'))",
            "héllo €",
        ),
        ("new TextEncoder().encode('é').length", "2"),
        (
            "Proxymate.parseJson('{\"n\":9007199254740993}').n === 9007199254740993n",
            "true",
        ),
        (
            "Proxymate.stringifyJson({ n: 9007199254740993n })",
            "{\"n\":9007199254740993}",
        ),
        (
            "new URL('/b?x=1#h', 'https://a.test:8443/a').href",
            "https://a.test:8443/b?x=1#h",
        ),
        ("new URL('https://a.test/p?q=1').searchParams.get('q')", "1"),
        (
            "(() => { const u = new URL('https://a.test/p?q=1'); u.searchParams.set('r', 'a b'); return u.href; })()",
            "https://a.test/p?q=1&r=a+b",
        ),
        ("new URL('https://ex.test:443/').port === ''", "true"),
        ("URL.canParse('nope')", "false"),
        ("new URLSearchParams('a=1&b=2&a=3').getAll('a').join()", "1,3"),
        ("new URLSearchParams({ x: 'é&' }).toString()", "x=%C3%A9%26"),
        // gzip of "hello" (from Python's gzip.compress with mtime=0).
        (
            "new TextDecoder().decode(Proxymate.decompress(Proxymate.base64Decode('H4sIAAAAAAAC/8tIzcnJBwCGphA2BQAAAA=='), 'gzip'))",
            "hello",
        ),
    ];
    for (expr, want) in cases {
        assert_eq!(eval(expr).await, want, "{expr}");
    }
}

#[tokio::test]
async fn scripts_have_no_io() {
    for (expr, want) in [
        ("typeof fetch", "undefined"),
        ("typeof require", "undefined"),
        ("typeof setTimeout", "undefined"),
        ("typeof XMLHttpRequest", "undefined"),
        ("typeof std + typeof os", "undefinedundefined"),
        (
            "import('/etc/passwd').then(() => 'loaded', (e) => 'refused: ' + e.message)",
            "refused: could not load module '/etc/passwd'",
        ),
    ] {
        assert_eq!(eval(expr).await, want, "{expr}");
    }
    let result = run(
        "import fs from 'fs'; export function onRequest() {}",
        Call::Request(request("text/plain", b"")),
    )
    .await;
    assert!(result.output.is_err());
}

/// `scripts/api_surface.txt` lists exactly the globals a script sees
/// beyond the language's own.
#[tokio::test]
async fn the_api_surface_matches_api_surface_txt() {
    let builtins = eval(
        "Object.getOwnPropertyNames(globalThis).filter((n) => !['Object','Function','Error','EvalError','RangeError',\
         'ReferenceError','SyntaxError','TypeError','URIError','InternalError','AggregateError','SuppressedError',\
         'Iterator','Array','parseInt','parseFloat','isNaN','isFinite','queueMicrotask','decodeURI',\
         'decodeURIComponent','encodeURI','encodeURIComponent','escape','unescape','Infinity','NaN','undefined',\
         'eval','Number','Boolean','String','Math','Reflect','Symbol','DisposableStack','globalThis','BigInt','Date',\
         'RegExp','JSON','Proxy','Map','Set','WeakMap','WeakSet','ArrayBuffer','SharedArrayBuffer',\
         'Uint8ClampedArray','Int8Array','Uint8Array','Int16Array','Uint16Array','Int32Array','Uint32Array',\
         'BigInt64Array','BigUint64Array','Float16Array','Float32Array','Float64Array','DataView','Atomics',\
         'Promise','AsyncDisposableStack','WeakRef','FinalizationRegistry'].includes(n)).sort().join('\\n')",
    )
    .await;
    let documented: Vec<&str> = API_SURFACE
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split('.').next())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(builtins.lines().collect::<Vec<_>>(), documented);
    // Every documented member exists.
    for line in API_SURFACE
        .lines()
        .map(str::trim)
        .filter(|l| l.contains('.') && !l.starts_with('#'))
    {
        let check = format!("typeof {line} !== 'undefined'");
        assert_eq!(eval(&check).await, "true", "{line}");
    }
}
