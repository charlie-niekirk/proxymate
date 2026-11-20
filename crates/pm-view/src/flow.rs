//! What the flow table shows for a row, ported from `lib/flow.ts` and
//! `lib/tls.ts`: the coloured URL's parts, which method and status token a
//! pill takes, the status pill's text, and the compact sizes, durations and
//! times. Its table of tests is `flow.test.ts`'s.

use pm_model::{FlowState, HttpVersion, Scheme, TlsStatus};

/// A URL split into the parts the table colours.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlParts {
    pub scheme: String,
    pub host: String,
    /// `:3000`, or empty for the scheme's default port.
    pub port: String,
    pub path: String,
    /// `?page=2`, or empty.
    pub query: String,
}

fn default_port(scheme: Scheme) -> u16 {
    if scheme.is_tls() { 443 } else { 80 }
}

pub fn url_parts(scheme: Scheme, host: &str, port: u16, path: &str, query: &str) -> UrlParts {
    UrlParts {
        scheme: format!("{}://", scheme.as_str()),
        host: host.to_owned(),
        port: if port == default_port(scheme) {
            String::new()
        } else {
            format!(":{port}")
        },
        path: path.to_owned(),
        query: if query.is_empty() {
            String::new()
        } else {
            format!("?{query}")
        },
    }
}

/// The absolute URL a summary describes (the default port left out).
pub fn flow_url(scheme: Scheme, host: &str, port: u16, path: &str, query: &str) -> String {
    let p = url_parts(scheme, host, port, path, query);
    format!("{}{}{}{}{}", p.scheme, p.host, p.port, p.path, p.query)
}

/// Which `--m-*` token a method's pill takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodTone {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
    Connect,
    Other,
}

impl MethodTone {
    pub fn of(method: &str) -> MethodTone {
        match method.to_ascii_lowercase().as_str() {
            "get" => MethodTone::Get,
            "post" => MethodTone::Post,
            "put" => MethodTone::Put,
            "patch" => MethodTone::Patch,
            "delete" => MethodTone::Delete,
            "head" => MethodTone::Head,
            "options" => MethodTone::Options,
            "connect" => MethodTone::Connect,
            _ => MethodTone::Other,
        }
    }

    /// The token's suffix (`--m-<this>`).
    pub fn as_str(self) -> &'static str {
        match self {
            MethodTone::Get => "get",
            MethodTone::Post => "post",
            MethodTone::Put => "put",
            MethodTone::Patch => "patch",
            MethodTone::Delete => "delete",
            MethodTone::Head => "head",
            MethodTone::Options => "options",
            MethodTone::Connect => "connect",
            MethodTone::Other => "other",
        }
    }
}

/// Which `--s-*` token a status pill takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTone {
    S1xx,
    S2xx,
    S3xx,
    S4xx,
    S5xx,
    Active,
    Paused,
    Blocked,
    Tunnel,
    /// Outlined in `--s-failed`, not filled.
    Failed,
}

impl StatusTone {
    pub fn as_str(self) -> &'static str {
        match self {
            StatusTone::S1xx => "1xx",
            StatusTone::S2xx => "2xx",
            StatusTone::S3xx => "3xx",
            StatusTone::S4xx => "4xx",
            StatusTone::S5xx => "5xx",
            StatusTone::Active => "active",
            StatusTone::Paused => "paused",
            StatusTone::Blocked => "blocked",
            StatusTone::Tunnel => "tunnel",
            StatusTone::Failed => "failed",
        }
    }
}

const GRPC_CODES: [&str; 17] = [
    "OK",
    "CANCELLED",
    "UNKNOWN",
    "INVALID_ARGUMENT",
    "DEADLINE_EXCEEDED",
    "NOT_FOUND",
    "ALREADY_EXISTS",
    "PERMISSION_DENIED",
    "RESOURCE_EXHAUSTED",
    "FAILED_PRECONDITION",
    "ABORTED",
    "OUT_OF_RANGE",
    "UNIMPLEMENTED",
    "INTERNAL",
    "UNAVAILABLE",
    "DATA_LOSS",
    "UNAUTHENTICATED",
];

/// gRPC status names by code.
pub fn grpc_name(code: u8) -> String {
    GRPC_CODES
        .get(code as usize)
        .map_or_else(|| format!("UNKNOWN({code})"), |s| (*s).to_owned())
}

/// Server-side failures colour like 5xx, the caller's like 4xx.
fn grpc_server_fault(code: u8) -> bool {
    matches!(code, 2 | 4 | 13 | 14 | 15)
}

