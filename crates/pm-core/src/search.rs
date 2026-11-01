//! Listing and searching recorded flows outside a view, for agents (MCP)
//! and headless tools: the filter DSL over summaries, and a deep search of
//! URLs, headers and decoded bodies. Both page with a flow-id cursor, so a
//! clear or new traffic never shifts a page.

use pm_model::{BodyPart, FlowId, FlowState, FlowSummary, Header, Pattern, PatternKind, SearchScope};
use regex::bytes::Regex;

use crate::rules::compile::{Use, compile_pattern};
use crate::store::{FlowQuery, Store};
use crate::{Engine, Error};

/// Bodies are searched up to this many (decoded) bytes.
const SEARCH_BODY_MAX: usize = 1024 * 1024;
/// A deep search looks at this many flows per call at most; the cursor
/// continues from there.
const SEARCH_SCAN_MAX: usize = 2000;
/// Characters of context on each side of a snippet's match.
const SNIPPET_CONTEXT: usize = 60;

/// A page of flows, oldest or newest first.
#[derive(Debug, Clone)]
pub struct FlowPage {
    pub flows: Vec<FlowSummary>,
    /// Flows in the store matching the filter.
    pub matched: u32,
    /// Pass back to continue after the last flow of this page.
    pub next: Option<FlowId>,
}

#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub scope: SearchScope,
    /// The match with some context; for headers, `Name: value`.
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub flow: FlowSummary,
    pub matches: Vec<SearchMatch>,
}

#[derive(Debug, Clone)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    /// Flows looked at in this call.
    pub scanned: u32,
    /// Set when there are flows left to search: pass it back to go on.
    pub next: Option<FlowId>,
}

/// Flows to visit, oldest first or newest first, starting past `cursor`
/// and never at or below `since`. Walks the arena, so ids evicted from
/// between kept old flows and newer ones cost nothing.
fn visit<'a>(
    store: &'a Store,
    since: Option<FlowId>,
    cursor: Option<FlowId>,
    newest_first: bool,
) -> Box<dyn Iterator<Item = &'a FlowSummary> + 'a> {
    let low = since.map_or(0, |s| s.saturating_add(1));
    if newest_first {
        let high = cursor.unwrap_or(FlowId::MAX);
        Box::new(store.flows_in(low, high).iter().rev())
    } else {
        let low = cursor.map_or(low, |c| c.saturating_add(1).max(low));
        Box::new(store.flows_in(low, FlowId::MAX).iter())
    }
}

impl Engine {
    /// Parses a filter DSL query strictly (as MCP does), with `op:` terms
    /// bound to the imported OpenAPI specs.
    pub fn parse_query(&self, query: &str) -> Result<FlowQuery, Error> {
        Ok(FlowQuery::parse(query)
            .map_err(Error::Query)?
            .with_operations(&self.operations()))
    }

    /// Flows matching `query` (the filter DSL; empty matches all) with ids
    /// above `since`, `limit` at a time. `cursor` is the previous page's
    /// `next`. Header and body terms are checked flow by flow, only until
    /// the page is full; `matched` then counts what was checked.
    pub fn find_flows(
        &self,
        query: &str,
        since: Option<FlowId>,
        cursor: Option<FlowId>,
        limit: usize,
        newest_first: bool,
    ) -> Result<FlowPage, Error> {
        let query = self.parse_query(query)?;
        let deep = query.deep();
        let mut flows = Vec::new();
        let mut matched = 0u32;
        let mut next = None;
        let Some(deep) = deep else {
            let store = self.store.read();
            for flow in visit(&store, since, cursor, newest_first) {
                if !query.matches(flow) {
                    continue;
                }
                matched += 1;
                if flows.len() < limit {
                    flows.push(flow.clone());
                } else if next.is_none() {
                    next = flows.last().map(|f: &FlowSummary| f.id);
                }
            }
            return Ok(FlowPage { flows, matched, next });
        };
        // Bodies are read without holding the store lock.
        let candidates: Vec<FlowId> = {
            let store = self.store.read();
            visit(&store, since, cursor, newest_first)
                .filter(|f| query.matches(f))
                .map(|f| f.id)
                .collect()
        };
        for id in candidates {
            if !deep.matches(self.details(), id) {
                continue;
            }
            if flows.len() == limit {
                next = flows.last().map(|f: &FlowSummary| f.id);
                break;
            }
            let Some(flow) = self.get_flow(id) else { continue };
            matched += 1;
            flows.push(flow);
        }
        Ok(FlowPage { flows, matched, next })
    }

