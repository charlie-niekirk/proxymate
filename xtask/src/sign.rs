//! The updater's signatures, in Tauri's format so that installed Tauri
//! copies (tauri-plugin-updater) and the native updater (minisign-verify)
//! both accept them:
//!
//! - `TAURI_SIGNING_PRIVATE_KEY` is a minisign secret key box ("untrusted
//!   comment: …\n<key>\n"), base64-encoded, encrypted with
//!   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (empty when unset);
//! - a `.sig` is the minisign signature box, base64-encoded, whose trusted
//!   comment is `timestamp:<unix>\tfile:<name>\tversion:<version>`. The
//!   updater checks the signed version against the one `latest.json`
//!   announces, since the feed itself is not signed.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use minisign::{KeyPair, SecretKey, SecretKeyBox};

use crate::Result;

/// The untrusted comment Tauri's bundler writes; nothing reads it.
const UNTRUSTED_COMMENT: &str = "signature from tauri secret key";

/// The secret key from Tauri's environment variables.
pub fn secret_key(base64_key: &str, password: Option<String>) -> Result<SecretKey> {
    let text = decode(base64_key.trim()).map_err(|e| format!("TAURI_SIGNING_PRIVATE_KEY: {e}"))?;
    let key_box = SecretKeyBox::from_string(&text).map_err(|e| format!("TAURI_SIGNING_PRIVATE_KEY: {e}"))?;
    // `None` would prompt on the terminal; an unset password is the empty one.
    key_box
        .into_secret_key(Some(password.unwrap_or_default()))
        .map_err(|e| format!("TAURI_SIGNING_PRIVATE_KEY: {e} (is TAURI_SIGNING_PRIVATE_KEY_PASSWORD right?)"))
}

/// Signs `data` (the file called `file_name`) for `version`: the `.sig`
/// file's contents.
pub fn sign(key: &SecretKey, data: &[u8], file_name: &str, version: &str) -> Result<String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let trusted = format!("timestamp:{timestamp}\tfile:{file_name}\tversion:{version}");
    let signature = minisign::sign(None, key, data, Some(&trusted), Some(UNTRUSTED_COMMENT))
        .map_err(|e| format!("can't sign {file_name}: {e}"))?;
    Ok(STANDARD.encode(signature.to_string()))
}

/// `cargo xtask updater-key <path>`: a new key pair with an empty password
/// (as `tauri signer generate --ci` makes), the secret key at `path` and
/// the public key, as `tauri.conf.json`'s `pubkey` takes it, at `path.pub`.
pub fn write_key_pair(path: &Path) -> Result {
    let (secret, public) = key_pair()?;
    let public_path = path.with_file_name(format!(
        "{}.pub",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    pm_write(path, &secret)?;
    pm_write(&public_path, &public)?;
    println!("secret key: {}\npublic key: {}", path.display(), public_path.display());
    Ok(())
}

/// A new key pair as (secret, public), each base64-wrapped.
pub fn key_pair() -> Result<(String, String)> {
    let KeyPair { pk, sk } =
        KeyPair::generate_encrypted_keypair(Some(String::new())).map_err(|e| format!("can't make a key: {e}"))?;
    let secret = sk.to_box(None).map_err(|e| e.to_string())?.into_string();
    let public = pk.to_box().map_err(|e| e.to_string())?.into_string();
    Ok((STANDARD.encode(secret), STANDARD.encode(public)))
}

fn pm_write(path: &Path, contents: &str) -> Result {
    std::fs::write(path, contents).map_err(|e| format!("can't write {}: {e}", path.display()))
}

fn decode(base64_text: &str) -> std::result::Result<String, String> {
    let bytes = STANDARD.decode(base64_text).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What tauri-plugin-updater does with a `.sig` and the configured
    /// `pubkey` (`verify_signature`), and what the native updater does.
    fn verify(public: &str, sig: &str, data: &[u8]) -> std::result::Result<String, String> {
        let pk = minisign_verify::PublicKey::decode(&decode(public)?).map_err(|e| e.to_string())?;
        let sig = minisign_verify::Signature::decode(&decode(sig)?).map_err(|e| e.to_string())?;
        pk.verify(data, &sig, true).map_err(|e| e.to_string())?;
        Ok(sig.trusted_comment().to_owned())
    }

    #[test]
    fn a_signature_verifies_like_tauri_and_names_its_version() {
        let (secret, public) = key_pair().unwrap();
        let key = secret_key(&secret, None).unwrap();
        let sig = sign(&key, b"bundle", "Proxymate.app.tar.gz", "1.2.3").unwrap();
        let trusted = verify(&public, &sig, b"bundle").unwrap();
        let fields: Vec<&str> = trusted.split('\t').collect();
        assert!(fields[0].starts_with("timestamp:"));
        assert_eq!(&fields[1..], ["file:Proxymate.app.tar.gz", "version:1.2.3"]);
        assert!(verify(&public, &sig, b"bundlf").is_err(), "tampered data");
        let (_, other) = key_pair().unwrap();
        assert!(verify(&other, &sig, b"bundle").is_err(), "another key");
    }

    /// A key from `npx tauri signer generate --ci -w k` (CLI 2.12, empty
    /// password; a throwaway made for this test): the release secret is in
    /// this format.
    #[test]
    fn reads_a_tauri_cli_key() {
        const TAURI_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V5ClJXUlRZMEl5bld4ajlwbVlwWjc3VXYrRXVzRzc1enFTZmxDR1N6NGY5NjdoNHE4TWd6RUFBQkFBQUFBQUFBQUFBQUlBQUFBQUY2TythTmFJdEJUMXFwZTNDYmxTY0J1WkNOMGhKZzQ4bHRwWW5YSStHN0xYS2t6T0lOUXRpQTVQOFM0ekxsUlJ4Q0QyRGJnVTNTM0JMUnl1cmxsK0FUblpLOE81bXd5Zi96aXl5Umg5MlNDeVF6bWZBL3RqeG9LMnNEcVdVQU1qMHNFV2J6WWtVTnc9Cg==";
        const TAURI_PUB: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEU4MDEzQTgzMzI5REFCNjQKUldSa3E1MHlnem9CNkF5a0VNWk5TM2ZsQlozV2FBQlFwL3l0aCtmZktCOTByejZTUC90ZkorTjIK";
        let key = secret_key(TAURI_KEY, None).unwrap();
        let sig = sign(&key, b"x", "x", "0.0.1").unwrap();
        verify(TAURI_PUB, &sig, b"x").unwrap();
        assert!(secret_key(TAURI_KEY, Some("wrong".into())).is_err());
    }
}
