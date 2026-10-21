//! Compiled form of a [`FilterSpec`], evaluated against summaries only.

use std::cell::RefCell;

use pm_model::{ContentGroup, FilterSpec, FlowSummary, ProtocolFilter};

#[derive(Debug, Clone, Default)]
pub struct CompiledFilter {
    /// Lower-cased search text; empty matches everything.
    needle: Vec<u8>,
    protocol: ProtocolFilter,
    /// Bitmask over `ContentGroup as u16`; zero means all.
    content_mask: u16,
    host: Option<String>,
    client: Option<String>,
}

impl CompiledFilter {
    pub fn compile(spec: &FilterSpec) -> Self {
        let content_mask = spec.content.iter().fold(0u16, |mask, group| mask | group_bit(*group));
        CompiledFilter {
            needle: spec.text.trim().to_ascii_lowercase().into_bytes(),
            protocol: spec.protocol,
            content_mask,
            host: spec.host.clone().filter(|h| !h.is_empty()),
            client: spec.client.clone().filter(|c| !c.is_empty()),
        }
    }

    /// True if the filter accepts every flow, so views can skip evaluation.
    pub fn is_match_all(&self) -> bool {
        self.needle.is_empty()
            && self.protocol == ProtocolFilter::All
            && self.content_mask == 0
            && self.host.is_none()
            && self.client.is_none()
    }

    pub fn matches(&self, f: &FlowSummary) -> bool {
        let protocol_ok = match self.protocol {
            ProtocolFilter::All => true,
            ProtocolFilter::Http => !f.scheme.is_tls() && !f.scheme.is_websocket(),
            ProtocolFilter::Https => f.scheme.is_tls() && !f.scheme.is_websocket(),
            ProtocolFilter::WebSocket => f.scheme.is_websocket(),
        };
        if !protocol_ok {
            return false;
        }
        if self.content_mask != 0 && self.content_mask & group_bit(ContentGroup::of(f.content_kind)) == 0 {
            return false;
        }
        if let Some(host) = &self.host {
            let ok = match host.strip_prefix("*.") {
                Some(domain) => super::in_domain(&f.host, domain),
                None => *f.host == **host,
            };
            if !ok {
                return false;
            }
        }
        if let Some(client) = &self.client
            && f.client.as_deref() != Some(client.as_str())
        {
            return false;
        }
        self.needle.is_empty() || self.url_contains_needle(f)
    }

    /// Matches against `host[:port]path[?query]` as one string, so a needle
    /// may span parts (e.g. `stripe.com/v1`). Reuses a per-thread buffer.
    fn url_contains_needle(&self, f: &FlowSummary) -> bool {
        thread_local! {
            static URL: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        }
        URL.with_borrow_mut(|url| {
            url.clear();
            url.extend_from_slice(f.host.as_bytes());
            if !f.is_default_port() {
                url.push(b':');
                url.extend_from_slice(f.port.to_string().as_bytes());
            }
            url.extend_from_slice(f.path.as_bytes());
            if !f.query.is_empty() {
                url.push(b'?');
                url.extend_from_slice(f.query.as_bytes());
            }
            contains_ignore_ascii_case(url, &self.needle)
        })
    }
}

const fn group_bit(group: ContentGroup) -> u16 {
    1 << group as u16
}

/// `needle` must already be lower-case ASCII (non-ASCII bytes compare exactly).
pub(crate) fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    let first = needle[0];
    haystack
        .windows(needle.len())
        .any(|w| w[0].to_ascii_lowercase() == first && w[1..].eq_ignore_ascii_case(&needle[1..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::{ContentKind, FlowState, HttpVersion, Scheme, TlsStatus};

    fn flow(scheme: Scheme, host: &str, path: &str, kind: ContentKind) -> FlowSummary {
        FlowSummary {
            id: 0,
            state: FlowState::Completed,
            scheme,
            method: "GET".into(),
            host: host.into(),
            port: 443,
            path: path.into(),
            query: String::new(),
            status: Some(200),
            content_kind: kind,
            client: Some("Safari".into()),
            started_at_ms: 0.0,
            duration_ms: Some(1),
            req_size: 0,
            resp_size: 0,
            edited: false,
            http_version: HttpVersion::Http11,
            tls: TlsStatus::None,
            graphql_op: None,
        }
    }

    #[test]
    fn text_is_case_insensitive_over_url_parts() {
        let f = flow(Scheme::Https, "api.example.com", "/V1/Users", ContentKind::Json);
        let compile = |text: &str| {
            CompiledFilter::compile(&FilterSpec {
                text: text.into(),
                ..Default::default()
            })
        };
        assert!(compile("EXAMPLE").matches(&f));
        assert!(compile("v1/users").matches(&f));
        assert!(compile("example.com/v1").matches(&f));
        assert!(!compile("orders").matches(&f));
        assert!(compile("").is_match_all());
    }

    #[test]
    fn chips_combine_protocol_and_content() {
        let spec = FilterSpec {
            protocol: ProtocolFilter::Https,
            content: vec![ContentGroup::Json, ContentGroup::Media],
            ..Default::default()
        };
        let filter = CompiledFilter::compile(&spec);
        assert!(filter.matches(&flow(Scheme::Https, "a", "/", ContentKind::Json)));
        assert!(filter.matches(&flow(Scheme::Https, "a", "/", ContentKind::Image)));
        assert!(!filter.matches(&flow(Scheme::Http, "a", "/", ContentKind::Json)));
        assert!(!filter.matches(&flow(Scheme::Wss, "a", "/", ContentKind::Json)));
        assert!(!filter.matches(&flow(Scheme::Https, "a", "/", ContentKind::Css)));
    }

    #[test]
    fn host_and_client_are_exact() {
        let spec = FilterSpec {
            host: Some("a.com".into()),
            client: Some("Safari".into()),
            ..Default::default()
        };
        let filter = CompiledFilter::compile(&spec);
        assert!(filter.matches(&flow(Scheme::Https, "a.com", "/", ContentKind::Json)));
        assert!(!filter.matches(&flow(Scheme::Https, "b.a.com", "/", ContentKind::Json)));

        let domain = CompiledFilter::compile(&FilterSpec {
            host: Some("*.a.com".into()),
            ..Default::default()
        });
        assert!(domain.matches(&flow(Scheme::Https, "a.com", "/", ContentKind::Json)));
        assert!(domain.matches(&flow(Scheme::Https, "b.a.com", "/", ContentKind::Json)));
        assert!(!domain.matches(&flow(Scheme::Https, "ba.com", "/", ContentKind::Json)));
    }
}
