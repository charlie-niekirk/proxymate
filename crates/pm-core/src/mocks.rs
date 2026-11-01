//! The mock library (M20): recorded sessions that `Action::ServeFrom`
//! answers from.
//!
//! "Mock from session…" copies a store (a saved session, an imported HAR
//! or Charles file, or the live capture) into
//! `<data dir>/mocks/<uuid>.proxymate`, so a rule keeps working when the
//! original file moves. The entries are listed in `app.sqlite`'s
//! `mock_sources` table.
//!
//! When rules compile, each source an enabled `ServeFrom` names is opened
//! (once, then cached) and indexed by method, scheme, host and path; the
//! query (decoded pairs, any order) and the request body hash narrow the
//! candidates at lookup. Bodies are read through the session's
//! [`SqliteBackend`], off the async threads.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use http::{Response, StatusCode};
use parking_lot::{Mutex, RwLock};
use pm_model::{
    Action, BodyPart, FlowId, FlowSelection, MockSource, Pattern, PresetKind, Rule, RuleOrigin, Scheme, ServeKey,
    ServeMiss, ServePick, StoreId,
};
use rusqlite::params;

use crate::io::{decode, session};
use crate::proxy::body::Inbound;
use crate::rules::compile::{PATTERN_MAX_LEN, query_pairs};
use crate::rules::db::AppDb;
use crate::rules::eval::{RequestDraft, ResponseDraft};
use crate::store::{BlobBackend, SqliteBackend};
use crate::{Engine, Error};

/// The library's folder under the data dir.
pub const MOCKS_DIR: &str = "mocks";

pub(crate) struct Mocks {
    db: Mutex<AppDb>,
    dir: PathBuf,
    list: RwLock<Vec<MockSource>>,
    /// Opened and indexed sources, by id.
    loaded: Mutex<HashMap<String, Arc<MockIndex>>>,
}

impl Mocks {
    pub fn load(data_dir: Option<&Path>, root: &Path) -> Mocks {
        let db = data_dir
            .map(AppDb::open)
            .unwrap_or_else(AppDb::in_memory)
            .or_else(|err| {
                eprintln!("proxymate: can't open the app database ({err}); the mock library won't be saved");
                AppDb::in_memory()
            })
            .expect("in-memory SQLite");
        if let Err(err) = db
            .conn()
            .execute_batch("CREATE TABLE IF NOT EXISTS mock_sources (id TEXT PRIMARY KEY, json TEXT NOT NULL);")
        {
            eprintln!("proxymate: can't create the mock library table: {err}");
        }
        let list = read_sources(&db).unwrap_or_else(|err| {
            eprintln!("proxymate: can't read the mock library: {err}");
            Vec::new()
        });
        Mocks {
            db: Mutex::new(db),
            dir: root.join(MOCKS_DIR),
            list: RwLock::new(list),
            loaded: Mutex::new(HashMap::new()),
        }
    }

    fn path_of(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.{}", session::EXTENSION))
    }

    /// What compiling `rules` needs: every known source, and the indexes
    /// of those the rules serve from (opened now if they weren't).
    pub fn map_for<'a>(&self, rules: impl IntoIterator<Item = &'a Rule>) -> MockMap {
        let mut map = MockMap {
            known: self.list.read().iter().map(|s| s.id.clone()).collect(),
            ..MockMap::default()
        };
        for source in rules.into_iter().flat_map(served_sources) {
            if !map.known.contains(source) || map.loaded.contains_key(source) {
                continue;
            }
            match self.index(source) {
                Ok(index) => {
                    map.loaded.insert(source.to_owned(), index);
                }
                Err(err) => {
                    map.failed.insert(source.to_owned(), err.to_string());
                }
            }
        }
        map
    }

    fn index(&self, id: &str) -> io::Result<Arc<MockIndex>> {
        if let Some(index) = self.loaded.lock().get(id) {
            return Ok(index.clone());
        }
        let name = self
            .list
            .read()
            .iter()
            .find(|s| s.id == id)
            .map_or_else(|| id.to_owned(), |s| s.name.clone());
        let index = Arc::new(MockIndex::open(&self.path_of(id), name)?);
        self.loaded.lock().insert(id.to_owned(), index.clone());
        Ok(index)
    }
}

