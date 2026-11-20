//! `cargo xtask bundle-macos`: the native app as `Proxymate.app`, the way
//! the Tauri bundler and `scripts/release/macos.sh` make the Tauri one.
//!
//! 1. Build `proxymate-native` per architecture (`MACOSX_DEPLOYMENT_TARGET`
//!    13.3, the version compiled in as `PROXYMATE_APP_VERSION`), and `lipo`
//!    them into one universal executable, `Contents/MacOS/proxymate`.
//! 2. Write `Info.plist` (the Tauri app's keys: `dev.proxymate.desktop`,
//!    13.3, Developer Tools) and copy the icon.
//! 3. Sign: ad-hoc with the hardened runtime and the empty entitlements,
//!    unless `APPLE_SIGNING_IDENTITY` is set (Developer ID, timestamped,
//!    notarised and stapled when the App Store Connect key is there too).
//! 4. Check what ships, as `macos.sh` does: both architectures, every
//!    Mach-O signed with the runtime flag, the deployment target, and (with
//!    a Developer ID) the DMG's signature and notarisation.
//! 5. Stage: the DMG (`hdiutil`), the updater's `.app.tar.gz` with its
//!    `.sig`, and `updater-darwin.json`, named as the Tauri release names them.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::Compression;
use flate2::write::GzEncoder;
use plist::{Dictionary, Value};

use crate::cmd::{self, note, warn};
use crate::{Result, sign};

const APP_NAME: &str = "Proxymate";
const BUNDLE_ID: &str = "dev.proxymate.desktop";
/// The bundle's executable: the Tauri app's name, so paths in scripts and
/// docs (`Contents/MacOS/proxymate`) stay true across the update.
const EXECUTABLE: &str = "proxymate";
const PACKAGE: &str = "proxymate-native";
const MIN_MACOS: &str = "13.3";
const COPYRIGHT: &str = "© 2026 Charlie Niekirk";
const RELEASES: &str = "https://github.com/charlie-niekirk/proxymate/releases/download";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    Universal,
    Aarch64,
    X86_64,
}

impl Arch {
    fn parse(text: &str) -> Result<Arch> {
        match text {
            "universal" => Ok(Arch::Universal),
            "aarch64" | "arm64" => Ok(Arch::Aarch64),
            "x86_64" | "x64" => Ok(Arch::X86_64),
            "host" if cfg!(target_arch = "aarch64") => Ok(Arch::Aarch64),
            "host" => Ok(Arch::X86_64),
            other => Err(format!("unknown --arch {other}: universal, host, aarch64 or x86_64")),
        }
    }

    fn triples(self) -> &'static [&'static str] {
        match self {
            Arch::Universal => &["aarch64-apple-darwin", "x86_64-apple-darwin"],
            Arch::Aarch64 => &["aarch64-apple-darwin"],
            Arch::X86_64 => &["x86_64-apple-darwin"],
        }
    }

    /// The architectures `lipo -archs` must report.
    fn lipo_archs(self) -> &'static [&'static str] {
        match self {
            Arch::Universal => &["arm64", "x86_64"],
            Arch::Aarch64 => &["arm64"],
            Arch::X86_64 => &["x86_64"],
        }
    }

    /// The file-name label Tauri gives each target.
    fn label(self) -> &'static str {
        match self {
            Arch::Universal => "universal",
            Arch::Aarch64 => "aarch64",
            Arch::X86_64 => "x64",
        }
    }

    /// The `latest.json` platforms this bundle serves: the updater looks up
    /// `darwin-<arch>`, and one universal bundle serves both.
    fn platforms(self) -> &'static [&'static str] {
        match self {
            Arch::Universal => &["darwin-aarch64", "darwin-x86_64"],
            Arch::Aarch64 => &["darwin-aarch64"],
            Arch::X86_64 => &["darwin-x86_64"],
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Options {
    arch: Arch,
    debug: bool,
    version: Option<String>,
    features: Option<String>,
    out: Option<PathBuf>,
    dmg: bool,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Options> {
        let mut options = Options {
            arch: Arch::Universal,
            debug: false,
            version: None,
            features: None,
            out: None,
            dmg: true,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--arch" => options.arch = Arch::parse(&value()?)?,
                "--debug" => options.debug = true,
                "--version" => options.version = Some(value()?),
                "--features" => options.features = Some(value()?),
                "--out" => options.out = Some(PathBuf::from(value()?)),
                "--no-dmg" => options.dmg = false,
                other => return Err(format!("unknown option {other} (cargo xtask --help)")),
            }
        }
        Ok(options)
    }

    fn profile(&self) -> &'static str {
        if self.debug { "debug" } else { "release" }
    }
}

