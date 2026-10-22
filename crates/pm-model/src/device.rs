//! Device setup (M9): LAN access, the client allow-list, the iOS Simulator
//! and Android devices.

use serde::{Deserialize, Serialize};
use specta::Type;

/// An IPv4 address of this machine that other devices can reach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct LanInterface {
    /// `en0`, `bridge100`, …
    pub name: String,
    pub ip: String,
}

/// LAN access as the device setup wizard shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct DeviceAccess {
    /// The proxy listens on every interface, so phones can reach it.
    pub enabled: bool,
    /// The interface the toolbar and the QR codes use, by name.
    pub interface: Option<String>,
    /// Best first.
    pub interfaces: Vec<LanInterface>,
    pub port: Option<u16>,
    /// The chosen interface's address and port, e.g. `10.0.0.5:9090`.
    pub proxy: Option<String>,
    /// `http://<lan-ip>:<port>/`: the onboarding page, for QR codes.
    pub setup_url: Option<String>,
    /// A QR code of `setup_url` as an SVG document.
    pub qr_svg: Option<String>,
    /// The last failure to switch LAN access.
    pub error: Option<String>,
}

/// What the user decided about a LAN client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum DeviceDecision {
    /// Not answered yet: its connections are held for up to 30 s.
    Pending,
    Allowed,
    Denied,
}

/// A LAN client: one IP address, with its nickname and what it sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct DeviceClient {
    pub ip: String,
    pub nickname: Option<String>,
    pub decision: DeviceDecision,
    /// What the Client column and the sidebar show for its flows.
    pub label: String,
    /// Since launch.
    pub flows: u32,
    /// Flows decrypted by the MITM: the device trusts the CA.
    pub https_decrypted: u32,
    /// HTTPS connections the device refused (it doesn't trust the CA yet).
    pub https_refused: u32,
    /// The device fetched the onboarding page or a certificate.
    pub onboarding: bool,
    /// Epoch ms.
    pub last_seen_ms: Option<f64>,
    /// Connections currently held while the decision is pending.
    pub waiting: u32,
    /// When the newest held connection gives up and is refused, epoch ms
    /// (for the prompt's countdown).
    #[serde(default)]
    pub held_until_ms: Option<f64>,
}

/// Traffic from local clients of one kind (the Simulator, the emulator).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct LocalActivity {
    pub flows: u32,
    pub https_decrypted: u32,
    pub https_refused: u32,
}

/// LAN clients and local device activity, pushed whenever they change.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Type)]
pub struct DevicesSnapshot {
    /// Pending first, then by IP.
    pub devices: Vec<DeviceClient>,
    pub simulator: LocalActivity,
    pub emulator: LocalActivity,
}

/// A booted iOS Simulator device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SimDevice {
    pub udid: String,
    pub name: String,
    /// `iOS 18.0`.
    pub runtime: String,
    /// The CA was added to its keychain since launch.
    pub cert_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct SimulatorStatus {
    /// `xcrun simctl` works (Xcode is installed and selected).
    pub available: bool,
    /// Why it doesn't, or the error listing devices.
    pub detail: Option<String>,
    pub booted: Vec<SimDevice>,
}

/// One device's result from "Install to all booted".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SimInstallResult {
    pub udid: String,
    pub name: String,
    pub error: Option<String>,
}

/// A device or emulator `adb` sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AndroidDevice {
    pub serial: String,
    pub model: Option<String>,
    pub emulator: bool,
    /// `device`, `offline`, `unauthorized`, …
    pub state: String,
    /// Its global `http_proxy` setting, if any.
    pub proxy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct AndroidStatus {
    /// `adb` was found.
    pub available: bool,
    pub detail: Option<String>,
    pub devices: Vec<AndroidDevice>,
    /// The `http_proxy` value emulators should use (`10.0.2.2:<port>`).
    pub emulator_proxy: Option<String>,
}