fn read_sources(db: &AppDb) -> rusqlite::Result<Vec<MockSource>> {
    let mut stmt = db.conn().prepare("SELECT id, json FROM mock_sources")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (id, json) = row?;
        match serde_json::from_str::<MockSource>(&json) {
            Ok(source) => out.push(source),
            Err(err) => eprintln!("proxymate: skipping unreadable mock source {id}: {err}"),
        }
    }
    out.sort_by(|a, b| a.created_at_ms.total_cmp(&b.created_at_ms));
    Ok(out)
}

/// The sources a rule's `ServeFrom` actions name.
fn served_sources(rule: &Rule) -> impl Iterator<Item = &str> {
    rule.actions.iter().filter_map(|a| match a {
        Action::ServeFrom { source, .. } => Some(source.as_str()),
        _ => None,
    })
}

/// The mock library as rule compilation sees it.
#[derive(Default)]
pub(crate) struct MockMap {
    known: HashSet<String>,
    loaded: HashMap<String, Arc<MockIndex>>,
    /// Sources that couldn't be opened, with why.
    failed: HashMap<String, String>,
}

impl MockMap {
    pub fn new() -> MockMap {
        MockMap::default()
    }

    /// `Ok(None)` for a known source that wasn't opened (a disabled rule,
    /// or a check that doesn't run the rule).
    pub fn resolve(&self, id: &str) -> Result<Option<Arc<MockIndex>>, String> {
        if let Some(err) = self.failed.get(id) {
            return Err(format!("the mock source can't be read: {err}"));
        }
        if let Some(index) = self.loaded.get(id) {
            return Ok(Some(index.clone()));
        }
        if self.known.contains(id) {
            Ok(None)
        } else {
            Err(format!("there is no mock source with id {id}"))
        }
    }
}

/// What a request must share with a recorded one before its query and
/// body are compared.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Route {
    /// Upper case.
    method: String,
    scheme: Scheme,
    /// Lower case.
    host: String,
    path: String,
}

struct Entry {
    flow: FlowId,
    /// Decoded pairs, sorted.
    query: Vec<(String, String)>,
    /// The decoded request body's hash, computed on first use.
    body: OnceLock<u64>,
}

/// An opened mock source.
pub(crate) struct MockIndex {
    pub name: String,
    backend: SqliteBackend,
    routes: HashMap<Route, Vec<Entry>>,
    flows: u32,
    hosts: Vec<String>,
}

impl MockIndex {
    fn open(path: &Path, name: String) -> io::Result<MockIndex> {
        let opened = session::open(path)?;
        let mut routes: HashMap<Route, Vec<Entry>> = HashMap::new();
        let mut hosts = std::collections::BTreeSet::new();
        let mut flows = 0;
        for s in &opened.summaries {
            if s.status.is_none() || &*s.method == "CONNECT" || s.scheme.is_websocket() {
                continue;
            }
            flows += 1;
            hosts.insert(s.host.to_ascii_lowercase());
            routes
                .entry(Route {
                    method: s.method.to_ascii_uppercase(),
                    scheme: s.scheme,
                    host: s.host.to_ascii_lowercase(),
                    path: s.path.clone(),
                })
                .or_default()
                .push(Entry {
                    flow: s.id,
                    query: sorted_query(&s.query),
                    body: OnceLock::new(),
                });
        }
        Ok(MockIndex {
            name,
            backend: opened.backend,
            routes,
            flows,
            hosts: hosts.into_iter().collect(),
        })
    }