/// What this build can do, from the environment (as `macos.sh` decided it).
struct Signing {
    /// `-` is ad-hoc.
    identity: String,
    /// The App Store Connect API key for notarytool: (key path, key id, issuer).
    notary: Option<(String, String, String)>,
    /// The updater key, if there is one.
    updater: Option<minisign::SecretKey>,
}

impl Signing {
    fn from_env() -> Result<Signing> {
        let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
        let identity = var("APPLE_SIGNING_IDENTITY").unwrap_or_else(|| "-".into());
        let notary = match (var("APPLE_API_KEY_PATH"), var("APPLE_API_KEY"), var("APPLE_API_ISSUER")) {
            _ if identity == "-" => {
                println!(
                    "Ad-hoc signing (no APPLE_SIGNING_IDENTITY): not notarised; users allow it once past Gatekeeper (scripts/release/install-notes-macos.md)."
                );
                None
            }
            (Some(path), Some(key), Some(issuer)) => Some((path, key, issuer)),
            _ => {
                warn(&format!(
                    "signing as \"{identity}\" but APPLE_API_KEY/APPLE_API_ISSUER/APPLE_API_KEY_PATH are missing: NOT notarised."
                ));
                None
            }
        };
        let updater = match var("TAURI_SIGNING_PRIVATE_KEY") {
            Some(key) => Some(sign::secret_key(&key, var("TAURI_SIGNING_PRIVATE_KEY_PASSWORD"))?),
            None => {
                warn("no TAURI_SIGNING_PRIVATE_KEY: no updater bundle or updater-darwin.json.");
                None
            }
        };
        Ok(Signing {
            identity,
            notary,
            updater,
        })
    }

    fn developer_id(&self) -> bool {
        self.identity != "-"
    }
}