pub fn status_tone(status: Option<u16>, state: FlowState, grpc: Option<u8>) -> StatusTone {
    if let (FlowState::Completed, Some(code)) = (state, grpc) {
        return match code {
            0 => StatusTone::S2xx,
            c if grpc_server_fault(c) => StatusTone::S5xx,
            _ => StatusTone::S4xx,
        };
    }
    match state {
        FlowState::Active => StatusTone::Active,
        FlowState::Paused => StatusTone::Paused,
        FlowState::Blocked => StatusTone::Blocked,
        FlowState::Tunnel => StatusTone::Tunnel,
        FlowState::Failed => StatusTone::Failed,
        FlowState::Completed => match status {
            None => StatusTone::S1xx,
            Some(s) if s >= 500 => StatusTone::S5xx,
            Some(s) if s >= 400 => StatusTone::S4xx,
            Some(s) if s >= 300 => StatusTone::S3xx,
            Some(s) if s >= 200 => StatusTone::S2xx,
            Some(_) => StatusTone::S1xx,
        },
    }
}

/// The status pill's text.
pub fn status_label(status: Option<u16>, state: FlowState, grpc: Option<u8>) -> String {
    if let (FlowState::Completed | FlowState::Blocked, Some(code)) = (state, grpc) {
        return grpc_name(code);
    }
    match state {
        // Switched protocols and still open: a live WebSocket.
        FlowState::Active => if status == Some(101) { "Active" } else { "…" }.into(),
        FlowState::Paused => "Paused".into(),
        FlowState::Failed => "Failed".into(),
        FlowState::Tunnel => "Tunnel".into(),
        FlowState::Blocked | FlowState::Completed => status.map_or_else(|| "—".into(), |s| s.to_string()),
    }
}

/// The lowercase state name (the state dot's tooltip).
pub fn state_name(state: FlowState) -> &'static str {
    match state {
        FlowState::Active => "active",
        FlowState::Paused => "paused",
        FlowState::Completed => "completed",
        FlowState::Failed => "failed",
        FlowState::Blocked => "blocked",
        FlowState::Tunnel => "tunnel",
    }
}

/// Short protocol label for the table: `h2`, `1.1`, `1.0`.
pub fn version_label(version: HttpVersion) -> &'static str {
    match version {
        HttpVersion::Http2 => "h2",
        HttpVersion::Http11 => "1.1",
        HttpVersion::Http10 => "1.0",
    }
}

/// The SSL column's tooltip.
pub fn ssl_title(tls: TlsStatus) -> &'static str {
    match tls {
        TlsStatus::None => "",
        TlsStatus::Intercepted => "Decrypted by Proxymate",
        TlsStatus::Passthrough => "Not decrypted: SSL passthrough",
        TlsStatus::LikelyPinned => "The client rejected the Proxymate certificate (likely pinned)",
    }
}

/// JavaScript's `toFixed`: ties round away from zero (Rust's `{:.1}` rounds
/// 1.25 to "1.2"; the Tauri UI shows "1.3").
fn fixed(x: f64, digits: usize) -> String {
    let p = 10f64.powi(digits as i32);
    format!("{:.digits$}", (x * p).round() / p)
}

pub fn format_ms(ms: Option<f64>) -> String {
    match ms {
        None => "—".into(),
        Some(ms) if ms < 1.0 => format!("{} ms", fixed(ms, 2)),
        Some(ms) if ms < 100.0 => format!("{} ms", fixed(ms, 1)),
        Some(ms) => format!("{} ms", ms.round()),
    }
}

pub fn format_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    match bytes {
        0 => "—".into(),
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{} KB", fixed(b / K, 1)),
        n if n < 1024 * 1024 * 1024 => format!("{} MB", fixed(b / K / K, 1)),
        _ => format!("{} GB", fixed(b / K / K / K, 2)),
    }
}

pub fn format_duration(ms: Option<u32>) -> String {
    match ms {
        None => "—".into(),
        Some(ms) if ms < 1000 => format!("{ms} ms"),
        Some(ms) if ms < 60_000 => format!("{} s", fixed(ms as f64 / 1000.0, 2)),
        Some(ms) => format!("{}m {}s", ms / 60_000, ((ms % 60_000) as f64 / 1000.0).round()),
    }
}

/// `HH:MM:SS.mmm` in local time.
pub fn format_time(ms: Option<f64>) -> String {
    use chrono::{Local, TimeZone as _, Timelike as _};
    let Some(ms) = ms else { return "—".into() };
    match Local.timestamp_millis_opt(ms as i64).single() {
        Some(t) => format!(
            "{:02}:{:02}:{:02}.{:03}",
            t.hour(),
            t.minute(),
            t.second(),
            t.timestamp_subsec_millis().min(999)
        ),
        None => "—".into(),
    }
}

