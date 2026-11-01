//! Capture targets and noise (M21).
//!
//! - **Recording lists.** `RecordingSettings{only, never}` compile to two
//!   host `GlobSet`s, checked in [`Engine::begin_flow`]. A flow they keep
//!   out gets no recorder: it is proxied exactly like one recorded while
//!   paused (SSL policy and rules apply, nothing is teed), and counted.
//! - **Intercepted launches.** Terminals and browsers started through
//!   `pm_platform::launcher`, each in its own directory under
//!   `<data dir>/intercept/`. Their pid trees are handed to the client
//!   resolver, which names their flows `<App> (Intercepted)`. A thread per
//!   launch waits for the root to exit, then forgets it and removes the
//!   directory; ones a crash left behind are swept at the next launch.
//! - **Auto-save.** The publisher tick starts a save of the live capture
//!   when one is due (through M8's background save, on its own thread),
//!   into `Proxymate Autosave <time>.proxymate`, keeping the newest K.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use arc_swap::ArcSwapOption;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use parking_lot::Mutex;
use pm_model::{
    AutoSaveSettings, AutoSaveStatus, InterceptApp, InterceptSession, InterceptTarget, LIVE_STORE, RecordingSettings,
    SaveProgress,
};
use pm_platform::launcher::{self, App, AppLauncher, Launcher, Setup};

use crate::io::session;
use crate::{Engine, Error};

const INTERCEPT_DIR: &str = "intercept";
/// How long a terminal's shell has to write its pid.
const PID_FILE_WAIT: Duration = Duration::from_secs(30);
const PID_FILE_POLL: Duration = Duration::from_millis(100);
pub(crate) const AUTOSAVE_PREFIX: &str = "Proxymate Autosave ";
const SESSION_EXT: &str = "proxymate";

/// The recording lists, compiled. Absent when both are empty, so the
/// common case is one atomic load.
pub(crate) struct RecordingFilter {
    only: Option<GlobSet>,
    never: Option<GlobSet>,
}

impl RecordingFilter {
    pub fn compile(settings: &RecordingSettings) -> Result<Option<RecordingFilter>, Error> {
        let only = host_globs(&settings.only)?;
        let never = host_globs(&settings.never)?;
        Ok((only.is_some() || never.is_some()).then_some(RecordingFilter { only, never }))
    }

    /// Whether a flow to `host` is recorded. Doesn't allocate.
    pub fn records(&self, host: &str) -> bool {
        if let Some(only) = &self.only
            && !only.is_match(host)
        {
            return false;
        }
        !self.never.as_ref().is_some_and(|never| never.is_match(host))
    }
}

/// `*.example.com` also matches `example.com`, as everywhere else hosts
/// are matched.
fn host_globs(list: &[String]) -> Result<Option<GlobSet>, Error> {
    if list.is_empty() {
        return Ok(None);
    }
    let mut set = GlobSetBuilder::new();
    for glob in list {
        let mut add = |pattern: &str| {
            let glob = GlobBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .map_err(|e| Error::Invalid(format!("{pattern:?} isn't a valid host pattern: {}", e.kind())))?;
            set.add(glob);
            Ok::<_, Error>(())
        };
        add(glob)?;
        if let Some(apex) = glob.strip_prefix("*.") {
            add(apex)?;
        }
    }
    set.build().map(Some).map_err(|e| Error::Invalid(e.to_string()))
}

/// What a user typed or pasted as a host (`https://Example.com:8443/x`) as
/// the pattern that is stored (`example.com`). Empty if nothing is left.
pub(crate) fn normalize_host(raw: &str) -> String {
    let s = raw.trim();
    let s = s.split_once("://").map_or(s, |(_, rest)| rest);
    let s = s.split(['/', '?', '#']).next().unwrap_or_default();
    let s = s.rsplit_once('@').map_or(s, |(_, host)| host);
    // A port, but not the colons of a bare IPv6 address.
    let s = match s.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
        Some((v6, _port)) => v6,
        None => match s.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') && port.bytes().all(|b| b.is_ascii_digit()) => host,
            _ => s,
        },
    };
    s.to_ascii_lowercase()
}

fn normalize_list(list: &[String]) -> Result<Vec<String>, Error> {
    let mut out: Vec<String> = Vec::new();
    for raw in list {
        let host = normalize_host(raw);
        if host.is_empty() {
            continue;
        }
        if host.chars().any(char::is_whitespace) {
            return Err(Error::Invalid(format!("{raw:?} isn't a host: hosts have no spaces")));
        }
        if !out.contains(&host) {
            out.push(host);
        }
    }
    Ok(out)
}

