//! The Rust built-ins behind the prelude: UTF-8, base64 (standard and
//! URL-safe, padded or not), URL parsing (the `url` crate, WHATWG rules),
//! hashes and HMAC (`ring`), UUIDs, decompression (the inspector's
//! decoder) and the console. None of them touches the network or files.

use std::cell::RefCell;
use std::rc::Rc;

use base64::Engine as _;
use pm_model::ConsoleLevel;
use rquickjs::function::Opt;
use rquickjs::{Ctx, Exception, Function, Object, Result, TypedArray, Value};

/// Console lines a run collected.
pub(super) type ConsoleBuffer = Rc<RefCell<Vec<(ConsoleLevel, String)>>>;
/// Console lines kept per run; the rest are counted.
const CONSOLE_LINES: usize = 500;

/// The bytes of a `Uint8Array` (or a view of part of one), copied out.
pub(super) fn bytes_of(value: &Value<'_>) -> Option<Vec<u8>> {
    let array = TypedArray::<u8>::from_value(value.clone()).ok()?;
    // SAFETY: `as_bytes` borrows memory the engine owns; it stays valid
    // while no JavaScript runs, and the slice is copied out before any does.
    #[allow(unsafe_code)]
    let bytes = unsafe { array.as_bytes() }.map(<[u8]>::to_vec);
    bytes
}

fn byte_arg(ctx: &Ctx<'_>, value: &Value<'_>, what: &str) -> Result<Vec<u8>> {
    bytes_of(value).ok_or_else(|| Exception::throw_type(ctx, &format!("{what} must be a Uint8Array")))
}

fn uint8<'js>(ctx: &Ctx<'js>, bytes: &[u8]) -> Result<Value<'js>> {
    Ok(TypedArray::<u8>::new_copy(ctx.clone(), bytes)?.into_value())
}

/// Decodes standard or URL-safe base64, with or without padding, ignoring
/// whitespace.
pub(crate) fn base64_decode(text: &str) -> std::result::Result<Vec<u8>, String> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim_end_matches('=');
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(trimmed)
        .map_err(|e| format!("not base64: {e}"))
}

/// Installs the built-ins on `native` (the prelude's argument).
pub(super) fn install<'js>(ctx: &Ctx<'js>, native: &Object<'js>, console: ConsoleBuffer) -> Result<()> {
    native.set(
        "utf8Encode",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, text: String| uint8(&ctx, text.as_bytes()))?,
    )?;
    native.set(
        "utf8Decode",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, bytes: Value<'js>, fatal: Opt<bool>, ignore_bom: Opt<bool>| -> Result<String> {
                let bytes = byte_arg(&ctx, &bytes, "the input")?;
                let body = match (ignore_bom.0.unwrap_or(false), bytes.strip_prefix(b"\xEF\xBB\xBF")) {
                    (false, Some(rest)) => rest,
                    _ => &bytes[..],
                };
                if fatal.0.unwrap_or(false) {
                    std::str::from_utf8(body)
                        .map(str::to_owned)
                        .map_err(|e| Exception::throw_type(&ctx, &format!("the input isn't UTF-8: {e}")))
                } else {
                    Ok(String::from_utf8_lossy(body).into_owned())
                }
            },
        )?,
    )?;
    native.set(
        "percentDecode",
        Function::new(ctx.clone(), |text: String| crate::rules::compile::percent_decode(&text))?,
    )?;
    native.set(
        "base64Decode",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, text: String| {
            let bytes = base64_decode(&text).map_err(|e| Exception::throw_type(&ctx, &e))?;
            uint8(&ctx, &bytes)
        })?,
    )?;
    native.set(
        "base64Encode",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, bytes: Value<'js>, url_safe: Opt<bool>| -> Result<String> {
                let bytes = byte_arg(&ctx, &bytes, "the data")?;
                Ok(if url_safe.0.unwrap_or(false) {
                    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
                } else {
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                })
            },
        )?,
    )?;
    native.set(
        "parseUrl",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, input: String, base: Opt<String>| -> Result<Object<'js>> {
                let parsed = match base.0 {
                    Some(base) => url::Url::parse(&base).and_then(|b| b.join(&input)),
                    None => url::Url::parse(&input),
                };
                let url = parsed.map_err(|e| Exception::throw_type(&ctx, &format!("invalid URL {input:?}: {e}")))?;
                url_parts(&ctx, &url)
            },
        )?,
    )?;
    native.set(
        "setUrl",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, href: String, field: String, value: String| -> Result<Object<'js>> {
                let mut url =
                    url::Url::parse(&href).map_err(|e| Exception::throw_type(&ctx, &format!("invalid URL: {e}")))?;
                set_url_field(&mut url, &field, &value);
                url_parts(&ctx, &url)
            },
        )?,
    )?;
    native.set(
        "digest",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, algorithm: String, data: Value<'js>| -> Result<Value<'js>> {
                let data = byte_arg(&ctx, &data, "the data")?;
                let algorithm = digest_algorithm(&ctx, &algorithm)?;
                uint8(&ctx, ring::digest::digest(algorithm, &data).as_ref())
            },
        )?,
    )?;
    native.set(
        "hmac",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, algorithm: String, key: Value<'js>, data: Value<'js>| -> Result<Value<'js>> {
                let key = byte_arg(&ctx, &key, "the key")?;
                let data = byte_arg(&ctx, &data, "the data")?;
                let algorithm = match algorithm.to_ascii_lowercase().replace('-', "").as_str() {
                    "sha1" => ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
                    "sha256" => ring::hmac::HMAC_SHA256,
                    "sha384" => ring::hmac::HMAC_SHA384,
                    "sha512" => ring::hmac::HMAC_SHA512,
                    _ => {
                        return Err(Exception::throw_type(
                            &ctx,
                            &format!("unknown HMAC algorithm {algorithm:?}; use sha1, sha256, sha384 or sha512"),
                        ));
                    }
                };
                let key = ring::hmac::Key::new(algorithm, &key);
                uint8(&ctx, ring::hmac::sign(&key, &data).as_ref())
            },
        )?,
    )?;
    native.set(
        "randomUUID",
        Function::new(ctx.clone(), || uuid::Uuid::new_v4().to_string())?,
    )?;
    native.set(
        "decompress",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, data: Value<'js>, encoding: String| -> Result<Value<'js>> {
                let data = byte_arg(&ctx, &data, "the data")?;
                let out = crate::io::decode::decode(&encoding.to_ascii_lowercase(), &data)
                    .map_err(|e| Exception::throw_type(&ctx, &e.to_string()))?;
                uint8(&ctx, &out)
            },
        )?,
    )?;
    native.set(
        "console",
        Function::new(ctx.clone(), move |level: String, message: String| {
            let level = match level.as_str() {
                "debug" => ConsoleLevel::Debug,
                "info" => ConsoleLevel::Info,
                "warn" => ConsoleLevel::Warn,
                "error" => ConsoleLevel::Error,
                _ => ConsoleLevel::Log,
            };
            let mut lines = console.borrow_mut();
            match lines.len() {
                n if n < CONSOLE_LINES => lines.push((level, message)),
                n if n == CONSOLE_LINES => lines.push((
                    ConsoleLevel::Warn,
                    format!("(more than {CONSOLE_LINES} lines in one run; the rest are dropped)"),
                )),
                _ => {}
            }
        })?,
    )?;
    Ok(())
}

