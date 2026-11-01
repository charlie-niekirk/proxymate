//! OpenAPI import (M20): 3.0 and 3.1 specs, JSON or YAML, stored as their
//! source text in `app.sqlite`'s `openapi_specs` table and compiled into
//! an [`OpIndex`] at load.
//!
//! A flow whose method and path match an operation (under one of the
//! spec's server base paths) shows its `operationId` and summary in the
//! inspector, and the filter DSL's `op:` term finds them. The host is not
//! compared: the same API usually runs on several (local, staging,
//! production).
//!
//! "Generate mocks" drafts one disabled Map Local rule per operation from
//! its response example, or a minimal instance of its response schema.
//! Only local `$ref`s (`#/…`) are followed; validating responses against
//! the schema is a follow-up.

use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use pm_model::{
    Action, MapLocalSource, Matcher, OpenApiSpec, OperationMatch, Pattern, PresetKind, Rule, RuleOrigin, StoreId,
};
use regex::bytes::Regex;
use rusqlite::params;
use serde_json::{Map, Value};

use crate::rules::compile::{Use, compile_pattern};
use crate::rules::db::AppDb;
use crate::{Engine, Error};

/// Specs larger than this aren't imported.
pub const SPEC_MAX_BYTES: usize = 16 * 1024 * 1024;
/// How deep `$ref`s and nested schemas are followed.
const MAX_DEPTH: usize = 24;
const METHODS: [&str; 8] = ["get", "put", "post", "delete", "options", "head", "patch", "trace"];

pub(crate) struct OpenApi {
    db: Mutex<AppDb>,
    state: ArcSwap<OpIndex>,
}

/// Every imported spec, compiled. A snapshot: filters bind to the one
/// current when they are set.
#[derive(Default)]
pub struct OpIndex {
    specs: Vec<Compiled>,
    /// Every operation, most specific path first.
    ops: Vec<Op>,
}

struct Compiled {
    meta: OpenApiSpec,
    doc: Value,
}

/// One operation of one spec.
pub(crate) struct Op {
    spec: u32,
    /// Index into `OpIndex::specs`.
    spec_index: usize,
    /// Upper case.
    method: String,
    template: String,
    pub operation_id: Option<String>,
    summary: Option<String>,
    /// The template under each base path, anchored.
    path: Regex,
    /// The same as a rule's path pattern.
    pattern: String,
    /// Literal path segments: `/pets/mine` beats `/pets/{id}`.
    literal: usize,
}

impl OpIndex {
    /// The operation `method path` is, if any.
    pub(crate) fn find(&self, method: &str, path: &str) -> Option<&Op> {
        self.ops
            .iter()
            .find(|op| op.method.eq_ignore_ascii_case(method) && op.path.is_match(path.as_bytes()))
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// The operations whose `operationId` passes `keep` (`None` without one).
    pub(crate) fn ops_where(&self, keep: impl Fn(Option<&str>) -> bool) -> Vec<usize> {
        (0..self.ops.len())
            .filter(|&i| keep(self.ops[i].operation_id.as_deref()))
            .collect()
    }

    /// Whether operation `i` covers `method path`.
    pub(crate) fn op_matches(&self, i: usize, method: &str, path: &str) -> bool {
        let op = &self.ops[i];
        op.method.eq_ignore_ascii_case(method) && op.path.is_match(path.as_bytes())
    }

    fn matched(&self, op: &Op) -> OperationMatch {
        OperationMatch {
            spec_id: op.spec,
            spec_name: self.specs[op.spec_index].meta.name.clone(),
            operation_id: op.operation_id.clone(),
            summary: op.summary.clone(),
            method: op.method.clone(),
            path: op.template.clone(),
        }
    }
}

impl OpenApi {
    pub fn load(dir: Option<&Path>) -> OpenApi {
        let db = dir
            .map(AppDb::open)
            .unwrap_or_else(AppDb::in_memory)
            .or_else(|err| {
                eprintln!("proxymate: can't open the app database ({err}); OpenAPI specs won't be saved");
                AppDb::in_memory()
            })
            .expect("in-memory SQLite");
        if let Err(err) = db.conn().execute_batch(
            "CREATE TABLE IF NOT EXISTS openapi_specs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                source TEXT NOT NULL,
                added_at_ms INTEGER NOT NULL
            );",
        ) {
            eprintln!("proxymate: can't create the OpenAPI spec table: {err}");
        }
        let index = build(&db).unwrap_or_else(|err| {
            eprintln!("proxymate: can't read OpenAPI specs: {err}");
            OpIndex::default()
        });
        OpenApi {
            db: Mutex::new(db),
            state: ArcSwap::from_pointee(index),
        }
    }

