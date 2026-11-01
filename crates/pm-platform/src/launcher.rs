//! Starting apps already set up for the proxy (M21), behind [`Launcher`]:
//!
//! - Terminal.app and iTerm open a `.command` script that exports the proxy
//!   environment, writes its pid, then `exec`s the user's login shell, so
//!   everything run in that window descends from a pid we know.
//! - Chromium browsers (Chrome, Edge, Brave, Arc) start with a throwaway
//!   `--user-data-dir`, `--proxy-server` and our CA's SPKI hash in
//!   `--ignore-certificate-errors-spki-list`, so they decrypt without the
//!   CA in the keychain.
//! - Firefox starts with a throwaway profile whose `user.js` sets the proxy.
//!   NSS's `certutil` (Homebrew's `nss`), if installed, adds the CA to that
//!   profile; otherwise Firefox trusts what the keychain trusts.
//!
//! Every command line goes through a [`Runner`], so tests check them
//! against a fake without starting anything.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::PlatformError;
use crate::command::{self, Runner};

/// What can be launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum App {
    Terminal,
    Iterm,
    Chrome,
    Edge,
    Brave,
    Arc,
    Firefox,
}

impl App {
    /// The name shown in menus, which is also the bundle's (`<name>.app`)
    /// for all but iTerm.
    pub fn name(self) -> &'static str {
        match self {
            App::Terminal => "Terminal",
            App::Iterm => "iTerm",
            App::Chrome => "Google Chrome",
            App::Edge => "Microsoft Edge",
            App::Brave => "Brave Browser",
            App::Arc => "Arc",
            App::Firefox => "Firefox",
        }
    }

    fn bundle(self) -> &'static str {
        match self {
            App::Terminal => "Terminal.app",
            App::Iterm => "iTerm.app",
            App::Chrome => "Google Chrome.app",
            App::Edge => "Microsoft Edge.app",
            App::Brave => "Brave Browser.app",
            App::Arc => "Arc.app",
            App::Firefox => "Firefox.app",
        }
    }

    /// The executable in `Contents/MacOS`, for the apps started directly.
    fn executable(self) -> &'static str {
        match self {
            App::Firefox => "firefox",
            other => other.name(),
        }
    }

    fn is_chromium(self) -> bool {
        matches!(self, App::Chrome | App::Edge | App::Brave | App::Arc)
    }
}

/// How the launched app should reach the proxy.
#[derive(Debug, Clone)]
pub struct Setup {
    /// The proxy as the app should dial it (loopback).
    pub proxy_host: String,
    pub proxy_port: u16,
    /// The CA certificate (PEM).
    pub ca_pem: PathBuf,
    /// Base64 SHA-256 of the CA's SubjectPublicKeyInfo.
    pub ca_spki_sha256: String,
    /// Exported in terminals, in order.
    pub env: Vec<(String, String)>,
    /// A fresh directory this launch owns: the browser profile or the
    /// terminal's script. The caller removes it once the app is gone.
    pub dir: PathBuf,
}

/// A started app.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Launched {
    /// The root of the process tree to track, if known now.
    pub pid: Option<u32>,
    /// A terminal's shell writes its pid here when it starts.
    pub pid_file: Option<PathBuf>,
    /// What the user should know (how the CA is trusted, say).
    pub notes: Vec<String>,
}

pub trait Launcher: Send + Sync {
    /// The app's bundle, if it is installed.
    fn installed(&self, app: App) -> Option<PathBuf>;
    /// Starts `app` set up for the proxy.
    fn launch(&self, app: App, setup: &Setup) -> Result<Launched, PlatformError>;
}

/// The pid file a terminal script writes, in its launch directory.
pub const PID_FILE: &str = "pid";
const SCRIPT_FILE: &str = "Proxymate.command";

/// Launches apps from the usual application folders through a [`Runner`].
pub struct AppLauncher {
    runner: Box<dyn Runner>,
    roots: Vec<PathBuf>,
}

impl AppLauncher {
    /// Real processes and the macOS application folders.
    pub fn system() -> AppLauncher {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        roots.push("/System/Applications/Utilities".into());
        roots.push("/Applications/Utilities".into());
        AppLauncher::with_runner(Box::new(command::System), roots)
    }

    /// `roots` are searched in order for `<App>.app`.
    pub fn with_runner(runner: Box<dyn Runner>, roots: Vec<PathBuf>) -> AppLauncher {
        AppLauncher { runner, roots }
    }

    fn bundle(&self, app: App) -> Result<PathBuf, PlatformError> {
        self.installed(app)
            .ok_or_else(|| PlatformError::Os(format!("{} isn't installed in /Applications", app.name())))
    }

