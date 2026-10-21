//! The iOS Simulator through `xcrun simctl`: list booted devices and add
//! the Proxymate CA to a device's keychain as a trusted root. Simulators
//! use the Mac's network stack and so its system proxy; nothing else needs
//! setting up.

use std::ffi::OsString;
use std::path::Path;

use serde::Deserialize;

use crate::PlatformError;
use crate::command::{self, Runner};

/// A booted simulator device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimDevice {
    pub udid: String,
    pub name: String,
    /// `iOS 18.0`, from the runtime identifier.
    pub runtime: String,
}

pub struct Simctl {
    runner: Box<dyn Runner>,
}

impl Simctl {
    pub fn system() -> Simctl {
        Simctl::with_runner(Box::new(command::System))
    }

    pub fn with_runner(runner: Box<dyn Runner>) -> Simctl {
        Simctl { runner }
    }

    /// `Ok` if `simctl` can be run. Command Line Tools alone don't have it;
    /// it ships with Xcode.
    pub fn check(&self) -> Result<(), PlatformError> {
        let out = self.xcrun(command::args(["--find", "simctl"]))?;
        out.ok().map(drop).map_err(|_| {
            PlatformError::Os(
                "simctl wasn't found. Install Xcode, then select it with `sudo xcode-select -s /Applications/Xcode.app`."
                    .into(),
            )
        })
    }

    pub fn booted(&self) -> Result<Vec<SimDevice>, PlatformError> {
        let json = self.xcrun(list_booted_args())?.ok()?;
        parse_devices(&json)
    }

    /// Adds `cert` (PEM or DER) to the device's keychain as a trusted root.
    pub fn add_root_cert(&self, udid: &str, cert: &Path) -> Result<(), PlatformError> {
        self.xcrun(add_root_cert_args(udid, cert))?.ok().map(drop)
    }

    fn xcrun(&self, args: Vec<OsString>) -> Result<command::Output, PlatformError> {
        self.runner.run(&"xcrun".into(), &args).map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                PlatformError::Os("xcrun wasn't found. The iOS Simulator needs Xcode on macOS.".into())
            }
            _ => err.into(),
        })
    }
}

pub fn list_booted_args() -> Vec<OsString> {
    command::args(["simctl", "list", "devices", "booted", "-j"])
}

pub fn add_root_cert_args(udid: &str, cert: &Path) -> Vec<OsString> {
    let mut args = command::args(["simctl", "keychain", udid, "add-root-cert"]);
    args.push(cert.as_os_str().to_owned());
    args
}

#[derive(Deserialize)]
struct DeviceList {
    devices: std::collections::BTreeMap<String, Vec<RawDevice>>,
}

#[derive(Deserialize)]
struct RawDevice {
    udid: String,
    name: String,
    state: String,
}

/// Booted devices from `simctl list devices -j`, sorted by runtime and name.
pub fn parse_devices(json: &str) -> Result<Vec<SimDevice>, PlatformError> {
    let list: DeviceList = serde_json::from_str(json)?;
    let mut devices: Vec<SimDevice> = list
        .devices
        .into_iter()
        .flat_map(|(runtime, devices)| {
            let runtime = runtime_name(&runtime);
            devices
                .into_iter()
                .filter(|d| d.state == "Booted")
                .map(move |d| SimDevice {
                    udid: d.udid,
                    name: d.name,
                    runtime: runtime.clone(),
                })
        })
        .collect();
    devices.sort_by(|a, b| a.runtime.cmp(&b.runtime).then_with(|| a.name.cmp(&b.name)));
    Ok(devices)
}

/// `com.apple.CoreSimulator.SimRuntime.iOS-18-0` → `iOS 18.0`.
fn runtime_name(id: &str) -> String {
    let short = id.rsplit('.').next().unwrap_or(id);
    match short.split_once('-') {
        Some((os, version)) => format!("{os} {}", version.replace('-', ".")),
        None => short.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::command::fake::Fake;

    const BOOTED: &str = r#"{
      "devices" : {
        "com.apple.CoreSimulator.SimRuntime.iOS-18-0" : [
          { "udid" : "B2C4", "isAvailable" : true, "state" : "Booted", "name" : "iPhone 16 Pro" },
          { "udid" : "0F00", "isAvailable" : true, "state" : "Shutdown", "name" : "iPhone SE" }
        ],
        "com.apple.CoreSimulator.SimRuntime.iOS-26-2" : [
          { "udid" : "A1B2", "isAvailable" : true, "state" : "Booted", "name" : "iPad Air" }
        ],
        "com.apple.CoreSimulator.SimRuntime.watchOS-11-0" : []
      }
    }"#;

    #[test]
    fn parses_booted_devices() {
        let devices = parse_devices(BOOTED).unwrap();
        assert_eq!(
            devices,
            [
                SimDevice {
                    udid: "B2C4".into(),
                    name: "iPhone 16 Pro".into(),
                    runtime: "iOS 18.0".into()
                },
                SimDevice {
                    udid: "A1B2".into(),
                    name: "iPad Air".into(),
                    runtime: "iOS 26.2".into()
                },
            ]
        );
        assert!(parse_devices("{}").is_err());
    }

    #[test]
    fn builds_simctl_command_lines() {
        let fake = std::sync::Arc::new(Fake::default());
        fake.reply("/usr/bin/simctl").reply(BOOTED).reply("");
        let simctl = Simctl::with_runner(Box::new(fake.clone()));
        simctl.check().unwrap();
        assert_eq!(simctl.booted().unwrap().len(), 2);
        let pem = PathBuf::from("/Users/me/Library/Application Support/Proxymate/ca/proxymate-ca.pem");
        simctl.add_root_cert("B2C4", &pem).unwrap();
        assert_eq!(
            fake.calls(),
            [
                "xcrun --find simctl",
                "xcrun simctl list devices booted -j",
                "xcrun simctl keychain B2C4 add-root-cert /Users/me/Library/Application Support/Proxymate/ca/proxymate-ca.pem",
            ]
        );
        // The path stays one argument despite its spaces.
        assert_eq!(add_root_cert_args("B2C4", &pem).len(), 5);
    }

    #[test]
    fn explains_missing_tools() {
        let fake = std::sync::Arc::new(Fake::default());
        fake.fail("xcrun: error: unable to find utility \"simctl\"")
            .missing()
            .fail("Invalid device: X");
        let simctl = Simctl::with_runner(Box::new(fake.clone()));
        assert!(simctl.check().unwrap_err().to_string().contains("Install Xcode"));
        assert!(simctl.booted().unwrap_err().to_string().contains("xcrun wasn't found"));
        let err = simctl.add_root_cert("X", Path::new("/ca.pem")).unwrap_err();
        assert_eq!(err.to_string(), "Invalid device: X");
    }
}
