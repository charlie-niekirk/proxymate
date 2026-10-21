//! The macOS system proxy through `networksetup`.
//!
//! M4 spike (macOS 27.0, 26A428): as an admin user, `networksetup -setwebproxy`
//! and friends write without any prompt (about 35 ms a call). The tool is
//! Apple-entitled for the `system.services.systemconfiguration.network`
//! right, so the `SCPreferencesCreateWithAuthorization` fallback the plan
//! kept in reserve isn't needed. Non-admin users get its error message.
//!
//! Writes to an active service take longer (configd applies them), so the
//! independent parts of a read or write run concurrently: `networksetup`
//! locks the preferences, and concurrent writes were checked not to lose
//! updates. A restore after SIGKILL must land within a second.

use std::process::Command;

use crate::PlatformError;
use crate::sysproxy::{Parts, ProxyBackend, ProxySetting, ServiceSettings};

const TOOL: &str = "/usr/sbin/networksetup";

pub struct NetworkSetup;

impl ProxyBackend for NetworkSetup {
    fn active_services(&self) -> Result<Vec<String>, PlatformError> {
        let listing = run(&["-listallnetworkservices"])?;
        let services: Vec<String> = parse_services(&listing);
        let active = std::thread::scope(|scope| {
            let checks: Vec<_> = services
                .iter()
                .map(|s| scope.spawn(move || run(&["-getinfo", s]).map(|info| has_address(&info))))
                .collect();
            checks
                .into_iter()
                .zip(&services)
                .filter_map(|(check, s)| matches!(check.join(), Ok(Ok(true))).then(|| s.clone()))
                .collect()
        });
        Ok(active)
    }

    fn read(&self, service: &str) -> Result<ServiceSettings, PlatformError> {
        let (web, secure, bypass) = join3(
            || run(&["-getwebproxy", service]),
            || run(&["-getsecurewebproxy", service]),
            || run(&["-getproxybypassdomains", service]),
        );
        Ok(ServiceSettings {
            service: service.to_owned(),
            web: parse_proxy(&web?),
            secure: parse_proxy(&secure?),
            bypass: parse_bypass(&bypass?),
        })
    }

    fn point_at(&self, service: &str, host: &str, port: u16, bypass: &[String]) -> Result<(), PlatformError> {
        let port = port.to_string();
        let (web, secure, bypass) = join3(
            || run(&["-setwebproxy", service, host, &port]).map(drop),
            || run(&["-setsecurewebproxy", service, host, &port]).map(drop),
            || set_bypass(service, bypass),
        );
        web.and(secure).and(bypass)
    }

    fn write(&self, saved: &ServiceSettings, now: &ServiceSettings, parts: Parts) -> Result<(), PlatformError> {
        let service = saved.service.as_str();
        let (web, secure, bypass) = join3(
            || match parts.web {
                true => write_proxy(service, ["-setwebproxy", "-setwebproxystate"], &saved.web, &now.web),
                false => Ok(()),
            },
            || match parts.secure {
                true => {
                    let verbs = ["-setsecurewebproxy", "-setsecurewebproxystate"];
                    write_proxy(service, verbs, &saved.secure, &now.secure)
                }
                false => Ok(()),
            },
            || match parts.bypass {
                true => set_bypass(service, &saved.bypass),
                false => Ok(()),
            },
        );
        web.and(secure).and(bypass)
    }
}

/// Puts one proxy entry back, in one call when the server is unchanged
/// (only the switch flips). A disabled entry with no server can't be
/// written as such (`networksetup` wants a host), so ours stays in the
/// fields, switched off.
fn write_proxy(
    service: &str,
    [set, state]: [&str; 2],
    saved: &ProxySetting,
    now: &ProxySetting,
) -> Result<(), PlatformError> {
    let same_server = saved.server == now.server && saved.port == now.port;
    if !saved.server.is_empty() && !same_server {
        // Setting the server also switches the entry on.
        run(&[set, service, &saved.server, &saved.port.to_string()])?;
        if saved.enabled {
            return Ok(());
        }
    }
    if saved.enabled != now.enabled || !same_server {
        run(&[state, service, if saved.enabled { "on" } else { "off" }])?;
    }
    Ok(())
}

