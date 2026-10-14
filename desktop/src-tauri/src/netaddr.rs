//! Which of the PC's addresses go into the pairing QR code (protocol.md, "QR code contents").
//!
//! Only private (RFC 1918) addresses of adapters that are up. Loopback, link-local, VPN (including `100.64.0.0/10`)
//! and virtual adapters (Hyper-V, WSL, VirtualBox, VMware, Docker) are left out, but **the address of the adapter that
//! holds the default route is never left out because of its adapter**: a Hyper-V external switch can be the PC's
//! only network. At most 3 go in; if there are more, the default-route address stays and physical adapters fill the
//! rest.
use std::net::{Ipv4Addr, SocketAddrV4};

pub const MAX_ADDRESSES: usize = 3;

const IF_TYPE_ETHERNET: u32 = 6;
const IF_TYPE_PPP: u32 = 23;
const IF_TYPE_LOOPBACK: u32 = 24;
const IF_TYPE_WIFI: u32 = 71;
const IF_TYPE_TUNNEL: u32 = 131;

/// Names of adapters a phone on the home Wi-Fi can't reach. The same list as `tools/net-check.ts`, plus VPN drivers.
const VIRTUAL_NAMES: [&str; 25] = [
    "vethernet",
    "hyper-v",
    "wsl",
    "virtualbox",
    "vbox",
    "vmware",
    "vmnet",
    "docker",
    "virbr",
    "tailscale",
    "zerotier",
    "hamachi",
    "npcap",
    "bluetooth",
    "loopback",
    "wireguard",
    "openvpn",
    "tap-windows",
    "vpn",
    "teredo",
    "isatap",
    "pseudo",
    "tunnel",
    "virtual",
    "miniport",
];

#[derive(Clone, Debug)]
pub struct Adapter {
    pub if_index: u32,
    pub name: String,
    pub description: String,
    pub if_type: u32,
    pub up: bool,
    pub ipv4: Vec<Ipv4Addr>,
}

impl Adapter {
    fn looks_virtual(&self) -> bool {
        let text = format!("{} {}", self.name, self.description).to_lowercase();
        VIRTUAL_NAMES.iter().any(|v| text.contains(v)) || matches!(self.if_type, IF_TYPE_TUNNEL | IF_TYPE_PPP | IF_TYPE_LOOPBACK)
    }

    fn is_physical(&self) -> bool {
        !self.looks_virtual() && matches!(self.if_type, IF_TYPE_ETHERNET | IF_TYPE_WIFI)
    }
}

