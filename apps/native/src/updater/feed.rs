//! The update feed and its signatures: the Tauri app's `latest.json` on
//! GitHub Releases, and minisign signatures from the same key
//! (`plugins.updater` in `apps/desktop/src-tauri/tauri.conf.json`), so a
//! release serves both apps.
//!
//! Requests never go through a proxy (the system proxy may be Proxymate
//! itself). The feed must answer within 30 s; a download may take longer,
//! but not stall for 30 s. A 404 means nothing is published yet.

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use semver::Version;
use serde::Deserialize;

/// The feed every release updates (`latest.json` of the newest published,
/// non-prerelease GitHub Release).
pub const ENDPOINT: &str = "https://github.com/charlie-niekirk/proxymate/releases/latest/download/latest.json";

/// The updater's public key, as `tauri.conf.json` holds it (base64 of the
/// minisign public key box). `the_public_key_is_the_tauri_apps` keeps them
/// equal until G16 deletes the Tauri app.
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDNBREFCREMyQUYwNzQ4ODEKUldTQlNBZXZ3cjNhT3E4QzJRbEtISmFySzAzYmtJWEw5Q2pDcHhVVVJDTWNqOENQQ0JzWWdYK3YK";

pub const TIMEOUT: Duration = Duration::from_secs(30);

/// Where to look and whom to trust. A debug build can be compiled against a
/// local feed and a throwaway key (`PROXYMATE_UPDATER_ENDPOINT`,
/// `PROXYMATE_UPDATER_PUBKEY` at build time) for the end-to-end test in
/// docs/RELEASING.md, as the Tauri app's `--config` override does; a release
/// build ignores both.
#[derive(Clone, Debug)]
pub struct Config {
    pub endpoint: String,
    pub public_key: String,
}

impl Config {
    pub fn app() -> Config {
        let debug = |value: Option<&'static str>| value.filter(|_| cfg!(debug_assertions));
        Config {
            endpoint: debug(option_env!("PROXYMATE_UPDATER_ENDPOINT"))
                .unwrap_or(ENDPOINT)
                .to_owned(),
            public_key: debug(option_env!("PROXYMATE_UPDATER_PUBKEY"))
                .unwrap_or(PUBLIC_KEY)
                .to_owned(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Network(String),
    #[error("the update server answered {0}")]
    Status(u16),
    #[error("the update feed can't be read: {0}")]
    Feed(String),
    #[error("the update has no build for {0}")]
    NoPlatform(String),
    #[error("the update's signature is not valid: {0}")]
    Signature(String),
    #[error("updates aren't available on this platform yet")]
    Unsupported,
}

/// An update newer than this build.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub version: String,
    pub current: String,
    pub notes: Option<String>,
    pub url: String,
    pub signature: String,
}

#[derive(Deserialize)]
struct Feed {
    version: String,
    notes: Option<String>,
    platforms: HashMap<String, Platform>,
}

#[derive(Deserialize)]
struct Platform {
    url: String,
    signature: String,
}

/// The feed's key for this build, as tauri-plugin-updater names it.
pub fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-aarch64"),
        ("macos", "x86_64") => Some("darwin-x86_64"),
        _ => None,
    }
}

/// The update `feed` offers `current` on `target`, if it is newer. Like
/// the Tauri plugin, `<target>-app` (the bundle installer) wins over the
/// bare `<target>`.
pub fn pick(feed: &[u8], current: &str, target: &str) -> Result<Option<Update>, Error> {
    let feed: Feed = serde_json::from_slice(feed).map_err(|e| Error::Feed(e.to_string()))?;
    let parse = |v: &str| Version::parse(v.trim_start_matches('v')).map_err(|e| Error::Feed(format!("{v}: {e}")));
    if parse(&feed.version)? <= parse(current)? {
        return Ok(None);
    }
    let platform = feed
        .platforms
        .get(&format!("{target}-app"))
        .or_else(|| feed.platforms.get(target))
        .ok_or_else(|| Error::NoPlatform(target.to_owned()))?;
    Ok(Some(Update {
        version: feed.version.trim_start_matches('v').to_owned(),
        current: current.to_owned(),
        notes: feed.notes.filter(|n| !n.trim().is_empty()),
        url: platform.url.clone(),
        signature: platform.signature.clone(),
    }))
}

