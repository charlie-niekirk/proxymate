//! The TLS and Timing tabs' data: the port of `lib/tls.ts` (timing phases,
//! certificate expiry, the route) and of the rows `TlsView.tsx` builds,
//! with their tests.

use pm_model::{CertSummary, FlowDetail, Timing, Via};

/// Which colour a phase's bar takes (`--timing-*` tokens).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhaseTone {
    Dns,
    Connect,
    Tls,
    Wait,
    Receive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Phase {
    pub label: &'static str,
    pub ms: f64,
    /// Time before this phase starts, from the first phase.
    pub offset: f64,
    pub tone: PhaseTone,
}

/// The phases that have a value, in the order they happen.
pub fn timing_phases(t: &Timing) -> Vec<Phase> {
    let all = [
        ("DNS", t.dns_ms, PhaseTone::Dns),
        ("Connect", t.connect_ms, PhaseTone::Connect),
        ("TLS", t.tls_ms, PhaseTone::Tls),
        ("Waiting (TTFB)", t.ttfb_ms, PhaseTone::Wait),
        ("Download", t.transfer_ms, PhaseTone::Receive),
    ];
    let mut offset = 0.0;
    let mut phases = Vec::new();
    for (label, ms, tone) in all {
        let Some(ms) = ms else { continue };
        phases.push(Phase {
            label,
            ms,
            offset,
            tone,
        });
        offset += ms;
    }
    phases
}

/// True if `cert` had expired at `at_ms` (when the flow was captured).
pub fn cert_expired_at(cert: &CertSummary, at_ms: f64) -> bool {
    chrono::DateTime::parse_from_rfc3339(&cert.not_after)
        .is_ok_and(|not_after| (not_after.timestamp_millis() as f64) < at_ms)
}

/// How the upstream connection was made (M18), as label/value rows.
pub fn route_rows(via: Option<&Via>) -> Vec<(String, String)> {
    let Some(via) = via else { return Vec::new() };
    let route = match &via.proxy {
        Some(proxy) => format!("Through {proxy}"),
        None => "Direct".into(),
    };
    let mut rows = vec![("Route".into(), route)];
    if let Some(addr) = &via.addr {
        rows.push(("Connected to".into(), addr.clone()));
    }
    if let Some(dns) = &via.dns_override {
        rows.push(("DNS override".into(), dns.clone()));
    }
    if let Some(port) = via.reverse_port {
        rows.push(("Arrived on".into(), format!("Reverse proxy :{port}")));
    }
    rows
}

/// The TLS tab's Connection table; `None` for a plain HTTP flow.
pub fn connection_rows(detail: &FlowDetail) -> Option<Vec<(String, String)>> {
    let tls = detail.tls.as_ref()?;
    let dash = |v: &str| if v.is_empty() { "—".to_owned() } else { v.to_owned() };
    let verification = if tls.version.is_empty() {
        "Failed (see the error)"
    } else if tls.upstream_verified {
        "Verified"
    } else {
        "Not verified: insecure upstream allowed for this host"
    };
    let mut rows = vec![
        (
            "Client SNI".into(),
            tls.sni.clone().unwrap_or_else(|| "— (no SNI: IP address)".into()),
        ),
        (
            "Client offered ALPN".into(),
            if tls.client_alpn.is_empty() {
                "—".into()
            } else {
                tls.client_alpn.join(", ")
            },
        ),
        (
            "Negotiated ALPN".into(),
            tls.alpn.clone().unwrap_or_else(|| "— (none: HTTP/1.1)".into()),
        ),
        ("Upstream TLS version".into(), dash(&tls.version)),
        ("Upstream cipher".into(), dash(&tls.cipher)),
        ("Upstream certificate".into(), verification.into()),
    ];
    if let Some(via) = &detail.via {
        if let Some(proxy) = &via.proxy {
            rows.push(("Upstream proxy".into(), proxy.clone()));
        }
        if let Some(cert) = &via.client_cert {
            rows.push(("Client certificate".into(), cert.clone()));
        }
    }
    Some(rows)
}

/// A certificate card's rows (under its subject line).
pub fn cert_rows(cert: &CertSummary, captured_at_ms: f64) -> Vec<(String, String)> {
    let expired = if cert_expired_at(cert, captured_at_ms) {
        " (expired when captured)"
    } else {
        ""
    };
    let mut rows = vec![
        ("Issuer".into(), cert.issuer.clone()),
        (
            "Valid".into(),
            format!(
                "{} → {}{expired}",
                format_date(&cert.not_before),
                format_date(&cert.not_after)
            ),
        ),
    ];
    if !cert.san.is_empty() {
        rows.push(("Names".into(), cert.san.join(", ")));
    }
    rows.push(("Serial".into(), cert.serial.clone()));
    rows.push(("SHA-256".into(), cert.sha256.clone()));
    rows
}