fn digest_algorithm(ctx: &Ctx<'_>, name: &str) -> Result<&'static ring::digest::Algorithm> {
    Ok(match name.to_ascii_lowercase().replace('-', "").as_str() {
        "sha1" => &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
        "sha256" => &ring::digest::SHA256,
        "sha384" => &ring::digest::SHA384,
        "sha512" => &ring::digest::SHA512,
        _ => return Err(Exception::throw_type(ctx, &format!("unknown hash {name:?}"))),
    })
}

fn url_parts<'js>(ctx: &Ctx<'js>, url: &url::Url) -> Result<Object<'js>> {
    let parts = Object::new(ctx.clone())?;
    parts.set("href", url.as_str())?;
    parts.set("origin", url.origin().ascii_serialization())?;
    parts.set("protocol", format!("{}:", url.scheme()))?;
    parts.set("username", url.username())?;
    parts.set("password", url.password().unwrap_or(""))?;
    let hostname = url.host_str().unwrap_or("");
    let port = url.port().map(|p| p.to_string()).unwrap_or_default();
    parts.set("hostname", hostname)?;
    parts.set("port", port.as_str())?;
    parts.set(
        "host",
        if port.is_empty() {
            hostname.to_owned()
        } else {
            format!("{hostname}:{port}")
        },
    )?;
    parts.set("pathname", url.path())?;
    parts.set("search", url.query().map(|q| format!("?{q}")).unwrap_or_default())?;
    parts.set("hash", url.fragment().map(|f| format!("#{f}")).unwrap_or_default())?;
    Ok(parts)
}

/// WHATWG setters: an invalid value leaves the URL as it was.
fn set_url_field(url: &mut url::Url, field: &str, value: &str) {
    let strip = |prefix: char| value.strip_prefix(prefix).unwrap_or(value);
    let _ = match field {
        "protocol" => url.set_scheme(value.trim_end_matches(':')),
        "username" => url.set_username(value),
        "password" => url.set_password(Some(value).filter(|v| !v.is_empty())),
        "host" => {
            let (host, port) = match value.rsplit_once(':') {
                Some((h, p)) if !h.ends_with(']') || value.starts_with('[') => (h, p.parse().ok()),
                _ => (value, None),
            };
            url.set_host(Some(host))
                .map_err(|_| ())
                .and_then(|()| url.set_port(port))
        }
        "hostname" => url.set_host(Some(value)).map_err(|_| ()),
        "port" => url.set_port(if value.is_empty() { None } else { value.parse().ok() }),
        "pathname" => {
            url.set_path(value);
            Ok(())
        }
        "search" => {
            let q = strip('?');
            url.set_query(if q.is_empty() { None } else { Some(q) });
            Ok(())
        }
        "hash" => {
            let f = strip('#');
            url.set_fragment(if f.is_empty() { None } else { Some(f) });
            Ok(())
        }
        _ => Ok(()),
    };
}
