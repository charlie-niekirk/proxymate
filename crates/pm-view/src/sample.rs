//! A sample flow, its row and its detail: fixtures for tests and offscreen
//! renders, which build the inspector without an engine. Tests change the
//! fields they care about.

use std::sync::Arc;

use pm_model::{
    BodyInfo, CertSummary, ContentKind, FlowDetail, FlowRow, FlowRules, FlowState, FlowSummary, Header, HttpVersion,
    RequestHead, ResponseHead, Scheme, Timing, TlsInfo, TlsStatus, Via,
};

pub fn header(name: &str, value: &str) -> Header {
    Header {
        name: name.into(),
        value: value.into(),
    }
}

/// `GET https://api.example.com/v1/users?page=2&q=a+b`, 200 OK over h2 from Safari.
pub fn summary() -> FlowSummary {
    FlowSummary {
        id: 3,
        state: FlowState::Completed,
        scheme: Scheme::Https,
        method: Arc::from("GET"),
        host: Arc::from("api.example.com"),
        port: 443,
        path: "/v1/users".into(),
        query: "page=2&q=a+b".into(),
        status: Some(200),
        content_kind: ContentKind::Json,
        client: Some(Arc::from("Safari")),
        started_at_ms: 1_767_225_600_000.0,
        duration_ms: Some(184),
        req_size: 0,
        resp_size: 1536,
        edited: false,
        http_version: HttpVersion::Http2,
        tls: TlsStatus::Intercepted,
        graphql_op: None,
        sent_by: None,
        pinned: false,
        grpc_status: None,
        label: None,
        comment: None,
        extra: Vec::new(),
    }
}

/// The table row of [`summary`].
pub fn row() -> FlowRow {
    row_of(&summary())
}

/// The table row of a summary.
pub fn row_of(f: &FlowSummary) -> FlowRow {
    FlowRow(
        f.id,
        f.state,
        f.scheme,
        f.method.clone(),
        f.host.clone(),
        f.port,
        f.path.clone(),
        f.query.clone(),
        f.status,
        f.content_kind,
        f.client.clone(),
        f.started_at_ms,
        f.duration_ms,
        f.req_size,
        f.resp_size,
        f.edited,
        f.http_version,
        f.tls,
        f.graphql_op.clone(),
        f.sent_by,
        f.pinned,
        f.grpc_status,
        f.label,
        f.comment.clone(),
        f.extra.clone(),
    )
}

fn body(size: u64, kind: ContentKind) -> BodyInfo {
    BodyInfo {
        size,
        captured: size,
        truncated: false,
        encoding: None,
        complete: true,
        kind,
        mime: None,
    }
}

/// The detail of [`summary`]: request and response heads, a TLS leg with a
/// two-certificate chain, timings and a direct route.
pub fn detail() -> FlowDetail {
    FlowDetail {
        id: 3,
        request: RequestHead {
            method: "GET".into(),
            url: "https://api.example.com/v1/users?page=2&q=a+b".into(),
            version: HttpVersion::Http2,
            headers: vec![
                header(":authority", "api.example.com"),
                header("accept", "application/json"),
                header("authorization", "Bearer eyJhbGciOi.payload.sig"),
                header("cookie", "session=abc123; theme=dark"),
                header("user-agent", "Safari/605.1.15"),
            ],
        },
        response: Some(ResponseHead {
            status: 200,
            reason: "OK".into(),
            version: HttpVersion::Http2,
            headers: vec![
                header("content-type", "application/json; charset=utf-8"),
                header("set-cookie", "session=xyz789; Path=/; HttpOnly"),
                header("x-request-id", "req-42"),
            ],
        }),
        request_body: body(0, ContentKind::None),
        response_body: body(1536, ContentKind::Json),
        error: None,
        tls: Some(TlsInfo {
            sni: Some("api.example.com".into()),
            client_alpn: vec!["h2".into(), "http/1.1".into()],
            alpn: Some("h2".into()),
            version: "TLSv1.3".into(),
            cipher: "TLS13_AES_128_GCM_SHA256".into(),
            upstream_verified: true,
            chain: vec![
                CertSummary {
                    subject: "CN=api.example.com".into(),
                    issuer: "CN=Example Issuing CA".into(),
                    serial: "0A:1B:2C".into(),
                    not_before: "2025-06-01T00:00:00Z".into(),
                    not_after: "2026-06-01T00:00:00Z".into(),
                    san: vec!["api.example.com".into(), "*.example.com".into()],
                    sha256: "AB:CD:EF:01".into(),
                },
                CertSummary {
                    subject: "CN=Example Issuing CA".into(),
                    issuer: "CN=Example Root".into(),
                    serial: "05".into(),
                    not_before: "2020-01-01T00:00:00Z".into(),
                    not_after: "2030-01-01T00:00:00Z".into(),
                    san: vec![],
                    sha256: "12:34:56:78".into(),
                },
            ],
        }),
        timing: Timing {
            dns_ms: Some(3.2),
            connect_ms: Some(11.0),
            tls_ms: Some(24.5),
            ttfb_ms: Some(120.0),
            transfer_ms: Some(25.3),
            reused_connection: false,
        },
        rules: FlowRules::default(),
        trailers: None,
        via: Some(Via {
            addr: Some("93.184.215.14:443".into()),
            ..Default::default()
        }),
        error_kind: None,
    }
}