/// RFC 1918 only. This is what the PC writes; the phone's parser also accepts `100.64.0.0/10`.
pub fn is_private(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

/// The shared address space the phone accepts (carrier-grade NAT, Tailscale). Used to check `--dev-qr-address`.
pub fn is_shared_space(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && (64..128).contains(&b)
}

/// Picks up to 3 `address:port` entries. `default_route_if` is the interface index that holds the default route.
pub fn select_addresses(adapters: &[Adapter], default_route_if: Option<u32>, port: u16) -> Vec<SocketAddrV4> {
    // (is the default route, is physical) for every candidate, in adapter order.
    let mut candidates: Vec<(bool, bool, Ipv4Addr)> = Vec::new();
    for a in adapters.iter().filter(|a| a.up) {
        let on_default_route = default_route_if == Some(a.if_index);
        if !on_default_route && a.looks_virtual() {
            continue;
        }
        for ip in a.ipv4.iter().copied().filter(|ip| is_private(*ip)) {
            if !candidates.iter().any(|c| c.2 == ip) {
                candidates.push((on_default_route, a.is_physical(), ip));
            }
        }
    }
    // The default route first, then physical adapters, then the rest. The sort is stable, so adapter order stays.
    candidates.sort_by_key(|(default, physical, _)| (!default, !physical));
    candidates.into_iter().take(MAX_ADDRESSES).map(|(_, _, ip)| SocketAddrV4::new(ip, port)).collect()
}

/// The adapters of this PC and the interface that holds the default route.
#[cfg(windows)]
pub fn list_adapters() -> crate::Result<(Vec<Adapter>, Option<u32>)> {
    use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
    use windows::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, GetBestInterface,
        IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size = 16 * 1024u32;
    // u64 elements keep the buffer 8-byte aligned, which IP_ADAPTER_ADDRESSES needs.
    let mut buf: Vec<u64>;
    loop {
        buf = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: `buf` is at least `size` bytes and aligned for IP_ADAPTER_ADDRESSES_LH.
        let ret = unsafe {
            GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH), &mut size)
        };
        if ret == ERROR_BUFFER_OVERFLOW.0 {
            continue;
        }
        if ret != ERROR_SUCCESS.0 {
            return Err(format!("GetAdaptersAddresses failed with {ret}").into());
        }
        break;
    }

    let mut adapters = Vec::new();
    // SAFETY: the list was filled in by GetAdaptersAddresses into `buf`, which outlives this loop. Every pointer
    // followed here is either null or points inside that buffer.
    unsafe {
        let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !p.is_null() {
            let a = &*p;
            let mut ipv4 = Vec::new();
            let mut u = a.FirstUnicastAddress;
            while !u.is_null() {
                let sa = (*u).Address.lpSockaddr;
                if !sa.is_null() && (*sa).sa_family == AF_INET {
                    let sin = &*(sa as *const SOCKADDR_IN);
                    ipv4.push(Ipv4Addr::from(sin.sin_addr.S_un.S_addr.to_ne_bytes()));
                }
                u = (*u).Next;
            }
            adapters.push(Adapter {
                if_index: a.Anonymous1.Anonymous.IfIndex,
                name: a.FriendlyName.to_string().unwrap_or_default(),
                description: a.Description.to_string().unwrap_or_default(),
                if_type: a.IfType,
                up: a.OperStatus == IfOperStatusUp,
                ipv4,
            });
            p = a.Next;
        }
    }

    // The interface the system would use to reach a public address: the one holding the default route. No packet is sent.
    let mut best = 0u32;
    // SAFETY: plain out parameter.
    let found = unsafe { GetBestInterface(u32::from_ne_bytes([8, 8, 8, 8]), &mut best) } == ERROR_SUCCESS.0;
    Ok((adapters, found.then_some(best)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(if_index: u32, name: &str, description: &str, if_type: u32, ip: &str) -> Adapter {
        Adapter { if_index, name: name.into(), description: description.into(), if_type, up: true, ipv4: vec![ip.parse().unwrap()] }
    }

    fn wifi(i: u32, ip: &str) -> Adapter {
        adapter(i, "Wi-Fi", "Intel(R) Wi-Fi 6 AX201", IF_TYPE_WIFI, ip)
    }

    fn strings(v: Vec<SocketAddrV4>) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn writes_private_addresses_of_real_adapters() {
        let adapters = [wifi(1, "192.168.1.20"), adapter(2, "Ethernet", "Realtek PCIe GbE", IF_TYPE_ETHERNET, "10.0.0.5")];
        assert_eq!(strings(select_addresses(&adapters, Some(1), 47821)), ["192.168.1.20:47821", "10.0.0.5:47821"]);
    }

    #[test]
    fn leaves_out_loopback_link_local_public_and_shared_space() {
        let adapters = [
            adapter(1, "Loopback", "Software Loopback Interface 1", IF_TYPE_LOOPBACK, "127.0.0.1"),
            adapter(2, "Ethernet 2", "Realtek", IF_TYPE_ETHERNET, "169.254.3.4"),
            adapter(3, "Ethernet 3", "Realtek", IF_TYPE_ETHERNET, "8.8.4.4"),
            adapter(4, "Ethernet 4", "Realtek", IF_TYPE_ETHERNET, "100.100.1.1"),
            wifi(5, "192.168.1.20"),
        ];
        assert_eq!(strings(select_addresses(&adapters, Some(5), 1)), ["192.168.1.20:1"]);
    }

    #[test]
    fn leaves_out_down_vpn_and_virtual_adapters() {
        let mut down = wifi(9, "192.168.9.9");
        down.up = false;
        let adapters = [
            down,
            adapter(2, "vEthernet (WSL)", "Hyper-V Virtual Ethernet Adapter", IF_TYPE_ETHERNET, "172.20.0.1"),
            adapter(3, "Tailscale", "Tailscale Tunnel", IF_TYPE_TUNNEL, "10.99.0.1"),
            adapter(4, "OpenVPN", "TAP-Windows Adapter V9", IF_TYPE_ETHERNET, "10.8.0.2"),
            adapter(5, "Ethernet", "VMware Virtual Ethernet Adapter for VMnet8", IF_TYPE_ETHERNET, "192.168.56.1"),
            wifi(6, "192.168.1.20"),
        ];
        assert_eq!(strings(select_addresses(&adapters, Some(6), 1)), ["192.168.1.20:1"]);
    }

    #[test]
    fn never_leaves_out_the_default_route_address_for_its_adapter() {
        // A Hyper-V external switch that is the PC's only network.
        let adapters = [adapter(7, "vEthernet (External)", "Hyper-V Virtual Ethernet Adapter", IF_TYPE_ETHERNET, "192.168.1.50")];
        assert_eq!(strings(select_addresses(&adapters, Some(7), 47821)), ["192.168.1.50:47821"]);
        // Without a default route it counts as virtual again.
        assert!(select_addresses(&adapters, None, 47821).is_empty());
        // But it still has to be a private address.
        let public = [adapter(7, "vEthernet (External)", "Hyper-V", IF_TYPE_ETHERNET, "203.0.113.5")];
        assert!(select_addresses(&public, Some(7), 1).is_empty());
    }

    #[test]
    fn more_than_three_keeps_the_default_route_and_prefers_physical_adapters() {
        let adapters = [
            adapter(1, "Docker", "Docker NAT", IF_TYPE_ETHERNET, "172.17.0.1"), // left out as virtual
            adapter(2, "vEthernet (Default Switch)", "Hyper-V", IF_TYPE_ETHERNET, "172.28.0.1"), // default route, virtual
            adapter(3, "Ethernet", "Realtek", IF_TYPE_ETHERNET, "10.0.0.5"),
            adapter(4, "Ethernet 2", "Intel", IF_TYPE_ETHERNET, "192.168.7.2"),
            wifi(5, "192.168.1.20"),
            adapter(6, "Ethernet 3", "Intel", IF_TYPE_ETHERNET, "10.1.1.1"),
        ];
        assert_eq!(strings(select_addresses(&adapters, Some(2), 9)), ["172.28.0.1:9", "10.0.0.5:9", "192.168.7.2:9"]);
    }

    #[test]
    fn several_addresses_on_one_adapter_and_duplicates() {
        let mut a = wifi(1, "192.168.1.20");
        a.ipv4.push("192.168.1.21".parse().unwrap());
        a.ipv4.push("192.168.1.20".parse().unwrap());
        assert_eq!(strings(select_addresses(&[a], Some(1), 5)), ["192.168.1.20:5", "192.168.1.21:5"]);
    }

    #[test]
    fn shared_space_range() {
        assert!(is_shared_space("100.64.0.1".parse().unwrap()));
        assert!(is_shared_space("100.127.255.255".parse().unwrap()));
        assert!(!is_shared_space("100.128.0.0".parse().unwrap()));
        assert!(!is_shared_space("100.63.255.255".parse().unwrap()));
    }

    /// Reads the real adapter list of this PC: no socket is opened.
    #[cfg(windows)]
    #[test]
    fn lists_the_real_adapters() {
        let (adapters, default_route) = list_adapters().unwrap();
        assert!(!adapters.is_empty());
        eprintln!("default route interface: {default_route:?}");
        for a in &adapters {
            eprintln!("  {} {:?} {:?} type {} up {} {:?}", a.if_index, a.name, a.description, a.if_type, a.up, a.ipv4);
        }
        eprintln!("would write: {:?}", select_addresses(&adapters, default_route, 47821));
    }
}
