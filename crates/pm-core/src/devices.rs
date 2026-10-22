//! Device setup (M9): LAN access, the client allow-list, and the iOS
//! Simulator and Android helpers behind the Device Setup Wizard.
//!
//! - **Admission.** Loopback and this machine's own addresses are always
//!   allowed. A LAN address the user hasn't answered for yet is shown as an
//!   "Allow?" prompt, and its connections wait up to [`HOLD`] for the answer
//!   before they are refused. Answers persist in `app.sqlite` (a `devices`
//!   table), as do LAN access on/off and the chosen interface.
//! - **LAN access** rebinds the listener from loopback to every interface
//!   on the same port. The chosen interface only decides which address the
//!   toolbar and the QR codes show.
//! - **Activity.** Flows from LAN devices, the Simulator and the Android
//!   emulator are counted (and whether their HTTPS was decrypted), so the
//!   wizard can say when a device's first flow arrives.

use std::collections::{BTreeMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pm_model::{
    AndroidDevice, AndroidStatus, DeviceAccess, DeviceClient, DeviceDecision, DevicesSnapshot, FlowSummary,
    LanInterface, LocalActivity, SimDevice, SimInstallResult, SimulatorStatus, TlsStatus,
};
use pm_platform::adb::{self, Adb};
use pm_platform::netif;
use pm_platform::simctl::Simctl;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::client::{ClientApp, ClientKind};
use crate::proxy::now_ms;
use crate::rules::db::{AppDb, SavedDevice};
use crate::{Engine, Error};

/// How long a new LAN client's connections wait for the user's answer.
pub const HOLD: Duration = Duration::from_secs(30);
/// The friendly onboarding host: `http://proxy.mate/` through the proxy.
pub const ONBOARDING_HOST: &str = "proxy.mate";
/// How an Android emulator reaches the host's loopback.
const EMULATOR_HOST: &str = "10.0.2.2";
const LOCAL_IPS_TTL: Duration = Duration::from_secs(5);

/// Receives device snapshots. Returning `false` unsubscribes.
pub type DevicesSink = Box<dyn FnMut(DevicesSnapshot) -> bool + Send>;

/// What a new flow says about its device.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Seen {
    /// A request decrypted by the MITM: the client trusts the CA.
    decrypted: bool,
    /// The client rejected our certificate.
    refused: bool,
}

impl Seen {
    pub fn of(summary: &FlowSummary) -> Seen {
        Seen {
            decrypted: summary.tls == TlsStatus::Intercepted && &*summary.method != "CONNECT",
            refused: summary.tls == TlsStatus::LikelyPinned,
        }
    }
}

/// Whether a new connection may proceed.
pub(crate) enum Admission {
    /// Loopback or one of our own addresses.
    Local,
    /// An allowed LAN device.
    Remote(Arc<ClientApp>),
    Refused,
}

/// The `lan` settings row in `app.sqlite`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LanSettings {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    interface: Option<String>,
}

const LAN_SETTINGS_KEY: &str = "lan";

struct Client {
    nickname: Option<String>,
    decided: watch::Sender<DeviceDecision>,
    flows: u32,
    https_decrypted: u32,
    https_refused: u32,
    onboarding: bool,
    last_seen_ms: Option<f64>,
    waiting: u32,
    /// When the newest held connection is refused, epoch ms.
    held_until_ms: Option<f64>,
}

impl Client {
    fn new(nickname: Option<String>, decision: DeviceDecision) -> Client {
        Client {
            nickname,
            decided: watch::Sender::new(decision),
            flows: 0,
            https_decrypted: 0,
            https_refused: 0,
            onboarding: false,
            last_seen_ms: None,
            waiting: 0,
            held_until_ms: None,
        }
    }

    fn decision(&self) -> DeviceDecision {
        *self.decided.borrow()
    }
}

#[derive(Default)]
struct State {
    lan_enabled: bool,
    interface: Option<String>,
    lan_error: Option<String>,
    clients: BTreeMap<IpAddr, Client>,
    simulator: LocalActivity,
    emulator: LocalActivity,
}

pub(crate) struct DeviceRegistry {
    db: Mutex<AppDb>,
    hold: Duration,
    state: Mutex<State>,
    dirty: AtomicBool,
    sink: Mutex<Option<DevicesSink>>,
    local_ips: Mutex<Option<(Instant, Arc<Vec<IpAddr>>)>>,
    /// Simulators the CA was installed into since launch.
    sim_installed: Mutex<HashSet<String>>,
    /// Emulators whose proxy we set, to clear on quit.
    emulator_proxies: Mutex<HashSet<String>>,
}