    pub fn snapshot(&self) -> Arc<OpIndex> {
        self.state.load_full()
    }
}

fn build(db: &AppDb) -> rusqlite::Result<OpIndex> {
    let mut stmt = db
        .conn()
        .prepare("SELECT id, name, source, added_at_ms FROM openapi_specs ORDER BY id")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, u32>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut index = OpIndex::default();
    for row in rows {
        let (id, name, source, added_at_ms) = row?;
        match parse(&source) {
            Ok(doc) => add(&mut index, id, &name, added_at_ms as f64, doc),
            Err(err) => eprintln!("proxymate: skipping OpenAPI spec {name:?}: {err}"),
        }
    }
    index.ops.sort_by_key(|o| std::cmp::Reverse(o.literal));
    Ok(index)
}

/// Parses a spec (JSON, or YAML) and checks it is OpenAPI 3.0 or 3.1.
pub(crate) fn parse(source: &str) -> Result<Value, String> {
    let trimmed = source.trim_start_matches('\u{feff}').trim_start();
    let doc: Value = if trimmed.starts_with('{') {
        serde_json::from_str(trimmed).map_err(|e| format!("not valid JSON: {e}"))?
    } else {
        serde_saphyr::from_str(trimmed).map_err(|e| format!("not valid YAML: {e}"))?
    };
    if doc.get("swagger").is_some() {
        return Err("Swagger 2.0 isn't supported; convert it to OpenAPI 3 first".into());
    }
    let version = doc.get("openapi").and_then(scalar).unwrap_or_default();
    if !(version.starts_with("3.0") || version.starts_with("3.1")) {
        return Err(match version.is_empty() {
            true => "not an OpenAPI document: it has no `openapi` version".into(),
            false => format!("OpenAPI {version} isn't supported (3.0 and 3.1 are)"),
        });
    }
    if !doc.get("paths").is_none_or(Value::is_object) {
        return Err("`paths` isn't an object".into());
    }
    Ok(doc)
}

/// A string, or a number YAML read as one (`openapi: 3.0`).
fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn add(index: &mut OpIndex, id: u32, fallback_name: &str, added_at_ms: f64, doc: Value) {
    let info = doc.get("info");
    let text = |key: &str| info.and_then(|i| i.get(key)).and_then(scalar).filter(|s| !s.is_empty());
    let servers = server_urls(&doc);
    let bases = base_paths(&servers);
    let spec_index = index.specs.len();
    let mut count = 0;
    if let Some(paths) = doc.get("paths").and_then(Value::as_object) {
        for (template, item) in paths {
            if !template.starts_with('/') {
                continue;
            }
            let item = resolve(&doc, item, 0);
            let Some(item) = item.as_object() else { continue };
            for method in METHODS {
                let Some(op) = item.get(method).and_then(Value::as_object) else {
                    continue;
                };
                let source = path_regex(&bases, template);
                let Ok(path) = compile_pattern(&Pattern::regex(source.clone()), Use::Match, false) else {
                    continue;
                };
                count += 1;
                index.ops.push(Op {
                    spec: id,
                    spec_index,
                    method: method.to_ascii_uppercase(),
                    template: template.clone(),
                    operation_id: op.get("operationId").and_then(scalar),
                    summary: op.get("summary").and_then(scalar),
                    path,
                    pattern: source,
                    literal: template
                        .split('/')
                        .filter(|s| !s.is_empty() && !s.contains('{'))
                        .count(),
                });
            }
        }
    }
    index.specs.push(Compiled {
        meta: OpenApiSpec {
            id,
            name: text("title").unwrap_or_else(|| fallback_name.to_owned()),
            version: text("version").unwrap_or_default(),
            openapi: doc.get("openapi").and_then(scalar).unwrap_or_default(),
            operations: count,
            servers,
            added_at_ms,
        },
        doc,
    });
}

