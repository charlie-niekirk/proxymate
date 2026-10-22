//! Deterministic synthetic traffic, for seeding the UI before the proxy
//! exists and for tests and benches afterwards.

use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pm_model::{ContentKind, FlowState, FlowSummary, HttpVersion, Scheme, TlsStatus};

use crate::Engine;

struct Site {
    host: &'static str,
    client: &'static str,
    paths: &'static [&'static str],
    kinds: &'static [ContentKind],
}

const SITES: &[Site] = &[
    Site {
        host: "api.github.com",
        client: "Safari",
        paths: &["/repos/rust-lang/rust/pulls", "/user", "/graphql", "/notifications"],
        kinds: &[ContentKind::Json, ContentKind::Json, ContentKind::GraphQl],
    },
    Site {
        host: "github.githubassets.com",
        client: "Safari",
        paths: &["/assets/app.js", "/assets/primer.css", "/images/mona.png"],
        kinds: &[ContentKind::Javascript, ContentKind::Css, ContentKind::Image],
    },
    Site {
        host: "www.google.com",
        client: "Google Chrome",
        paths: &["/search", "/complete/search", "/gen_204", "/"],
        kinds: &[ContentKind::Html, ContentKind::Json, ContentKind::None],
    },
    Site {
        host: "fonts.gstatic.com",
        client: "Google Chrome",
        paths: &["/s/inter/v13/inter.woff2", "/s/roboto/v30/roboto.woff2"],
        kinds: &[ContentKind::Font],
    },
    Site {
        host: "api.stripe.com",
        client: "Postman",
        paths: &["/v1/charges", "/v1/customers", "/v1/payment_intents"],
        kinds: &[ContentKind::Json, ContentKind::FormUrlEncoded],
    },
    Site {
        host: "slack.com",
        client: "Slack",
        paths: &["/api/conversations.history", "/api/users.info", "/api/rtm.connect"],
        kinds: &[ContentKind::Json],
    },
    Site {
        host: "wss-primary.slack.com",
        client: "Slack",
        paths: &["/websocket/abc123"],
        kinds: &[ContentKind::None],
    },
    Site {
        host: "i.ytimg.com",
        client: "Safari",
        paths: &["/vi/dQw4w9WgXcQ/hqdefault.jpg", "/vi/abc/maxresdefault.webp"],
        kinds: &[ContentKind::Image],
    },
    Site {
        host: "rr3---sn-a5mekn6s.googlevideo.com",
        client: "Safari",
        paths: &["/videoplayback"],
        kinds: &[ContentKind::Video, ContentKind::Audio],
    },
    Site {
        host: "localhost",
        client: "node",
        paths: &["/api/health", "/api/todos", "/api/todos/42", "/graphql"],
        kinds: &[ContentKind::Json, ContentKind::GraphQl, ContentKind::Text],
    },
    Site {
        host: "feeds.example.org",
        client: "NetNewsWire",
        paths: &["/rss.xml", "/atom.xml", "/sitemap.xml"],
        kinds: &[ContentKind::Xml],
    },
    Site {
        host: "cdn.jsdelivr.net",
        client: "Firefox",
        paths: &["/npm/react@19/umd/react.production.min.js", "/npm/logo.svg"],
        kinds: &[ContentKind::Javascript, ContentKind::Svg],
    },
    Site {
        host: "docs.example.com",
        client: "Firefox",
        paths: &["/guide/getting-started", "/files/manual.pdf", "/upload"],
        kinds: &[ContentKind::Html, ContentKind::Pdf, ContentKind::Multipart],
    },
    Site {
        host: "telemetry.example.net",
        client: "Spotify",
        paths: &["/v1/events", "/collect"],
        kinds: &[ContentKind::None, ContentKind::Binary],
    },
];

/// Operation names for synthetic GraphQL flows.
const GRAPHQL_OPS: &[&str] = &["GetViewer", "ListPullRequests", "UpdateTodo", "SearchRepositories"];

const METHODS: &[(&str, u32)] = &[
    ("GET", 70),
    ("POST", 16),
    ("PUT", 4),
    ("PATCH", 3),
    ("DELETE", 2),
    ("OPTIONS", 3),
    ("HEAD", 2),
];