/// `2026-01-01T12:00:00Z` → `2026-01-01 12:00 UTC`.
pub fn format_date(rfc3339: &str) -> String {
    if rfc3339.is_empty() {
        return "—".into();
    }
    let spaced = rfc3339.replacen('T', " ", 1);
    // `:SS` (and a fraction) before a trailing `Z` go, as in `TlsView.tsx`.
    if let Some(body) = spaced.strip_suffix('Z')
        && let Some(colon) = body.rfind(':')
    {
        let secs = &body[colon + 1..];
        let (whole, frac) = secs.split_once('.').unwrap_or((secs, "0"));
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if whole.len() == 2 && digits(whole) && digits(frac) {
            return format!("{} UTC", &body[..colon]);
        }
    }
    spaced
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::TlsInfo;

    fn timing(dns: Option<f64>, connect: Option<f64>, tls: Option<f64>, ttfb: Option<f64>, dl: Option<f64>) -> Timing {
        Timing {
            dns_ms: dns,
            connect_ms: connect,
            tls_ms: tls,
            ttfb_ms: ttfb,
            transfer_ms: dl,
            reused_connection: false,
        }
    }

    #[test]
    fn keeps_recorded_phases_in_order_with_running_offsets() {
        let phases = timing_phases(&timing(Some(2.), None, Some(10.), Some(30.), Some(5.)));
        let got: Vec<(&str, f64, f64)> = phases.iter().map(|p| (p.label, p.offset, p.ms)).collect();
        assert_eq!(
            got,
            [
                ("DNS", 0., 2.),
                ("TLS", 2., 10.),
                ("Waiting (TTFB)", 12., 30.),
                ("Download", 42., 5.)
            ]
        );
        assert!(timing_phases(&timing(None, None, None, None, None)).is_empty());
    }

    fn cert(not_after: &str) -> CertSummary {
        CertSummary {
            subject: "CN=a".into(),
            issuer: "CN=ca".into(),
            serial: "01".into(),
            not_before: "2025-01-01T00:00:00Z".into(),
            not_after: not_after.into(),
            san: vec![],
            sha256: "AB:CD".into(),
        }
    }

    fn ms(rfc3339: &str) -> f64 {
        chrono::DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .timestamp_millis() as f64
    }

    #[test]
    fn judges_expiry_at_capture_time() {
        let c = cert("2026-01-01T00:00:00Z");
        assert!(!cert_expired_at(&c, ms("2025-12-31T00:00:00Z")));
        assert!(cert_expired_at(&c, ms("2026-01-02T00:00:00Z")));
        assert!(!cert_expired_at(&cert("garbage"), 1e15));
        let rows = cert_rows(&c, ms("2026-01-02T00:00:00Z"));
        assert_eq!(
            rows[1].1,
            "2025-01-01 00:00 UTC → 2026-01-01 00:00 UTC (expired when captured)"
        );
        assert_eq!(rows.len(), 4, "no Names row without SANs");
    }

    #[test]
    fn formats_dates() {
        assert_eq!(format_date(""), "—");
        assert_eq!(format_date("2026-03-04T05:06:07Z"), "2026-03-04 05:06 UTC");
        assert_eq!(format_date("2026-03-04T05:06:07.123Z"), "2026-03-04 05:06 UTC");
        assert_eq!(format_date("2026-03-04T05:06:07+01:00"), "2026-03-04 05:06:07+01:00");
    }

    #[test]
    fn names_the_upstream_proxy_the_address_overrides_and_the_reverse_port() {
        assert!(route_rows(None).is_empty());
        let direct = Via {
            addr: Some("93.184.215.14:443".into()),
            ..Default::default()
        };
        assert_eq!(
            route_rows(Some(&direct)),
            [
                ("Route".into(), "Direct".into()),
                ("Connected to".into(), "93.184.215.14:443".into())
            ]
        );
        let all = Via {
            proxy: Some("socks5://127.0.0.1:1080".into()),
            addr: Some("127.0.0.1:1080".into()),
            dns_override: Some("*.test → 127.0.0.1".into()),
            client_cert: Some("CN=me".into()),
            reverse_port: Some(8443),
        };
        assert_eq!(
            route_rows(Some(&all)),
            [
                ("Route".into(), "Through socks5://127.0.0.1:1080".into()),
                ("Connected to".into(), "127.0.0.1:1080".into()),
                ("DNS override".into(), "*.test → 127.0.0.1".into()),
                ("Arrived on".into(), "Reverse proxy :8443".into()),
            ]
        );
    }

    #[test]
    fn the_connection_table_says_how_both_legs_were_negotiated() {
        let mut detail = crate::sample::detail();
        detail.tls = None;
        assert_eq!(connection_rows(&detail), None);
        detail.tls = Some(TlsInfo {
            sni: None,
            client_alpn: vec!["h2".into(), "http/1.1".into()],
            alpn: Some("h2".into()),
            version: "TLSv1.3".into(),
            cipher: "TLS13_AES_128_GCM_SHA256".into(),
            upstream_verified: true,
            chain: vec![],
        });
        detail.via = Some(Via {
            proxy: Some("http://p:3128".into()),
            client_cert: Some("CN=me".into()),
            ..Default::default()
        });
        let rows = connection_rows(&detail).unwrap();
        let get = |k: &str| rows.iter().find(|(l, _)| l == k).map(|(_, v)| v.as_str());
        assert_eq!(get("Client SNI"), Some("— (no SNI: IP address)"));
        assert_eq!(get("Client offered ALPN"), Some("h2, http/1.1"));
        assert_eq!(get("Upstream certificate"), Some("Verified"));
        assert_eq!(get("Upstream proxy"), Some("http://p:3128"));
        assert_eq!(get("Client certificate"), Some("CN=me"));
        let tls = detail.tls.as_mut().unwrap();
        tls.version.clear();
        assert_eq!(connection_rows(&detail).unwrap()[5].1, "Failed (see the error)");
        let tls = detail.tls.as_mut().unwrap();
        tls.version = "TLSv1.2".into();
        tls.upstream_verified = false;
        assert!(connection_rows(&detail).unwrap()[5].1.starts_with("Not verified"));
    }
}