/// One running intercepted launch.
struct Running {
    session: InterceptSession,
}

#[derive(Default)]
struct AutoSaveRun {
    /// When the next save is due; reset when the settings change.
    due: Option<Instant>,
    /// The capture as of the last save, to skip saving one that hasn't changed.
    saved: Option<CaptureMark>,
    status: AutoSaveStatus,
}

/// Enough of the live store to tell whether it changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaptureMark {
    next_id: u32,
    total: u32,
    sent: u64,
    received: u64,
}

pub(crate) struct Capture {
    filter: ArcSwapOption<RecordingFilter>,
    unrecorded: AtomicU64,
    launcher: Box<dyn Launcher>,
    dir: PathBuf,
    running: Mutex<Vec<Running>>,
    next_id: AtomicU32,
    autosave: Mutex<AutoSaveRun>,
}

impl Capture {
    pub fn new(root: &Path, recording: &RecordingSettings) -> Capture {
        let filter = RecordingFilter::compile(recording).unwrap_or_else(|err| {
            eprintln!("proxymate: {err}; recording every host");
            None
        });
        let dir = root.join(INTERCEPT_DIR);
        sweep(&dir);
        Capture {
            filter: ArcSwapOption::new(filter.map(Arc::new)),
            unrecorded: AtomicU64::new(0),
            launcher: Box::new(AppLauncher::system()),
            dir,
            running: Mutex::new(Vec::new()),
            next_id: AtomicU32::new(1),
            autosave: Mutex::new(AutoSaveRun::default()),
        }
    }

    /// Whether a flow to `host` is recorded; counts the ones that aren't.
    pub fn records(&self, host: &str) -> bool {
        let guard = self.filter.load();
        let Some(filter) = guard.as_deref() else {
            return true;
        };
        let recorded = filter.records(host);
        if !recorded {
            self.unrecorded.fetch_add(1, Ordering::Relaxed);
        }
        recorded
    }

    pub fn unrecorded(&self) -> u64 {
        self.unrecorded.load(Ordering::Relaxed)
    }

    pub fn autosave_status(&self) -> AutoSaveStatus {
        self.autosave.lock().status.clone()
    }

    fn roots(&self) -> Vec<u32> {
        self.running.lock().iter().filter_map(|r| r.session.pid).collect()
    }
}

