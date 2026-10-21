//! Certificates turned into what the TLS tab shows.

use pm_model::CertSummary;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use x509_parser::extensions::GeneralName;

/// Summarises one DER certificate. Unparseable input still yields its
/// fingerprint, so the chain keeps its length.
pub fn summarize(der: &[u8]) -> CertSummary {
    let sha256 = sha256_hex(der);
    let Ok((_, cert)) = x509_parser::parse_x509_certificate(der) else {
        return CertSummary {
            subject: "(unparseable certificate)".into(),
            issuer: String::new(),
            serial: String::new(),
            not_before: String::new(),
            not_after: String::new(),
            san: Vec::new(),
            sha256,
        };
    };
    let san = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|ext| {
            ext.value
                .general_names
                .iter()
                .filter_map(|name| match name {
                    GeneralName::DNSName(dns) => Some((*dns).to_owned()),
                    GeneralName::IPAddress(bytes) => ip_string(bytes),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let validity = cert.validity();
    CertSummary {
        subject: cert.subject().to_string(),
        issuer: cert.issuer().to_string(),
        serial: hex(cert.raw_serial(), ":"),
        not_before: rfc3339(validity.not_before.to_datetime()),
        not_after: rfc3339(validity.not_after.to_datetime()),
        san,
        sha256,
    }
}

pub fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

pub fn sha256_hex(der: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, der).as_ref(), "")
}

fn hex(bytes: &[u8], sep: &str) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(sep)
}

fn ip_string(bytes: &[u8]) -> Option<String> {
    let ip: std::net::IpAddr = match bytes.len() {
        4 => <[u8; 4]>::try_from(bytes).ok()?.into(),
        16 => <[u8; 16]>::try_from(bytes).ok()?.into(),
        _ => return None,
    };
    Some(ip.to_string())
}