pub fn run(options: Options) -> Result {
    let root = workspace_root();
    let metadata = cargo_metadata(&root)?;
    let version = match &options.version {
        Some(version) => version.clone(),
        None => package_version(&metadata)?,
    };
    if let Some(tag) = std::env::var("RELEASE_TAG").ok().filter(|t| !t.is_empty())
        && tag != format!("v{version}")
    {
        return Err(format!(
            "tag {tag} doesn't match the app version {version} (Cargo.toml); bump the version first"
        ));
    }
    let download_url = std::env::var("DOWNLOAD_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| format!("{RELEASES}/v{version}"));
    let target_dir = PathBuf::from(
        metadata["target_directory"]
            .as_str()
            .ok_or("cargo metadata has no target_directory")?,
    );
    let out = options
        .out
        .clone()
        .or_else(|| std::env::var_os("OUT_DIR").map(PathBuf::from))
        .unwrap_or_else(|| target_dir.join("release/native-macos"));
    let signing = Signing::from_env()?;
    let keychain = Keychain::import_if_given(&signing)?;

    note(&format!(
        "Building {APP_NAME} {version} ({}, {}; identity: {}, notarise: {}, updater: {})",
        options.arch.label(),
        options.profile(),
        signing.identity,
        signing.notary.is_some(),
        signing.updater.is_some(),
    ));
    let mut binaries = Vec::new();
    for triple in options.arch.triples() {
        binaries.push(build(&root, &target_dir, triple, &options, &version)?);
    }

    let bundle_dir = target_dir.join("native-bundle").join(options.profile());
    let app = bundle_dir.join(format!("{APP_NAME}.app"));
    note(&format!("Assembling {}", app.display()));
    assemble(&root, &app, &binaries, &version)?;

    note("Signing the app");
    codesign(
        &signing,
        &app,
        Some(&root.join("apps/native/resources/macos/Entitlements.plist")),
    )?;
    if let Some(notary) = &signing.notary {
        note("Notarising the app");
        let zip = bundle_dir.join(format!("{APP_NAME}.zip"));
        cmd::run(
            "ditto",
            [
                os("-c"),
                os("-k"),
                os("--keepParent"),
                app.as_os_str().into(),
                zip.as_os_str().into(),
            ],
        )?;
        notarise(notary, &zip)?;
        cmd::run("xcrun", [os("stapler"), os("staple"), app.as_os_str().into()])?;
        let _ = std::fs::remove_file(&zip);
    }

    note("Checking the bundle");
    check_app(&app, options.arch)?;

    let dmg = if options.dmg {
        note("Making the DMG");
        let dmg = bundle_dir.join(format!("{APP_NAME}_{version}_{}.dmg", options.arch.label()));
        make_dmg(&app, &dmg, &bundle_dir)?;
        if signing.developer_id() {
            codesign(&signing, &dmg, None)?;
            if let Some(notary) = &signing.notary {
                note("Notarising the DMG");
                notarise(notary, &dmg)?;
                cmd::run("xcrun", [os("stapler"), os("staple"), dmg.as_os_str().into()])?;
            }
        }
        check_dmg(&signing, &dmg)?;
        Some(dmg)
    } else {
        None
    };
    if signing.notary.is_some() {
        cmd::run("xcrun", [os("stapler"), os("validate"), app.as_os_str().into()])?;
        cmd::run(
            "spctl",
            [os("-a"), os("-vv"), os("-t"), os("exec"), app.as_os_str().into()],
        )?;
    }
    drop(keychain);

    note(&format!("Staging in {}", out.display()));
    if out.exists() {
        std::fs::remove_dir_all(&out).map_err(|e| format!("can't clear {}: {e}", out.display()))?;
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("can't create {}: {e}", out.display()))?;
    if let Some(dmg) = &dmg {
        copy(dmg, &out.join(dmg.file_name().unwrap_or_default()))?;
    }
    if let Some(key) = &signing.updater {
        let archive = format!("{APP_NAME}_{version}_{}.app.tar.gz", options.arch.label());
        let bytes = tar_gz(&app)?;
        let signature = sign::sign(key, &bytes, &archive, &version)?;
        write(&out.join(&archive), &bytes)?;
        write(&out.join(format!("{archive}.sig")), signature.as_bytes())?;
        let fragment = updater_fragment(options.arch, &signature, &format!("{download_url}/{archive}"));
        write(&out.join("updater-darwin.json"), fragment.as_bytes())?;
    }
    println!("app: {}", app.display());
    for entry in std::fs::read_dir(&out).map_err(|e| e.to_string())?.flatten() {
        let size = entry.metadata().map(|m| m.len()).unwrap_or_default();
        println!("{size:>12}  {}", entry.path().display());
    }
    Ok(())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_path_buf()
}

fn cargo_metadata(root: &Path) -> Result<serde_json::Value> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root);
    let out = command.output().map_err(|e| format!("can't run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))
}

fn package_version(metadata: &serde_json::Value) -> Result<String> {
    metadata["packages"]
        .as_array()
        .and_then(|packages| packages.iter().find(|p| p["name"] == PACKAGE))
        .and_then(|p| p["version"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("no {PACKAGE} package in cargo metadata"))
}

/// One architecture's executable.
fn build(root: &Path, target_dir: &Path, triple: &str, options: &Options, version: &str) -> Result<PathBuf> {
    note(&format!("cargo build ({triple})"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .current_dir(root)
        .args([
            "build",
            "--locked",
            "--package",
            PACKAGE,
            "--bin",
            PACKAGE,
            "--target",
            triple,
        ])
        // The app reports this version (and compares updates against it);
        // Info.plist says the same.
        .env("PROXYMATE_APP_VERSION", version)
        // As the Tauri bundler does from `minimumSystemVersion`: rustc and
        // every C dependency target 13.3, and `check_app` checks it.
        .env("MACOSX_DEPLOYMENT_TARGET", MIN_MACOS);
    if !options.debug {
        command.arg("--release");
    }
    if let Some(features) = &options.features {
        command.args(["--features", features]);
    }
    cmd::status(command)?;
    let binary = target_dir.join(triple).join(options.profile()).join(PACKAGE);
    if binary.is_file() {
        Ok(binary)
    } else {
        Err(format!("no executable at {}", binary.display()))
    }
}

fn assemble(root: &Path, app: &Path, binaries: &[PathBuf], version: &str) -> Result {
    if app.exists() {
        std::fs::remove_dir_all(app).map_err(|e| format!("can't clear {}: {e}", app.display()))?;
    }
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    for dir in [&macos, &resources] {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    }
    let executable = macos.join(EXECUTABLE);
    match binaries {
        [one] => copy(one, &executable)?,
        many => {
            let mut args = vec![os("-create"), os("-output"), executable.as_os_str().into()];
            args.extend(many.iter().map(|b| b.as_os_str().into()));
            cmd::run("lipo", args)?;
        }
    }
    copy(
        &root.join("apps/native/resources/macos/icon.icns"),
        &resources.join("icon.icns"),
    )?;
    info_plist(version)
        .to_file_xml(contents.join("Info.plist"))
        .map_err(|e| format!("can't write Info.plist: {e}"))
}

/// The Tauri app's `Info.plist`, key for key, so LaunchServices, the data
/// dir and the keychain see the same app before and after the update.
fn info_plist(version: &str) -> Value {
    let mut dict = Dictionary::new();
    let mut put = |key: &str, value: Value| {
        dict.insert(key.into(), value);
    };
    put("CFBundleDevelopmentRegion", "English".into());
    put("CFBundleDisplayName", APP_NAME.into());
    put("CFBundleExecutable", EXECUTABLE.into());
    put("CFBundleIconFile", "icon.icns".into());
    put("CFBundleIdentifier", BUNDLE_ID.into());
    put("CFBundleInfoDictionaryVersion", "6.0".into());
    put("CFBundleName", APP_NAME.into());
    put("CFBundlePackageType", "APPL".into());
    put("CFBundleShortVersionString", version.into());
    put("CFBundleVersion", version.into());
    put("CSResourcesFileMapped", true.into());
    put(
        "LSApplicationCategoryType",
        "public.app-category.developer-tools".into(),
    );
    put("LSMinimumSystemVersion", MIN_MACOS.into());
    put("LSRequiresCarbon", true.into());
    put("NSHighResolutionCapable", true.into());
    put("NSHumanReadableCopyright", COPYRIGHT.into());
    Value::Dictionary(dict)
}

/// Signs `path`: the app with the hardened runtime and its entitlements,
/// or the DMG. A Developer ID signature is timestamped; ad-hoc can't be.
fn codesign(signing: &Signing, path: &Path, entitlements: Option<&Path>) -> Result {
    let mut args = vec![os("--force"), os("--sign"), os(&signing.identity)];
    if let Some(entitlements) = entitlements {
        args.extend([
            os("--options"),
            os("runtime"),
            os("--entitlements"),
            entitlements.as_os_str().into(),
        ]);
    }
    if signing.developer_id() {
        args.push(os("--timestamp"));
    }
    args.push(path.as_os_str().into());
    cmd::run("codesign", args)
}

fn notarise((key_path, key, issuer): &(String, String, String), path: &Path) -> Result {
    cmd::run(
        "xcrun",
        [
            os("notarytool"),
            os("submit"),
            path.as_os_str().into(),
            os("--key"),
            os(key_path),
            os("--key-id"),
            os(key),
            os("--issuer"),
            os(issuer),
            os("--wait"),
            os("--timeout"),
            os("30m"),
        ],
    )
}

/// What `macos.sh` checks, plus the deployment target.
fn check_app(app: &Path, arch: Arch) -> Result {
    let executable = app.join("Contents/MacOS").join(EXECUTABLE);
    let archs = cmd::output("lipo", [os("-archs"), executable.as_os_str().into()])?;
    for want in arch.lipo_archs() {
        if !archs.split_whitespace().any(|a| a == *want) {
            return Err(format!("{} lacks {want}: {}", executable.display(), archs.trim()));
        }
    }
    println!("architectures: {}", archs.trim());
    let build = cmd::output("vtool", [os("-show-build"), executable.as_os_str().into()])?;
    let minos: Vec<&str> = build
        .lines()
        .filter_map(|l| l.trim().strip_prefix("minos"))
        .map(str::trim)
        .collect();
    if minos.is_empty() || minos.iter().any(|m| *m != MIN_MACOS) {
        return Err(format!(
            "{} targets macOS {minos:?}, not {MIN_MACOS}",
            executable.display()
        ));
    }
    println!("minimum macOS: {MIN_MACOS}");
    // Every Mach-O must be signed with the runtime flag. Today that's only
    // the main executable, which also runs as `--sysproxy-guardian` and
    // `mcp-stdio`.
    for file in files(app)? {
        if is_mach_o(&file) {
            cmd::run(
                "codesign",
                [
                    os("--verify"),
                    os("--strict"),
                    os("--verbose=2"),
                    file.as_os_str().into(),
                ],
            )?;
            let info = cmd::output("codesign", [os("-dv"), file.as_os_str().into()])?;
            if !info.lines().any(|l| l.contains("flags=") && l.contains("runtime")) {
                return Err(format!("{} is signed without the hardened runtime", file.display()));
            }
        }
    }
    cmd::run(
        "codesign",
        [
            os("--verify"),
            os("--deep"),
            os("--strict"),
            os("--verbose=2"),
            app.as_os_str().into(),
        ],
    )?;
    let info = cmd::output("codesign", [os("-dv"), os("--verbose=2"), app.as_os_str().into()])?;
    for line in info.lines() {
        if [
            "Identifier",
            "Authority",
            "TeamIdentifier",
            "Timestamp",
            "Runtime Version",
            "CodeDirectory",
        ]
        .iter()
        .any(|key| line.starts_with(key))
        {
            println!("{line}");
        }
    }
    if !info.lines().any(|l| l == format!("Identifier={BUNDLE_ID}")) {
        return Err(format!("the app isn't signed as {BUNDLE_ID}"));
    }
    Ok(())
}

fn check_dmg(signing: &Signing, dmg: &Path) -> Result {
    if !signing.developer_id() {
        return Ok(());
    }
    let info = cmd::output("codesign", [os("-dv"), os("--verbose=2"), dmg.as_os_str().into()]).unwrap_or_default();
    if info.contains("Authority=Developer ID Application") {
        cmd::run(
            "codesign",
            [
                os("--verify"),
                os("--strict"),
                os("--verbose=2"),
                dmg.as_os_str().into(),
            ],
        )?;
    } else if std::env::var("PROXYMATE_ALLOW_UNSIGNED_DMG").as_deref() != Ok("1") {
        return Err("the DMG isn't signed with the Developer ID".into());
    }
    if signing.notary.is_some() {
        cmd::run("xcrun", [os("stapler"), os("validate"), dmg.as_os_str().into()])?;
        cmd::run(
            "spctl",
            [
                os("-a"),
                os("-vv"),
                os("-t"),
                os("open"),
                os("--context"),
                os("context:primary-signature"),
                dmg.as_os_str().into(),
            ],
        )?;
    }
    Ok(())
}

/// A compressed DMG holding the app and an Applications link to drag it to.
fn make_dmg(app: &Path, dmg: &Path, work: &Path) -> Result {
    let stage = work.join("dmg-stage");
    if stage.exists() {
        std::fs::remove_dir_all(&stage).map_err(|e| format!("can't clear {}: {e}", stage.display()))?;
    }
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    // ditto keeps the signature's extended attributes and the bundle as is.
    cmd::run(
        "ditto",
        [
            app.as_os_str().into(),
            stage.join(format!("{APP_NAME}.app")).into_os_string(),
        ],
    )?;
    symlink("/Applications", &stage.join("Applications"))?;
    // A leftover DMG (an interrupted build) makes hdiutil fail with "File exists".
    let _ = std::fs::remove_file(dmg);
    let result = cmd::run(
        "hdiutil",
        [
            os("create"),
            os("-volname"),
            os(APP_NAME),
            os("-srcfolder"),
            stage.as_os_str().into(),
            os("-fs"),
            os("HFS+"),
            os("-format"),
            os("UDZO"),
            os("-ov"),
            dmg.as_os_str().into(),
        ],
    );
    let _ = std::fs::remove_dir_all(&stage);
    result
}

#[cfg(unix)]
fn symlink(target: &str, link: &Path) -> Result {
    std::os::unix::fs::symlink(target, link).map_err(|e| format!("can't link {}: {e}", link.display()))
}

#[cfg(not(unix))]
fn symlink(_: &str, _: &Path) -> Result {
    Err("bundle-macos runs on macOS".into())
}

/// The updater's archive: `Proxymate.app/…`, as the Tauri bundler makes it
/// (the updater drops the first path component and swaps in the rest).
fn tar_gz(app: &Path) -> Result<Vec<u8>> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    builder.follow_symlinks(false);
    builder
        .append_dir_all(format!("{APP_NAME}.app"), app)
        .map_err(|e| format!("can't archive {}: {e}", app.display()))?;
    builder
        .into_inner()
        .and_then(|gz| gz.finish())
        .map_err(|e| format!("can't archive {}: {e}", app.display()))
}

/// `latest.json`'s `platforms` entries for this bundle; release.yml merges
/// the fragments (`jq -s`) into the feed.
fn updater_fragment(arch: Arch, signature: &str, url: &str) -> String {
    let entry = serde_json::json!({ "signature": signature, "url": url });
    let platforms: serde_json::Map<String, serde_json::Value> = arch
        .platforms()
        .iter()
        .map(|p| ((*p).to_owned(), entry.clone()))
        .collect();
    let mut text = serde_json::to_string_pretty(&platforms).expect("JSON");
    text.push('\n');
    text
}

fn files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| format!("can't list {}: {e}", dir.display()))? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                out.push(entry.path());
            }
        }
    }
    Ok(out)
}