    /// The recorded flow that answers `req`, if any. `turns` keeps a
    /// round-robin's place per candidate set.
    pub fn lookup(
        &self,
        req: &RequestDraft,
        key: ServeKey,
        pick: ServePick,
        turns: &Mutex<HashMap<FlowId, usize>>,
    ) -> Option<FlowId> {
        let route = Route {
            method: req.method.to_ascii_uppercase(),
            scheme: req.scheme,
            host: req.host.to_ascii_lowercase(),
            path: req.path.clone(),
        };
        let entries = self.routes.get(&route)?;
        let query = key.query.then(|| sorted_query(&req.query));
        let body = match key.body {
            true => Some(hash(req.body.as_deref()?)),
            false => None,
        };
        let matching: Vec<&Entry> = entries
            .iter()
            .filter(|e| query.as_ref().is_none_or(|q| *q == e.query))
            .filter(|e| body.is_none_or(|b| self.body_hash(e) == Some(b)))
            .collect();
        let first = matching.first()?;
        let chosen = match pick {
            ServePick::First => first,
            ServePick::Last => matching.last()?,
            ServePick::RoundRobin => {
                let mut turns = turns.lock();
                let turn = turns.entry(first.flow).or_insert(0);
                let chosen = matching[*turn % matching.len()];
                *turn = turn.wrapping_add(1);
                chosen
            }
        };
        Some(chosen.flow)
    }

    fn body_hash(&self, entry: &Entry) -> Option<u64> {
        if let Some(h) = entry.body.get() {
            return Some(*h);
        }
        let detail = self.backend.detail(entry.flow).ok()??;
        let raw = self
            .backend
            .body(entry.flow, BodyPart::Request)
            .ok()?
            .unwrap_or_default();
        let bytes = match detail.request_body.encoding.as_deref() {
            Some(encoding) if !raw.is_empty() => decode::decode(encoding, &raw).unwrap_or(raw),
            _ => raw,
        };
        Some(*entry.body.get_or_init(|| hash(&bytes)))
    }

    /// The recorded response of `flow`, as it came over the wire: its
    /// headers (without hop-by-hop ones) and its body still
    /// content-encoded. A body that was stored decoded (HAR, Charles) loses
    /// its `Content-Encoding`. Reads the session: call it off the async
    /// threads.
    pub fn response(&self, flow: FlowId, method: &str) -> io::Result<Response<Inbound>> {
        let (status, mut headers, body) = self.recorded(flow)?;
        let bodiless = method.eq_ignore_ascii_case("HEAD") || matches!(status, 100..=199 | 204 | 304);
        if !bodiless {
            headers.remove(header::TRANSFER_ENCODING);
            headers.insert(header::CONTENT_LENGTH, HeaderValue::from(body.len()));
        }
        let mut response = Response::new(Inbound::local(body));
        *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
        *response.headers_mut() = headers;
        Ok(response)
    }

    /// The recorded response of `flow` with its body decoded, for a dry run.
    pub fn draft(&self, flow: FlowId) -> io::Result<ResponseDraft> {
        let (status, mut headers, body) = self.recorded(flow)?;
        let encoding = headers
            .get(header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_ascii_lowercase());
        let body = match encoding {
            Some(e) if !e.is_empty() && e != "identity" && !body.is_empty() => match decode::decode(&e, &body) {
                Ok(decoded) => {
                    headers.remove(header::CONTENT_ENCODING);
                    decoded
                }
                Err(_) => body,
            },
            _ => body,
        };
        Ok(ResponseDraft {
            status,
            headers,
            body: Some(body),
        })
    }

    fn recorded(&self, flow: FlowId) -> io::Result<(u16, HeaderMap, Vec<u8>)> {
        let detail = self
            .backend
            .detail(flow)?
            .ok_or_else(|| io::Error::other(format!("the recording has no flow {flow}")))?;
        let head = detail
            .response
            .ok_or_else(|| io::Error::other("the recorded flow has no response"))?;
        let body = self.backend.body(flow, BodyPart::Response)?.unwrap_or_default();
        let mut headers = HeaderMap::new();
        for h in &head.headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(h.name.as_bytes()),
                HeaderValue::from_str(&h.value),
            ) {
                headers.append(name, value);
            }
        }
        crate::proxy::strip_hop_by_hop(&mut headers);
        if detail.response_body.encoding.is_none() {
            headers.remove(header::CONTENT_ENCODING);
        }
        Ok((head.status, headers, body))
    }
}

fn sorted_query(query: &str) -> Vec<(String, String)> {
    let mut pairs = query_pairs(query);
    pairs.sort();
    pairs
}

