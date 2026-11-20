//! Finding boards in the local network (protocol/SPEC.md "Discovery"): a
//! 64-byte request to UDP port 45046, unencrypted, answered by every board
//! with its name, MAC address, firmware version and input port.
//!
//! The request goes to the broadcast addresses and, in networks of up to
//! [`SWEEP_MAX`] addresses, to every address once more: a firewall on this PC
//! (ufw, firewalld) drops answers to a broadcast, which it cannot match to a
//! request it let out, but lets answers to a request to that address in.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use crate::protocol::{DISCOVERY_PORT, UDP_PORT_DEFAULT};
use crate::update::Version;

pub const DISCOVERY_LEN: usize = 64;
const MAGIC: &[u8; 8] = b"PSHBDISC";
const PROTO_VERSION: u8 = 1;
const REQUEST: u8 = 1;
const ANSWER: u8 = 2;
const FLAG_KEY_SET: u8 = 0x01;
const FLAG_STATIC_IP: u8 = 0x02;
const FLAG_NO_PARTITION_TABLE: u8 = 0x04;
const NAME_OFFSET: usize = 24;
const NAME_LEN: usize = 32;

/// How long to listen for answers.
pub const DISCOVER_TIMEOUT: Duration = Duration::from_millis(1200);
/// Networks with at most this many addresses also get one request per
/// address (a /23). Larger ones only the broadcast: the kernel's neighbour
/// table holds about a thousand entries.
pub const SWEEP_MAX: u32 = 512;
/// The sweep's requests go out within this part of the time to listen.
const SEND_BUDGET: Duration = Duration::from_millis(400);

/// A board that answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundBoard {
    pub ip: Ipv4Addr,
    /// "ps-hid-bridge-96c33d"
    pub name: String,
    pub mac: [u8; 6],
    /// 0xMMmmpppp, as in a PONG
    pub fw_version: u32,
    /// Port of the input socket (the app's address carries it if not 45045).
    pub udp_port: u16,
    /// A key is set on the board (the app can connect once it has it too).
    pub key_set: bool,
    pub static_ip: bool,
    pub no_partition_table: bool,
}

impl FoundBoard {
    /// "02:68:9d:96:c3:3d"
    pub fn mac_string(&self) -> String {
        format_mac(&self.mac)
    }

    /// The board address for the settings: "10.14.0.21", with the port
    /// only if the board does not use the standard one.
    pub fn address(&self) -> String {
        if self.udp_port == UDP_PORT_DEFAULT {
            self.ip.to_string()
        } else {
            format!("{}:{}", self.ip, self.udp_port)
        }
    }

    pub fn firmware(&self) -> Version {
        Version::of_firmware(self.fw_version)
    }

    /// The board setting for this board, found under a new address while
    /// `old` was set: a port given there by hand stays.
    pub fn moved_from(&self, old: &str) -> String {
        match old.parse::<SocketAddr>() {
            Ok(a) => format!("{}:{}", self.ip, a.port()),
            Err(_) => self.address(),
        }
    }
}

/// The network name the board derives from its name (the same rules as
/// firmware/src/core/board_name.c): lower-case letters and digits, ä ö ü ß
/// as ae oe ue ss, everything else one "-", no "-" at either end, at most 31
/// characters; `fallback` if nothing is left.
pub fn hostname_from_name(name: &str, fallback: &str) -> String {
    const MAX: usize = 31;
    let mut out = String::new();
    let mut dash = false;
    for ch in name.chars() {
        let put = match ch {
            'a'..='z' | '0'..='9' => Some(ch.to_string()),
            'A'..='Z' => Some(ch.to_ascii_lowercase().to_string()),
            'ä' | 'Ä' => Some("ae".to_string()),
            'ö' | 'Ö' => Some("oe".to_string()),
            'ü' | 'Ü' => Some("ue".to_string()),
            'ß' => Some("ss".to_string()),
            _ => None,
        };
        let Some(put) = put else {
            dash = !out.is_empty();
            continue;
        };
        let need = put.len() + usize::from(dash);
        if out.len() + need > MAX {
            break;
        }
        if dash {
            out.push('-');
        }
        dash = false;
        out.push_str(&put);
    }
    if out.is_empty() { fallback.to_string() } else { out }
}

