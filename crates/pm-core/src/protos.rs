//! Protobuf schemas and decoded bodies (M16).
//!
//! Schemas are `.proto` files compiled in Rust by `protox` (no `protoc`),
//! or `FileDescriptorSet`s from `buf build -o` / `protoc
//! --descriptor_set_out`. Each is stored compiled, imports included, in
//! `app.sqlite`'s `proto_schemas` table; together they make one
//! `prost-reflect` pool. "Decode as…" mappings (a `Matcher`, a part and a
//! message type) are a settings row.
//!
//! [`Engine::decode_body_in`] turns a gRPC, protobuf, MessagePack or CBOR
//! body into JSON trees for the inspector and MCP alike. A gRPC path
//! (`/pkg.Service/Method`) finds its own message types; anything without
//! a schema decodes schema-less. Results are cached by flow, part, bytes
//! seen and schema generation, so a live stream re-decodes as it grows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;
use moka::sync::Cache;
use parking_lot::Mutex;
use pm_model::{
    Badge, BodyPart, ContentKind, DecodedBody, DecodedFormat, DecodedMessage, FlowDetail, FlowId, FlowSummary, Header,
    ProtoMapping, ProtoSchema, Rule, RuleOrigin, StoreId,
};
use prost::Message as _;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, SerializeOptions};
use rusqlite::params;

use crate::io::content::kind_from_content_type;
use crate::io::grpc::{self, Flavor};
use crate::io::{objects, wire};
use crate::mocks::MockMap;
use crate::rules::compile::{CompiledRule, RequestInfo};
use crate::rules::db::AppDb;
use crate::rules::eval::header_map;
use crate::scripts::ScriptMap;
use crate::{Engine, Error};

const MAPPINGS_KEY: &str = "proto_mappings";
/// Bytes of decoded bodies kept.
const CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// Bodies past this are decoded only up to it.
const DECODE_MAX: usize = 64 * 1024 * 1024;

pub(crate) struct Protos {
    db: Mutex<AppDb>,
    state: ArcSwap<Compiled>,
    generation: AtomicU64,
    cache: Cache<CacheKey, Arc<DecodedBody>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    store: StoreId,
    flow: FlowId,
    part: BodyPart,
    /// Bytes recorded so far: a live stream keys a new entry as it grows.
    size: u64,
    generation: u64,
}

struct Compiled {
    generation: u64,
    pool: DescriptorPool,
    schemas: Vec<ProtoSchema>,
    mappings: Vec<(ProtoMapping, CompiledRule)>,
}

/// One stored schema row.
struct Stored {
    meta: ProtoSchema,
    descriptor: Vec<u8>,
}

impl Protos {
    pub fn load(dir: Option<&Path>) -> Protos {
        let db = dir
            .map(AppDb::open)
            .unwrap_or_else(AppDb::in_memory)
            .or_else(|err| {
                eprintln!("proxymate: can't open the app database ({err}); proto schemas won't be saved");
                AppDb::in_memory()
            })
            .expect("in-memory SQLite");
        if let Err(err) = db.conn().execute_batch(
            "CREATE TABLE IF NOT EXISTS proto_schemas (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                files TEXT NOT NULL,
                descriptor BLOB NOT NULL,
                added_at_ms INTEGER NOT NULL
            );",
        ) {
            eprintln!("proxymate: can't create the proto schema table: {err}");
        }
        let stored = read_schemas(&db).unwrap_or_else(|err| {
            eprintln!("proxymate: can't read proto schemas: {err}");
            Vec::new()
        });
        let mappings: Vec<ProtoMapping> = db.setting(MAPPINGS_KEY).unwrap_or_default();
        let compiled = compile(&stored, mappings, 0).unwrap_or_else(|err| {
            eprintln!("proxymate: proto schemas don't load together ({err}); decoding schema-less");
            Compiled {
                generation: 0,
                pool: DescriptorPool::new(),
                schemas: stored.iter().map(|s| s.meta.clone()).collect(),
                mappings: Vec::new(),
            }
        });
        Protos {
            db: Mutex::new(db),
            state: ArcSwap::from_pointee(compiled),
            generation: AtomicU64::new(0),
            cache: Cache::builder()
                .max_capacity(CACHE_BYTES)
                .weigher(|_, body: &Arc<DecodedBody>| {
                    let bytes: usize = body.messages.iter().map(|m| m.json.len() + m.badges.len() * 32).sum();
                    u32::try_from(bytes + 256).unwrap_or(u32::MAX)
                })
                .build(),
        }
    }

    /// Recompiles from the database after a change, and drops cached
    /// decodings (the generation bumps).
    fn reload(&self, db: &AppDb) -> Result<(), Error> {
        let stored = read_schemas(db).map_err(|e| Error::Proto(e.to_string()))?;
        let mappings: Vec<ProtoMapping> = db.setting(MAPPINGS_KEY).unwrap_or_default();
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let compiled = compile(&stored, mappings, generation).map_err(Error::Proto)?;
        self.state.store(Arc::new(compiled));
        Ok(())
    }
}

