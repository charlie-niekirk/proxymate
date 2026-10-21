//! The onboarding server: what a device gets when it asks the proxy itself
//! for a page (`http://<lan-ip>:<port>/`, or `http://proxy.mate/` once its
//! proxy is set) instead of a page to forward.
//!
//! - `/` — the setup page: proxy address, CA fingerprint and the steps for
//!   iOS and Android.
//! - `/cert.pem` — the CA as PEM.
//! - `/cert.crt` — the CA as DER, which Android installs.
//! - `/proxymate.mobileconfig` — an iOS configuration profile holding the CA
//!   as a root certificate payload.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use pm_model::CaInfo;

/// Everything a response is built from.
pub struct Onboarding<'a> {
    pub ca: &'a CaInfo,
    pub der: &'a [u8],
    /// The address devices should set as their HTTP proxy, if known.
    pub proxy: Option<String>,
    pub user_agent: &'a str,
}

/// A response, independent of hyper so it can be tested directly.
#[derive(Debug)]
pub struct Page {
    pub status: u16,
    pub content_type: &'static str,
    /// `attachment` file name, for downloads.
    pub file_name: Option<&'static str>,
    pub body: Vec<u8>,
}

impl Page {
    fn new(status: u16, content_type: &'static str, body: impl Into<Vec<u8>>) -> Page {
        Page {
            status,
            content_type,
            file_name: None,
            body: body.into(),
        }
    }

    fn download(mut self, file_name: &'static str) -> Page {
        self.file_name = Some(file_name);
        self
    }
}

pub const MOBILECONFIG_TYPE: &str = "application/x-apple-aspen-config";
pub const DER_TYPE: &str = "application/x-x509-ca-cert";
pub const PEM_TYPE: &str = "application/x-pem-file";
const HTML_TYPE: &str = "text/html; charset=utf-8";

/// True if a request for `path` is one of the downloads, which count as a
/// device taking part in setup.
pub fn is_download(path: &str) -> bool {
    matches!(path, "/cert.pem" | "/cert.crt" | "/proxymate.mobileconfig")
}

/// The response for a GET of `path`.
pub fn route(path: &str, ctx: &Onboarding<'_>) -> Page {
    match path {
        "/" | "/index.html" | "/ssl" => Page::new(200, HTML_TYPE, setup_page(ctx)),
        "/cert.pem" => Page::new(200, PEM_TYPE, pem(ctx.der)).download("proxymate-ca.pem"),
        "/cert.crt" => Page::new(200, DER_TYPE, ctx.der.to_vec()).download("proxymate-ca.crt"),
        // No attachment: iOS Safari only offers to install a profile it
        // is shown, not one it is told to save.
        "/proxymate.mobileconfig" => Page::new(200, MOBILECONFIG_TYPE, mobileconfig(ctx.ca, ctx.der)),
        _ => Page::new(
            404,
            HTML_TYPE,
            page("Not found", "<p>Proxymate's setup page is at <a href=\"/\">/</a>.</p>"),
        ),
    }
}

pub fn method_not_allowed() -> Page {
    Page::new(
        405,
        "text/plain; charset=utf-8",
        "Proxymate: only GET is served here.\n",
    )
}