/// The IPv4 address in a board setting ("10.14.0.21", "10.14.0.21:45045");
/// None for a host name or IPv6.
pub fn board_ipv4(board: &str) -> Option<Ipv4Addr> {
    match board.trim().parse::<SocketAddr>() {
        Ok(SocketAddr::V4(a)) => Some(*a.ip()),
        Ok(_) => None,
        Err(_) => board.trim().parse().ok(),
    }
}

/// "02:68:9d:96:c3:3d" (lower case, as stored in the settings).
pub fn format_mac(mac: &[u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The request every board answers.
pub fn request() -> [u8; DISCOVERY_LEN] {
    let mut r = [0u8; DISCOVERY_LEN];
    r[..8].copy_from_slice(MAGIC);
    r[8] = PROTO_VERSION;
    r[9] = REQUEST;
    r
}

/// A board's answer from `ip`; None for anything else.
pub fn parse_answer(ip: Ipv4Addr, data: &[u8]) -> Option<FoundBoard> {
    if data.len() != DISCOVERY_LEN || &data[..8] != MAGIC || data[8] != PROTO_VERSION || data[9] != ANSWER {
        return None;
    }
    let flags = data[10];
    let name_bytes = &data[NAME_OFFSET..NAME_OFFSET + NAME_LEN];
    let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
    let name: String = String::from_utf8_lossy(&name_bytes[..end])
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    Some(FoundBoard {
        ip,
        name,
        mac: data[12..18].try_into().ok()?,
        udp_port: u16::from_le_bytes([data[18], data[19]]),
        fw_version: u32::from_le_bytes(data[20..24].try_into().ok()?),
        key_set: flags & FLAG_KEY_SET != 0,
        static_ip: flags & FLAG_STATIC_IP != 0,
        no_partition_table: flags & FLAG_NO_PARTITION_TABLE != 0,
    })
}

/// An IPv4 address of this PC and its network mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Net {
    pub addr: Ipv4Addr,
    pub mask: Ipv4Addr,
}

impl Ipv4Net {
    pub fn prefix_len(&self) -> u32 {
        u32::from(self.mask).leading_ones()
    }

    /// The network's broadcast address; None for /31 and /32 (no broadcast).
    pub fn broadcast(&self) -> Option<Ipv4Addr> {
        (self.prefix_len() < 31).then(|| Ipv4Addr::from(u32::from(self.addr) | !u32::from(self.mask)))
    }

    /// Every address a board may have in this network (without the network
    /// and broadcast address); empty if the network has more than `max`.
    pub fn hosts(&self, max: u32) -> Vec<Ipv4Addr> {
        let prefix = self.prefix_len();
        if prefix == 0 || prefix >= 32 {
            return Vec::new();
        }
        let size = 1u64 << (32 - prefix);
        if size > u64::from(max) {
            return Vec::new();
        }
        let net = u32::from(self.addr) & u32::from(self.mask);
        if prefix == 31 {
            return vec![Ipv4Addr::from(net), Ipv4Addr::from(net + 1)];
        }
        (1..size as u32 - 1).map(|i| Ipv4Addr::from(net + i)).collect()
    }
}

/// The IPv4 networks of this PC's interfaces that are up (not loopback).
pub fn ipv4_networks() -> Vec<Ipv4Net> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list`, freed below with freeifaddrs.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return Vec::new();
    }
    let mut nets = Vec::new();
    let mut cur = list;
    while !cur.is_null() {
        // SAFETY: `cur` is an element of the list getifaddrs returned.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        let up = ifa.ifa_flags & libc::IFF_UP as u32 != 0;
        let loopback = ifa.ifa_flags & libc::IFF_LOOPBACK as u32 != 0;
        if !up || loopback || ifa.ifa_addr.is_null() || ifa.ifa_netmask.is_null() {
            continue;
        }
        // SAFETY: ifa_addr and ifa_netmask point to sockaddrs of the family
        // ifa_addr names; for AF_INET both are sockaddr_in.
        unsafe {
            if i32::from((*ifa.ifa_addr).sa_family) != libc::AF_INET {
                continue;
            }
            let addr = &*(ifa.ifa_addr as *const libc::sockaddr_in);
            let mask = &*(ifa.ifa_netmask as *const libc::sockaddr_in);
            nets.push(Ipv4Net {
                addr: Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr)),
                mask: Ipv4Addr::from(u32::from_be(mask.sin_addr.s_addr)),
            });
        }
    }
    // SAFETY: the list came from getifaddrs and is not used afterwards.
    unsafe { libc::freeifaddrs(list) };
    nets
}

/// Where the request goes for these networks: the general and each
/// network's broadcast address, then every address of the small networks.
pub fn targets(nets: &[Ipv4Net]) -> Vec<Ipv4Addr> {
    let mut broadcast = vec![Ipv4Addr::BROADCAST];
    broadcast.extend(nets.iter().filter_map(Ipv4Net::broadcast));
    broadcast.dedup();
    let mut sweep: Vec<Ipv4Addr> = nets
        .iter()
        .flat_map(|n| n.hosts(SWEEP_MAX))
        .filter(|a| !broadcast.contains(a))
        .collect();
    sweep.sort();
    sweep.dedup();
    broadcast.extend(sweep);
    broadcast
}

/// All boards in the networks of this PC, sorted by name and address.
pub fn discover(timeout: Duration) -> io::Result<Vec<FoundBoard>> {
    discover_at(&targets(&ipv4_networks()), DISCOVERY_PORT, timeout)
}

/// The boards that answer a request to `targets` (port `port`) within
/// `timeout`; each board once (by MAC address).
pub fn discover_at(targets: &[Ipv4Addr], port: u16, timeout: Duration) -> io::Result<Vec<FoundBoard>> {
    let mut found = ask(targets, port, timeout, |_| false)?;
    found.sort_by(|a: &FoundBoard, b| a.name.cmp(&b.name).then(a.ip.cmp(&b.ip)));
    Ok(found)
}

/// The board with this MAC address ("02:68:9d:96:c3:3d") among those that
/// answer a request to `targets`: as soon as it answers, None after
/// `timeout` (crate::follow, while the bridge waits for it).
pub fn find_mac(
    mac: &str,
    targets: &[Ipv4Addr],
    port: u16,
    timeout: Duration,
) -> io::Result<Option<FoundBoard>> {
    let is_it = |b: &FoundBoard| b.mac_string().eq_ignore_ascii_case(mac);
    let found = ask(targets, port, timeout, |found| found.iter().any(is_it))?;
    Ok(found.into_iter().find(is_it))
}

/// Send the request to `targets` and collect the answers until `timeout`
/// or until `enough` says so.
fn ask(
    targets: &[Ipv4Addr],
    port: u16,
    timeout: Duration,
    mut enough: impl FnMut(&[FoundBoard]) -> bool,
) -> io::Result<Vec<FoundBoard>> {
    let start = Instant::now();
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.set_broadcast(true)?;
    // A full send queue (requests waiting for an ARP answer) must not hold
    // up the sweep for long.
    sock.set_write_timeout(Some(Duration::from_millis(20)))?;
    let req = request();
    let mut found = Vec::new();
    let mut buf = [0u8; 512];
    for (i, &t) in targets.iter().enumerate() {
        // Errors (no route, no neighbour, buffer full) only cost that address.
        let _ = sock.send_to(&req, SocketAddrV4::new(t, port));
        if i % 64 == 63 {
            // Take in what arrived meanwhile: the receive buffer is limited.
            drain(&sock, &mut buf, &mut found)?;
            if enough(&found) {
                return Ok(found);
            }
            if start.elapsed() > SEND_BUDGET.min(timeout) {
                break;
            }
        }
    }
    let deadline = start + timeout;
    while !enough(&found) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        sock.set_read_timeout(Some(left))?;
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => add(&mut found, from, &buf[..n]),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => break,
            // ICMP errors from swept addresses (port unreachable) end up here.
            Err(_) => continue,
        }
    }
    Ok(found)
}

/// The board at `ip`, if it answers within `timeout` (asked twice, in case
/// a datagram gets lost).
pub fn probe(ip: Ipv4Addr, port: u16, timeout: Duration) -> io::Result<Option<FoundBoard>> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.connect(SocketAddrV4::new(ip, port))?;
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 512];
    for attempt in 0..2 {
        let _ = sock.send(&request());
        let until = if attempt == 0 {
            Instant::now() + timeout / 2
        } else {
            deadline
        };
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            sock.set_read_timeout(Some(left))?;
            match sock.recv(&mut buf) {
                Ok(n) => {
                    if let Some(b) = parse_answer(ip, &buf[..n]) {
                        return Ok(Some(b));
                    }
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => break,
                // Port unreachable: nothing listens there; ask once more.
                Err(_) => break,
            }
        }
    }
    Ok(None)
}

fn drain(sock: &UdpSocket, buf: &mut [u8], found: &mut Vec<FoundBoard>) -> io::Result<()> {
    sock.set_nonblocking(true)?;
    loop {
        match sock.recv_from(buf) {
            Ok((n, from)) => add(found, from, &buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => continue,
        }
    }
    sock.set_nonblocking(false)
}

fn add(found: &mut Vec<FoundBoard>, from: SocketAddr, data: &[u8]) {
    let IpAddr::V4(ip) = from.ip() else { return };
    if let Some(b) = parse_answer(ip, data)
        && !found.iter().any(|f| f.mac == b.mac)
    {
        found.push(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The answer of firmware/tests/test_discovery.c.
    fn answer() -> [u8; DISCOVERY_LEN] {
        let mut a = [0u8; DISCOVERY_LEN];
        let head = [
            b'P', b'S', b'H', b'B', b'D', b'I', b'S', b'C', 0x01, 0x02, 0x01, 0x00, 0x02, 0x68, 0x9d, 0x96,
            0xc3, 0x3d, 0xf5, 0xaf, 0x42, 0x00, 0x00, 0x01,
        ];
        a[..24].copy_from_slice(&head);
        a[24..44].copy_from_slice(b"ps-hid-bridge-96c33d");
        a
    }

    #[test]
    fn request_layout() {
        let r = request();
        assert_eq!(&r[..10], b"PSHBDISC\x01\x01");
        assert!(r[10..].iter().all(|&b| b == 0));
    }

    #[test]
    fn answer_of_the_firmware() {
        let ip = Ipv4Addr::new(10, 14, 0, 21);
        let b = parse_answer(ip, &answer()).unwrap();
        assert_eq!(b.name, "ps-hid-bridge-96c33d");
        assert_eq!(b.mac_string(), "02:68:9d:96:c3:3d");
        assert_eq!(b.udp_port, 45045);
        assert_eq!(b.firmware(), Version(1, 0, 66));
        assert!(b.key_set && !b.static_ip && !b.no_partition_table);
        assert_eq!(b.address(), "10.14.0.21");
        let other_port = FoundBoard { udp_port: 4000, ..b };
        assert_eq!(other_port.address(), "10.14.0.21:4000");
    }

    #[test]
    fn network_names_as_the_firmware() {
        // The cases of firmware/tests/test_board_name.c.
        let host = |n: &str| hostname_from_name(n, "ps-hid-bridge-96c33d");
        assert_eq!(host("Wohnzimmer PS5"), "wohnzimmer-ps5");
        assert_eq!(host("Küche Ärger Straße"), "kueche-aerger-strasse");
        assert_eq!(host("  --Hello__World!!  "), "hello-world");
        assert_eq!(host("🎮 PS5"), "ps5");
        assert_eq!(host("café"), "caf");
        assert_eq!(host(""), "ps-hid-bridge-96c33d");
        assert_eq!(host("!!! ???"), "ps-hid-bridge-96c33d");
        assert_eq!(host("🎮"), "ps-hid-bridge-96c33d");
        assert_eq!(
            host("abcdefghijklmnopqrstuvwxyz abcd efgh"),
            "abcdefghijklmnopqrstuvwxyz-abcd"
        );
        assert_eq!(
            host("abcdefghijklmnopqrstuvwxyzabcd x"),
            "abcdefghijklmnopqrstuvwxyzabcd"
        );
        assert_eq!(
            host("abcdefghijklmnopqrstuvwxyzabcdä"),
            "abcdefghijklmnopqrstuvwxyzabcd"
        );
    }

    #[test]
    fn settings_addresses() {
        assert_eq!(board_ipv4("10.14.0.21"), Some(Ipv4Addr::new(10, 14, 0, 21)));
        assert_eq!(board_ipv4("10.14.0.21:4000"), Some(Ipv4Addr::new(10, 14, 0, 21)));
        assert_eq!(board_ipv4("ps-hid-bridge-96c33d"), None);
        assert_eq!(board_ipv4("[fe80::1]:45045"), None);
        assert_eq!(board_ipv4(""), None);

        let b = parse_answer(Ipv4Addr::new(10, 14, 0, 30), &answer()).unwrap();
        assert_eq!(b.moved_from("10.14.0.21"), "10.14.0.30");
        assert_eq!(b.moved_from("10.14.0.21:4000"), "10.14.0.30:4000");
    }

    #[test]
    fn anything_else_is_no_answer() {
        let ip = Ipv4Addr::LOCALHOST;
        let a = answer();
        assert!(parse_answer(ip, &a[..63]).is_none());
        assert!(parse_answer(ip, &request()).is_none()); // a request, e.g. our own broadcast
        for (i, v) in [(0, b'X'), (8, 2), (9, 3)] {
            let mut bad = a;
            bad[i] = v;
            assert!(parse_answer(ip, &bad).is_none(), "byte {i}");
        }
        let mut long_name = a;
        long_name[24..56].fill(b'x');
        assert_eq!(parse_answer(ip, &long_name).unwrap().name.len(), 32);
    }

    #[test]
    fn networks_to_sweep() {
        let net = |a: [u8; 4], p: u32| Ipv4Net {
            addr: Ipv4Addr::from(a),
            mask: Ipv4Addr::from(if p == 0 { 0 } else { u32::MAX << (32 - p) }),
        };
        let lan = net([192, 168, 1, 17], 24);
        let hosts = lan.hosts(SWEEP_MAX);
        assert_eq!(hosts.len(), 254);
        assert_eq!(hosts[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(hosts[253], Ipv4Addr::new(192, 168, 1, 254));
        assert_eq!(lan.broadcast(), Some(Ipv4Addr::new(192, 168, 1, 255)));
        assert_eq!(net([10, 0, 0, 5], 23).hosts(SWEEP_MAX).len(), 510);
        assert!(net([172, 17, 0, 1], 16).hosts(SWEEP_MAX).is_empty());
        assert_eq!(net([10, 0, 0, 5], 31).hosts(SWEEP_MAX).len(), 2);
        assert!(net([10, 8, 0, 2], 32).hosts(SWEEP_MAX).is_empty());
        assert_eq!(net([10, 8, 0, 2], 32).broadcast(), None);

        let t = targets(&[lan, net([172, 17, 0, 1], 16)]);
        assert_eq!(
            &t[..3],
            &[
                Ipv4Addr::BROADCAST,
                Ipv4Addr::new(192, 168, 1, 255),
                Ipv4Addr::new(172, 17, 255, 255)
            ]
        );
        assert_eq!(t.len(), 3 + 254);
    }

    /// A board on this PC: answers every request with `answer()`.
    fn fake_board() -> u16 {
        let sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = sock.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut buf = [0u8; 128];
            while let Ok((n, from)) = sock.recv_from(&mut buf) {
                if buf[..n] == request() {
                    let _ = sock.send_to(&answer(), from);
                }
            }
        });
        port
    }

    #[test]
    fn finds_each_board_once() {
        let port = fake_board();
        // Asked twice (broadcast and sweep): listed once.
        let found = discover_at(
            &[Ipv4Addr::LOCALHOST, Ipv4Addr::LOCALHOST],
            port,
            Duration::from_millis(300),
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ip, Ipv4Addr::LOCALHOST);
        assert_eq!(found[0].name, "ps-hid-bridge-96c33d");

        let b = probe(Ipv4Addr::LOCALHOST, port, Duration::from_millis(300)).unwrap();
        assert_eq!(b.map(|b| b.mac_string()), Some("02:68:9d:96:c3:3d".into()));
    }

    #[test]
    fn find_mac_returns_as_soon_as_the_board_answers() {
        let port = fake_board();
        let t = Instant::now();
        let b = find_mac(
            "02:68:9D:96:C3:3D",
            &[Ipv4Addr::LOCALHOST],
            port,
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(b.map(|b| b.ip), Some(Ipv4Addr::LOCALHOST));
        assert!(t.elapsed() < Duration::from_millis(500), "{:?}", t.elapsed());
        // Another board's MAC address: nothing, after the timeout.
        let other = find_mac(
            "02:00:00:00:00:01",
            &[Ipv4Addr::LOCALHOST],
            port,
            Duration::from_millis(200),
        );
        assert_eq!(other.unwrap(), None);
    }

    #[test]
    fn probe_without_board() {
        // Nothing listens on this port: no answer, no error.
        let free = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        assert_eq!(
            probe(Ipv4Addr::LOCALHOST, port, Duration::from_millis(100)).unwrap(),
            None
        );
    }
}