/// `servers[].url` with their variables' defaults filled in.
fn server_urls(doc: &Value) -> Vec<String> {
    let Some(servers) = doc.get("servers").and_then(Value::as_array) else {
        return Vec::new();
    };
    servers
        .iter()
        .filter_map(|s| {
            let mut url = s.get("url").and_then(Value::as_str)?.to_owned();
            if let Some(vars) = s.get("variables").and_then(Value::as_object) {
                for (name, var) in vars {
                    if let Some(default) = var.get("default").and_then(scalar) {
                        url = url.replace(&format!("{{{name}}}"), &default);
                    }
                }
            }
            Some(url)
        })
        .collect()
}

/// The path part of each server URL, without a trailing `/`; no servers
/// (or only `/`) is the root.
fn base_paths(servers: &[String]) -> Vec<String> {
    let mut bases: Vec<String> = servers
        .iter()
        .map(|url| {
            let path = match url.split_once("://") {
                Some((_, rest)) => rest.find('/').map_or("", |i| &rest[i..]),
                None => url.as_str(),
            };
            path.split(['?', '#'])
                .next()
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_owned()
        })
        .collect();
    if bases.is_empty() {
        bases.push(String::new());
    }
    bases.sort();
    bases.dedup();
    bases
}

/// The one host the spec's absolute servers name, if they agree (a
/// templated host agrees with nothing).
fn server_host(servers: &[String]) -> Option<String> {
    let mut hosts = servers.iter().filter_map(|url| {
        let rest = url.split_once("://")?.1;
        let authority = &rest[..rest.find('/').unwrap_or(rest.len())];
        let host = authority.rsplit('@').next().unwrap_or_default();
        let host = match host.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or_default(),
            None => host.split(':').next().unwrap_or_default(),
        };
        Some((!host.is_empty() && !host.contains('{')).then(|| host.to_ascii_lowercase()))
    });
    let first = hosts.next()??;
    hosts.all(|h| h.as_ref() == Some(&first)).then_some(first)
}

/// `^(?:/v1|/v2)/pets/[^/]+/?$` for `/pets/{petId}` under `/v1` and `/v2`.
fn path_regex(bases: &[String], template: &str) -> String {
    let mut path = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        path.push_str(&regex::escape(&rest[..open]));
        match rest[open..].find('}') {
            Some(close) => {
                path.push_str("[^/]+");
                rest = &rest[open + close + 1..];
            }
            None => {
                rest = &rest[open..];
                break;
            }
        }
    }
    path.push_str(&regex::escape(rest));
    let path = path.trim_end_matches('/');
    let bases: Vec<String> = bases.iter().map(|b| regex::escape(b)).collect();
    format!("^(?:{}){path}/?$", bases.join("|"))
}

/// Follows a local `$ref` (`#/components/…`), up to [`MAX_DEPTH`] times.
fn resolve<'a>(doc: &'a Value, value: &'a Value, depth: usize) -> &'a Value {
    let mut value = value;
    for _ in depth..MAX_DEPTH {
        match value.get("$ref").and_then(Value::as_str) {
            Some(r) if r.starts_with('#') => match doc.pointer(&r[1..]) {
                Some(target) => value = target,
                None => return &Value::Null,
            },
            _ => return value,
        }
    }
    &Value::Null
}

