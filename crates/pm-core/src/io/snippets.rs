//! Copy as code (M17): a recorded request as a snippet that sends it again,
//! in JavaScript `fetch`, Python `requests`, HTTPie, Go `net/http`, Swift
//! `URLSession`, Kotlin OkHttp or PowerShell. cURL has its own module
//! (`curl.rs`), which works on the recorded detail.
//!
//! - The input is the flow's editable [`RequestSpec`]: Host,
//!   Content-Length and hop-by-hop headers are already gone, and a
//!   content-encoded body is decoded (the snippet sends it plain).
//! - Repeated headers are joined into one (`cookie` with `; `, the rest with
//!   `, `), since most of these APIs keep one value per name.
//! - Text bodies are inlined as string literals, escaped for the language;
//!   binary bodies are inlined as Base64 and decoded by the snippet.

use std::fmt::Write as _;

use base64::Engine as _;
use pm_model::{CodeLanguage, RequestSpec};

use super::curl;

/// A request body as the snippet sends it.
enum Body {
    Text(String),
    /// Base64 of the bytes.
    Binary(String),
}

/// What every renderer works from.
struct Request {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Body>,
}

impl Request {
    fn from_spec(spec: &RequestSpec) -> Request {
        let mut headers: Vec<(String, String)> = Vec::new();
        for h in &spec.headers {
            match headers.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&h.name)) {
                Some((name, value)) => {
                    let sep = if name.eq_ignore_ascii_case("cookie") {
                        "; "
                    } else {
                        ", "
                    };
                    value.push_str(sep);
                    value.push_str(&h.value);
                }
                None => headers.push((h.name.clone(), h.value.clone())),
            }
        }
        let body = match &spec.body {
            None => None,
            Some(b) if b.is_empty() => None,
            Some(b) if spec.body_base64 => Some(Body::Binary(b.clone())),
            Some(b) => Some(Body::Text(b.clone())),
        };
        Request {
            method: spec.method.to_ascii_uppercase(),
            url: spec.url.clone(),
            headers,
            body,
        }
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The headers except `skip` (names the API sets another way).
    fn headers_except<'a>(&'a self, skip: &'a [&str]) -> impl Iterator<Item = &'a (String, String)> + 'a {
        self.headers
            .iter()
            .filter(move |(n, _)| !skip.iter().any(|s| n.eq_ignore_ascii_case(s)))
    }
}

/// The snippet for `spec` in `language`. cURL goes through
/// [`curl::command`] instead (it needs the recorded detail); asked for here,
/// it gets a command built from the spec.
pub fn render(language: CodeLanguage, spec: &RequestSpec) -> String {
    let request = Request::from_spec(spec);
    match language {
        CodeLanguage::Curl => curl_from_spec(&request),
        CodeLanguage::Fetch => fetch(&request),
        CodeLanguage::Python => python(&request),
        CodeLanguage::Httpie => httpie(&request),
        CodeLanguage::Go => go(&request),
        CodeLanguage::Swift => swift(&request),
        CodeLanguage::Kotlin => kotlin(&request),
        CodeLanguage::PowerShell => powershell(&request),
    }
}

fn curl_from_spec(r: &Request) -> String {
    let mut out = format!("curl -X {} {}", curl::quote(&r.method), curl::quote(&r.url));
    for (name, value) in &r.headers {
        let _ = write!(out, " \\\n  -H {}", curl::quote(&format!("{name}: {value}")));
    }
    match &r.body {
        Some(Body::Text(text)) => {
            let _ = write!(out, " \\\n  --data-raw {}", curl::quote(text));
        }
        Some(Body::Binary(b64)) => {
            out = format!(
                "printf %s {} | base64 --decode | {out} \\\n  --data-binary @-",
                curl::quote(b64)
            );
        }
        None => {}
    }
    out
}