impl DeviceRegistry {
    /// Loads the allow-list from `<dir>/app.sqlite`, or keeps it in memory
    /// without a directory (tests, throwaway engines), like the rules do.
    pub fn load(dir: Option<&Path>, hold: Duration) -> DeviceRegistry {
        let db = dir
            .map(AppDb::open)
            .unwrap_or_else(AppDb::in_memory)
            .or_else(|err| {
                eprintln!("proxymate: can't open the app database: {err}; devices won't be saved");
                AppDb::in_memory()
            })
            .expect("in-memory SQLite");
        DeviceRegistry::new(db, hold)
    }

    fn new(db: AppDb, hold: Duration) -> DeviceRegistry {
        let lan: LanSettings = db.setting(LAN_SETTINGS_KEY).unwrap_or_default();
        let saved = db.devices().unwrap_or_else(|err| {
            eprintln!("proxymate: can't read the device allow-list: {err}");
            Vec::new()
        });
        let clients = saved
            .into_iter()
            .map(|c| {
                let decision = if c.allowed {
                    DeviceDecision::Allowed
                } else {
                    DeviceDecision::Denied
                };
                (c.ip, Client::new(c.nickname, decision))
            })
            .collect();
        DeviceRegistry {
            db: Mutex::new(db),
            hold,
            state: Mutex::new(State {
                lan_enabled: lan.enabled,
                interface: lan.interface,
                clients,
                ..State::default()
            }),
            dirty: AtomicBool::new(true),
            sink: Mutex::new(None),
            local_ips: Mutex::new(None),
            sim_installed: Mutex::new(HashSet::new()),
            emulator_proxies: Mutex::new(HashSet::new()),
        }
    }

    /// Lets `ip` in, holds it while the user decides, or refuses it.
    /// `remote` names an admitted device's flows.
    pub async fn admit(&self, ip: IpAddr, remote: impl Fn(IpAddr, &str) -> Arc<ClientApp>) -> Admission {
        let ip = ip.to_canonical();
        if self.is_local(ip) {
            return Admission::Local;
        }
        let mut decided = {
            let mut state = self.state.lock();
            let client = state
                .clients
                .entry(ip)
                .or_insert_with(|| Client::new(None, DeviceDecision::Pending));
            match client.decision() {
                DeviceDecision::Allowed => {
                    return Admission::Remote(remote(ip, &label(ip, client.nickname.as_deref())));
                }
                DeviceDecision::Denied => return Admission::Refused,
                DeviceDecision::Pending => {
                    client.waiting += 1;
                    client.held_until_ms = Some(crate::proxy::now_ms() + self.hold.as_secs_f64() * 1000.0);
                    self.mark_dirty();
                    client.decided.subscribe()
                }
            }
        };
        let answer = tokio::time::timeout(self.hold, decided.wait_for(|d| *d != DeviceDecision::Pending))
            .await
            .ok()
            .and_then(Result::ok)
            .map(|d| *d);
        let mut state = self.state.lock();
        let Some(client) = state.clients.get_mut(&ip) else {
            // Forgotten while we waited.
            return Admission::Refused;
        };
        client.waiting = client.waiting.saturating_sub(1);
        if client.waiting == 0 {
            client.held_until_ms = None;
        }
        self.mark_dirty();
        match answer {
            Some(DeviceDecision::Allowed) => Admission::Remote(remote(ip, &label(ip, client.nickname.as_deref()))),
            _ => Admission::Refused,
        }
    }

    /// Loopback, unspecified, or one of this machine's interface addresses.
    pub fn is_local(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        if ip.is_loopback() || ip.is_unspecified() {
            return true;
        }
        let mut cache = self.local_ips.lock();
        let ips = match &*cache {
            Some((at, ips)) if at.elapsed() < LOCAL_IPS_TTL => ips.clone(),
            _ => {
                let ips = Arc::new(netif::local_ips());
                *cache = Some((Instant::now(), ips.clone()));
                ips
            }
        };
        ips.contains(&ip)
    }