/// Runs three closures at once.
fn join3<A: Send, B: Send, C: Send>(
    a: impl FnOnce() -> A + Send,
    b: impl FnOnce() -> B + Send,
    c: impl FnOnce() -> C + Send,
) -> (A, B, C) {
    std::thread::scope(|scope| {
        let (b, c) = (scope.spawn(b), scope.spawn(c));
        let a = a();
        let worker = "networksetup worker panicked";
        (a, b.join().expect(worker), c.join().expect(worker))
    })
}

fn set_bypass(service: &str, bypass: &[String]) -> Result<(), PlatformError> {
    let mut args = vec!["-setproxybypassdomains", service];
    if bypass.is_empty() {
        // The tool's spelling for "no bypass domains".
        args.push("Empty");
    } else {
        args.extend(bypass.iter().map(String::as_str));
    }
    run(&args).map(drop)
}

fn run(args: &[&str]) -> Result<String, PlatformError> {
    let out = Command::new(TOOL).args(args).output()?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // networksetup exits 0 on many failures and says so on stdout.
    let failed = !out.status.success() || stdout.starts_with("** Error") || stdout.contains("is not a recognized");
    if failed {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let message = [stdout.trim(), stderr.trim()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(": ");
        return Err(PlatformError::Os(format!("networksetup {}: {message}", args[0])));
    }
    Ok(stdout)
}

/// Enabled services from `-listallnetworkservices`; disabled ones start
/// with `*`, and the first line is a legend.
fn parse_services(listing: &str) -> Vec<String> {
    listing
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('*'))
        .map(str::to_owned)
        .collect()
}

/// True if `-getinfo` shows an IPv4 or IPv6 address.
fn has_address(info: &str) -> bool {
    info.lines().any(|line| {
        let Some((key, value)) = line.split_once(':') else {
            return false;
        };
        let (key, value) = (key.trim(), value.trim());
        (key == "IP address" || key == "IPv6 IP address") && !value.is_empty() && value != "none"
    })
}

fn parse_proxy(out: &str) -> ProxySetting {
    let mut setting = ProxySetting::default();
    for line in out.lines() {
        match line.split_once(':').map(|(k, v)| (k.trim(), v.trim())) {
            Some(("Enabled", v)) => setting.enabled = v == "Yes",
            Some(("Server", v)) => setting.server = v.to_owned(),
            Some(("Port", v)) => setting.port = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    setting
}

fn parse_bypass(out: &str) -> Vec<String> {
    if out.starts_with("There aren't any bypass domains") {
        return Vec::new();
    }
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_networksetup_output() {
        let listing = "An asterisk (*) denotes that a network service is disabled.\nAX88179B\n*Old VPN\nWi-Fi\n";
        assert_eq!(parse_services(listing), vec!["AX88179B", "Wi-Fi"]);

        let wifi = "DHCP Configuration\nIP address: 192.168.1.121\nSubnet mask: 255.255.255.0\nIPv6: Automatic\nIPv6 IP address: none\n";
        let idle = "DHCP Configuration\nClient ID: \nIPv6: Automatic\nIPv6 IP address: none\nIPv6 Router: none\n";
        assert!(has_address(wifi));
        assert!(!has_address(idle));

        let proxy = parse_proxy("Enabled: Yes\nServer: 127.0.0.1\nPort: 9090\nAuthenticated Proxy Enabled: 0\n");
        assert_eq!(
            proxy,
            ProxySetting {
                enabled: true,
                server: "127.0.0.1".into(),
                port: 9090
            }
        );
        assert_eq!(parse_proxy("Enabled: No\nServer: \nPort: 0\n"), ProxySetting::default());

        assert_eq!(parse_bypass("*.local\n169.254/16\n"), vec!["*.local", "169.254/16"]);
        assert!(parse_bypass("There aren't any bypass domains set on ProtonVPN.\n").is_empty());
    }

    /// Reads the real settings; changes nothing.
    #[test]
    fn reads_the_real_settings() {
        let backend = NetworkSetup;
        let services = backend.active_services().unwrap();
        for service in services {
            backend.read(&service).unwrap();
        }
    }
}