fn read_schemas(db: &AppDb) -> rusqlite::Result<Vec<Stored>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT id, name, files, descriptor, added_at_ms FROM proto_schemas ORDER BY id")?;
    let rows = stmt.query_map([], |row| {
        let files: String = row.get(2)?;
        Ok(Stored {
            meta: ProtoSchema {
                id: row.get(0)?,
                name: row.get(1)?,
                files: serde_json::from_str(&files).unwrap_or_default(),
                services: Vec::new(),
                messages: 0,
                added_at_ms: row.get::<_, i64>(4)? as u64,
            },
            descriptor: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// One pool from every schema (shared imports are added once), plus each
/// schema's services and message count.
fn compile(stored: &[Stored], mappings: Vec<ProtoMapping>, generation: u64) -> Result<Compiled, String> {
    let mut pool = DescriptorPool::new();
    let mut schemas = Vec::with_capacity(stored.len());
    for s in stored {
        pool.decode_file_descriptor_set(s.descriptor.as_slice())
            .map_err(|e| format!("schema “{}”: {e}", s.meta.name))?;
        let mut meta = s.meta.clone();
        let (services, messages) = summarize(&pool, &meta.files);
        meta.services = services;
        meta.messages = messages;
        schemas.push(meta);
    }
    let mappings = mappings
        .into_iter()
        .filter_map(|m| {
            CompiledRule::compile(&mapping_rule(&m), &ScriptMap::new(), &MockMap::new())
                .ok()
                .map(|c| (m, c))
        })
        .collect();
    Ok(Compiled {
        generation,
        pool,
        schemas,
        mappings,
    })
}

/// A mapping's matcher, compiled through the rules compiler.
fn mapping_rule(m: &ProtoMapping) -> Rule {
    Rule {
        id: m.id.clone(),
        name: m.message_type.clone(),
        enabled: true,
        order: 0,
        origin: RuleOrigin::User,
        matcher: m.matcher.clone(),
        actions: Vec::new(),
    }
}

fn summarize(pool: &DescriptorPool, files: &[String]) -> (Vec<String>, u32) {
    let mut services = Vec::new();
    let mut messages = 0u32;
    for file in pool.files().filter(|f| files.iter().any(|n| n == f.name())) {
        services.extend(file.services().map(|s| s.full_name().to_owned()));
        let mut stack: Vec<MessageDescriptor> = file.messages().collect();
        while let Some(m) = stack.pop() {
            if !m.is_map_entry() {
                messages += 1;
            }
            stack.extend(m.child_messages());
        }
    }
    services.sort();
    (services, messages)
}

/// A `.proto` file given by content, for imports from memory (tests, MCP).
struct MemoryFiles(HashMap<String, String>);

impl protox::file::FileResolver for MemoryFiles {
    fn open_file(&self, name: &str) -> Result<protox::file::File, protox::Error> {
        match self.0.get(name) {
            Some(source) => protox::file::File::from_source(name, source),
            None => Err(protox::Error::file_not_found(name)),
        }
    }
}

/// Compiles `.proto` sources by name, imports resolved among them, then
/// in `includes`, then the well-known types.
fn compile_sources(files: &[(String, String)], includes: &[PathBuf]) -> Result<(Vec<String>, Vec<u8>), String> {
    let mut resolver = protox::file::ChainFileResolver::new();
    resolver.add(MemoryFiles(files.iter().cloned().collect()));
    for dir in includes {
        resolver.add(protox::file::IncludeFileResolver::new(dir.clone()));
    }
    resolver.add(protox::file::GoogleFileResolver::new());
    let mut compiler = protox::Compiler::with_file_resolver(resolver);
    compiler.include_imports(true);
    let names: Vec<String> = files.iter().map(|(name, _)| name.clone()).collect();
    compiler.open_files(&names).map_err(|e| match e.file() {
        Some(file) if !e.to_string().contains(file) => format!("{file}: {e}"),
        _ => e.to_string(),
    })?;
    Ok((names, compiler.encode_file_descriptor_set()))
}

/// The files a descriptor set defines that aren't well-known types.
fn descriptor_files(bytes: &[u8]) -> Result<Vec<String>, String> {
    let set = prost_types::FileDescriptorSet::decode(bytes).map_err(|e| format!("not a FileDescriptorSet: {e}"))?;
    if set.file.is_empty() {
        return Err("the descriptor set has no files".into());
    }
    Ok(set
        .file
        .iter()
        .filter_map(|f| f.name.clone())
        .filter(|n| !n.starts_with("google/protobuf/"))
        .collect())
}

/// What a `.proto` import is made of.
pub enum ProtoImport {
    /// `.proto` sources by import name (`shop/v1/order.proto`).
    Sources(Vec<(String, String)>),
    /// An encoded `FileDescriptorSet`.
    DescriptorSet(Vec<u8>),
}

impl Engine {
    /// Every imported schema.
    pub fn proto_schemas(&self) -> Vec<ProtoSchema> {
        self.protos.state.load().schemas.clone()
    }

    /// Imports a schema. It must load alongside the ones already imported
    /// (a file of the same name with different content is refused).
    pub fn import_proto_schema(&self, name: &str, import: ProtoImport) -> Result<ProtoSchema, Error> {
        self.import_schema(name, import, &[])
    }

    /// Imports `.proto` files, or one descriptor set (`.pb`, `.binpb`,
    /// `.desc`, `.protoset`), from disk. Imports resolve against the
    /// picked files and their folders.
    pub fn import_proto_files(&self, paths: &[PathBuf]) -> Result<ProtoSchema, Error> {
        let first = paths.first().ok_or_else(|| Error::Proto("no files given".into()))?;
        let read = |p: &Path| std::fs::read(p).map_err(|e| Error::Proto(format!("can't read {}: {e}", p.display())));
        let is_proto = |p: &PathBuf| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("proto"));
        let name = first
            .file_stem()
            .map_or_else(|| "schema".to_owned(), |s| s.to_string_lossy().into_owned());
        if !paths.iter().all(is_proto) {
            if paths.len() != 1 {
                return Err(Error::Proto(
                    "import either .proto files or one descriptor set, not both".into(),
                ));
            }
            return self.import_schema(&name, ProtoImport::DescriptorSet(read(first)?), &[]);
        }
        // Named relative to their common folder, so `import "a/b.proto"`
        // between picked files resolves.
        let root = common_parent(paths);
        let mut files = Vec::with_capacity(paths.len());
        let mut includes: Vec<PathBuf> = vec![root.clone()];
        for path in paths {
            let source =
                String::from_utf8(read(path)?).map_err(|_| Error::Proto(format!("{} isn't UTF-8", path.display())))?;
            let rel = path.strip_prefix(&root).unwrap_or(path);
            files.push((rel.to_string_lossy().replace('\\', "/"), source));
            if let Some(parent) = path.parent()
                && !includes.iter().any(|i| i == parent)
            {
                includes.push(parent.to_owned());
            }
        }
        let name = if paths.len() == 1 {
            name
        } else {
            format!("{name} + {} more", paths.len() - 1)
        };
        self.import_schema(&name, ProtoImport::Sources(files), &includes)
    }

    fn import_schema(&self, name: &str, import: ProtoImport, includes: &[PathBuf]) -> Result<ProtoSchema, Error> {
        let (files, descriptor) = match import {
            ProtoImport::Sources(sources) => compile_sources(&sources, includes).map_err(Error::Proto)?,
            ProtoImport::DescriptorSet(bytes) => (descriptor_files(&bytes).map_err(Error::Proto)?, bytes),
        };
        // Check it loads with the others before saving.
        let mut pool = self.protos.state.load().pool.clone();
        pool.decode_file_descriptor_set(descriptor.as_slice())
            .map_err(|e| Error::Proto(e.to_string()))?;
        let added_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let db = self.protos.db.lock();
        let files_json = serde_json::to_string(&files).unwrap_or_default();
        db.conn()
            .execute(
                "INSERT INTO proto_schemas (name, files, descriptor, added_at_ms) VALUES (?1, ?2, ?3, ?4)",
                params![name, files_json, descriptor, added_at_ms],
            )
            .map_err(|e| Error::Proto(e.to_string()))?;
        let id = db.conn().last_insert_rowid() as u32;
        self.protos.reload(&db)?;
        drop(db);
        self.protos.cache.invalidate_all();
        self.proto_schemas()
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| Error::Proto("the schema didn't save".into()))
    }

    pub fn remove_proto_schema(&self, id: u32) -> Result<(), Error> {
        let db = self.protos.db.lock();
        db.conn()
            .execute("DELETE FROM proto_schemas WHERE id = ?1", [id])
            .map_err(|e| Error::Proto(e.to_string()))?;
        self.protos.reload(&db)?;
        drop(db);
        self.protos.cache.invalidate_all();
        Ok(())
    }

    /// Fully qualified names of every message type the schemas know,
    /// sorted, for "Decode as…".
    pub fn proto_message_types(&self) -> Vec<String> {
        let state = self.protos.state.load();
        let mut names: Vec<String> = state
            .pool
            .all_messages()
            .filter(|m| !m.is_map_entry())
            .map(|m| m.full_name().to_owned())
            .collect();
        names.sort();
        names
    }

    pub fn proto_mappings(&self) -> Vec<ProtoMapping> {
        self.protos
            .state
            .load()
            .mappings
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }

    /// Replaces the "Decode as…" mappings. Each must name a known message
    /// type and have a valid matcher.
    pub fn set_proto_mappings(&self, mappings: Vec<ProtoMapping>) -> Result<(), Error> {
        let state = self.protos.state.load();
        for m in &mappings {
            if state.pool.get_message_by_name(&m.message_type).is_none() {
                return Err(Error::Proto(format!("no imported schema defines {}", m.message_type)));
            }
            CompiledRule::compile(&mapping_rule(m), &ScriptMap::new(), &MockMap::new())?;
        }
        drop(state);
        let db = self.protos.db.lock();
        db.save_setting(MAPPINGS_KEY, &mappings)
            .map_err(|e| Error::Proto(e.to_string()))?;
        self.protos.reload(&db)?;
        drop(db);
        self.protos.cache.invalidate_all();
        Ok(())
    }

    /// A gRPC, protobuf, MessagePack or CBOR body decoded to JSON trees.
    /// Works on a live flow too: a stream decodes as far as it has come.
    pub fn decode_body_in(&self, store: StoreId, id: FlowId, part: BodyPart) -> Result<DecodedBody, Error> {
        let summary = self.get_flow_in(store, id).ok_or(Error::UnknownFlow(id))?;
        let detail = self.flow_detail_in(store, id)?.ok_or(Error::UnknownFlow(id))?;
        let info = match part {
            BodyPart::Request => &detail.request_body,
            BodyPart::Response => &detail.response_body,
        };
        let state = self.protos.state.load_full();
        let key = CacheKey {
            store,
            flow: id,
            part,
            size: info.size + u64::from(info.complete) + detail.trailers.as_ref().map_or(0, |t| t.len() as u64 + 1),
            generation: state.generation,
        };
        if let Some(hit) = self.protos.cache.get(&key) {
            return Ok(DecodedBody::clone(&hit));
        }
        let headers = match part {
            BodyPart::Request => detail.request.headers.as_slice(),
            BodyPart::Response => detail.response.as_ref().map_or(&[][..], |r| r.headers.as_slice()),
        };
        let content_type = header(headers, "content-type");
        let kind = match info.kind {
            ContentKind::None => content_type.map_or(ContentKind::None, kind_from_content_type),
            kind => kind,
        };
        let body = self.body_head_in(store, id, part, true, Some(DECODE_MAX))?;
        let decoded = match kind {
            ContentKind::Grpc => decode_grpc(&state, &summary, &detail, part, &body.bytes),
            ContentKind::Protobuf => decode_protobuf(&state, &summary, &detail, part, content_type, &body.bytes),
            ContentKind::MessagePack => objects_body(DecodedFormat::MessagePack, objects::msgpack(&body.bytes)),
            ContentKind::Cbor => objects_body(DecodedFormat::Cbor, objects::cbor(&body.bytes)),
            other => {
                return Err(Error::Invalid(format!(
                    "a {} body has no decoded form",
                    serde_json::to_value(other)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default()
                )));
            }
        };
        self.protos.cache.insert(key, Arc::new(decoded.clone()));
        Ok(decoded)
    }
}

fn header<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

/// The common folder of `paths` (their parent, for one file).
fn common_parent(paths: &[PathBuf]) -> PathBuf {
    let mut root = paths[0].parent().map(Path::to_path_buf).unwrap_or_default();
    for p in &paths[1..] {
        while !p.starts_with(&root) {
            if !root.pop() {
                return PathBuf::new();
            }
        }
    }
    root
}

/// The message type a "Decode as…" mapping gives this body, if any.
fn mapped_type(state: &Compiled, summary: &FlowSummary, detail: &FlowDetail, part: BodyPart) -> Option<String> {
    if state.mappings.is_empty() {
        return None;
    }
    let headers = header_map(&detail.request.headers);
    let info = RequestInfo {
        method: &detail.request.method,
        scheme: summary.scheme,
        host: &summary.host,
        port: summary.port,
        path: &summary.path,
        query: &summary.query,
        headers: &headers,
        client_app: summary.client.as_deref(),
    };
    state
        .mappings
        .iter()
        .find(|(m, rule)| {
            m.part.is_none_or(|p| p == part)
                && rule.mismatch(&info, true).is_none()
                && match (&rule.response, &detail.response) {
                    (None, _) => true,
                    (Some(_), None) => false,
                    (Some(r), Some(resp)) => r.matches(resp.status, &header_map(&resp.headers)),
                }
        })
        .map(|(m, _)| m.message_type.clone())
}

fn decode_grpc(
    state: &Compiled,
    summary: &FlowSummary,
    detail: &FlowDetail,
    part: BodyPart,
    body: &[u8],
) -> DecodedBody {
    let headers = match part {
        BodyPart::Request => detail.request.headers.as_slice(),
        BodyPart::Response => detail.response.as_ref().map_or(&[][..], |r| r.headers.as_slice()),
    };
    let content_type = header(headers, "content-type");
    let flavor = Flavor::of(content_type);
    let json_payloads = content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("+json"));
    let framed = grpc::split(body, flavor, header(headers, "grpc-encoding"));
    // `/pkg.Service/Method` names its own types; a mapping overrides it.
    let descriptor = mapped_type(state, summary, detail, part)
        .and_then(|name| state.pool.get_message_by_name(&name))
        .or_else(|| {
            let (service, method) = summary.path.strip_prefix('/')?.split_once('/')?;
            let method = state
                .pool
                .get_service_by_name(service)?
                .methods()
                .find(|m| m.name() == method)?;
            Some(match part {
                BodyPart::Request => method.input(),
                BodyPart::Response => method.output(),
            })
        });
    let messages = framed
        .messages
        .iter()
        .enumerate()
        .map(|(index, raw)| {
            let mut message = match &raw.payload {
                Ok(payload) if json_payloads => json_message(payload),
                Ok(payload) => proto_message(payload, descriptor.as_ref()),
                Err(e) => DecodedMessage {
                    index: 0,
                    compressed: false,
                    size: 0,
                    json: "null".into(),
                    badges: Vec::new(),
                    schema_less: true,
                    error: Some(e.clone()),
                },
            };
            message.index = index as u32;
            message.compressed = raw.compressed;
            message.size = raw.size;
            message
        })
        .collect();
    let (status, trailers) = match part {
        BodyPart::Request => (None, Vec::new()),
        BodyPart::Response => {
            let trailers = detail
                .trailers
                .clone()
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| framed.trailers.clone());
            // A trailers-only response carries the status in its headers.
            let status = grpc::status(&trailers).or_else(|| grpc::status(headers));
            (status, trailers)
        }
    };
    DecodedBody {
        format: if flavor.is_web() {
            DecodedFormat::GrpcWeb
        } else {
            DecodedFormat::Grpc
        },
        messages,
        message_type: descriptor.map(|d| d.full_name().to_owned()),
        status,
        trailers,
        pending_bytes: framed.pending as u32,
        error: framed.error,
    }
}