fn hash(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// The answer to a `ServeFrom` miss that doesn't pass through.
pub(crate) fn miss_answer(on_miss: ServeMiss, source: &str, req: &RequestDraft) -> Option<ResponseDraft> {
    let status = match on_miss {
        ServeMiss::PassThrough => return None,
        ServeMiss::NotFound => 404,
        ServeMiss::BadGateway => 502,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    let body = format!(
        "Proxymate: nothing recorded in {source:?} answers {} {}\n",
        req.method,
        req.url()
    );
    Some(ResponseDraft {
        status,
        headers,
        body: Some(body.into_bytes()),
    })
}

fn now_ms() -> f64 {
    crate::proxy::now_ms()
}

impl Engine {
    /// The mock library, oldest first.
    pub fn mock_sources(&self) -> Vec<MockSource> {
        self.mocks.list.read().clone()
    }

    /// Copies every flow of `store` (an opened document, or the live
    /// capture) into the mock library.
    pub fn create_mock_source(&self, store: StoreId, name: Option<String>) -> Result<MockSource, Error> {
        let (default_name, origin) = match store {
            pm_model::LIVE_STORE => ("Live capture".to_owned(), "Live capture".to_owned()),
            _ => {
                let doc = self.docs.get(store)?;
                (stem(&doc.info.name), doc.info.path.clone())
            }
        };
        let flows = self.selected_flows(store, &FlowSelection::All)?;
        if flows.is_empty() {
            return Err(Error::Document("there are no flows to mock from".into()));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let path = self.mocks.path_of(&id);
        std::fs::create_dir_all(&self.mocks.dir)?;
        let source = flows.into_iter().map(|summary| {
            self.with_backend(store, |backend| Ok(session::collect(backend, summary)?))
                .map_err(io::Error::other)
        });
        session::write(&path, source, &mut |_| {})?;
        self.add_mock_source(id, name.unwrap_or(default_name), origin, &path)
    }

    /// Copies a session, HAR or Charles file into the mock library without
    /// opening it as a document.
    pub fn create_mock_source_from_file(&self, file: &Path, name: Option<String>) -> Result<MockSource, Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = self.mocks.path_of(&id);
        std::fs::create_dir_all(&self.mocks.dir)?;
        if crate::documents::is_sqlite(file) {
            // Checked before it's copied, so a foreign SQLite file is refused.
            session::open(file).map_err(|e| Error::Document(e.to_string()))?;
            let partial = self.mocks.dir.join(format!(".{id}.partial"));
            std::fs::copy(file, &partial)?;
            std::fs::rename(&partial, &path)?;
        } else {
            let (_, flows) = crate::documents::import_file(file)?;
            session::write(&path, flows.into_iter().map(Ok), &mut |_| {})?;
        }
        let file_name = file
            .file_name()
            .map_or_else(|| file.display().to_string(), |n| n.to_string_lossy().into_owned());
        self.add_mock_source(
            id,
            name.unwrap_or_else(|| stem(&file_name)),
            file.display().to_string(),
            &path,
        )
    }

    fn add_mock_source(&self, id: String, name: String, origin: String, path: &Path) -> Result<MockSource, Error> {
        let index = match MockIndex::open(path, name.clone()) {
            Ok(index) => index,
            Err(err) => {
                let _ = std::fs::remove_file(path);
                return Err(Error::Document(err.to_string()));
            }
        };
        let source = MockSource {
            id: id.clone(),
            name,
            origin,
            flows: index.flows,
            hosts: index.hosts.clone(),
            created_at_ms: now_ms(),
            size: std::fs::metadata(path).map_or(0, |m| m.len()),
        };
        let json = serde_json::to_string(&source).map_err(|e| Error::Document(e.to_string()))?;
        if let Err(err) = self
            .mocks
            .db
            .lock()
            .conn()
            .execute("INSERT INTO mock_sources (id, json) VALUES (?1, ?2)", params![id, json])
        {
            let _ = std::fs::remove_file(path);
            return Err(Error::Document(format!("can't save the mock source: {err}")));
        }
        self.mocks.loaded.lock().insert(id, Arc::new(index));
        self.mocks.list.write().push(source.clone());
        Ok(source)
    }

    /// Removes a source from the library (and its file). A source a rule
    /// serves from can't be removed.
    pub fn remove_mock_source(&self, id: &str) -> Result<(), Error> {
        let users: Vec<String> = self
            .rules()
            .into_iter()
            .filter(|r| served_sources(r).any(|s| s == id))
            .map(|r| format!("{:?}", r.name))
            .collect();
        if !users.is_empty() {
            return Err(Error::Invalid(format!(
                "the mock source is used by {}; delete or change {} first",
                users.join(", "),
                if users.len() == 1 { "that rule" } else { "those rules" }
            )));
        }
        let removed = self
            .mocks
            .db
            .lock()
            .conn()
            .execute("DELETE FROM mock_sources WHERE id = ?1", [id])
            .map_err(|e| Error::Document(e.to_string()))?;
        if removed == 0 {
            return Err(Error::Invalid(format!("there is no mock source with id {id}")));
        }
        self.mocks.list.write().retain(|s| s.id != id);
        self.mocks.loaded.lock().remove(id);
        let _ = std::fs::remove_file(self.mocks.path_of(id));
        Ok(())
    }

    /// The "Mock from session…" preset: a new, unsaved rule that serves
    /// from `source` for every host it recorded.
    pub fn draft_serve_from_rule(&self, source: &str) -> Result<Rule, Error> {
        let info = self
            .mock_sources()
            .into_iter()
            .find(|s| s.id == source)
            .ok_or_else(|| Error::Invalid(format!("there is no mock source with id {source}")))?;
        Ok(Rule {
            id: String::new(),
            name: format!("Serve from {}", info.name),
            enabled: true,
            order: 0,
            origin: RuleOrigin::Preset {
                preset: PresetKind::ServeFrom,
            },
            matcher: pm_model::Matcher {
                host: hosts_pattern(&info.hosts),
                ..Default::default()
            },
            actions: vec![Action::ServeFrom {
                source: info.id,
                key: ServeKey::default(),
                on_miss: ServeMiss::PassThrough,
                pick: ServePick::First,
            }],
        })
    }

    /// The library as rule compilation needs it for `rules`.
    pub(crate) fn mock_map<'a>(&self, rules: impl IntoIterator<Item = &'a Rule>) -> MockMap {
        self.mocks.map_for(rules)
    }
}

/// One host exactly, several as a glob alternation; none (or too many
/// for a pattern) matches every host, which a miss passes through.
fn hosts_pattern(hosts: &[String]) -> Option<Pattern> {
    match hosts {
        [] => None,
        [host] => Some(Pattern::exact(host.clone())),
        _ => {
            let escaped: Vec<String> = hosts
                .iter()
                .map(|h| {
                    h.chars()
                        .flat_map(|c| match c {
                            '*' | '?' | '[' | ']' | '{' | '}' | ',' | '\\' => vec!['\\', c],
                            c => vec![c],
                        })
                        .collect()
                })
                .collect();
            let value = format!("{{{}}}", escaped.join(","));
            (value.len() <= PATTERN_MAX_LEN).then(|| Pattern::glob(value))
        }
    }
}

fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map_or_else(|| name.to_owned(), |s| s.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_hosts_become_one_glob() {
        assert_eq!(hosts_pattern(&[]), None);
        assert_eq!(hosts_pattern(&["a.test".into()]), Some(Pattern::exact("a.test")));
        let p = hosts_pattern(&["a.test".into(), "b,c.test".into()]).unwrap();
        assert_eq!(p.value, "{a.test,b\\,c.test}");
        let re = crate::rules::compile::compile_pattern(&p, crate::rules::compile::Use::Match, true).unwrap();
        assert!(re.is_match(b"a.test") && re.is_match(b"b,c.test") && !re.is_match(b"c.test"));
    }

    #[test]
    fn the_query_is_compared_in_any_order() {
        assert_eq!(sorted_query("b=2&a=1&a=0"), sorted_query("a=0&a=1&b=2"));
        assert_ne!(sorted_query("a=1"), sorted_query("a=2"));
        assert_eq!(sorted_query("q=a%20b"), sorted_query("q=a+b"));
    }
}