/// Thin or fat Mach-O, by its magic number.
fn is_mach_o(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    matches!(
        u32::from_be_bytes(magic),
        0xfeedface | 0xfeedfacf | 0xcefaedfe | 0xcffaedfe | 0xcafebabe | 0xbebafeca
    )
}

fn copy(from: &Path, to: &Path) -> Result {
    std::fs::copy(from, to)
        .map(drop)
        .map_err(|e| format!("can't copy {} to {}: {e}", from.display(), to.display()))
}

fn write(path: &Path, bytes: &[u8]) -> Result {
    std::fs::write(path, bytes).map_err(|e| format!("can't write {}: {e}", path.display()))
}

fn os(text: &str) -> std::ffi::OsString {
    text.into()
}

/// The Developer ID certificate (`APPLE_CERTIFICATE`, a base64 `.p12`) in a
/// temporary keychain for `codesign`, as Tauri's bundler imported it; the
/// keychain is deleted when this is dropped. Dormant until the `APPLE_*`
/// secrets exist.
struct Keychain(Option<PathBuf>);

impl Keychain {
    fn import_if_given(signing: &Signing) -> Result<Keychain> {
        let certificate = std::env::var("APPLE_CERTIFICATE").ok().filter(|c| !c.is_empty());
        let (Some(certificate), true) = (certificate, signing.developer_id()) else {
            return Ok(Keychain(None));
        };
        use base64::Engine as _;
        let p12 = base64::engine::general_purpose::STANDARD
            .decode(certificate.split_whitespace().collect::<String>())
            .map_err(|e| format!("APPLE_CERTIFICATE: {e}"))?;
        let dir = std::env::temp_dir().join(format!("proxymate-signing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let keychain = dir.join("signing.keychain-db");
        let p12_path = dir.join("certificate.p12");
        write(&p12_path, &p12)?;
        let password = random_hex();
        let cert_password = std::env::var("APPLE_CERTIFICATE_PASSWORD").unwrap_or_default();
        let guard = Keychain(Some(keychain.clone()));
        let k = keychain.as_os_str();
        quiet(
            "create-keychain",
            &[os("create-keychain"), os("-p"), os(&password), k.into()],
        )?;
        quiet(
            "set-keychain-settings",
            &[os("set-keychain-settings"), os("-lut"), os("3600"), k.into()],
        )?;
        quiet(
            "unlock-keychain",
            &[os("unlock-keychain"), os("-p"), os(&password), k.into()],
        )?;
        let imported = quiet(
            "import",
            &[
                os("import"),
                p12_path.as_os_str().into(),
                os("-k"),
                k.into(),
                os("-P"),
                os(&cert_password),
                os("-T"),
                os("/usr/bin/codesign"),
            ],
        );
        let _ = std::fs::remove_file(&p12_path);
        imported?;
        quiet(
            "set-key-partition-list",
            &[
                os("set-key-partition-list"),
                os("-S"),
                os("apple-tool:,apple:,codesign:"),
                os("-s"),
                os("-k"),
                os(&password),
                k.into(),
            ],
        )?;
        // codesign looks the identity up in the search list.
        let list = cmd::output("security", [os("list-keychains"), os("-d"), os("user")])?;
        let mut args = vec![os("list-keychains"), os("-d"), os("user"), os("-s"), k.into()];
        args.extend(
            list.lines()
                .map(|l| os(l.trim().trim_matches('"')))
                .filter(|l| !l.is_empty()),
        );
        quiet("list-keychains", &args)?;
        Ok(guard)
    }
}

impl Drop for Keychain {
    fn drop(&mut self) {
        if let Some(keychain) = &self.0 {
            let _ = Command::new("security").arg("delete-keychain").arg(keychain).status();
            if let Some(dir) = keychain.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// `security <args>`, never echoing them (they hold passwords).
fn quiet(what: &str, args: &[std::ffi::OsString]) -> Result {
    let out = Command::new("security")
        .args(args)
        .output()
        .map_err(|e| format!("can't run security {what}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("security {what} failed ({})", out.status))
    }
}

fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut bytes);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn options_default_to_a_universal_release_with_a_dmg() {
        let options = Options::parse(&[]).unwrap();
        assert_eq!(options.arch, Arch::Universal);
        assert_eq!(options.profile(), "release");
        assert!(options.dmg);
        let options = Options::parse(&args("--arch x64 --debug --version 0.0.2 --no-dmg --out /tmp/x")).unwrap();
        assert_eq!(
            options,
            Options {
                arch: Arch::X86_64,
                debug: true,
                version: Some("0.0.2".into()),
                features: None,
                out: Some("/tmp/x".into()),
                dmg: false,
            }
        );
        assert!(Options::parse(&args("--arch ppc")).is_err());
        assert!(Options::parse(&args("--version")).is_err());
        assert!(Options::parse(&args("--frobnicate")).is_err());
    }

    #[test]
    fn info_plist_matches_the_tauri_apps_keys() {
        let Value::Dictionary(dict) = info_plist("1.2.3") else {
            panic!("a dictionary")
        };
        assert_eq!(dict["CFBundleIdentifier"].as_string(), Some("dev.proxymate.desktop"));
        assert_eq!(dict["CFBundleExecutable"].as_string(), Some("proxymate"));
        assert_eq!(dict["CFBundleShortVersionString"].as_string(), Some("1.2.3"));
        assert_eq!(dict["LSMinimumSystemVersion"].as_string(), Some("13.3"));
        assert_eq!(
            dict["LSApplicationCategoryType"].as_string(),
            Some("public.app-category.developer-tools")
        );
    }

    /// The fragment `macos.sh` writes with node, byte for byte in shape.
    #[test]
    fn updater_fragment_lists_each_platform_the_bundle_serves() {
        let text = updater_fragment(Arch::Universal, "c2ln", "https://x/P.app.tar.gz");
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        for platform in ["darwin-aarch64", "darwin-x86_64"] {
            assert_eq!(json[platform]["signature"], "c2ln");
            assert_eq!(json[platform]["url"], "https://x/P.app.tar.gz");
        }
        assert!(text.ends_with("}\n"));
        let arm: serde_json::Value = serde_json::from_str(&updater_fragment(Arch::Aarch64, "s", "u")).unwrap();
        assert_eq!(arm.as_object().unwrap().len(), 1);
    }

    /// The archive's layout is what tauri-plugin-updater (and the native
    /// updater) unpack: `Proxymate.app/` first, modes kept.
    #[test]
    fn the_archive_holds_the_bundle_under_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Proxymate.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let exe = app.join("Contents/MacOS/proxymate");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let bytes = tar_gz(&app).unwrap();
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
        let entries: Vec<(String, u32)> = archive
            .entries()
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.path().unwrap().to_string_lossy().into_owned(),
                    e.header().mode().unwrap(),
                )
            })
            .collect();
        assert!(entries.iter().all(|(p, _)| p.starts_with("Proxymate.app")));
        let exe_mode = entries.iter().find(|(p, _)| p.ends_with("MacOS/proxymate")).unwrap().1;
        #[cfg(unix)]
        assert_eq!(exe_mode & 0o777, 0o755);
        let _ = exe_mode;
    }

    #[test]
    fn mach_o_is_told_by_its_magic() {
        let dir = tempfile::tempdir().unwrap();
        let thin = dir.path().join("thin");
        std::fs::write(&thin, [0xcf, 0xfa, 0xed, 0xfe, 0, 0]).unwrap();
        let text = dir.path().join("text");
        std::fs::write(&text, b"hello").unwrap();
        assert!(is_mach_o(&thin));
        assert!(!is_mach_o(&text));
    }
}
