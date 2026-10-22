//! Apps and Domains counts for the sidebar, kept current as flows arrive
//! and change, so a snapshot is one pass over the distinct names rather
//! than over every flow.

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::Arc;

use pm_model::{DomainNode, FlowSummary, HostCount};

#[derive(Default)]
pub struct SidebarCounts {
    hosts: HashMap<Arc<str>, u32>,
    apps: HashMap<Arc<str>, u32>,
    dirty: bool,
}

impl SidebarCounts {
    pub fn add(&mut self, f: &FlowSummary) {
        bump(&mut self.hosts, &f.host);
        if let Some(app) = &f.client {
            bump(&mut self.apps, app);
        }
        self.dirty = true;
    }

    /// Takes an evicted flow out of the counts.
    pub fn remove(&mut self, f: &FlowSummary) {
        drop_one(&mut self.hosts, &f.host);
        if let Some(app) = &f.client {
            drop_one(&mut self.apps, app);
        }
        self.dirty = true;
    }

    /// Moves a flow's counts after its host or client changed.
    pub fn change(&mut self, old_host: &Arc<str>, old_client: Option<&Arc<str>>, new: &FlowSummary) {
        if **old_host != *new.host {
            drop_one(&mut self.hosts, old_host);
            bump(&mut self.hosts, &new.host);
            self.dirty = true;
        }
        if old_client.map(|c| &**c) != new.client.as_deref() {
            if let Some(old) = old_client {
                drop_one(&mut self.apps, old);
            }
            if let Some(app) = &new.client {
                bump(&mut self.apps, app);
            }
            self.dirty = true;
        }
    }

    pub fn clear(&mut self) {
        self.hosts.clear();
        self.apps.clear();
        self.dirty = true;
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// True once per change: the publisher sends a snapshot only then.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// App names and counts, sorted by name ignoring case.
    pub fn apps(&self) -> Vec<(Arc<str>, u32)> {
        let mut apps: Vec<(Arc<str>, u32)> = self.apps.iter().map(|(a, n)| (a.clone(), *n)).collect();
        apps.sort_by_cached_key(|(a, _)| a.to_lowercase());
        apps
    }

    /// Hosts grouped under their registrable domain, sorted.
    pub fn domains(&self) -> Vec<DomainNode> {
        let mut groups: BTreeMap<&str, Vec<HostCount>> = BTreeMap::new();
        for (host, &count) in &self.hosts {
            groups.entry(registrable_domain(host)).or_default().push(HostCount {
                host: host.to_string(),
                count,
            });
        }
        groups
            .into_iter()
            .map(|(domain, mut hosts)| {
                hosts.sort_by(|a, b| a.host.cmp(&b.host));
                let count = hosts.iter().map(|h| h.count).sum();
                if hosts.len() == 1 && hosts[0].host == domain {
                    hosts.clear();
                }
                DomainNode {
                    domain: domain.to_owned(),
                    count,
                    hosts,
                }
            })
            .collect()
    }
}

fn bump(map: &mut HashMap<Arc<str>, u32>, key: &Arc<str>) {
    *map.entry(key.clone()).or_default() += 1;
}

fn drop_one(map: &mut HashMap<Arc<str>, u32>, key: &Arc<str>) {
    if let Some(n) = map.get_mut(key) {
        *n -= 1;
        if *n == 0 {
            map.remove(key);
        }
    }
}

/// Second-level labels under a two-letter country code that act as public
/// suffixes (`co.uk`, `com.au`). A heuristic, not the Public Suffix List.
const SECOND_LEVEL: [&str; 10] = ["ac", "co", "com", "edu", "go", "gov", "ne", "net", "or", "org"];

/// The domain a host groups under in the sidebar: `api.example.com` →
/// `example.com`, `www.bbc.co.uk` → `bbc.co.uk`. IP literals and single
/// labels (`localhost`) stand alone.
pub fn registrable_domain(host: &str) -> &str {
    if host.parse::<IpAddr>().is_ok() {
        return host;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    if n <= 2 {
        return host;
    }
    let keep = if labels[n - 1].len() == 2 && SECOND_LEVEL.contains(&labels[n - 2]) {
        3
    } else {
        2
    };
    if n <= keep {
        return host;
    }
    let skip: usize = labels[..n - keep].iter().map(|l| l.len() + 1).sum();
    &host[skip..]
}

/// True if `host` is `domain` or a host under it.
pub fn in_domain(host: &str, domain: &str) -> bool {
    host.len() >= domain.len()
        && host[host.len() - domain.len()..].eq_ignore_ascii_case(domain)
        && (host.len() == domain.len() || host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_group_by_registrable_domain() {
        assert_eq!(registrable_domain("api.github.com"), "github.com");
        assert_eq!(registrable_domain("github.com"), "github.com");
        assert_eq!(registrable_domain("a.b.c.example.org"), "example.org");
        assert_eq!(registrable_domain("www.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(registrable_domain("www.abc.de"), "abc.de");
        assert_eq!(registrable_domain("localhost"), "localhost");
        assert_eq!(registrable_domain("127.0.0.1"), "127.0.0.1");
        assert_eq!(registrable_domain("::1"), "::1");
        assert!(in_domain("api.github.com", "github.com"));
        assert!(in_domain("GitHub.com", "github.com"));
        assert!(!in_domain("notgithub.com", "github.com"));
    }

    #[test]
    fn counts_follow_changes() {
        let mut flow = crate::synth::Generator::new(1).completed_flow();
        flow.host = "api.example.com".into();
        flow.client = None;
        let mut counts = SidebarCounts::default();
        counts.add(&flow);
        counts.add(&flow);
        assert!(counts.take_dirty());
        assert!(counts.apps().is_empty());

        let old_host = flow.host.clone();
        flow.client = Some("Safari".into());
        counts.change(&old_host, None, &flow);
        assert_eq!(counts.apps(), vec![("Safari".into(), 1)]);

        flow.host = "example.com".into();
        let mut other = flow.clone();
        other.host = "cdn.other.net".into();
        counts.change(&old_host, flow.client.as_ref(), &flow);
        counts.add(&other);
        let domains = counts.domains();
        assert_eq!(domains.len(), 2);
        assert_eq!(domains[0].domain, "example.com");
        assert_eq!(domains[0].count, 2);
        assert_eq!(
            domains[0].hosts,
            vec![
                HostCount {
                    host: "api.example.com".into(),
                    count: 1
                },
                HostCount {
                    host: "example.com".into(),
                    count: 1
                },
            ]
        );
        assert_eq!(domains[1].domain, "other.net");
        assert_eq!(domains[1].hosts.len(), 1);
    }
}