    /// Searches URLs, headers and decoded bodies (the first 1 MB) of the
    /// flows matching `filter`, newest first. `pattern` is a literal
    /// (case-insensitive) or, with `regex`, a size-limited regex.
    pub fn search_flows(
        &self,
        pattern: &str,
        regex: bool,
        scopes: &[SearchScope],
        filter: &str,
        cursor: Option<FlowId>,
        limit: usize,
    ) -> Result<SearchPage, Error> {
        let query = self.parse_query(filter)?;
        let pattern = Pattern {
            kind: if regex { PatternKind::Regex } else { PatternKind::Exact },
            value: pattern.to_owned(),
            case_insensitive: true,
        };
        if pattern.value.is_empty() {
            return Err(Error::Query("the search pattern is empty".into()));
        }
        let re = compile_pattern(&pattern, Use::Find, false).map_err(Error::Query)?;
        let candidates: Vec<FlowSummary> = {
            let store = self.store.read();
            visit(&store, None, cursor, true)
                .filter(|f| query.matches(f))
                .take(SEARCH_SCAN_MAX)
                .cloned()
                .collect()
        };
        let mut hits = Vec::new();
        let mut scanned = 0u32;
        let mut next = None;
        for flow in &candidates {
            if hits.len() >= limit {
                next = hits.last().map(|h: &SearchHit| h.flow.id);
                break;
            }
            scanned += 1;
            let matches = self.search_one(flow, &re, scopes);
            if !matches.is_empty() {
                hits.push(SearchHit {
                    flow: flow.clone(),
                    matches,
                });
            }
        }
        if next.is_none() && candidates.len() == SEARCH_SCAN_MAX {
            next = candidates.last().map(|f| f.id);
        }
        Ok(SearchPage { hits, scanned, next })
    }

    fn search_one(&self, flow: &FlowSummary, re: &Regex, scopes: &[SearchScope]) -> Vec<SearchMatch> {
        let mut matches = Vec::new();
        let detail = if scopes.iter().any(|s| *s != SearchScope::Url) {
            self.flow_detail(flow.id).ok().flatten()
        } else {
            None
        };
        for &scope in scopes {
            let found = match scope {
                SearchScope::Url => {
                    let url = format!(
                        "{}://{}{}{}{}",
                        flow.scheme.as_str(),
                        flow.host,
                        if flow.is_default_port() {
                            String::new()
                        } else {
                            format!(":{}", flow.port)
                        },
                        flow.path,
                        if flow.query.is_empty() {
                            String::new()
                        } else {
                            format!("?{}", flow.query)
                        }
                    );
                    snippet(re, url.as_bytes())
                }
                SearchScope::RequestHeaders => detail.as_ref().and_then(|d| header_match(re, &d.request.headers)),
                SearchScope::ResponseHeaders => detail
                    .as_ref()
                    .and_then(|d| d.response.as_ref())
                    .and_then(|r| header_match(re, &r.headers)),
                SearchScope::RequestBody | SearchScope::ResponseBody => {
                    let part = if scope == SearchScope::RequestBody {
                        BodyPart::Request
                    } else {
                        BodyPart::Response
                    };
                    let has_body = detail.as_ref().is_some_and(|d| match part {
                        BodyPart::Request => d.request_body.captured > 0,
                        BodyPart::Response => d.response_body.captured > 0,
                    });
                    has_body
                        .then(|| self.body_head(flow.id, part, true, Some(SEARCH_BODY_MAX)).ok())
                        .flatten()
                        .and_then(|body| snippet(re, &body.bytes))
                }
            };
            if let Some(snippet) = found {
                matches.push(SearchMatch { scope, snippet });
            }
        }
        matches
    }