/// A double-quoted literal with JSON escapes, which JavaScript, Python and
/// Go all read the same way.
fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// A double-quoted literal with the escapes a C-like language knows
/// (`\n \r \t \" \\`), other control characters as `unicode(code)`, and
/// `extra` escaped with a backslash.
fn c_str(s: &str, unicode: impl Fn(u32) -> String, extra: &[char]) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&unicode(c as u32)),
            c if extra.contains(&c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn fetch(r: &Request) -> String {
    let mut out = format!("const response = await fetch({}", json_str(&r.url));
    let mut options = Vec::new();
    if r.method != "GET" {
        options.push(format!("  method: {},", json_str(&r.method)));
    }
    if !r.headers.is_empty() {
        let mut headers = String::from("  headers: {\n");
        for (name, value) in &r.headers {
            let _ = writeln!(headers, "    {}: {},", json_str(name), json_str(value));
        }
        headers.push_str("  },");
        options.push(headers);
    }
    match &r.body {
        Some(Body::Text(text)) => options.push(format!("  body: {},", json_str(text))),
        Some(Body::Binary(b64)) => options.push(format!(
            "  body: Uint8Array.from(atob({}), (c) => c.charCodeAt(0)),",
            json_str(b64)
        )),
        None => {}
    }
    if !options.is_empty() {
        out.push_str(", {\n");
        out.push_str(&options.join("\n"));
        out.push_str("\n}");
    }
    out.push_str(");\n");
    out.push_str("console.log(response.status);\nconsole.log(await response.text());");
    out
}

fn python(r: &Request) -> String {
    let binary = matches!(r.body, Some(Body::Binary(_)));
    let mut out = String::new();
    if binary {
        out.push_str("import base64\n\n");
    }
    out.push_str("import requests\n\n");
    let call = match r.method.as_str() {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" => {
            format!(
                "requests.{}(\n    {},\n",
                r.method.to_ascii_lowercase(),
                json_str(&r.url)
            )
        }
        other => format!(
            "requests.request(\n    {},\n    {},\n",
            json_str(other),
            json_str(&r.url)
        ),
    };
    let _ = write!(out, "response = {call}");
    if !r.headers.is_empty() {
        out.push_str("    headers={\n");
        for (name, value) in &r.headers {
            let _ = writeln!(out, "        {}: {},", json_str(name), json_str(value));
        }
        out.push_str("    },\n");
    }
    match &r.body {
        // Encoded as UTF-8, as it was recorded; a str would go out as Latin-1.
        Some(Body::Text(text)) => {
            let _ = writeln!(out, "    data={}.encode(),", json_str(text));
        }
        Some(Body::Binary(b64)) => {
            let _ = writeln!(out, "    data=base64.b64decode({}),", json_str(b64));
        }
        None => {}
    }
    out.push_str(")\nprint(response.status_code)\nprint(response.text)");
    out
}

fn httpie(r: &Request) -> String {
    let mut args = Vec::new();
    if let Some(Body::Text(text)) = &r.body {
        args.push(format!("--raw {}", curl::quote(text)));
    }
    args.push(r.method.clone());
    args.push(curl::quote(&r.url));
    for (name, value) in &r.headers {
        // `Name;` is HTTPie's way of sending an empty header.
        let item = if value.is_empty() {
            format!("{name};")
        } else {
            format!("{name}:{value}")
        };
        args.push(curl::quote(&item));
    }
    let mut out = String::new();
    if let Some(Body::Binary(b64)) = &r.body {
        let _ = write!(out, "printf %s {} | base64 --decode | ", curl::quote(b64));
    }
    out.push_str("http ");
    out.push_str(&args.join(" \\\n  "));
    out
}

fn go(r: &Request) -> String {
    let (imports, body_line) = match &r.body {
        Some(Body::Text(text)) => (
            vec!["\"fmt\"", "\"io\"", "\"net/http\"", "\"strings\""],
            format!("\tbody := strings.NewReader({})\n", json_str(text)),
        ),
        Some(Body::Binary(b64)) => (
            vec!["\"bytes\"", "\"encoding/base64\"", "\"fmt\"", "\"io\"", "\"net/http\""],
            format!(
                "\tpayload, err := base64.StdEncoding.DecodeString({})\n\tif err != nil {{\n\t\tpanic(err)\n\t}}\n\tbody := bytes.NewReader(payload)\n",
                json_str(b64)
            ),
        ),
        None => (vec!["\"fmt\"", "\"io\"", "\"net/http\""], String::new()),
    };
    let mut out = String::from("package main\n\nimport (\n");
    for import in imports {
        let _ = writeln!(out, "\t{import}");
    }
    out.push_str(")\n\nfunc main() {\n");
    out.push_str(&body_line);
    let body = if r.body.is_some() { "body" } else { "nil" };
    let _ = writeln!(
        out,
        "\treq, err := http.NewRequest({}, {}, {body})",
        json_str(&r.method),
        json_str(&r.url)
    );
    out.push_str("\tif err != nil {\n\t\tpanic(err)\n\t}\n");
    for (name, value) in &r.headers {
        let _ = writeln!(out, "\treq.Header.Set({}, {})", json_str(name), json_str(value));
    }
    out.push_str(
        "\tresp, err := http.DefaultClient.Do(req)\n\tif err != nil {\n\t\tpanic(err)\n\t}\n\tdefer resp.Body.Close()\n\
         \tdata, err := io.ReadAll(resp.Body)\n\tif err != nil {\n\t\tpanic(err)\n\t}\n\
         \tfmt.Println(resp.Status)\n\tfmt.Println(string(data))\n}",
    );
    out
}

fn swift_str(s: &str) -> String {
    c_str(s, |code| format!("\\u{{{code:X}}}"), &[])
}