    fn terminal(&self, app: App, setup: &Setup) -> Result<Launched, PlatformError> {
        let bundle = self.bundle(app)?;
        let pid_file = setup.dir.join(PID_FILE);
        let script = setup.dir.join(SCRIPT_FILE);
        crate::fs::write_private(&script, terminal_script(setup, &pid_file).as_bytes())?;
        set_executable(&script)?;
        let mut args = command::args(["-a"]);
        args.push(bundle.into_os_string());
        args.push(script.into_os_string());
        self.runner.run(&"open".into(), &args)?.ok()?;
        Ok(Launched {
            pid: None,
            pid_file: Some(pid_file),
            notes: Vec::new(),
        })
    }

    fn chromium(&self, app: App, setup: &Setup) -> Result<Launched, PlatformError> {
        let exe = self.bundle(app)?.join("Contents/MacOS").join(app.executable());
        let pid = self.runner.spawn(&exe.into_os_string(), &chromium_args(setup), &[])?;
        Ok(Launched {
            pid: Some(pid),
            pid_file: None,
            notes: Vec::new(),
        })
    }

    fn firefox(&self, setup: &Setup) -> Result<Launched, PlatformError> {
        let exe = self
            .bundle(App::Firefox)?
            .join("Contents/MacOS")
            .join(App::Firefox.executable());
        std::fs::write(setup.dir.join("user.js"), firefox_prefs(setup))?;
        let mut notes = Vec::new();
        if let Err(note) = self.trust_in_profile(setup) {
            notes.push(note);
        }
        let mut args = command::args(["-profile"]);
        args.push(setup.dir.clone().into_os_string());
        args.extend(command::args(["-no-remote", "-new-instance"]));
        let pid = self.runner.spawn(&exe.into_os_string(), &args, &[])?;
        Ok(Launched {
            pid: Some(pid),
            pid_file: None,
            notes,
        })
    }

    /// Adds the CA to the profile's NSS database with `certutil`, or says
    /// why it couldn't.
    fn trust_in_profile(&self, setup: &Setup) -> Result<(), String> {
        let db: OsString = format!("sql:{}", setup.dir.display()).into();
        let certutil: OsString = "certutil".into();
        let mut create = command::args(["-N", "-d"]);
        create.extend([db.clone(), "--empty-password".into()]);
        let fallback = "Firefox trusts the Proxymate CA only if the keychain does (install Homebrew's `nss` to trust it in this profile alone).";
        match self.runner.run(&certutil, &create) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(fallback.into()),
            Err(err) => return Err(format!("certutil couldn't run ({err}). {fallback}")),
            Ok(out) => out.ok().map_err(|err| format!("certutil failed: {err}. {fallback}"))?,
        };
        let mut add = command::args(["-A", "-d"]);
        add.push(db);
        add.extend(command::args(["-t", "C,,", "-n", "Proxymate CA", "-i"]));
        add.push(setup.ca_pem.clone().into_os_string());
        let out = self.runner.run(&certutil, &add).map_err(|err| err.to_string())?;
        out.ok()
            .map(drop)
            .map_err(|err| format!("certutil failed: {err}. {fallback}"))
    }
}

impl Launcher for AppLauncher {
    fn installed(&self, app: App) -> Option<PathBuf> {
        self.roots
            .iter()
            .map(|root| root.join(app.bundle()))
            .find(|p| p.is_dir())
    }

    fn launch(&self, app: App, setup: &Setup) -> Result<Launched, PlatformError> {
        std::fs::create_dir_all(&setup.dir)?;
        match app {
            App::Terminal | App::Iterm => self.terminal(app, setup),
            App::Firefox => self.firefox(setup),
            chromium => {
                debug_assert!(chromium.is_chromium());
                self.chromium(chromium, setup)
            }
        }
    }
}

/// The script a terminal runs: record the pid, export, `exec` the shell
/// (same pid, so the shell is the tracked root).
pub fn terminal_script(setup: &Setup, pid_file: &Path) -> String {
    let mut out = String::from("#!/bin/sh\n# Proxymate: this window's traffic goes through the proxy.\n");
    out.push_str(&format!("echo $$ > {}\n", shell_quote(&pid_file.display().to_string())));
    for (name, value) in &setup.env {
        out.push_str(&format!("export {name}={}\n", shell_quote(value)));
    }
    out.push_str(&format!(
        "printf 'Proxymate: traffic from this window goes through %s\\n' {}\n",
        shell_quote(&format!("{}:{}", setup.proxy_host, setup.proxy_port))
    ));
    out.push_str("exec \"${SHELL:-/bin/zsh}\" -l\n");
    out
}

pub fn chromium_args(setup: &Setup) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![format!("--user-data-dir={}", setup.dir.display()).into()];
    args.extend(command::args([
        format!("--proxy-server=http://{}:{}", setup.proxy_host, setup.proxy_port),
        // Loopback goes through the proxy too (Chrome skips it by default).
        "--proxy-bypass-list=<-loopback>".to_owned(),
        format!("--ignore-certificate-errors-spki-list={}", setup.ca_spki_sha256),
        "--no-first-run".to_owned(),
        "--no-default-browser-check".to_owned(),
    ]));
    args
}