    /// Records the user's answer for `ip` and releases its held connections.
    pub fn decide(&self, ip: IpAddr, allow: bool, nickname: Option<String>) -> Result<(), Error> {
        let ip = ip.to_canonical();
        let nickname = nickname.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
        let decision = if allow {
            DeviceDecision::Allowed
        } else {
            DeviceDecision::Denied
        };
        {
            let mut state = self.state.lock();
            let client = state
                .clients
                .entry(ip)
                .or_insert_with(|| Client::new(None, DeviceDecision::Pending));
            if nickname.is_some() || !allow {
                client.nickname = nickname.or(client.nickname.take());
            }
            client.decided.send_replace(decision);
        }
        self.mark_dirty();
        self.save()
    }

    /// Drops `ip` from the list; its next connection asks again.
    pub fn forget(&self, ip: IpAddr) -> Result<(), Error> {
        let removed = self.state.lock().clients.remove(&ip.to_canonical());
        if let Some(client) = removed {
            // Held connections stop waiting and are refused.
            client.decided.send_replace(DeviceDecision::Denied);
        }
        self.mark_dirty();
        self.save()
    }

    pub fn note_flow(&self, app: &ClientApp, seen: Seen) {
        let mut state = self.state.lock();
        let activity = match app.kind {
            ClientKind::Local => return,
            ClientKind::Simulator => &mut state.simulator,
            ClientKind::Emulator => &mut state.emulator,
            ClientKind::Remote(ip) => {
                let Some(client) = state.clients.get_mut(&ip) else {
                    return;
                };
                client.flows += 1;
                client.https_decrypted += u32::from(seen.decrypted);
                client.https_refused += u32::from(seen.refused);
                client.last_seen_ms = Some(now_ms());
                drop(state);
                self.mark_dirty();
                return;
            }
        };
        activity.flows += 1;
        activity.https_decrypted += u32::from(seen.decrypted);
        activity.https_refused += u32::from(seen.refused);
        drop(state);
        self.mark_dirty();
    }

    /// A LAN device fetched the onboarding page or a certificate.
    pub fn note_onboarding(&self, ip: IpAddr) {
        if let Some(client) = self.state.lock().clients.get_mut(&ip) {
            client.onboarding = true;
            client.last_seen_ms = Some(now_ms());
        }
        self.mark_dirty();
    }

    pub fn snapshot(&self) -> DevicesSnapshot {
        let state = self.state.lock();
        let mut devices: Vec<DeviceClient> = state
            .clients
            .iter()
            .map(|(ip, c)| DeviceClient {
                ip: ip.to_string(),
                nickname: c.nickname.clone(),
                decision: c.decision(),
                label: label(*ip, c.nickname.as_deref()),
                flows: c.flows,
                https_decrypted: c.https_decrypted,
                https_refused: c.https_refused,
                onboarding: c.onboarding,
                last_seen_ms: c.last_seen_ms,
                waiting: c.waiting,
                held_until_ms: c.held_until_ms,
            })
            .collect();
        // BTreeMap order is by address; pending prompts go first.
        devices.sort_by_key(|d| d.decision != DeviceDecision::Pending);
        DevicesSnapshot {
            devices,
            simulator: state.simulator,
            emulator: state.emulator,
        }
    }

    pub fn subscribe(&self, sink: DevicesSink) {
        *self.sink.lock() = Some(sink);
        self.mark_dirty();
    }

    /// Part of the publisher tick: pushes a snapshot if anything changed.
    pub fn publish(&self) {
        let mut sink = self.sink.lock();
        let Some(send) = sink.as_mut() else {
            return;
        };
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return;
        }
        if !send(self.snapshot()) {
            *sink = None;
        }
    }

    pub fn lan_enabled(&self) -> bool {
        self.state.lock().lan_enabled
    }

    fn set_lan(&self, enabled: bool, error: Option<String>) -> Result<(), Error> {
        {
            let mut state = self.state.lock();
            state.lan_enabled = enabled;
            state.lan_error = error;
        }
        self.save()
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    fn save(&self) -> Result<(), Error> {
        let (lan, devices) = {
            let state = self.state.lock();
            let lan = LanSettings {
                enabled: state.lan_enabled,
                interface: state.interface.clone(),
            };
            let devices: Vec<SavedDevice> = state
                .clients
                .iter()
                .filter(|(_, c)| c.decision() != DeviceDecision::Pending)
                .map(|(ip, c)| SavedDevice {
                    ip: *ip,
                    nickname: c.nickname.clone(),
                    allowed: c.decision() == DeviceDecision::Allowed,
                })
                .collect();
            (lan, devices)
        };
        let mut db = self.db.lock();
        db.save_setting(LAN_SETTINGS_KEY, &lan)
            .and_then(|()| db.save_devices(&devices))
            .map_err(|e| Error::Device(format!("can't save the device allow-list: {e}")))
    }
}