fn decode_protobuf(
    state: &Compiled,
    summary: &FlowSummary,
    detail: &FlowDetail,
    part: BodyPart,
    content_type: Option<&str>,
    body: &[u8],
) -> DecodedBody {
    // A mapping, else a `messageType=` / `proto=` parameter naming a known type.
    let named = content_type.and_then(|ct| {
        ct.split(';').skip(1).find_map(|param| {
            let (key, value) = param.split_once('=')?;
            matches!(
                key.trim().to_ascii_lowercase().as_str(),
                "messagetype" | "proto" | "type"
            )
            .then(|| value.trim().trim_matches('"').to_owned())
        })
    });
    let descriptor = mapped_type(state, summary, detail, part)
        .or(named)
        .and_then(|name| state.pool.get_message_by_name(&name));
    let mut message = proto_message(body, descriptor.as_ref());
    message.size = body.len() as u32;
    DecodedBody {
        format: DecodedFormat::Protobuf,
        messages: vec![message],
        message_type: descriptor.map(|d| d.full_name().to_owned()),
        status: None,
        trailers: Vec::new(),
        pending_bytes: 0,
        error: None,
    }
}

/// One protobuf message: with its schema as proto3 JSON, else (or when it
/// doesn't fit the schema) schema-less.
fn proto_message(payload: &[u8], descriptor: Option<&MessageDescriptor>) -> DecodedMessage {
    let mut schema_error = None;
    if let Some(descriptor) = descriptor {
        match DynamicMessage::decode(descriptor.clone(), payload) {
            Ok(message) => match typed_json(&message) {
                Ok(json) => {
                    let unknown = message.unknown_fields().count();
                    let badges = if unknown > 0 {
                        vec![Badge {
                            pointer: String::new(),
                            label: format!("{unknown} unknown field{}", if unknown == 1 { "" } else { "s" }),
                        }]
                    } else {
                        Vec::new()
                    };
                    return DecodedMessage {
                        index: 0,
                        compressed: false,
                        size: payload.len() as u32,
                        json,
                        badges,
                        schema_less: false,
                        error: None,
                    };
                }
                Err(e) => schema_error = Some(format!("{}: {e}", descriptor.full_name())),
            },
            Err(e) => schema_error = Some(format!("doesn't decode as {}: {e}", descriptor.full_name())),
        }
    }
    match wire::decode(payload) {
        Ok((node, badges)) => DecodedMessage {
            index: 0,
            compressed: false,
            size: payload.len() as u32,
            json: node.to_json(),
            badges,
            schema_less: true,
            error: schema_error,
        },
        Err(e) => bytes_message(payload, schema_error.unwrap_or(e)),
    }
}