const STATUSES: &[(u16, u32)] = &[
    (200, 72),
    (201, 4),
    (204, 4),
    (304, 6),
    (301, 2),
    (302, 2),
    (400, 2),
    (401, 2),
    (403, 1),
    (404, 3),
    (429, 1),
    (500, 1),
    (502, 1),
    (503, 1),
];

/// splitmix64-based generator; the same seed always yields the same flows.
pub struct Generator {
    state: u64,
    clock_ms: f64,
    methods: Vec<Arc<str>>,
    hosts: Vec<Arc<str>>,
    clients: Vec<Arc<str>>,
}

impl Generator {
    pub fn new(seed: u64) -> Self {
        Generator {
            state: seed,
            clock_ms: now_ms(),
            methods: METHODS.iter().map(|(m, _)| Arc::from(*m)).collect(),
            hosts: SITES.iter().map(|s| Arc::from(s.host)).collect(),
            clients: SITES.iter().map(|s| Arc::from(s.client)).collect(),
        }
    }

    /// Sets the start time of the next flow; later flows advance from it.
    pub fn set_clock(&mut self, ms_since_epoch: f64) {
        self.clock_ms = ms_since_epoch;
    }

    /// A flow that has finished (completed, failed or a tunnel).
    pub fn completed_flow(&mut self) -> FlowSummary {
        let mut flow = self.active_flow();
        self.complete(&mut flow);
        flow
    }

    /// A flow whose request is in flight.
    pub fn active_flow(&mut self) -> FlowSummary {
        let site_idx = self.below(SITES.len() as u64) as usize;
        let site = &SITES[site_idx];
        let is_ws = site.host.starts_with("wss-");
        let scheme = match (is_ws, site.host) {
            (true, _) => Scheme::Wss,
            (false, "localhost") => Scheme::Http,
            _ if self.below(10) == 0 => Scheme::Http,
            _ => Scheme::Https,
        };
        let method = if is_ws {
            self.methods[0].clone()
        } else {
            let i = self.weighted(METHODS.iter().map(|(_, w)| *w));
            self.methods[i].clone()
        };
        let path = site.paths[self.below(site.paths.len() as u64) as usize];
        let query = match self.below(4) {
            0 => format!("page={}&per_page=50", self.below(20) + 1),
            1 => format!("q=proxymate+{}&utm_source=synthetic", self.below(1000)),
            _ => String::new(),
        };
        let port = match scheme {
            Scheme::Http if site.host == "localhost" => 3000,
            Scheme::Http | Scheme::Ws => 80,
            Scheme::Https | Scheme::Wss => 443,
        };
        let req_size = if matches!(&*method, "POST" | "PUT" | "PATCH") {
            self.log_size(200, 40_000)
        } else {
            0
        };
        self.clock_ms += 1.0 + self.below(40) as f64;
        let content_kind = site.kinds[self.below(site.kinds.len() as u64) as usize];
        let graphql_op = (content_kind == ContentKind::GraphQl)
            .then(|| Arc::from(GRAPHQL_OPS[self.below(GRAPHQL_OPS.len() as u64) as usize]));
        FlowSummary {
            id: 0,
            state: FlowState::Active,
            scheme,
            method,
            host: self.hosts[site_idx].clone(),
            port,
            path: path.to_owned(),
            query,
            status: None,
            content_kind,
            client: Some(self.clients[site_idx].clone()),
            started_at_ms: self.clock_ms,
            duration_ms: None,
            req_size,
            resp_size: 0,
            edited: self.below(50) == 0,
            http_version: if scheme.is_tls() && self.below(3) > 0 {
                HttpVersion::Http2
            } else {
                HttpVersion::Http11
            },
            tls: if scheme.is_tls() {
                TlsStatus::Intercepted
            } else {
                TlsStatus::None
            },
            graphql_op,
            sent_by: None,
        }
    }