/// One client for a check and its download. `pm_mcp::install_crypto_provider`
/// first: the workspace's reqwest has rustls with no provider of its own.
pub fn client() -> Result<reqwest::Client, Error> {
    pm_mcp::install_crypto_provider();
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(TIMEOUT)
        .read_timeout(TIMEOUT)
        .user_agent(concat!("Proxymate/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Network(e.to_string()))
}

/// Reads the feed: `None` when there's nothing newer, or nothing published
/// (404).
pub async fn check(client: &reqwest::Client, config: &Config, current: &str) -> Result<Option<Update>, Error> {
    let target = target().ok_or(Error::Unsupported)?;
    check_for(client, &config.endpoint, current, target).await
}

pub async fn check_for(
    client: &reqwest::Client,
    endpoint: &str,
    current: &str,
    target: &str,
) -> Result<Option<Update>, Error> {
    let response = client
        .get(endpoint)
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    match response.status().as_u16() {
        404 => return Ok(None),
        200..=299 => {}
        other => return Err(Error::Status(other)),
    }
    let body = response.bytes().await.map_err(|e| Error::Network(e.to_string()))?;
    pick(&body, current, target)
}

/// Downloads the update and checks its signature against `public_key`
/// before anything touches the disk.
pub async fn download(client: &reqwest::Client, update: &Update, public_key: &str) -> Result<Vec<u8>, Error> {
    let response = client
        .get(&update.url)
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    if !response.status().is_success() {
        return Err(Error::Status(response.status().as_u16()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| Error::Network(e.to_string()))?
        .to_vec();
    verify(&bytes, &update.signature, public_key, &update.version)?;
    Ok(bytes)
}

/// Checks `signature` (base64 of a minisign signature box) over `data`
/// against `public_key` (base64 of a minisign public key box), then the
/// version its trusted comment was signed for against the one the feed
/// announced: the feed isn't signed, so this is what stops it pairing a new
/// version number with an older release. Unlike the Tauri app, a signature
/// without a version is refused: every release the native app can install
/// is signed by `cargo xtask bundle-macos`, which writes one.
pub fn verify(data: &[u8], signature: &str, public_key: &str, announced: &str) -> Result<(), Error> {
    let bad = |e: String| Error::Signature(e);
    let key = minisign_verify::PublicKey::decode(&unbase64(public_key).map_err(bad)?)
        .map_err(|e| bad(format!("public key: {e}")))?;
    let signature =
        minisign_verify::Signature::decode(&unbase64(signature).map_err(bad)?).map_err(|e| bad(e.to_string()))?;
    key.verify(data, &signature, false).map_err(|e| bad(e.to_string()))?;
    // Only now is the trusted comment trustworthy: verify checked the global
    // signature over it.
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
        .ok_or_else(|| bad("it names no version".into()))?;
    let same = match (
        Version::parse(signed.trim_start_matches('v')),
        Version::parse(announced.trim_start_matches('v')),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => signed == announced,
    };
    if same {
        Ok(())
    } else {
        Err(bad(format!("it was signed for {signed}, not {announced}")))
    }
}

fn unbase64(text: &str) -> Result<String, String> {
    let bytes = STANDARD.decode(text.trim()).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key pair and a signer in the format `cargo xtask bundle-macos` and
    /// the Tauri bundler write.
    fn keys() -> (minisign::SecretKey, String) {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let public = STANDARD.encode(pair.pk.to_box().unwrap().into_string());
        (pair.sk, public)
    }

    fn sign(key: &minisign::SecretKey, data: &[u8], trusted: &str) -> String {
        let signature =
            minisign::sign(None, key, data, Some(trusted), Some("signature from tauri secret key")).unwrap();
        STANDARD.encode(signature.to_string())
    }

    fn feed(version: &str, platforms: &[&str]) -> Vec<u8> {
        let platforms: serde_json::Map<_, _> = platforms
            .iter()
            .map(|p| {
                let entry = serde_json::json!({"url": format!("https://x/{p}.tar.gz"), "signature": "c2ln"});
                ((*p).to_owned(), entry)
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({
            "version": version, "notes": "See the release", "pub_date": "2026-11-20T00:00:00Z", "platforms": platforms,
        }))
        .unwrap()
    }

    #[test]
    fn a_newer_version_for_this_platform_is_an_update() {
        let update = pick(
            &feed("0.7.0", &["darwin-aarch64", "darwin-x86_64"]),
            "0.6.0",
            "darwin-aarch64",
        )
        .unwrap()
        .unwrap();
        assert_eq!(update.version, "0.7.0");
        assert_eq!(update.current, "0.6.0");
        assert_eq!(update.url, "https://x/darwin-aarch64.tar.gz");
        assert_eq!(update.notes.as_deref(), Some("See the release"));
        // A `v` prefix, as tags have, is fine either side.
        assert!(
            pick(&feed("v0.7.0", &["darwin-aarch64"]), "0.6.0", "darwin-aarch64")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn the_same_or_an_older_version_is_not() {
        let platforms = ["darwin-aarch64"];
        assert_eq!(
            pick(&feed("0.6.0", &platforms), "0.6.0", "darwin-aarch64").unwrap(),
            None
        );
        assert_eq!(
            pick(&feed("0.5.9", &platforms), "0.6.0", "darwin-aarch64").unwrap(),
            None
        );
        // A prerelease is older than its release.
        assert_eq!(
            pick(&feed("0.7.0-beta.1", &platforms), "0.7.0", "darwin-aarch64").unwrap(),
            None
        );
    }

    #[test]
    fn the_installer_specific_key_wins_and_a_missing_platform_is_an_error() {
        let update = pick(
            &feed("1.0.0", &["darwin-aarch64", "darwin-aarch64-app"]),
            "0.6.0",
            "darwin-aarch64",
        )
        .unwrap()
        .unwrap();
        assert_eq!(update.url, "https://x/darwin-aarch64-app.tar.gz");
        assert!(matches!(
            pick(&feed("1.0.0", &["linux-x86_64"]), "0.6.0", "darwin-aarch64"),
            Err(Error::NoPlatform(_))
        ));
        assert!(matches!(
            pick(b"<html>", "0.6.0", "darwin-aarch64"),
            Err(Error::Feed(_))
        ));
    }

    #[test]
    fn a_good_signature_for_the_announced_version_verifies() {
        let (key, public) = keys();
        let sig = sign(&key, b"bundle", "timestamp:1\tfile:P.app.tar.gz\tversion:0.7.0");
        verify(b"bundle", &sig, &public, "0.7.0").unwrap();
        verify(b"bundle", &sig, &public, "v0.7.0").unwrap();
    }

    #[test]
    fn a_bad_signature_is_refused() {
        let (key, public) = keys();
        let sig = sign(&key, b"bundle", "timestamp:1\tfile:P.app.tar.gz\tversion:0.7.0");
        // Tampered data, another key, a corrupt signature.
        assert!(matches!(
            verify(b"bundlf", &sig, &public, "0.7.0"),
            Err(Error::Signature(_))
        ));
        let (_, other) = keys();
        assert!(matches!(
            verify(b"bundle", &sig, &other, "0.7.0"),
            Err(Error::Signature(_))
        ));
        assert!(matches!(
            verify(b"bundle", "bm9wZQ==", &public, "0.7.0"),
            Err(Error::Signature(_))
        ));
        // Signed for another version, or for none.
        assert!(matches!(
            verify(b"bundle", &sig, &public, "0.8.0"),
            Err(Error::Signature(_))
        ));
        let old = sign(&key, b"bundle", "timestamp:1\tfile:P.app.tar.gz");
        assert!(matches!(
            verify(b"bundle", &old, &public, "0.7.0"),
            Err(Error::Signature(_))
        ));
    }

    /// One release feeds both apps until G16, so both trust one key.
    #[test]
    fn the_public_key_is_the_tauri_apps() {
        let conf = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../desktop/src-tauri/tauri.conf.json");
        let conf: serde_json::Value = serde_json::from_slice(&std::fs::read(conf).unwrap()).unwrap();
        assert_eq!(conf["plugins"]["updater"]["pubkey"], PUBLIC_KEY);
        assert_eq!(conf["plugins"]["updater"]["endpoints"][0], ENDPOINT);
        assert_eq!(conf["identifier"], pm_app::BUNDLE_ID);
    }

    /// The feed over HTTP: an update, nothing published (404), a server error.
    #[test]
    fn check_reads_the_feed_over_http() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let body = feed("9.0.0", &["darwin-aarch64"]);
            let server = serve(vec![
                ("200 OK", body.clone()),
                ("404 Not Found", Vec::new()),
                ("500 Internal Server Error", Vec::new()),
            ])
            .await;
            let client = client().unwrap();
            let update = check_for(&client, &server, "0.6.0", "darwin-aarch64").await.unwrap();
            assert_eq!(update.unwrap().version, "9.0.0");
            assert_eq!(
                check_for(&client, &server, "0.6.0", "darwin-aarch64").await.unwrap(),
                None
            );
            assert!(matches!(
                check_for(&client, &server, "0.6.0", "darwin-aarch64").await,
                Err(Error::Status(500))
            ));
        });
    }

    /// Answers one connection per response, in order; returns its URL.
    async fn serve(responses: Vec<(&'static str, Vec<u8>)>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/latest.json", listener.local_addr().unwrap());
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 4096];
                let _ = socket.read(&mut request).await;
                let head = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        url
    }
}