/// The status, content type and body a mock of `op` answers with.
fn mock_response(doc: &Value, op: &Map<String, Value>) -> (u16, Option<String>, String) {
    let responses = op.get("responses").and_then(Value::as_object);
    let mut codes: Vec<(&String, &Value)> = responses.map(|r| r.iter().collect()).unwrap_or_default();
    // The first success, else `default`, else whatever comes first.
    codes.sort_by_key(|(code, _)| match code.parse::<u16>() {
        Ok(c @ 200..=299) => (0, c),
        Ok(c) => (2, c),
        Err(_) if code.eq_ignore_ascii_case("2XX") => (0, 299),
        Err(_) if code.as_str() == "default" => (1, 0),
        Err(_) => (3, 0),
    });
    let Some((code, response)) = codes.first() else {
        return (200, None, String::new());
    };
    let status = match code.parse::<u16>() {
        Ok(c) if (100..=599).contains(&c) => c,
        _ => 200,
    };
    let response = resolve(doc, response, 0);
    let Some(content) = response.get("content").and_then(Value::as_object) else {
        return (status, None, String::new());
    };
    let json_like = |m: &str| {
        let m = m.to_ascii_lowercase();
        m == "application/json" || m.ends_with("+json") || m.contains("/json")
    };
    let (media_type, media) = content
        .iter()
        .find(|(m, _)| m.eq_ignore_ascii_case("application/json"))
        .or_else(|| content.iter().find(|(m, _)| json_like(m)))
        .or_else(|| content.iter().next())
        .expect("content isn't empty");
    let media = resolve(doc, media, 0);
    let value = media
        .get("example")
        .cloned()
        .or_else(|| {
            let examples = media.get("examples").and_then(Value::as_object)?;
            let first = resolve(doc, examples.values().next()?, 0);
            first.get("value").cloned()
        })
        .or_else(|| media.get("schema").map(|s| instance(doc, s, 0)));
    let content_type = (!media_type.contains('*')).then(|| media_type.clone());
    let body = match value {
        None => String::new(),
        Some(Value::String(s)) if !json_like(media_type) => s,
        Some(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
    };
    (status, content_type, body)
}

/// A minimal value `schema` accepts: its example or default when it has
/// one, else required properties only, `minItems` items, and the lowest
/// value of each type.
fn instance(doc: &Value, schema: &Value, depth: usize) -> Value {
    if depth >= MAX_DEPTH {
        return Value::Null;
    }
    let schema = resolve(doc, schema, depth);
    let Some(obj) = schema.as_object() else {
        return Value::Null;
    };
    if let Some(v) = obj
        .get("example")
        .or_else(|| obj.get("const"))
        .or_else(|| obj.get("default"))
    {
        return v.clone();
    }
    if let Some(v) = obj.get("examples").and_then(Value::as_array).and_then(|e| e.first()) {
        return v.clone();
    }
    if let Some(v) = obj.get("enum").and_then(Value::as_array).and_then(|e| e.first()) {
        return v.clone();
    }
    if let Some(all) = obj.get("allOf").and_then(Value::as_array) {
        let mut merged = Map::new();
        for part in all {
            match instance(doc, part, depth + 1) {
                Value::Object(m) => merged.extend(m),
                other if all.len() == 1 => return other,
                _ => {}
            }
        }
        return Value::Object(merged);
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(first) = obj.get(key).and_then(Value::as_array).and_then(|a| a.first()) {
            return instance(doc, first, depth + 1);
        }
    }
    let ty = match obj.get("type") {
        Some(Value::String(t)) => t.clone(),
        // 3.1: `type: [string, "null"]`.
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .unwrap_or("null")
            .to_owned(),
        _ if obj.contains_key("properties") => "object".into(),
        _ if obj.contains_key("items") => "array".into(),
        _ => return Value::Object(Map::new()),
    };
    let number = |key: &str| obj.get(key).and_then(Value::as_f64);
    match ty.as_str() {
        "object" => {
            let props = obj.get("properties").and_then(Value::as_object);
            let mut out = Map::new();
            for name in obj.get("required").and_then(Value::as_array).into_iter().flatten() {
                let Some(name) = name.as_str() else { continue };
                let value = props
                    .and_then(|p| p.get(name))
                    .map_or(Value::Null, |p| instance(doc, p, depth + 1));
                out.insert(name.to_owned(), value);
            }
            Value::Object(out)
        }
        "array" => {
            let n = number("minItems").unwrap_or(0.0).clamp(0.0, 16.0) as usize;
            let item = obj.get("items").map_or(Value::Null, |i| instance(doc, i, depth + 1));
            Value::Array(vec![item; n])
        }
        "string" => {
            let s = match obj.get("format").and_then(Value::as_str).unwrap_or_default() {
                "date-time" => "1970-01-01T00:00:00Z",
                "date" => "1970-01-01",
                "time" => "00:00:00",
                "uuid" => "00000000-0000-0000-0000-000000000000",
                "email" => "user@example.com",
                "uri" | "url" => "https://example.com/",
                "hostname" => "example.com",
                "ipv4" => "192.0.2.1",
                "ipv6" => "2001:db8::1",
                "byte" | "binary" => "",
                _ => "string",
            };
            let min = number("minLength").unwrap_or(0.0).clamp(0.0, 256.0) as usize;
            let mut s = s.to_owned();
            while s.chars().count() < min {
                s.push('x');
            }
            Value::String(s)
        }
        "integer" | "number" => {
            let low = match (obj.get("minimum"), obj.get("exclusiveMinimum")) {
                // 3.1: a number; 3.0: a flag on `minimum`.
                (_, Some(Value::Number(n))) => n.as_f64().map(|n| n + 1.0),
                (Some(Value::Number(n)), Some(Value::Bool(true))) => n.as_f64().map(|n| n + 1.0),
                (Some(Value::Number(n)), _) => n.as_f64(),
                _ => None,
            };
            let high = number("maximum");
            let v = match (low, high) {
                (Some(l), _) => l,
                (None, Some(h)) if h < 0.0 => h,
                _ => 0.0,
            };
            if ty == "integer" {
                Value::from(v.ceil() as i64)
            } else {
                serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
            }
        }
        "boolean" => Value::Bool(false),
        _ => Value::Null,
    }
}

impl Engine {
    /// Every imported OpenAPI spec, oldest first.
    pub fn openapi_specs(&self) -> Vec<OpenApiSpec> {
        self.openapi.snapshot().specs.iter().map(|s| s.meta.clone()).collect()
    }

    /// Imports an OpenAPI 3.0 or 3.1 file (JSON or YAML).
    pub fn import_openapi_file(&self, path: &Path) -> Result<OpenApiSpec, Error> {
        let meta = std::fs::metadata(path)?;
        if meta.len() > SPEC_MAX_BYTES as u64 {
            return Err(Error::Invalid(format!(
                "the spec is larger than {} MB",
                SPEC_MAX_BYTES / 1024 / 1024
            )));
        }
        let source =
            std::fs::read_to_string(path).map_err(|e| Error::Invalid(format!("can't read {}: {e}", path.display())))?;
        let name = path
            .file_stem()
            .map_or_else(|| "spec".to_owned(), |s| s.to_string_lossy().into_owned());
        self.import_openapi(&name, &source)
    }

    /// Imports a spec from its text; `name` is used when it has no title.
    pub fn import_openapi(&self, name: &str, source: &str) -> Result<OpenApiSpec, Error> {
        if source.len() > SPEC_MAX_BYTES {
            return Err(Error::Invalid(format!(
                "the spec is larger than {} MB",
                SPEC_MAX_BYTES / 1024 / 1024
            )));
        }
        parse(source).map_err(Error::Invalid)?;
        let db = self.openapi.db.lock();
        db.conn()
            .execute(
                "INSERT INTO openapi_specs (name, source, added_at_ms) VALUES (?1, ?2, ?3)",
                params![name, source, crate::proxy::now_ms() as i64],
            )
            .map_err(|e| Error::Invalid(format!("can't save the spec: {e}")))?;
        let id = db.conn().last_insert_rowid() as u32;
        self.reload_openapi(&db)?;
        drop(db);
        self.openapi_specs()
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| Error::Invalid("the spec didn't save".into()))
    }

    pub fn remove_openapi_spec(&self, id: u32) -> Result<(), Error> {
        let db = self.openapi.db.lock();
        let removed = db
            .conn()
            .execute("DELETE FROM openapi_specs WHERE id = ?1", [id])
            .map_err(|e| Error::Invalid(e.to_string()))?;
        if removed == 0 {
            return Err(Error::Invalid(format!("there is no OpenAPI spec {id}")));
        }
        self.reload_openapi(&db)
    }

    fn reload_openapi(&self, db: &AppDb) -> Result<(), Error> {
        let index = Arc::new(build(db).map_err(|e| Error::Invalid(e.to_string()))?);
        self.openapi.state.store(index.clone());
        self.set_store_operations(index);
        Ok(())
    }

    /// The operations snapshot new filters bind to.
    pub(crate) fn operations(&self) -> Arc<OpIndex> {
        self.openapi.snapshot()
    }

    /// The spec operation a flow is, if any.
    pub fn flow_operation_in(&self, store: StoreId, flow: pm_model::FlowId) -> Result<Option<OperationMatch>, Error> {
        let summary = self.get_flow_in(store, flow).ok_or(Error::UnknownFlow(flow))?;
        let index = self.openapi.snapshot();
        Ok(index.find(&summary.method, &summary.path).map(|op| index.matched(op)))
    }

    /// "Generate mocks": saves one Map Local rule per operation of spec
    /// `id`, disabled until reviewed, and returns them.
    pub fn generate_openapi_mocks(&self, id: u32) -> Result<Vec<Rule>, Error> {
        let index = self.openapi.snapshot();
        let spec = index
            .specs
            .iter()
            .position(|s| s.meta.id == id)
            .ok_or_else(|| Error::Invalid(format!("there is no OpenAPI spec {id}")))?;
        let compiled = &index.specs[spec];
        let host = server_host(&compiled.meta.servers);
        let mut ops: Vec<&Op> = index.ops.iter().filter(|o| o.spec_index == spec).collect();
        // In the spec's order, not the matcher's.
        ops.sort_by_key(|o| {
            let paths = compiled.doc.get("paths").and_then(Value::as_object);
            let at = paths
                .and_then(|p| p.keys().position(|k| *k == o.template))
                .unwrap_or(usize::MAX);
            (at, METHODS.iter().position(|m| m.eq_ignore_ascii_case(&o.method)))
        });
        let drafts: Vec<Rule> = ops
            .into_iter()
            .map(|op| {
                let item = compiled
                    .doc
                    .get("paths")
                    .and_then(|p| p.get(&op.template))
                    .map(|i| resolve(&compiled.doc, i, 0));
                let object = item
                    .and_then(|i| i.get(op.method.to_ascii_lowercase()))
                    .and_then(Value::as_object);
                let (status, content_type, body) =
                    object
                        .map(|o| mock_response(&compiled.doc, o))
                        .unwrap_or((200, None, String::new()));
                let label = op
                    .operation_id
                    .clone()
                    .unwrap_or_else(|| format!("{} {}", op.method, op.template));
                Rule {
                    id: String::new(),
                    name: format!("{label} · {}", compiled.meta.name),
                    enabled: false,
                    order: 0,
                    origin: RuleOrigin::Preset {
                        preset: PresetKind::OpenApi,
                    },
                    matcher: Matcher {
                        methods: vec![op.method.clone()],
                        host: host.clone().map(Pattern::exact),
                        path: Some(Pattern::regex(op.pattern.clone())),
                        ..Matcher::default()
                    },
                    actions: vec![Action::MapLocal {
                        source: MapLocalSource::Inline { body },
                        status,
                        headers: Vec::new(),
                        content_type,
                    }],
                }
            })
            .collect();
        self.create_rules(drafts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PETSTORE: &str = include_str!("../tests/fixtures/openapi/petstore.yaml");

    fn index_of(source: &str) -> OpIndex {
        let mut index = OpIndex::default();
        add(&mut index, 1, "spec", 0.0, parse(source).unwrap());
        index.ops.sort_by_key(|o| std::cmp::Reverse(o.literal));
        index
    }

    #[test]
    fn templates_match_under_the_base_path() {
        let index = index_of(PETSTORE);
        let op = |m: &str, p: &str| index.find(m, p).and_then(|o| o.operation_id.clone());
        assert_eq!(op("GET", "/v1/pets").as_deref(), Some("listPets"));
        assert_eq!(op("get", "/v1/pets/").as_deref(), Some("listPets"));
        assert_eq!(op("POST", "/v1/pets").as_deref(), Some("createPets"));
        assert_eq!(op("GET", "/v1/pets/42").as_deref(), Some("showPetById"));
        assert_eq!(op("GET", "/v1/pets/42/toys"), None);
        assert_eq!(op("DELETE", "/v1/pets"), None);
        assert_eq!(op("GET", "/pets"), None, "the server's base path is required");
    }

    #[test]
    fn literal_paths_win_over_templates() {
        let index = index_of(
            r#"{"openapi":"3.1.0","info":{"title":"t","version":"1"},"paths":{
                "/pets/{id}":{"get":{"operationId":"byId","responses":{}}},
                "/pets/mine":{"get":{"operationId":"mine","responses":{}}}}}"#,
        );
        assert_eq!(
            index.find("GET", "/pets/mine").unwrap().operation_id.as_deref(),
            Some("mine")
        );
        assert_eq!(
            index.find("GET", "/pets/7").unwrap().operation_id.as_deref(),
            Some("byId")
        );
    }

    #[test]
    fn other_versions_are_refused() {
        assert!(parse(r#"{"swagger":"2.0"}"#).unwrap_err().contains("Swagger 2.0"));
        assert!(parse("openapi: 2.5\n").unwrap_err().contains("isn't supported"));
        assert!(parse("title: x\n").unwrap_err().contains("no `openapi`"));
        assert!(parse("openapi: 3.0.3\npaths: {}\n").is_ok());
    }

    #[test]
    fn examples_win_then_minimal_instances() {
        let doc = parse(PETSTORE).unwrap();
        let op = |path: &str, method: &str| mock_response(&doc, doc["paths"][path][method].as_object().unwrap());
        let (status, ct, body) = op("/pets", "get");
        assert_eq!((status, ct.as_deref()), (200, Some("application/json")));
        let pets: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(pets[0]["name"], "Rex");
        // No example: the schema's required properties.
        let (status, _, body) = op("/pets/{petId}", "get");
        assert_eq!(status, 200);
        let pet: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(pet, serde_json::json!({"id": 0, "name": "string"}));
        // 201 with no content.
        assert_eq!(op("/pets", "post"), (201, None, String::new()));
    }

    #[test]
    fn instances_follow_the_schema_keywords() {
        let doc = serde_json::json!({"components": {"schemas": {"Id": {"type": "string", "format": "uuid"}}}});
        let schema = serde_json::json!({
            "type": "object",
            "required": ["id", "tags", "n", "kind", "when", "maybe", "both"],
            "properties": {
                "id": {"$ref": "#/components/schemas/Id"},
                "tags": {"type": "array", "minItems": 2, "items": {"type": "string", "minLength": 8}},
                "n": {"type": "integer", "minimum": 5, "exclusiveMinimum": true},
                "kind": {"enum": ["a", "b"]},
                "when": {"type": "string", "format": "date-time"},
                "maybe": {"type": ["null", "boolean"]},
                "both": {"allOf": [{"type": "object", "required": ["a"], "properties": {"a": {"type": "number"}}},
                                    {"type": "object", "required": ["b"], "properties": {"b": {"default": 3}}}]}
            }
        });
        assert_eq!(
            instance(&doc, &schema, 0),
            serde_json::json!({
                "id": "00000000-0000-0000-0000-000000000000",
                "tags": ["stringxx", "stringxx"],
                "n": 6,
                "kind": "a",
                "when": "1970-01-01T00:00:00Z",
                "maybe": false,
                "both": {"a": 0.0, "b": 3}
            })
        );
    }

    #[test]
    fn recursive_refs_stop() {
        let doc = serde_json::json!({"components": {"schemas": {"Node": {
            "type": "object", "required": ["next"], "properties": {"next": {"$ref": "#/components/schemas/Node"}}
        }}}});
        let v = instance(&doc, &serde_json::json!({"$ref": "#/components/schemas/Node"}), 0);
        assert!(v.get("next").is_some());
    }

    #[test]
    fn servers_give_base_paths_and_a_host() {
        let servers = vec![
            "https://{env}.example.com/api/v2/".to_owned(),
            "http://localhost:8080/api/v2".to_owned(),
        ];
        assert_eq!(base_paths(&servers), vec!["/api/v2".to_owned()]);
        assert_eq!(server_host(&servers), None);
        assert_eq!(
            server_host(&["https://a.test/v1".to_owned()]).as_deref(),
            Some("a.test")
        );
        assert_eq!(base_paths(&[]), vec![String::new()]);
        assert_eq!(path_regex(&[String::new()], "/a/{b}.json"), r"^(?:)/a/[^/]+\.json/?$");
    }
}