    /// True once a flow matching `query` (above `since`) has finished, or
    /// with `active`, as soon as one has started. Returns the first one.
    pub fn first_flow_matching(&self, query: &FlowQuery, since: Option<FlowId>, active: bool) -> Option<FlowSummary> {
        let deep = query.deep();
        let candidates: Vec<FlowSummary> = {
            let store = self.store.read();
            visit(&store, since, None, false)
                .filter(|f| (active || !matches!(f.state, FlowState::Active | FlowState::Paused)) && query.matches(f))
                .cloned()
                .collect()
        };
        candidates
            .into_iter()
            .find(|f| deep.as_ref().is_none_or(|d| d.matches(self.details(), f.id)))
    }
}

fn header_match(re: &Regex, headers: &[Header]) -> Option<String> {
    headers.iter().find_map(|h| {
        let line = format!("{}: {}", h.name, h.value);
        re.is_match(line.as_bytes())
            .then(|| cut_chars(&line, SNIPPET_CONTEXT * 3))
    })
}

/// The first match in `bytes` with some context, as lossy UTF-8.
fn snippet(re: &Regex, bytes: &[u8]) -> Option<String> {
    let m = re.find(bytes)?;
    let start = m.start().saturating_sub(SNIPPET_CONTEXT);
    let end = (m.end() + SNIPPET_CONTEXT).min(bytes.len());
    let text = String::from_utf8_lossy(&bytes[start..end]).replace(['\n', '\r'], " ");
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < bytes.len() { "…" } else { "" };
    Some(format!("{prefix}{text}{suffix}"))
}

fn cut_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Generator;

    #[test]
    fn pages_continue_from_the_cursor_both_ways() {
        let engine = Engine::new();
        let mut generator = Generator::new(3);
        engine.insert_flows((0..10).map(|_| generator.completed_flow()).collect());
        let ids = |p: &FlowPage| p.flows.iter().map(|f| f.id).collect::<Vec<_>>();

        let first = engine.find_flows("", None, None, 4, false).unwrap();
        assert_eq!(
            (ids(&first), first.matched, first.next),
            (vec![0, 1, 2, 3], 10, Some(3))
        );
        let second = engine.find_flows("", None, first.next, 4, false).unwrap();
        assert_eq!(ids(&second), [4, 5, 6, 7]);

        let newest = engine.find_flows("", Some(5), None, 3, true).unwrap();
        assert_eq!((ids(&newest), newest.next), (vec![9, 8, 7], Some(7)));
        let older = engine.find_flows("", Some(5), newest.next, 3, true).unwrap();
        assert_eq!((ids(&older), older.next), (vec![6], None));

        assert!(matches!(
            engine.find_flows("bogus:1", None, None, 1, false),
            Err(Error::Query(_))
        ));
    }

    #[test]
    fn url_search_finds_snippets() {
        let engine = Engine::new();
        let mut generator = Generator::new(3);
        engine.insert_flows((0..20).map(|_| generator.completed_flow()).collect());
        let host = engine.get_flow(4).unwrap().host.to_string();
        let page = engine
            .search_flows(&host.to_uppercase(), false, &[SearchScope::Url], "", None, 100)
            .unwrap();
        assert!(page.hits.iter().any(|h| h.flow.id == 4));
        assert!(page.hits.iter().all(|h| h.matches[0].snippet.contains(&host)));
        assert!(
            engine
                .search_flows("(", true, &[SearchScope::Url], "", None, 1)
                .is_err()
        );
    }
}