fn typed_json(message: &DynamicMessage) -> Result<String, serde_json::Error> {
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::pretty(&mut out);
    let options = SerializeOptions::new().skip_default_fields(false);
    message.serialize_with_options(&mut serializer, &options)?;
    Ok(String::from_utf8(out).unwrap_or_default())
}

/// `application/grpc+json`: each message is JSON text.
fn json_message(payload: &[u8]) -> DecodedMessage {
    match serde_json::from_slice::<serde_json::Value>(payload) {
        Ok(_) => DecodedMessage {
            index: 0,
            compressed: false,
            size: payload.len() as u32,
            json: String::from_utf8_lossy(payload).into_owned(),
            badges: Vec::new(),
            schema_less: false,
            error: None,
        },
        Err(e) => bytes_message(payload, format!("not JSON: {e}")),
    }
}

/// A payload nothing could read: its Base64, with why.
fn bytes_message(payload: &[u8], error: String) -> DecodedMessage {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
    DecodedMessage {
        index: 0,
        compressed: false,
        size: payload.len() as u32,
        json: serde_json::to_string(&b64).unwrap_or_default(),
        badges: vec![Badge {
            pointer: String::new(),
            label: format!("bytes · {}", payload.len()),
        }],
        schema_less: true,
        error: Some(error),
    }
}

