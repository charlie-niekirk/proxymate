//! Which app is on the other end of a loopback connection: peer port →
//! pid → app bundle (name, bundle id) → icon.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::PlatformError;

/// The app (or plain executable) behind a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    /// The `.app` name without the extension (`Google Chrome`), or the
    /// executable's file name for command-line tools (`curl`).
    pub name: String,
    pub bundle_id: Option<String>,
    /// The outermost `.app` bundle, for its icon.
    pub bundle_path: Option<PathBuf>,
    /// The pid the name came from: the app's, not its helper's.
    pub pid: u32,
    /// The process runs inside an iOS Simulator device.
    pub simulator: bool,
}

pub trait ProcessInfo: Send + Sync {
    /// One scan of every TCP socket this user can see: local port → pid
    /// for sockets connected to `remote_port` on a loopback address.
    fn loopback_peers(&self, remote_port: u16) -> Result<HashMap<u16, u32>, PlatformError>;
    /// The app behind `pid`. Helpers (XPC services, framework processes,
    /// apps nested in another app) map to the app that owns them.
    fn app_for_pid(&self, pid: u32) -> Option<AppInfo>;
    /// The bundle's icon as a PNG, `size` points square at the screen's scale.
    fn icon_png(&self, bundle_path: &Path, size: u32) -> Option<Vec<u8>>;
}

/// This OS's implementation.
pub fn system() -> Box<dyn ProcessInfo> {
    #[cfg(target_os = "macos")]
    {
        Box::new(crate::macos::procinfo::LibProc)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(crate::stub::NoProcessInfo)
    }
}

/// The outermost `*.app` directory in `path`, so `Google Chrome Helper.app`
/// nested inside `Google Chrome.app` maps to the latter.
pub fn outermost_app(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        out.push(part);
        if part.as_os_str().to_str().is_some_and(|p| p.ends_with(".app")) {
            return Some(out);
        }
    }
    None
}

/// True for executables that work on behalf of another app rather than as
/// a tool of their own: XPC services and processes inside frameworks
/// (Safari's network process lives in WebKit.framework).
pub fn is_helper(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.contains(".xpc/") || path.contains(".framework/")
}

/// True for executables of an iOS Simulator runtime or device: their
/// paths are under a `CoreSimulator` directory.
pub fn is_simulator(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "CoreSimulator")
}

/// The display name of an `.app` path: its file name without `.app`.
pub fn app_name(bundle: &Path) -> String {
    bundle
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_map_to_the_outermost_app() {
        let helper = Path::new(
            "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/1/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper",
        );
        let app = outermost_app(helper).unwrap();
        assert_eq!(app, Path::new("/Applications/Google Chrome.app"));
        assert_eq!(app_name(&app), "Google Chrome");
        assert_eq!(outermost_app(Path::new("/usr/bin/curl")), None);

        let webkit = Path::new(
            "/System/Library/Frameworks/WebKit.framework/Versions/A/XPCServices/com.apple.WebKit.Networking.xpc/Contents/MacOS/com.apple.WebKit.Networking",
        );
        assert!(is_helper(webkit));
        assert!(!is_helper(Path::new("/opt/homebrew/bin/node")));
        assert!(!is_simulator(webkit));

        let sim = Path::new(
            "/Library/Developer/CoreSimulator/Volumes/iOS_23C54/Library/Developer/CoreSimulator/Profiles/Runtimes/iOS 26.2.simruntime/Contents/Resources/RuntimeRoot/Applications/MobileSafari.app/MobileSafari",
        );
        assert!(is_simulator(sim));
        assert_eq!(app_name(&outermost_app(sim).unwrap()), "MobileSafari");
    }
}