/// How a LAN device's flows are labelled: `Pixel 8 (10.0.0.23)`, or just
/// the address without a nickname.
pub fn label(ip: IpAddr, nickname: Option<&str>) -> String {
    match nickname {
        Some(name) => format!("{name} ({ip})"),
        None => ip.to_string(),
    }
}

/// The onboarding page a device opens, and what its QR code encodes.
pub fn setup_url(ip: IpAddr, port: u16) -> String {
    format!("http://{}/", SocketAddr::new(ip, port))
}

/// A QR code of `payload` as a standalone SVG, dark on white with the
/// standard quiet zone so phone cameras pick it up in either theme.
pub fn qr_svg(payload: &str) -> Option<String> {
    let code = qrcode::QrCode::new(payload.as_bytes()).ok()?;
    Some(
        code.render::<qrcode::render::svg::Color<'_>>()
            .min_dimensions(200, 200)
            .dark_color(qrcode::render::svg::Color("#000000"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build(),
    )
}

impl Engine {
    pub(crate) fn device_registry(&self) -> &DeviceRegistry {
        &self.platform.devices
    }

    // ---- LAN access ----

    /// LAN interfaces, the chosen one, and the setup URL and QR code.
    pub fn device_access(&self) -> DeviceAccess {
        let interfaces = netif::lan_interfaces();
        let (enabled, chosen, error) = {
            let state = self.platform.devices.state.lock();
            (state.lan_enabled, state.interface.clone(), state.lan_error.clone())
        };
        let interface = chosen
            .as_deref()
            .and_then(|name| interfaces.iter().find(|i| i.name == name))
            .or(interfaces.first());
        let port = self.proxy_addr().map(|a| a.port());
        let addr = interface
            .zip(port)
            .map(|(i, port)| SocketAddr::new(IpAddr::V4(i.ip), port));
        let setup_url = addr.filter(|_| enabled).map(|a| setup_url(a.ip(), a.port()));
        DeviceAccess {
            enabled,
            interface: interface.map(|i| i.name.clone()),
            interfaces: interfaces
                .iter()
                .map(|i| LanInterface {
                    name: i.name.clone(),
                    ip: i.ip.to_string(),
                })
                .collect(),
            port,
            proxy: addr.map(|a| a.to_string()),
            qr_svg: setup_url.as_deref().and_then(qr_svg),
            setup_url,
            error,
        }
    }

    /// Listens on every interface (or loopback only again), on the same
    /// port. Unknown LAN clients still need the user's OK.
    pub async fn set_lan_access(self: &Arc<Self>, on: bool) -> DeviceAccess {
        let ip = if on {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        let result = self.rebind_proxy(ip).await;
        let saved = match result {
            Ok(_) => self.platform.devices.set_lan(on, None),
            Err(err) => self.platform.devices.set_lan(!on, Some(err.to_string())),
        };
        if let Err(err) = saved {
            self.platform.devices.state.lock().lan_error = Some(err.to_string());
        }
        self.device_access()
    }

    /// Picks the interface the toolbar and the QR codes show.
    pub fn set_lan_interface(&self, name: &str) -> Result<DeviceAccess, Error> {
        self.platform.devices.state.lock().interface = Some(name.to_owned());
        self.platform.devices.save()?;
        Ok(self.device_access())
    }

    /// The address the toolbar shows for a listener bound to `addr`: the
    /// chosen LAN interface when listening on every interface.
    pub(crate) fn display_addr(&self, addr: SocketAddr) -> SocketAddr {
        if !addr.ip().is_unspecified() {
            return addr;
        }
        let interfaces = netif::lan_interfaces();
        let chosen = self.platform.devices.state.lock().interface.clone();
        let ip = chosen
            .as_deref()
            .and_then(|name| interfaces.iter().find(|i| i.name == name))
            .or(interfaces.first())
            .map_or(IpAddr::V4(Ipv4Addr::LOCALHOST), |i| IpAddr::V4(i.ip));
        SocketAddr::new(ip, addr.port())
    }

    // ---- Allow-list ----

    pub(crate) async fn admit(&self, ip: IpAddr) -> Admission {
        let clients = self.clients();
        self.platform
            .devices
            .admit(ip, |ip, label| clients.remote(ip, label))
            .await
    }

    /// LAN clients (pending prompts first) and local device activity.
    pub fn devices(&self) -> DevicesSnapshot {
        self.platform.devices.snapshot()
    }

    /// Sends the devices to `sink` now and on every tick they change,
    /// replacing any previous subscriber.
    pub fn subscribe_devices(&self, sink: DevicesSink) {
        self.platform.devices.subscribe(sink);
    }

    /// Allows or denies a LAN client, with an optional nickname for the
    /// Client column. Held connections from it proceed or are refused.
    pub fn decide_device(&self, ip: &str, allow: bool, nickname: Option<String>) -> Result<DevicesSnapshot, Error> {
        self.platform.devices.decide(parse_ip(ip)?, allow, nickname)?;
        Ok(self.devices())
    }

    /// Removes a LAN client from the list; it is asked about again.
    pub fn forget_device(&self, ip: &str) -> Result<DevicesSnapshot, Error> {
        self.platform.devices.forget(parse_ip(ip)?)?;
        Ok(self.devices())
    }

    // ---- iOS Simulator ----

    /// Whether `simctl` works, and the booted devices.
    pub async fn simulator_status(self: &Arc<Self>) -> SimulatorStatus {
        let engine = self.clone();
        tokio::task::spawn_blocking(move || {
            let simctl = Simctl::system();
            if let Err(err) = simctl.check() {
                return SimulatorStatus {
                    available: false,
                    detail: Some(err.to_string()),
                    booted: Vec::new(),
                };
            }
            match simctl.booted() {
                Ok(devices) => SimulatorStatus {
                    available: true,
                    detail: None,
                    booted: engine.sim_devices(devices),
                },
                Err(err) => SimulatorStatus {
                    available: true,
                    detail: Some(err.to_string()),
                    booted: Vec::new(),
                },
            }
        })
        .await
        .unwrap_or_default()
    }

    fn sim_devices(&self, devices: Vec<pm_platform::simctl::SimDevice>) -> Vec<SimDevice> {
        let installed = self.platform.devices.sim_installed.lock();
        devices
            .into_iter()
            .map(|d| SimDevice {
                cert_installed: installed.contains(&d.udid),
                udid: d.udid,
                name: d.name,
                runtime: d.runtime,
            })
            .collect()
    }

    /// Adds the CA to the keychain of each booted simulator in `udids`
    /// (every booted one if empty) as a trusted root.
    pub async fn install_simulator_cert(self: &Arc<Self>, udids: Vec<String>) -> Result<Vec<SimInstallResult>, Error> {
        let engine = self.clone();
        tokio::task::spawn_blocking(move || {
            let simctl = Simctl::system();
            let pem = PathBuf::from(engine.ca_info().cert_path);
            let booted = simctl.booted()?;
            let results = booted
                .into_iter()
                .filter(|d| udids.is_empty() || udids.contains(&d.udid))
                .map(|d| {
                    let error = simctl.add_root_cert(&d.udid, &pem).err().map(|e| e.to_string());
                    if error.is_none() {
                        engine.platform.devices.sim_installed.lock().insert(d.udid.clone());
                    }
                    SimInstallResult {
                        udid: d.udid,
                        name: d.name,
                        error,
                    }
                })
                .collect();
            Ok(results)
        })
        .await
        .map_err(|e| Error::Device(e.to_string()))?
    }

    // ---- Android ----

    /// Devices and emulators `adb` sees, with their proxy settings.
    pub async fn android_status(self: &Arc<Self>) -> AndroidStatus {
        let port = self.proxy_addr().map(|a| a.port());
        tokio::task::spawn_blocking(move || {
            let emulator_proxy = port.map(|p| format!("{EMULATOR_HOST}:{p}"));
            let Some(adb) = Adb::system() else {
                return AndroidStatus {
                    available: false,
                    detail: Some("adb wasn't found. Install the Android SDK platform-tools.".into()),
                    devices: Vec::new(),
                    emulator_proxy,
                };
            };
            match adb.devices() {
                Ok(devices) => AndroidStatus {
                    available: true,
                    detail: None,
                    devices: devices
                        .into_iter()
                        .map(|d| AndroidDevice {
                            proxy: (d.state == "device")
                                .then(|| adb.proxy(&d.serial).ok().flatten())
                                .flatten(),
                            emulator: d.is_emulator(),
                            serial: d.serial,
                            model: d.model,
                            state: d.state,
                        })
                        .collect(),
                    emulator_proxy,
                },
                Err(err) => AndroidStatus {
                    available: true,
                    detail: Some(err.to_string()),
                    devices: Vec::new(),
                    emulator_proxy,
                },
            }
        })
        .await
        .unwrap_or_default()
    }

    /// Points an emulator's global HTTP proxy at us (`10.0.2.2:<port>`), or
    /// clears it. Proxies set here are cleared again on quit.
    pub async fn set_android_proxy(self: &Arc<Self>, serial: &str, on: bool) -> Result<AndroidStatus, Error> {
        let port = self
            .proxy_addr()
            .map(|a| a.port())
            .ok_or_else(|| Error::Proxy("the proxy isn't running".into()))?;
        if !serial.starts_with("emulator-") {
            return Err(Error::Device(
                "only emulators can be set up over adb; set a phone's proxy in its Wi-Fi settings".into(),
            ));
        }
        let engine = self.clone();
        let serial = serial.to_owned();
        tokio::task::spawn_blocking(move || {
            let adb = Adb::system().ok_or_else(|| Error::Device("adb wasn't found".into()))?;
            let mut ours = engine.platform.devices.emulator_proxies.lock();
            if on {
                adb.set_proxy(&serial, &format!("{EMULATOR_HOST}:{port}"))?;
                ours.insert(serial);
            } else {
                adb.clear_proxy(&serial)?;
                ours.remove(&serial);
            }
            Ok::<_, Error>(())
        })
        .await
        .map_err(|e| Error::Device(e.to_string()))??;
        Ok(self.android_status().await)
    }

    /// Copies the CA (DER) to the device's Downloads folder, for Settings →
    /// Security → Install a certificate → CA certificate. Returns where.
    pub async fn push_android_cert(self: &Arc<Self>, serial: &str) -> Result<String, Error> {
        let der = self.tls().ca.cert_der().to_vec();
        let local = self.root.join("ca").join("proxymate-ca.crt");
        let serial = serial.to_owned();
        tokio::task::spawn_blocking(move || {
            std::fs::write(&local, der)?;
            let adb = Adb::system().ok_or_else(|| Error::Device("adb wasn't found".into()))?;
            adb.push(&serial, &local, adb::DEVICE_CERT_PATH)?;
            Ok(adb::DEVICE_CERT_PATH.to_owned())
        })
        .await
        .map_err(|e| Error::Device(e.to_string()))?
    }

    /// Clears the proxy of every emulator we pointed at us. Blocking; for
    /// quit and signal handlers.
    pub fn restore_android_proxies(&self) {
        let serials: Vec<String> = self.platform.devices.emulator_proxies.lock().drain().collect();
        if serials.is_empty() {
            return;
        }
        let Some(adb) = Adb::system() else { return };
        for serial in serials {
            if let Err(err) = adb.clear_proxy(&serial) {
                eprintln!("proxymate: can't clear the proxy of {serial}: {err}");
            }
        }
    }
}

fn parse_ip(ip: &str) -> Result<IpAddr, Error> {
    ip.trim()
        .parse()
        .map_err(|_| Error::Device(format!("{ip:?} isn't an IP address")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(hold: Duration) -> (DeviceRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!("pm-devices-{}", uuid::Uuid::new_v4()));
        (DeviceRegistry::load(Some(&dir), hold), dir)
    }

    fn remote(ip: IpAddr, label: &str) -> Arc<ClientApp> {
        Arc::new(ClientApp {
            name: label.into(),
            bundle_id: None,
            bundle_path: None,
            kind: ClientKind::Remote(ip),
        })
    }

    const PHONE: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 23));

    #[tokio::test]
    async fn loopback_is_always_admitted() {
        let (devices, dir) = registry(Duration::from_millis(10));
        assert!(matches!(
            devices.admit("127.0.0.1".parse().unwrap(), remote).await,
            Admission::Local
        ));
        assert!(matches!(
            devices.admit("::1".parse().unwrap(), remote).await,
            Admission::Local
        ));
        assert!(matches!(
            devices.admit("::ffff:127.0.0.1".parse().unwrap(), remote).await,
            Admission::Local
        ));
        assert!(devices.snapshot().devices.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn new_devices_are_held_then_refused() {
        let (devices, dir) = registry(Duration::from_millis(50));
        let started = Instant::now();
        assert!(matches!(devices.admit(PHONE, remote).await, Admission::Refused));
        assert!(started.elapsed() >= Duration::from_millis(50));
        // The prompt stays up after the hold runs out.
        let snapshot = devices.snapshot();
        assert_eq!(snapshot.devices.len(), 1);
        assert_eq!(snapshot.devices[0].decision, DeviceDecision::Pending);
        assert_eq!(snapshot.devices[0].waiting, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn held_connections_proceed_once_allowed() {
        let (devices, dir) = registry(Duration::from_secs(10));
        let devices = Arc::new(devices);
        let waiting: Vec<_> = (0..2)
            .map(|_| {
                let devices = devices.clone();
                tokio::spawn(async move { devices.admit(PHONE, remote).await })
            })
            .collect();
        while devices.snapshot().devices.first().map(|d| d.waiting) != Some(2) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        devices.decide(PHONE, true, Some(" Pixel 8 ".into())).unwrap();
        for admission in waiting {
            match admission.await.unwrap() {
                Admission::Remote(app) => {
                    assert_eq!(&*app.name, "Pixel 8 (203.0.113.23)");
                    assert_eq!(app.kind, ClientKind::Remote(PHONE));
                }
                _ => panic!("expected the device to be admitted"),
            }
        }
        // Later connections go straight through.
        assert!(matches!(devices.admit(PHONE, remote).await, Admission::Remote(_)));

        // The answer persists.
        let again = DeviceRegistry::load(Some(&dir), HOLD);
        let snapshot = again.snapshot();
        assert_eq!(snapshot.devices[0].decision, DeviceDecision::Allowed);
        assert_eq!(snapshot.devices[0].label, "Pixel 8 (203.0.113.23)");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn denied_and_forgotten_devices() {
        let (devices, dir) = registry(Duration::from_secs(10));
        let devices = Arc::new(devices);
        let held = {
            let devices = devices.clone();
            tokio::spawn(async move { devices.admit(PHONE, remote).await })
        };
        while devices.snapshot().devices.is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        devices.decide(PHONE, false, None).unwrap();
        assert!(matches!(held.await.unwrap(), Admission::Refused));
        // Refused at once from now on.
        let started = Instant::now();
        assert!(matches!(devices.admit(PHONE, remote).await, Admission::Refused));
        assert!(started.elapsed() < Duration::from_secs(1));

        devices.forget(PHONE).unwrap();
        assert!(devices.snapshot().devices.is_empty());
        let again = DeviceRegistry::load(Some(&dir), HOLD);
        assert!(again.snapshot().devices.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn activity_is_counted_per_device() {
        let (devices, dir) = registry(HOLD);
        devices.decide(PHONE, true, None).unwrap();
        let phone = remote(PHONE, "203.0.113.23");
        devices.note_flow(&phone, Seen::default());
        devices.note_flow(
            &phone,
            Seen {
                decrypted: true,
                refused: false,
            },
        );
        let sim = ClientApp {
            name: "MobileSafari (Simulator)".into(),
            bundle_id: None,
            bundle_path: None,
            kind: ClientKind::Simulator,
        };
        devices.note_flow(
            &sim,
            Seen {
                decrypted: false,
                refused: true,
            },
        );
        devices.note_onboarding(PHONE);
        let snapshot = devices.snapshot();
        let device = &snapshot.devices[0];
        assert_eq!((device.flows, device.https_decrypted, device.onboarding), (2, 1, true));
        assert_eq!(
            snapshot.simulator,
            LocalActivity {
                flows: 1,
                https_decrypted: 0,
                https_refused: 1
            }
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn qr_codes_encode_the_setup_url() {
        let url = setup_url("10.0.0.5".parse().unwrap(), 9090);
        assert_eq!(url, "http://10.0.0.5:9090/");
        let svg = qr_svg(&url).unwrap();
        assert!(svg.starts_with("<?xml") && svg.contains("<svg"), "{svg}");
        // Version 2 at error correction M: 25×25 modules plus the quiet zone.
        let code = qrcode::QrCode::new(url.as_bytes()).unwrap();
        assert_eq!(code.width(), 25);
        assert_eq!(label(PHONE, None), "203.0.113.23");
    }
}