fn objects_body(format: DecodedFormat, (values, error): (Vec<objects::Decoded>, Option<String>)) -> DecodedBody {
    DecodedBody {
        format,
        messages: values
            .into_iter()
            .enumerate()
            .map(|(index, (node, badges))| DecodedMessage {
                index: index as u32,
                compressed: false,
                size: 0,
                json: node.to_json(),
                badges,
                schema_less: false,
                error: None,
            })
            .collect(),
        message_type: None,
        status: None,
        trailers: Vec::new(),
        pending_bytes: 0,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const SHOP_PROTO: &str = r#"
        syntax = "proto3";
        package shop.v1;
        import "google/protobuf/timestamp.proto";
        message GetOrderRequest { string id = 1; }
        message Order {
            string id = 1;
            int64 total_cents = 2;
            repeated string items = 3;
            google.protobuf.Timestamp placed_at = 4;
        }
        service Orders { rpc GetOrder(GetOrderRequest) returns (Order); }
    "#;

    #[test]
    fn compiles_sources_with_imports_and_well_known_types() {
        let files = vec![
            ("shop/v1/shop.proto".to_owned(), SHOP_PROTO.to_owned()),
            (
                "shop/v1/extra.proto".to_owned(),
                "syntax = \"proto3\"; package shop.v1; import \"shop/v1/shop.proto\"; message Wrapper { Order order = 1; }"
                    .to_owned(),
            ),
        ];
        let (names, descriptor) = compile_sources(&files, &[]).unwrap();
        assert_eq!(names, ["shop/v1/shop.proto", "shop/v1/extra.proto"]);
        let pool = DescriptorPool::decode(descriptor.as_slice()).unwrap();
        assert!(pool.get_message_by_name("shop.v1.Wrapper").is_some());
        assert!(pool.get_message_by_name("google.protobuf.Timestamp").is_some());
        assert_eq!(
            descriptor_files(&descriptor).unwrap(),
            ["shop/v1/shop.proto", "shop/v1/extra.proto"]
        );
        let (services, messages) = summarize(&pool, &names);
        assert_eq!((services, messages), (vec!["shop.v1.Orders".to_owned()], 3));
    }

    #[test]
    fn reports_compile_errors() {
        let err = compile_sources(&[("bad.proto".into(), "syntax = \"proto3\"; message { }".into())], &[]).unwrap_err();
        assert!(err.contains("bad.proto"), "{err}");
        let err = compile_sources(&[("a.proto".into(), "import \"missing.proto\";".into())], &[]).unwrap_err();
        assert!(err.contains("missing.proto"), "{err}");
        assert!(descriptor_files(b"\xff\xff").is_err());
    }

    #[test]
    fn typed_messages_fall_back_to_schema_less() {
        let (_, descriptor) = compile_sources(&[("shop.proto".into(), SHOP_PROTO.into())], &[]).unwrap();
        let pool = DescriptorPool::decode(descriptor.as_slice()).unwrap();
        let order = pool.get_message_by_name("shop.v1.Order").unwrap();
        // id: "o1", total_cents: 2^60, items: ["a"], and unknown field 9.
        let mut bytes = vec![0x0a, 2, b'o', b'1', 0x10];
        let mut v = 1u64 << 60;
        while v >= 0x80 {
            bytes.push((v as u8) | 0x80);
            v >>= 7;
        }
        bytes.push(v as u8);
        bytes.extend_from_slice(&[0x1a, 1, b'a', 0x48, 1]);
        let m = proto_message(&bytes, Some(&order));
        assert!(!m.schema_less);
        let json: serde_json::Value = serde_json::from_str(&m.json).unwrap();
        assert_eq!(json["id"], "o1");
        // proto3 JSON writes 64-bit integers as strings: exact.
        assert_eq!(json["totalCents"], "1152921504606846976");
        assert_eq!(json["items"][0], "a");
        assert_eq!(m.badges[0].label, "1 unknown field");

        // Bytes that aren't an Order (a wire type mismatch) go schema-less.
        let m = proto_message(&[0x0d, 1, 2, 3, 4], Some(&order));
        assert!(m.schema_less);
        assert!(m.error.unwrap().contains("shop.v1.Order"));
        // And nothing readable shows as Base64.
        let m = proto_message(&[0xff], None);
        assert_eq!(m.json, "\"/w==\"");
    }
}