fn swift(r: &Request) -> String {
    let mut out = String::from("import Foundation\n\n");
    let _ = writeln!(
        out,
        "var request = URLRequest(url: URL(string: {})!)",
        swift_str(&r.url)
    );
    if r.method != "GET" {
        let _ = writeln!(out, "request.httpMethod = {}", swift_str(&r.method));
    }
    for (name, value) in &r.headers {
        let _ = writeln!(
            out,
            "request.setValue({}, forHTTPHeaderField: {})",
            swift_str(value),
            swift_str(name)
        );
    }
    match &r.body {
        Some(Body::Text(text)) => {
            let _ = writeln!(out, "request.httpBody = Data({}.utf8)", swift_str(text));
        }
        Some(Body::Binary(b64)) => {
            let _ = writeln!(out, "request.httpBody = Data(base64Encoded: {})!", swift_str(b64));
        }
        None => {}
    }
    out.push_str(
        "\nlet (data, response) = try await URLSession.shared.data(for: request)\n\
         print((response as! HTTPURLResponse).statusCode)\n\
         print(String(decoding: data, as: UTF8.self))",
    );
    out
}

fn kotlin_str(s: &str) -> String {
    c_str(s, |code| format!("\\u{code:04X}"), &['$'])
}

fn kotlin(r: &Request) -> String {
    let binary = matches!(r.body, Some(Body::Binary(_)));
    let content_type = r.header("content-type");
    // OkHttp sets Content-Type from the body's media type.
    let needs_body = r.body.is_some() || matches!(r.method.as_str(), "POST" | "PUT" | "PATCH");
    let mut imports = Vec::new();
    if needs_body && content_type.is_some() {
        imports.push("okhttp3.MediaType.Companion.toMediaType");
    }
    imports.extend(["okhttp3.OkHttpClient", "okhttp3.Request"]);
    if needs_body {
        imports.push("okhttp3.RequestBody.Companion.toRequestBody");
    }
    if binary {
        imports.push("java.util.Base64");
    }
    let mut out = String::new();
    for import in imports {
        let _ = writeln!(out, "import {import}");
    }
    out.push_str("\nval client = OkHttpClient()\n\n");
    if needs_body {
        let media = content_type
            .map(|ct| format!("{}.toMediaType()", kotlin_str(ct)))
            .unwrap_or_default();
        let source = match &r.body {
            Some(Body::Text(text)) => kotlin_str(text),
            Some(Body::Binary(b64)) => format!("Base64.getDecoder().decode({})", kotlin_str(b64)),
            None => "\"\"".into(),
        };
        let _ = writeln!(out, "val body = {source}.toRequestBody({media})");
    }
    out.push_str("val request = Request.Builder()\n");
    let _ = writeln!(out, "    .url({})", kotlin_str(&r.url));
    if r.method != "GET" || r.body.is_some() {
        let body = if needs_body { "body" } else { "null" };
        let _ = writeln!(out, "    .method({}, {body})", kotlin_str(&r.method));
    }
    let skip: &[&str] = if needs_body { &["content-type"] } else { &[] };
    for (name, value) in r.headers_except(skip) {
        let _ = writeln!(out, "    .header({}, {})", kotlin_str(name), kotlin_str(value));
    }
    out.push_str(
        "    .build()\n\nclient.newCall(request).execute().use { response ->\n    println(response.code)\n    println(response.body?.string())\n}",
    );
    out
}

/// A single-quoted PowerShell string: nothing is expanded, `'` doubles.
fn ps_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn powershell(r: &Request) -> String {
    const STANDARD: &[&str] = &[
        "GET", "HEAD", "POST", "PUT", "DELETE", "TRACE", "OPTIONS", "MERGE", "PATCH",
    ];
    let mut out = String::from("$params = @{\n");
    let _ = writeln!(out, "    Uri = {}", ps_str(&r.url));
    if STANDARD.contains(&r.method.as_str()) {
        let _ = writeln!(out, "    Method = {}", ps_str(&r.method));
    } else {
        let _ = writeln!(out, "    CustomMethod = {}", ps_str(&r.method));
    }
    // Windows PowerShell refuses these two in -Headers; they have their own
    // parameters.
    let headers: Vec<_> = r.headers_except(&["content-type", "user-agent"]).collect();
    if !headers.is_empty() {
        out.push_str("    Headers = @{\n");
        for (name, value) in headers {
            let _ = writeln!(out, "        {} = {}", ps_str(name), ps_str(value));
        }
        out.push_str("    }\n");
    }
    if let Some(ct) = r.header("content-type") {
        let _ = writeln!(out, "    ContentType = {}", ps_str(ct));
    }
    if let Some(ua) = r.header("user-agent") {
        let _ = writeln!(out, "    UserAgent = {}", ps_str(ua));
    }
    match &r.body {
        Some(Body::Text(text)) => {
            let _ = writeln!(out, "    Body = {}", ps_str(text));
        }
        Some(Body::Binary(b64)) => {
            let _ = writeln!(out, "    Body = [Convert]::FromBase64String({})", ps_str(b64));
        }
        None => {}
    }
    out.push_str("}\n$response = Invoke-WebRequest @params\n$response.StatusCode\n$response.Content");
    out
}