/// 999, 1.2k, 12k, 1.2M: sidebar counts stay narrow.
pub fn format_count(n: u64) -> String {
    match n {
        n if n < 1000 => n.to_string(),
        n if n < 10_000 => format!("{}k", fixed(n as f64 / 1000.0, 1)),
        n if n < 1_000_000 => format!("{}k", (n as f64 / 1000.0).round()),
        n => format!("{}M", fixed(n as f64 / 1_000_000.0, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parts_omit_default_ports_and_prefix_the_query() {
        assert_eq!(
            url_parts(Scheme::Https, "api.example.com", 443, "/v1/users", "page=2"),
            UrlParts {
                scheme: "https://".into(),
                host: "api.example.com".into(),
                port: "".into(),
                path: "/v1/users".into(),
                query: "?page=2".into(),
            }
        );
        assert_eq!(url_parts(Scheme::Http, "localhost", 3000, "/", "").port, ":3000");
        assert_eq!(url_parts(Scheme::Wss, "h", 443, "/ws", "").port, "");
        assert_eq!(
            flow_url(Scheme::Http, "localhost", 3000, "/a", "b=1"),
            "http://localhost:3000/a?b=1"
        );
    }

    #[test]
    fn methods_map_to_m_tokens() {
        assert_eq!(MethodTone::of("POST").as_str(), "post");
        assert_eq!(MethodTone::of("PROPFIND").as_str(), "other");
    }

    #[test]
    fn status_classes_and_flow_states_map_to_s_tokens() {
        let t = |s, st| status_tone(s, st, None).as_str();
        assert_eq!(t(Some(204), FlowState::Completed), "2xx");
        assert_eq!(t(Some(302), FlowState::Completed), "3xx");
        assert_eq!(t(Some(429), FlowState::Completed), "4xx");
        assert_eq!(t(Some(503), FlowState::Completed), "5xx");
        assert_eq!(t(None, FlowState::Active), "active");
        assert_eq!(t(None, FlowState::Failed), "failed");
        assert_eq!(status_label(None, FlowState::Tunnel, None), "Tunnel");
        assert_eq!(status_label(Some(101), FlowState::Active, None), "Active");
        assert_eq!(status_label(None, FlowState::Active, None), "…");
        assert_eq!(status_label(None, FlowState::Completed, None), "—");
    }

    #[test]
    fn grpc_statuses_read_by_name() {
        let c = FlowState::Completed;
        assert_eq!(status_label(Some(200), c, Some(0)), "OK");
        assert_eq!(status_label(Some(200), c, Some(5)), "NOT_FOUND");
        assert_eq!(status_label(Some(200), c, Some(40)), "UNKNOWN(40)");
        assert_eq!(status_label(Some(200), FlowState::Active, Some(0)), "…");
        assert_eq!(status_tone(Some(200), c, Some(0)), StatusTone::S2xx);
        assert_eq!(status_tone(Some(200), c, Some(5)), StatusTone::S4xx);
        assert_eq!(status_tone(Some(200), c, Some(14)), StatusTone::S5xx);
    }

    #[test]
    fn sizes_and_durations_read_compactly() {
        assert_eq!(format_size(0), "—");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_size(3 << 30), "3.00 GB");
        assert_eq!(format_duration(None), "—");
        assert_eq!(format_duration(Some(42)), "42 ms");
        assert_eq!(format_duration(Some(1500)), "1.50 s");
        assert_eq!(format_duration(Some(125_000)), "2m 5s");
        assert_eq!(format_ms(Some(0.25)), "0.25 ms");
        assert_eq!(format_ms(Some(12.34)), "12.3 ms");
        assert_eq!(format_ms(Some(250.6)), "251 ms");
    }

    #[test]
    fn http_versions_read_compactly() {
        assert_eq!(version_label(HttpVersion::Http2), "h2");
        assert_eq!(version_label(HttpVersion::Http11), "1.1");
    }

    #[test]
    fn times_are_local_with_milliseconds() {
        let s = format_time(Some(1_700_000_000_123.0));
        assert_eq!(s.len(), 12, "{s}");
        assert!(s.ends_with(".123"), "{s}");
        assert_eq!(format_time(None), "—");
    }

    #[test]
    fn counts_stay_narrow() {
        let got: Vec<String> = [0, 999, 1000, 1234, 12_345, 999_999, 1_250_000]
            .into_iter()
            .map(format_count)
            .collect();
        assert_eq!(got, ["0", "999", "1.0k", "1.2k", "12k", "1000k", "1.3M"]);
    }
}