/// The DER as a PEM `CERTIFICATE` block with 64-column lines.
pub fn pem(der: &[u8]) -> String {
    let b64 = STANDARD.encode(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

/// An unsigned configuration profile with one `com.apple.security.root`
/// payload. Its identifiers and UUIDs derive from the CA fingerprint, so
/// installing it again replaces the old profile instead of adding one, and
/// a regenerated CA gets a profile of its own.
pub fn mobileconfig(ca: &CaInfo, der: &[u8]) -> String {
    let fp = ca.sha256.replace(':', "").to_ascii_lowercase();
    let short = fp.get(..16).unwrap_or(&fp);
    let profile_uuid = uuid_from(&fp, 0);
    let payload_uuid = uuid_from(&fp, 1);
    let name = xml_escape(&ca.common_name);
    let data = STANDARD
        .encode(der)
        .as_bytes()
        .chunks(52)
        .map(|c| format!("\t\t\t{}", String::from_utf8_lossy(c)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>PayloadContent</key>
	<array>
		<dict>
			<key>PayloadCertificateFileName</key>
			<string>proxymate-ca.cer</string>
			<key>PayloadContent</key>
			<data>
{data}
			</data>
			<key>PayloadDescription</key>
			<string>Adds the Proxymate root certificate, so Proxymate can show this device's HTTPS traffic.</string>
			<key>PayloadDisplayName</key>
			<string>{name}</string>
			<key>PayloadIdentifier</key>
			<string>app.proxymate.ca.{short}.root</string>
			<key>PayloadType</key>
			<string>com.apple.security.root</string>
			<key>PayloadUUID</key>
			<string>{payload_uuid}</string>
			<key>PayloadVersion</key>
			<integer>1</integer>
		</dict>
	</array>
	<key>PayloadDescription</key>
	<string>Installs the certificate Proxymate uses to decrypt HTTPS. After installing, turn on full trust for it in Settings → General → About → Certificate Trust Settings.</string>
	<key>PayloadDisplayName</key>
	<string>Proxymate CA</string>
	<key>PayloadIdentifier</key>
	<string>app.proxymate.ca.{short}</string>
	<key>PayloadOrganization</key>
	<string>Proxymate</string>
	<key>PayloadRemovalDisallowed</key>
	<false/>
	<key>PayloadType</key>
	<string>Configuration</string>
	<key>PayloadUUID</key>
	<string>{profile_uuid}</string>
	<key>PayloadVersion</key>
	<integer>1</integer>
</dict>
</plist>
"#
    )
}

/// A version-4-shaped UUID from 16 bytes of the fingerprint at `index`
/// (wrapping), so it is stable for a CA.
fn uuid_from(hex: &str, index: usize) -> String {
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .filter_map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok())
        .collect();
    let mut b = [0u8; 16];
    for (i, slot) in b.iter_mut().enumerate() {
        *slot = bytes.get((index * 16 + i) % bytes.len().max(1)).copied().unwrap_or(0);
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(b).hyphenated().to_string().to_ascii_uppercase()
}

fn setup_page(ctx: &Onboarding<'_>) -> String {
    let ua = ctx.user_agent;
    let ios = ua.contains("iPhone") || ua.contains("iPad") || ua.contains("iPod");
    let android = ua.contains("Android");
    let proxy = match &ctx.proxy {
        Some(proxy) => format!("<p>Proxy: <code>{}</code></p>", xml_escape(proxy)),
        None => String::new(),
    };
    let ios_section = r#"<section>
<h2>iPhone and iPad</h2>
<a class="button" href="/proxymate.mobileconfig">Download profile</a>
<ol>
<li>Tap <b>Allow</b> when Safari asks to download the configuration profile.</li>
<li>Open <b>Settings → Profile Downloaded</b> (or General → VPN &amp; Device Management) and tap <b>Install</b>.</li>
<li>Open <b>Settings → General → About → Certificate Trust Settings</b> and turn on full trust for the Proxymate CA.</li>
</ol>
</section>"#;
    let android_section = r#"<section>
<h2>Android</h2>
<a class="button" href="/cert.crt">Download certificate</a>
<ol>
<li>Open <b>Settings → Security &amp; privacy → More security settings → Encryption &amp; credentials → Install a certificate → CA certificate</b> and pick <code>proxymate-ca.crt</code> from Downloads.</li>
<li>Apps only trust user CAs in debug builds that opt in with a <code>network_security_config.xml</code>; Chrome trusts them already.</li>
</ol>
</section>"#;
    let other_section = r#"<section>
<h2>Other devices</h2>
<p>Download the certificate as <a href="/cert.pem">PEM</a> or <a href="/cert.crt">DER</a> and add it to the device's trusted roots.</p>
</section>"#;
    let sections = if android && !ios {
        [android_section, ios_section, other_section]
    } else {
        [ios_section, android_section, other_section]
    };
    let body = format!(
        r#"<p class="lead">Proxymate can show this device's traffic once its Wi-Fi proxy points here and it trusts the Proxymate certificate.</p>
{proxy}
{}
<footer><p>Certificate: {}<br>SHA-256: <code class="fp">{}</code></p></footer>"#,
        sections.join("\n"),
        xml_escape(&ctx.ca.common_name),
        xml_escape(&ctx.ca.sha256),
    );
    page("Proxymate setup", &body)
}

fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{ color-scheme: light dark; --accent: #f97316; }}
body {{ font: 16px/1.5 -apple-system, system-ui, sans-serif; max-width: 34rem; margin: 0 auto; padding: 1.5rem 1rem 3rem; }}
h1 {{ font-size: 1.5rem; margin: 0 0 .25rem; }}
h2 {{ font-size: 1.1rem; margin: 1.75rem 0 .5rem; }}
.lead {{ color: GrayText; margin-top: 0; }}
.button {{ display: inline-block; background: var(--accent); color: #fff; text-decoration: none; font-weight: 600; padding: .6rem 1.1rem; border-radius: .6rem; }}
ol {{ padding-left: 1.25rem; }} li {{ margin: .35rem 0; }}
code {{ font: .9em ui-monospace, Menlo, monospace; }}
.fp {{ word-break: break-all; font-size: .75rem; }}
footer {{ margin-top: 2rem; color: GrayText; font-size: .85rem; }}
</style>
</head>
<body>
<h1>{title}</h1>
{body}
</body>
</html>
"#
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ca() -> CaInfo {
        CaInfo {
            common_name: "Proxymate CA (Test & Co, 2026-10-21)".into(),
            cert_path: "/tmp/ca.pem".into(),
            not_before: String::new(),
            not_after: String::new(),
            sha256: "0f:1e:2d:3c:4b:5a:69:78:87:96:a5:b4:c3:d2:e1:f0:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff"
                .into(),
        }
    }

    fn ctx<'a>(ca: &'a CaInfo, der: &'a [u8], ua: &'a str) -> Onboarding<'a> {
        Onboarding {
            ca,
            der,
            proxy: Some("10.0.0.5:9090".into()),
            user_agent: ua,
        }
    }

    #[test]
    fn routes_have_the_right_content_types() {
        let ca = ca();
        let der = [0x30, 0x82, 0x01, 0x0a, 0xde, 0xad];
        let ctx = ctx(&ca, &der, "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)");
        let page = route("/", &ctx);
        assert_eq!((page.status, page.content_type), (200, HTML_TYPE));
        let html = String::from_utf8(page.body).unwrap();
        assert!(html.contains("10.0.0.5:9090") && html.contains("Test &amp; Co"));
        assert!(html.find("iPhone and iPad").unwrap() < html.find("<h2>Android").unwrap());

        let page = route("/cert.crt", &ctx);
        assert_eq!(
            (page.content_type, page.file_name),
            (DER_TYPE, Some("proxymate-ca.crt"))
        );
        assert_eq!(page.body, der);

        let page = route("/cert.pem", &ctx);
        assert_eq!(page.content_type, PEM_TYPE);
        let text = String::from_utf8(page.body).unwrap();
        assert_eq!(
            text,
            "-----BEGIN CERTIFICATE-----\nMIIBCt6t\n-----END CERTIFICATE-----\n"
        );

        let page = route("/proxymate.mobileconfig", &ctx);
        assert_eq!((page.content_type, page.file_name), (MOBILECONFIG_TYPE, None));
        assert_eq!(route("/favicon.ico", &ctx).status, 404);
        assert!(is_download("/cert.pem") && !is_download("/"));

        let android = Onboarding {
            user_agent: "Mozilla/5.0 (Linux; Android 15; Pixel 8)",
            ..ctx
        };
        let html = String::from_utf8(route("/", &android).body).unwrap();
        assert!(html.find("<h2>Android").unwrap() < html.find("iPhone and iPad").unwrap());
    }

    #[test]
    fn mobileconfig_is_a_well_formed_profile() {
        let ca = ca();
        let der: Vec<u8> = (0..=255u8).cycle().take(700).collect();
        let xml = mobileconfig(&ca, &der);
        let plist = plist::Value::from_reader_xml(xml.as_bytes()).expect("valid plist XML");
        let root = plist.as_dictionary().unwrap();
        assert_eq!(root["PayloadType"].as_string(), Some("Configuration"));
        assert_eq!(root["PayloadVersion"].as_signed_integer(), Some(1));
        let profile_uuid = root["PayloadUUID"].as_string().unwrap();
        assert!(uuid::Uuid::parse_str(profile_uuid).is_ok());
        let payload = root["PayloadContent"].as_array().unwrap()[0].as_dictionary().unwrap();
        assert_eq!(payload["PayloadType"].as_string(), Some("com.apple.security.root"));
        assert_eq!(payload["PayloadContent"].as_data(), Some(der.as_slice()));
        assert_eq!(
            payload["PayloadDisplayName"].as_string(),
            Some("Proxymate CA (Test & Co, 2026-10-21)")
        );
        let payload_uuid = payload["PayloadUUID"].as_string().unwrap();
        assert_ne!(profile_uuid, payload_uuid);
        // Stable for the same CA.
        assert_eq!(xml, mobileconfig(&ca, &der));
        assert_eq!(
            root["PayloadIdentifier"].as_string(),
            Some("app.proxymate.ca.0f1e2d3c4b5a6978")
        );
    }
}
