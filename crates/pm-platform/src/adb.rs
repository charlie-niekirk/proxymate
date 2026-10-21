//! Android devices and emulators through `adb`: list them, point an
//! emulator's global HTTP proxy at us (and clear it again), and push the CA
//! to its Downloads folder for the user to install.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::PlatformError;
use crate::command::{self, Runner};

/// Where the CA lands on the device, for Settings → Install a certificate.
pub const DEVICE_CERT_PATH: &str = "/sdcard/Download/proxymate-ca.crt";

/// A device line of `adb devices -l`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    /// `device`, `offline`, `unauthorized`, …
    pub state: String,
    pub model: Option<String>,
}

impl AdbDevice {
    pub fn is_emulator(&self) -> bool {
        self.serial.starts_with("emulator-")
    }
}

pub struct Adb {
    runner: Box<dyn Runner>,
    exe: OsString,
}

impl Adb {
    /// `adb` from the Android SDK or `PATH`, if there is one. Apps started
    /// from the Finder don't get the shell's `PATH`, so the SDK's usual
    /// places come first.
    pub fn system() -> Option<Adb> {
        let exe = locate(&std::env::var_os("PATH").unwrap_or_default(), &sdk_dirs())?;
        Some(Adb::with_runner(Box::new(command::System), exe.into_os_string()))
    }

    pub fn with_runner(runner: Box<dyn Runner>, exe: OsString) -> Adb {
        Adb { runner, exe }
    }

    pub fn devices(&self) -> Result<Vec<AdbDevice>, PlatformError> {
        Ok(parse_devices(&self.run(command::args(["devices", "-l"]))?))
    }

    /// The device's global `http_proxy`, or `None` if unset or cleared.
    pub fn proxy(&self, serial: &str) -> Result<Option<String>, PlatformError> {
        let out = self.run(shell(serial, ["settings", "get", "global", "http_proxy"]))?;
        let value = out.trim();
        Ok(match value {
            "" | "null" | ":0" => None,
            v => Some(v.to_owned()),
        })
    }

    /// Sets the global `http_proxy`, e.g. `10.0.2.2:9090` for an emulator.
    pub fn set_proxy(&self, serial: &str, host_port: &str) -> Result<(), PlatformError> {
        self.run(shell(serial, ["settings", "put", "global", "http_proxy", host_port]))
            .map(drop)
    }

    /// `:0` is what clears the proxy for running apps; deleting the key
    /// only takes effect after a reboot.
    pub fn clear_proxy(&self, serial: &str) -> Result<(), PlatformError> {
        self.set_proxy(serial, ":0")
    }

    pub fn push(&self, serial: &str, local: &Path, remote: &str) -> Result<(), PlatformError> {
        let mut args = command::args(["-s", serial, "push"]);
        args.push(local.as_os_str().to_owned());
        args.push(remote.into());
        self.run(args).map(drop)
    }

    fn run(&self, args: Vec<OsString>) -> Result<String, PlatformError> {
        self.runner.run(&self.exe, &args)?.ok()
    }
}

fn shell<'a>(serial: &'a str, cmd: impl IntoIterator<Item = &'a str>) -> Vec<OsString> {
    let mut args = command::args(["-s", serial, "shell"]);
    args.extend(cmd.into_iter().map(OsString::from));
    args
}

/// Device lines of `adb devices -l`; the header and daemon chatter
/// (`* daemon started successfully`) are skipped.
pub fn parse_devices(out: &str) -> Vec<AdbDevice> {
    out.lines()
        .filter(|line| !line.starts_with("List of devices") && !line.starts_with('*'))
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let serial = words.next()?.to_owned();
            let state = words.next()?.to_owned();
            let model = words
                .find_map(|w| w.strip_prefix("model:"))
                .map(|m| m.replace('_', " "));
            Some(AdbDevice { serial, state, model })
        })
        .collect()
}

/// SDK `platform-tools` directories to look in before `PATH`.
fn sdk_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(root) = std::env::var_os(var) {
            dirs.push(PathBuf::from(root).join("platform-tools"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join("Library/Android/sdk/platform-tools"));
        dirs.push(home.join("Android/Sdk/platform-tools"));
    }
    dirs
}

/// The first `adb` in `sdk_dirs`, then in `path`.
pub fn locate(path: &std::ffi::OsStr, sdk_dirs: &[PathBuf]) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
    sdk_dirs
        .iter()
        .cloned()
        .chain(std::env::split_paths(path))
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::command::fake::Fake;

    const DEVICES: &str = "* daemon not running; starting now at tcp:5037\n* daemon started successfully\nList of devices attached\nemulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\nR58M123ABC             unauthorized usb:1-1 transport_id:2\n\n";

    #[test]
    fn parses_device_lines() {
        let devices = parse_devices(DEVICES);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].serial, "emulator-5554");
        assert!(devices[0].is_emulator());
        assert_eq!(devices[0].model.as_deref(), Some("sdk gphone64 arm64"));
        assert_eq!(
            (devices[1].state.as_str(), devices[1].model.as_ref()),
            ("unauthorized", None)
        );
        assert!(!devices[1].is_emulator());
    }

    #[test]
    fn builds_adb_command_lines() {
        let fake = Arc::new(Fake::default());
        fake.reply("10.0.2.2:9090\n")
            .reply("null\n")
            .reply("")
            .reply("")
            .reply("");
        let adb = Adb::with_runner(Box::new(fake.clone()), "/sdk/adb".into());
        assert_eq!(adb.proxy("emulator-5554").unwrap().as_deref(), Some("10.0.2.2:9090"));
        assert_eq!(adb.proxy("emulator-5554").unwrap(), None);
        adb.set_proxy("emulator-5554", "10.0.2.2:9091").unwrap();
        adb.clear_proxy("emulator-5554").unwrap();
        adb.push("emulator-5554", Path::new("/tmp/ca dir/ca.crt"), DEVICE_CERT_PATH)
            .unwrap();
        assert_eq!(
            fake.calls(),
            [
                "/sdk/adb -s emulator-5554 shell settings get global http_proxy",
                "/sdk/adb -s emulator-5554 shell settings get global http_proxy",
                "/sdk/adb -s emulator-5554 shell settings put global http_proxy 10.0.2.2:9091",
                "/sdk/adb -s emulator-5554 shell settings put global http_proxy :0",
                "/sdk/adb -s emulator-5554 push /tmp/ca dir/ca.crt /sdcard/Download/proxymate-ca.crt",
            ]
        );
    }

    #[test]
    fn locates_adb_in_the_sdk_first() {
        let root = std::env::temp_dir().join(format!("pm-adb-{}", std::process::id()));
        let sdk = root.join("sdk/platform-tools");
        let bin = root.join("bin");
        std::fs::create_dir_all(&sdk).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("adb"), "").unwrap();
        let path = std::env::join_paths([&bin]).unwrap();
        assert_eq!(locate(&path, std::slice::from_ref(&sdk)), Some(bin.join("adb")));
        std::fs::write(sdk.join("adb"), "").unwrap();
        assert_eq!(locate(&path, std::slice::from_ref(&sdk)), Some(sdk.join("adb")));
        assert_eq!(locate(&std::ffi::OsString::new(), &[]), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