    /// Finishes an active flow with a status, duration and response size.
    pub fn complete(&mut self, flow: &mut FlowSummary) {
        let roll = self.below(100);
        if flow.scheme.is_websocket() {
            flow.state = FlowState::Completed;
            flow.status = Some(101);
        } else if roll < 2 {
            flow.state = FlowState::Failed;
        } else if roll < 3 {
            flow.state = FlowState::Blocked;
            flow.status = Some(403);
        } else if roll < 5 && flow.scheme == Scheme::Https {
            flow.state = FlowState::Tunnel;
            flow.method = Arc::from("CONNECT");
            flow.path = String::from("/");
            flow.query.clear();
            flow.content_kind = ContentKind::None;
            flow.graphql_op = None;
        } else {
            flow.state = FlowState::Completed;
            let i = self.weighted(STATUSES.iter().map(|(_, w)| *w));
            flow.status = Some(STATUSES[i].0);
            flow.resp_size = match flow.content_kind {
                ContentKind::None => 0,
                ContentKind::Video | ContentKind::Audio => self.log_size(100_000, 8_000_000),
                ContentKind::Image | ContentKind::Font | ContentKind::Pdf => self.log_size(2_000, 900_000),
                _ => self.log_size(80, 250_000),
            };
        }
        flow.duration_ms = Some(self.log_size(4, 4_000) as u32);
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Log-uniform in `[lo, hi)`, which looks like real size and latency data.
    fn log_size(&mut self, lo: u64, hi: u64) -> u64 {
        let (lo, hi) = ((lo as f64).ln(), (hi as f64).ln());
        (lo + (hi - lo) * self.unit()).exp() as u64
    }

    fn weighted(&mut self, weights: impl Iterator<Item = u32> + Clone) -> usize {
        let total: u32 = weights.clone().sum();
        let mut roll = self.below(total as u64) as u32;
        for (i, w) in weights.enumerate() {
            if roll < w {
                return i;
            }
            roll -= w;
        }
        unreachable!("roll is below the weight total")
    }
}

/// Inserts `count` finished flows in batches, ending at roughly "now".
pub fn seed(engine: &Engine, count: usize, seed: u64) {
    const BATCH: usize = 10_000;
    let mut generator = Generator::new(seed);
    // ~21 ms average spacing between flows, so the history ends now.
    generator.set_clock(now_ms() - count as f64 * 21.0);
    let mut remaining = count;
    while remaining > 0 {
        let n = remaining.min(BATCH);
        engine.insert_flows((0..n).map(|_| generator.completed_flow()).collect());
        remaining -= n;
    }
}

/// Starts a thread that keeps adding live traffic: about `per_sec` new flows
/// per second, each staying active for a few hundred ms before completing.
/// Stops when the engine is dropped.
pub fn spawn_live(engine: Weak<Engine>, per_sec: u32, seed: u64) {
    const TICK: Duration = Duration::from_millis(50);
    std::thread::Builder::new()
        .name("pm-synth".into())
        .spawn(move || {
            let mut generator = Generator::new(seed);
            let mut in_flight: Vec<(FlowSummary, u32)> = Vec::new();
            let mut budget = 0.0f64;
            loop {
                std::thread::sleep(TICK);
                let Some(engine) = engine.upgrade() else {
                    return;
                };
                budget += per_sec as f64 * TICK.as_secs_f64();
                let new: Vec<FlowSummary> = (0..budget as usize).map(|_| generator.active_flow()).collect();
                budget -= new.len() as f64;
                if !new.is_empty() {
                    let ids = engine.insert_flows(new.clone());
                    for (mut flow, id) in new.into_iter().zip(ids) {
                        flow.id = id;
                        let ticks = 2 + generator.below(20) as u32;
                        in_flight.push((flow, ticks));
                    }
                }
                let mut done = Vec::new();
                in_flight.retain_mut(|(flow, ticks)| {
                    *ticks -= 1;
                    if *ticks == 0 {
                        let mut flow = flow.clone();
                        generator.complete(&mut flow);
                        done.push(flow);
                        false
                    } else {
                        true
                    }
                });
                if !done.is_empty() {
                    engine.update_flows(done);
                }
            }
        })
        .expect("spawn synth thread");
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_flows() {
        let mut a = Generator::new(42);
        let mut b = Generator::new(42);
        b.set_clock(0.0);
        a.set_clock(0.0);
        for _ in 0..100 {
            assert_eq!(a.completed_flow(), b.completed_flow());
        }
    }

    #[test]
    fn completed_flows_are_finished() {
        let mut generator = Generator::new(3);
        for _ in 0..1_000 {
            let flow = generator.completed_flow();
            assert_ne!(flow.state, FlowState::Active);
            assert!(flow.duration_ms.is_some());
            assert!(flow.path.starts_with('/'));
        }
    }

    #[test]
    fn seed_inserts_the_requested_count() {
        let engine = Engine::new();
        seed(&engine, 25_000, 1);
        assert_eq!(engine.total(), 25_000);
    }
}