/// `user.js` for the throwaway Firefox profile.
pub fn firefox_prefs(setup: &Setup) -> String {
    let host = setup.proxy_host.replace('\\', "\\\\").replace('"', "\\\"");
    let port = setup.proxy_port;
    let prefs: [(&str, String); 13] = [
        ("network.proxy.type", "1".into()),
        ("network.proxy.http", format!("\"{host}\"")),
        ("network.proxy.http_port", port.to_string()),
        ("network.proxy.ssl", format!("\"{host}\"")),
        ("network.proxy.ssl_port", port.to_string()),
        ("network.proxy.share_proxy_settings", "true".into()),
        ("network.proxy.no_proxies_on", "\"\"".into()),
        ("network.proxy.allow_hijacking_localhost", "true".into()),
        // The keychain's trusted roots, for when certutil isn't there.
        ("security.enterprise_roots.enabled", "true".into()),
        ("browser.shell.checkDefaultBrowser", "false".into()),
        ("browser.aboutwelcome.enabled", "false".into()),
        (
            "datareporting.policy.dataSubmissionPolicyBypassNotification",
            "true".into(),
        ),
        ("toolkit.telemetry.reportingpolicy.firstRun", "false".into()),
    ];
    prefs
        .iter()
        .map(|(name, value)| format!("user_pref(\"{name}\", {value});\n"))
        .collect()
}

