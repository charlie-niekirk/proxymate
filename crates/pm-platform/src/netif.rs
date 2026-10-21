//! This machine's network addresses: the LAN interfaces phones can reach
//! the proxy on, and every local address (to tell our own connections and
//! requests apart from a LAN client's).

use std::net::{IpAddr, Ipv4Addr};

/// An IPv4 interface other devices may be able to reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanInterface {
    pub name: String,
    pub ip: Ipv4Addr,
}

/// IPv4 interfaces that are up and neither loopback nor link-local, best
/// first (see [`rank`]).
pub fn lan_interfaces() -> Vec<LanInterface> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut lan: Vec<LanInterface> = interfaces
        .into_iter()
        .filter(|i| i.is_oper_up() && !i.is_p2p())
        .filter_map(|i| match i.ip() {
            IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() => {
                Some(LanInterface { name: i.name, ip })
            }
            _ => None,
        })
        .collect();
    sort(&mut lan);
    lan
}

/// Every address of this machine, loopback included.
pub fn local_ips() -> Vec<IpAddr> {
    if_addrs::get_if_addrs()
        .map(|interfaces| interfaces.iter().map(|i| i.ip()).collect())
        .unwrap_or_default()
}

/// Sorts by [`rank`], then by name.
pub fn sort(interfaces: &mut [LanInterface]) {
    interfaces.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.name.cmp(&b.name)));
}

/// Lower is better. Wi-Fi and Ethernet (`en*`, `eth*`, `wl*`) on a private
/// network come first; VPN tunnels and virtual bridges (VMs, Internet
/// Sharing) last, since a phone is rarely on those.
pub fn rank(interface: &LanInterface) -> u8 {
    let name = interface.name.as_str();
    let physical = ["en", "eth", "wl"].iter().any(|p| name.starts_with(p));
    let virtual_ = [
        "utun", "tun", "tap", "ppp", "ipsec", "docker", "veth", "vmnet", "vboxnet",
    ]
    .iter()
    .any(|p| name.starts_with(p));
    match (physical, interface.ip.is_private(), virtual_) {
        (true, true, _) => 0,
        (true, false, _) => 1,
        (false, true, false) => 2,
        (false, _, false) => 3,
        (false, _, true) => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(name: &str, ip: &str) -> LanInterface {
        LanInterface {
            name: name.into(),
            ip: ip.parse().unwrap(),
        }
    }

    #[test]
    fn wifi_comes_before_bridges_and_tunnels() {
        let mut list = vec![
            iface("utun4", "10.8.0.2"),
            iface("bridge100", "192.168.64.1"),
            iface("en1", "192.168.1.20"),
            iface("en0", "10.0.0.5"),
            iface("en5", "82.10.1.4"),
        ];
        sort(&mut list);
        let names: Vec<_> = list.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["en0", "en1", "en5", "bridge100", "utun4"]);
    }

    #[test]
    fn local_ips_include_loopback() {
        let ips = local_ips();
        assert!(ips.iter().any(IpAddr::is_loopback), "{ips:?}");
        assert!(lan_interfaces().iter().all(|i| !i.ip.is_loopback()));
    }
}
