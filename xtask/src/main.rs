//! `cargo xtask`: Proxymate's build tooling, in Rust (the Native track's
//! G15). `bundle-macos` builds the native app into `Proxymate.app`, signs
//! it, and stages a release: the DMG, the updater archive and its minisign
//! signature, and the `updater-darwin.json` fragment of `latest.json`, all
//! in the formats the Tauri release (`scripts/release/macos.sh`) produces,
//! so installed Tauri copies update straight into the native app.
//! `updater-key` makes a throwaway updater key pair in Tauri's format.
//!
//! The Tauri release path stays as it is until G16 switches `release.yml`
//! to this.

mod bundle;
mod cmd;
mod sign;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  bundle-macos [options]   build, sign, check and stage the native macOS release
      --arch <a>             universal (default), host, aarch64 or x86_64
      --debug                a debug build (the updater's end-to-end test)
      --version <v>          the app's version (default: the workspace's); it is
                             compiled into the binary and written to Info.plist
      --features <f>         extra cargo features for proxymate-native
      --out <dir>            where to stage (default $OUT_DIR, else
                             target/release/native-macos)
      --no-dmg               skip the DMG
  updater-key <path>       write a new updater key pair, unencrypted (empty
                           password), to <path> and <path>.pub, base64-wrapped
                           like `tauri signer generate`

environment (bundle-macos), as for scripts/release/macos.sh:
  TAURI_SIGNING_PRIVATE_KEY, TAURI_SIGNING_PRIVATE_KEY_PASSWORD   the updater key
  APPLE_SIGNING_IDENTITY, APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD,
  APPLE_API_KEY, APPLE_API_ISSUER, APPLE_API_KEY_PATH             (dormant)
  RELEASE_TAG    if set, must be v<version>
  DOWNLOAD_URL   base URL of the release's assets, for updater-darwin.json
  PROXYMATE_ALLOW_UNSIGNED_DMG=1   accept a DMG without a Developer ID signature
";

type Result<T = ()> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("bundle-macos") => bundle::Options::parse(&args[1..]).and_then(bundle::run),
        Some("updater-key") => match args.get(1) {
            Some(path) => sign::write_key_pair(&PathBuf::from(path)),
            None => Err("updater-key needs a path".into()),
        },
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => Err(format!("unknown command\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            cmd::error(&err);
            ExitCode::FAILURE
        }
    }
}