/// Single-quotes `s` for POSIX shells.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::command::fake::Fake;

    struct Fixture {
        root: PathBuf,
        fake: Arc<Fake>,
        launcher: AppLauncher,
    }

    impl Fixture {
        /// Fake `/Applications` with these bundles in it.
        fn new(apps: &[App]) -> Fixture {
            let root = std::env::temp_dir().join(format!("pm-launch-{}-{}", std::process::id(), rand_suffix()));
            for app in apps {
                std::fs::create_dir_all(root.join("Applications").join(app.bundle())).unwrap();
            }
            let fake = Arc::new(Fake::default());
            let launcher = AppLauncher::with_runner(
                Box::new(fake.clone()),
                vec![root.join("Applications"), root.join("missing")],
            );
            Fixture { root, fake, launcher }
        }

        fn setup(&self, name: &str) -> Setup {
            Setup {
                proxy_host: "127.0.0.1".into(),
                proxy_port: 9090,
                ca_pem: "/Users/me/Library/Application Support/Proxymate/ca/proxymate-ca.pem".into(),
                ca_spki_sha256: "aGFzaA==".into(),
                env: vec![
                    ("HTTPS_PROXY".into(), "http://127.0.0.1:9090".into()),
                    ("NODE_EXTRA_CA_CERTS".into(), "/Users/me/it's here/ca.pem".into()),
                ],
                dir: self.root.join("intercept").join(name),
            }
        }

        fn apps(&self) -> PathBuf {
            self.root.join("Applications")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn rand_suffix() -> u64 {
        use std::hash::{BuildHasher, RandomState};
        RandomState::new().hash_one(std::time::SystemTime::now())
    }

    #[test]
    fn finds_installed_apps_only() {
        let f = Fixture::new(&[App::Chrome, App::Terminal]);
        assert_eq!(
            f.launcher.installed(App::Chrome),
            Some(f.apps().join("Google Chrome.app"))
        );
        assert_eq!(f.launcher.installed(App::Firefox), None);
        let err = f.launcher.launch(App::Firefox, &f.setup("ff")).unwrap_err();
        assert_eq!(err.to_string(), "Firefox isn't installed in /Applications");
        assert!(f.fake.calls().is_empty());
    }

    #[test]
    fn chromium_browsers_get_a_throwaway_profile_proxy_and_spki_pin() {
        let f = Fixture::new(&[App::Chrome, App::Edge, App::Brave, App::Arc]);
        for (app, exe) in [
            (App::Chrome, "Google Chrome.app/Contents/MacOS/Google Chrome"),
            (App::Edge, "Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
            (App::Brave, "Brave Browser.app/Contents/MacOS/Brave Browser"),
            (App::Arc, "Arc.app/Contents/MacOS/Arc"),
        ] {
            let setup = f.setup(app.name());
            let launched = f.launcher.launch(app, &setup).unwrap();
            assert!(launched.pid.is_some() && launched.pid_file.is_none());
            assert!(setup.dir.is_dir());
            let last = f.fake.calls().pop().unwrap();
            assert_eq!(
                last,
                format!(
                    "{} --user-data-dir={} --proxy-server=http://127.0.0.1:9090 --proxy-bypass-list=<-loopback> \
                     --ignore-certificate-errors-spki-list=aGFzaA== --no-first-run --no-default-browser-check",
                    f.apps().join(exe).display(),
                    setup.dir.display()
                )
            );
        }
        // Browsers don't get the terminal environment.
        assert!(f.fake.envs.lock().unwrap().iter().all(Vec::is_empty));
    }

    #[test]
    fn firefox_gets_a_profile_that_sets_the_proxy_and_trusts_the_ca() {
        let f = Fixture::new(&[App::Firefox]);
        let setup = f.setup("firefox");
        // certutil -N, then -A, both succeed.
        f.fake.reply("").reply("");
        let launched = f.launcher.launch(App::Firefox, &setup).unwrap();
        assert_eq!(launched.pid, Some(4244));
        assert!(launched.notes.is_empty());
        let db = format!("sql:{}", setup.dir.display());
        assert_eq!(
            f.fake.calls(),
            [
                format!("certutil -N -d {db} --empty-password"),
                format!(
                    "certutil -A -d {db} -t C,, -n Proxymate CA -i {}",
                    setup.ca_pem.display()
                ),
                format!(
                    "{} -profile {} -no-remote -new-instance",
                    f.apps().join("Firefox.app/Contents/MacOS/firefox").display(),
                    setup.dir.display()
                ),
            ]
        );
        let prefs = std::fs::read_to_string(setup.dir.join("user.js")).unwrap();
        for line in [
            "user_pref(\"network.proxy.type\", 1);",
            "user_pref(\"network.proxy.http\", \"127.0.0.1\");",
            "user_pref(\"network.proxy.http_port\", 9090);",
            "user_pref(\"network.proxy.ssl\", \"127.0.0.1\");",
            "user_pref(\"network.proxy.ssl_port\", 9090);",
            "user_pref(\"network.proxy.allow_hijacking_localhost\", true);",
            "user_pref(\"security.enterprise_roots.enabled\", true);",
        ] {
            assert!(prefs.lines().any(|l| l == line), "{line} in\n{prefs}");
        }
    }

    #[test]
    fn firefox_without_certutil_says_how_the_ca_is_trusted() {
        let f = Fixture::new(&[App::Firefox]);
        f.fake.missing();
        let launched = f.launcher.launch(App::Firefox, &f.setup("firefox")).unwrap();
        assert_eq!(launched.notes.len(), 1);
        assert!(
            launched.notes[0].contains("only if the keychain does"),
            "{:?}",
            launched.notes
        );
        // It still starts.
        assert!(f.fake.calls().last().unwrap().contains("-profile"));
    }

    #[test]
    fn terminals_open_a_script_that_exports_records_its_pid_and_execs_the_shell() {
        let f = Fixture::new(&[App::Terminal, App::Iterm]);
        for (app, bundle) in [(App::Terminal, "Terminal.app"), (App::Iterm, "iTerm.app")] {
            let setup = f.setup(app.name());
            f.fake.reply("");
            let launched = f.launcher.launch(app, &setup).unwrap();
            let script = setup.dir.join(SCRIPT_FILE);
            assert_eq!(launched.pid, None);
            assert_eq!(launched.pid_file, Some(setup.dir.join(PID_FILE)));
            assert_eq!(
                f.fake.calls().pop().unwrap(),
                format!("open -a {} {}", f.apps().join(bundle).display(), script.display())
            );
            let text = std::fs::read_to_string(&script).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines[0], "#!/bin/sh");
            assert_eq!(lines[2], format!("echo $$ > '{}'", setup.dir.join(PID_FILE).display()));
            assert_eq!(lines[3], "export HTTPS_PROXY='http://127.0.0.1:9090'");
            assert_eq!(lines[4], r"export NODE_EXTRA_CA_CERTS='/Users/me/it'\''s here/ca.pem'");
            assert_eq!(*lines.last().unwrap(), "exec \"${SHELL:-/bin/zsh}\" -l");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&script).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o700);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_terminal_script_runs_in_a_posix_shell() {
        // Run it with a stand-in shell that prints what it was given.
        let f = Fixture::new(&[App::Terminal]);
        let setup = f.setup("sh");
        std::fs::create_dir_all(&setup.dir).unwrap();
        let pid_file = setup.dir.join(PID_FILE);
        let script = terminal_script(&setup, &pid_file);
        let shell = setup.dir.join("shell");
        std::fs::write(&shell, "#!/bin/sh\nenv\n").unwrap();
        set_executable(&shell).unwrap();
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .env("SHELL", &shell)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("HTTPS_PROXY=http://127.0.0.1:9090\n"), "{stdout}");
        assert!(
            stdout.contains("NODE_EXTRA_CA_CERTS=/Users/me/it's here/ca.pem\n"),
            "{stdout}"
        );
        let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        assert!(pid > 0);
    }
}