/// Removes launch directories whose app is gone (a crash, a kill -9).
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let alive = std::fs::read_to_string(path.join(launcher::PID_FILE))
            .ok()
            .and_then(|pid| pid.trim().parse().ok())
            .is_some_and(pm_platform::process::is_alive);
        if !alive {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

fn app_of(target: InterceptTarget) -> App {
    match target {
        InterceptTarget::Terminal => App::Terminal,
        InterceptTarget::Iterm => App::Iterm,
        InterceptTarget::Chrome => App::Chrome,
        InterceptTarget::Edge => App::Edge,
        InterceptTarget::Brave => App::Brave,
        InterceptTarget::Arc => App::Arc,
        InterceptTarget::Firefox => App::Firefox,
    }
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}

impl Engine {
    // ---- Recording lists ----

    pub fn recording_settings(&self) -> RecordingSettings {
        self.settings.recording_row()
    }

    /// Saves the recording lists and applies them to flows that begin from
    /// now on. Hosts are stored lower-case without scheme, port or path.
    pub fn set_recording_settings(&self, settings: RecordingSettings) -> Result<RecordingSettings, Error> {
        let settings = RecordingSettings {
            only: normalize_list(&settings.only)?,
            never: normalize_list(&settings.never)?,
        };
        let filter = RecordingFilter::compile(&settings)?;
        self.settings.save_recording(settings.clone())?;
        self.capture.filter.store(filter.map(Arc::new));
        Ok(settings)
    }

    /// The flow table's "Never record this host": adds `host` to `never`.
    pub fn never_record(&self, host: &str) -> Result<RecordingSettings, Error> {
        let mut settings = self.recording_settings();
        settings.never.push(host.to_owned());
        self.set_recording_settings(settings)
    }

    // ---- Intercepted launches ----

    /// The Intercept menu: every target, and whether it is installed.
    pub fn intercept_apps(&self) -> Vec<InterceptApp> {
        InterceptTarget::ALL
            .iter()
            .map(|&target| {
                let app = app_of(target);
                InterceptApp {
                    target,
                    name: app.name().into(),
                    installed: self.capture.launcher.installed(app).is_some(),
                }
            })
            .collect()
    }

    /// Intercepted launches still running.
    pub fn intercepts(&self) -> Vec<InterceptSession> {
        self.capture.running.lock().iter().map(|r| r.session.clone()).collect()
    }

    /// Starts `target` set up for the proxy, in a fresh profile or with the
    /// proxy environment. Blocking (it writes the profile and may run
    /// `open`): adapters call it off the async threads.
    pub fn intercept(self: &Arc<Self>, target: InterceptTarget) -> Result<InterceptSession, Error> {
        let addr = self
            .proxy_addr()
            .ok_or_else(|| Error::Proxy("the proxy isn't running".into()))?;
        let app = app_of(target);
        let id = self.capture.next_id.fetch_add(1, Ordering::Relaxed);
        let dir = self.capture.dir.join(uuid::Uuid::new_v4().to_string());
        let setup = Setup {
            proxy_host: addr.ip().to_string(),
            proxy_port: addr.port(),
            ca_pem: self.ca_info().cert_path.into(),
            ca_spki_sha256: self.ca_spki(),
            env: if target.is_terminal() {
                self.proxy_env(true)
            } else {
                Vec::new()
            },
            dir: dir.clone(),
        };
        let launched = match self.capture.launcher.launch(app, &setup) {
            Ok(launched) => launched,
            Err(err) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(err.into());
            }
        };
        if let Some(pid) = launched.pid {
            // Swept at the next launch only once this pid is gone.
            let _ = std::fs::write(dir.join(launcher::PID_FILE), pid.to_string());
        }
        let session = InterceptSession {
            id,
            target,
            name: app.name().into(),
            pid: launched.pid,
            started_at_ms: now_ms(),
            notes: launched.notes,
        };
        self.capture.running.lock().push(Running {
            session: session.clone(),
        });
        self.platform.clients.set_intercepted(self.capture.roots());
        let weak = Arc::downgrade(self);
        let pid = launched.pid;
        let pid_file = launched.pid_file;
        std::thread::Builder::new()
            .name("pm-intercept".into())
            .spawn(move || watch_launch(weak, id, pid, pid_file, dir))?;
        Ok(session)
    }

    /// Tracks `pid` and its descendants as an intercepted launch, as if
    /// Proxymate had started it (tests, and a pid the user names).
    #[doc(hidden)]
    pub fn track_intercept(self: &Arc<Self>, target: InterceptTarget, pid: u32) -> InterceptSession {
        let id = self.capture.next_id.fetch_add(1, Ordering::Relaxed);
        let session = InterceptSession {
            id,
            target,
            name: app_of(target).name().into(),
            pid: Some(pid),
            started_at_ms: now_ms(),
            notes: Vec::new(),
        };
        self.capture.running.lock().push(Running {
            session: session.clone(),
        });
        self.platform.clients.set_intercepted(self.capture.roots());
        session
    }

    fn intercept_started(&self, id: u32, pid: u32) {
        if let Some(r) = self.capture.running.lock().iter_mut().find(|r| r.session.id == id) {
            r.session.pid = Some(pid);
        }
        self.platform.clients.set_intercepted(self.capture.roots());
    }

    fn intercept_ended(&self, id: u32) {
        self.capture.running.lock().retain(|r| r.session.id != id);
        self.platform.clients.set_intercepted(self.capture.roots());
    }

    // ---- Auto-save ----

    pub fn autosave_settings(&self) -> AutoSaveSettings {
        self.settings.autosave_row()
    }

    /// Saves Settings › Storage › Auto-save. The first save is one
    /// interval from now.
    pub fn set_autosave_settings(&self, settings: AutoSaveSettings) -> Result<AutoSaveSettings, Error> {
        let settings = AutoSaveSettings {
            folder: settings.folder.trim().to_owned(),
            ..settings
        };
        if !(1..=24 * 60).contains(&settings.interval_minutes) {
            return Err(Error::Invalid("save every 1 to 1,440 minutes".into()));
        }
        if !(1..=1000).contains(&settings.keep) {
            return Err(Error::Invalid("keep 1 to 1,000 saved sessions".into()));
        }
        if settings.enabled {
            let folder = Path::new(&settings.folder);
            if !folder.is_absolute() {
                return Err(Error::Invalid("choose a folder to save into".into()));
            }
            if !folder.is_dir() {
                return Err(Error::Invalid(format!("{} isn't a folder", folder.display())));
            }
        }
        self.settings.save_autosave(settings.clone())?;
        let mut run = self.capture.autosave.lock();
        run.due = None;
        run.status.error = None;
        Ok(settings)
    }

    /// Saves the live capture into the auto-save folder now, then removes
    /// the oldest auto-saves past the limit. Returns the flows it will
    /// write; the storage status says when it's done.
    pub fn autosave_now(self: &Arc<Self>) -> Result<u32, Error> {
        let settings = self.autosave_settings();
        if settings.folder.is_empty() {
            return Err(Error::Invalid("choose a folder to save into".into()));
        }
        {
            let mut run = self.capture.autosave.lock();
            if run.status.saving {
                return Err(Error::Invalid("an auto-save is already being written".into()));
            }
            run.status.saving = true;
            run.saved = Some(self.capture_mark());
        }
        let result = self.start_autosave(settings);
        if let Err(err) = &result {
            let mut run = self.capture.autosave.lock();
            run.status.saving = false;
            run.status.error = Some(err.to_string());
            run.saved = None;
        }
        result
    }

    fn capture_mark(&self) -> CaptureMark {
        let store = self.store.read();
        let (sent, received) = store.traffic();
        CaptureMark {
            next_id: store.next_id(),
            total: store.total(),
            sent,
            received,
        }
    }

    /// Part of the publisher tick: starts a save if one is due and the
    /// capture changed since the last.
    pub(crate) fn autosave_tick(self: &Arc<Self>) {
        let settings = self.settings.autosave_row();
        if !settings.enabled {
            return;
        }
        let interval = Duration::from_secs(u64::from(settings.interval_minutes) * 60);
        let now = Instant::now();
        {
            let mut run = self.capture.autosave.lock();
            match run.due {
                None => {
                    run.due = Some(now + interval);
                    return;
                }
                Some(due) if now < due || run.status.saving => return,
                Some(_) => run.due = Some(now + interval),
            }
            let mark = self.capture_mark();
            if mark.total == 0 || run.saved == Some(mark) {
                return;
            }
        }
        let _ = self.autosave_now();
    }

    fn start_autosave(self: &Arc<Self>, settings: AutoSaveSettings) -> Result<u32, Error> {
        let folder = PathBuf::from(&settings.folder);
        let path = autosave_path(&folder, SystemTime::now());
        let partial = path.with_extension("proxymate-partial");
        let total = self.total();
        let engine = self.clone();
        let weak = Arc::downgrade(self);
        let keep = settings.keep as usize;
        let (done_path, done_folder) = (path.clone(), folder.clone());
        let progress = Box::new(move |p: SaveProgress| {
            if !p.finished {
                return;
            }
            let Some(engine) = weak.upgrade() else { return };
            let error = match p.error {
                Some(err) => Some(err),
                None => std::fs::rename(&partial, &done_path)
                    .map_err(|e| e.to_string())
                    .err()
                    .or_else(|| rotate(&done_folder, keep).err().map(|e| e.to_string())),
            };
            let mut run = engine.capture.autosave.lock();
            run.status.saving = false;
            match error {
                Some(err) => {
                    let _ = std::fs::remove_file(&partial);
                    run.status.error = Some(err);
                    // So the next tick tries again.
                    run.saved = None;
                }
                None => {
                    run.status.error = None;
                    run.status.last_at_ms = Some(now_ms() as u64);
                    run.status.last_path = Some(done_path.display().to_string());
                }
            }
        });
        let partial = path.with_extension("proxymate-partial");
        self.spawn_save(partial, total, progress, move |path, report| {
            let flows = engine.store.read().flows().to_vec();
            let source = flows.into_iter().map(|summary| {
                engine
                    .with_backend(LIVE_STORE, |backend| Ok(session::collect(backend, summary)?))
                    .map_err(std::io::Error::other)
            });
            session::write(path, source, report)
        })
    }
}