/// Base64 for a body given as bytes, for callers building a spec.
pub fn base64_body(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::Header;

    fn spec(method: &str, url: &str, headers: &[(&str, &str)], body: Option<(&str, bool)>) -> RequestSpec {
        RequestSpec {
            method: method.into(),
            url: url.into(),
            headers: headers.iter().map(|(n, v)| Header::new(*n, *v)).collect(),
            body: body.map(|(b, _)| b.to_owned()),
            body_base64: body.is_some_and(|(_, b64)| b64),
        }
    }

    fn cases() -> Vec<(&'static str, RequestSpec)> {
        vec![
            (
                "get_with_headers",
                spec(
                    "GET",
                    "https://api.example.com/v1/users?page=2&sort=name",
                    &[
                        ("accept", "application/json"),
                        ("cookie", "session=abc"),
                        ("cookie", "theme=dark"),
                        ("user-agent", "Proxymate/1.0"),
                    ],
                    None,
                ),
            ),
            (
                "post_json",
                spec(
                    "POST",
                    "https://api.example.com/v1/notes",
                    &[
                        ("content-type", "application/json"),
                        ("x-note", "it's \"quoted\" $HOME"),
                    ],
                    Some((
                        "{\"title\":\"Rock 'n' roll\",\"body\":\"line one\\nline two\",\"price\":\"$5\"}\n",
                        false,
                    )),
                ),
            ),
            (
                "binary_put",
                spec(
                    "PUT",
                    "http://localhost:8080/upload",
                    &[("content-type", "application/octet-stream")],
                    Some((&base64_body(&[0, 1, 2, 0xff, 0xfe]), true)),
                ),
            ),
            (
                "custom_method",
                spec(
                    "PROPFIND",
                    "https://dav.example.com/files/",
                    &[("depth", "1"), ("x-empty", "")],
                    None,
                ),
            ),
        ]
    }

    fn extension(language: CodeLanguage) -> &'static str {
        match language {
            CodeLanguage::Curl | CodeLanguage::Httpie => "sh",
            CodeLanguage::Fetch => "js",
            CodeLanguage::Python => "py",
            CodeLanguage::Go => "go",
            CodeLanguage::Swift => "swift",
            CodeLanguage::Kotlin => "kt",
            CodeLanguage::PowerShell => "ps1",
        }
    }

    /// Compares every case in every language with
    /// `tests/golden/snippets/<language>/<case>.<ext>`; `UPDATE_GOLDEN=1`
    /// rewrites them instead.
    #[test]
    fn golden_files() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/snippets");
        let update = std::env::var_os("UPDATE_GOLDEN").is_some();
        let mut failures = Vec::new();
        for language in CodeLanguage::ALL {
            // cURL's golden files are curl.rs's own.
            if language == CodeLanguage::Curl {
                continue;
            }
            let dir = root.join(serde_json::to_value(language).unwrap().as_str().unwrap());
            for (name, spec) in cases() {
                let path = dir.join(format!("{name}.{}", extension(language)));
                let actual = format!("{}\n", render(language, &spec));
                if update {
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(&path, &actual).unwrap();
                    continue;
                }
                match std::fs::read_to_string(&path) {
                    Ok(expected) if expected == actual => {}
                    Ok(expected) => failures.push(format!(
                        "{}:\n--- expected\n{expected}--- actual\n{actual}",
                        path.display()
                    )),
                    Err(e) => failures.push(format!("{}: {e}", path.display())),
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn repeated_headers_are_joined() {
        let r = Request::from_spec(&spec(
            "GET",
            "http://a/",
            &[("A", "1"), ("a", "2"), ("Cookie", "x=1"), ("cookie", "y=2")],
            None,
        ));
        assert_eq!(
            r.headers,
            vec![("A".into(), "1, 2".into()), ("Cookie".into(), "x=1; y=2".into())]
        );
    }

    #[test]
    fn escapes_suit_each_language() {
        assert_eq!(swift_str("a\u{1}\"b"), "\"a\\u{1}\\\"b\"");
        assert_eq!(kotlin_str("$x\u{1f}"), "\"\\$x\\u001F\"");
        assert_eq!(ps_str("it's"), "'it''s'");
        assert_eq!(json_str("a\"\n"), "\"a\\\"\\n\"");
    }
}