/// Waits for a launch's root process (from a terminal's pid file if it
/// had none), then forgets the launch and removes its directory.
fn watch_launch(engine: std::sync::Weak<Engine>, id: u32, pid: Option<u32>, pid_file: Option<PathBuf>, dir: PathBuf) {
    let pid = pid.or_else(|| {
        let file = pid_file?;
        let deadline = Instant::now() + PID_FILE_WAIT;
        while Instant::now() < deadline {
            if let Some(pid) = std::fs::read_to_string(&file).ok().and_then(|s| s.trim().parse().ok()) {
                engine.upgrade()?.intercept_started(id, pid);
                return Some(pid);
            }
            std::thread::sleep(PID_FILE_POLL);
        }
        None
    });
    if let Some(pid) = pid {
        let _ = pm_platform::process::wait_for_exit(pid);
    }
    if let Some(engine) = engine.upgrade() {
        engine.intercept_ended(id);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `<folder>/Proxymate Autosave 2026-11-01T14-03-22Z.proxymate` (UTC, so
/// names sort by time), with `-2`… if that name is taken.
fn autosave_path(folder: &Path, at: SystemTime) -> PathBuf {
    let secs = at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let t = time::OffsetDateTime::from_unix_timestamp(secs).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let stamp = format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    );
    let mut path = folder.join(format!("{AUTOSAVE_PREFIX}{stamp}.{SESSION_EXT}"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{AUTOSAVE_PREFIX}{stamp}-{n}.{SESSION_EXT}"));
        n += 1;
    }
    path
}

/// Removes the oldest auto-saves in `folder` past the newest `keep`. Only
/// files named like ours are touched.
fn rotate(folder: &Path, keep: usize) -> std::io::Result<()> {
    let mut saves: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(folder)?
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with(AUTOSAVE_PREFIX) && name.ends_with(&format!(".{SESSION_EXT}"))
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    saves.sort_by(|a, b| b.cmp(a));
    for (_, path) in saves.into_iter().skip(keep) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lists(only: &[&str], never: &[&str]) -> RecordingFilter {
        let settings = RecordingSettings {
            only: only.iter().map(|s| s.to_string()).collect(),
            never: never.iter().map(|s| s.to_string()).collect(),
        };
        RecordingFilter::compile(&settings).unwrap().unwrap()
    }

    #[test]
    fn never_and_only_lists_match_host_globs() {
        let f = lists(&[], &["*.apple.com", "telemetry.example.org"]);
        assert!(!f.records("apple.com"));
        assert!(!f.records("gateway.icloud.apple.com"));
        assert!(!f.records("Telemetry.Example.org"));
        assert!(f.records("example.org"));
        assert!(f.records("notapple.com"));

        let f = lists(&["*.example.com", "localhost"], &["ads.example.com"]);
        assert!(f.records("api.example.com"));
        assert!(f.records("example.com"));
        assert!(f.records("localhost"));
        assert!(!f.records("other.org"));
        // never wins over only.
        assert!(!f.records("ads.example.com"));

        assert!(
            RecordingFilter::compile(&RecordingSettings::default())
                .unwrap()
                .is_none()
        );
        let bad = RecordingSettings {
            never: vec!["a{b".into()],
            ..Default::default()
        };
        assert!(RecordingFilter::compile(&bad).is_err());
    }

    #[test]
    fn hosts_are_normalized_before_they_are_stored() {
        assert_eq!(normalize_host(" https://Example.COM:8443/path?q "), "example.com");
        assert_eq!(normalize_host("user:pw@api.example.com"), "api.example.com");
        assert_eq!(normalize_host("*.Apple.com"), "*.apple.com");
        assert_eq!(normalize_host("[::1]:9090"), "::1");
        assert_eq!(normalize_host("::1"), "::1");
        assert_eq!(normalize_host("   "), "");
        assert_eq!(
            normalize_list(&["a.com".into(), "A.com".into(), "".into(), "b.com".into()]).unwrap(),
            ["a.com", "b.com"]
        );
        assert!(normalize_list(&["a b.com".into()]).is_err());
    }

    #[test]
    fn autosaves_are_named_by_time_and_rotated_oldest_first() {
        let dir = std::env::temp_dir().join(format!("pm-autosave-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_793_541_802);
        let first = autosave_path(&dir, at);
        assert_eq!(
            first.file_name().unwrap().to_string_lossy(),
            "Proxymate Autosave 2026-11-01T14-03-22Z.proxymate"
        );
        let mut made = Vec::new();
        for _ in 0..4 {
            let path = autosave_path(&dir, at);
            std::fs::write(&path, b"x").unwrap();
            made.push(path);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(made[1].to_string_lossy().ends_with("14-03-22Z-2.proxymate"));
        let other = dir.join("mine.proxymate");
        std::fs::write(&other, b"x").unwrap();
        rotate(&dir, 2).unwrap();
        let left: Vec<bool> = made.iter().map(|p| p.exists()).collect();
        assert_eq!(left, [false, false, true, true]);
        assert!(other.exists(), "not ours, so kept");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
