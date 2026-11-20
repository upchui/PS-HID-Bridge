//! Finding the board again when it stops answering at its address: it got
//! another one (DHCP) without telling the app (older firmware, a lost MOVED
//! notice, see crate::transport), or the name it is set up under points
//! elsewhere now. The search runs on a thread of the
//! [`Link`](crate::transport::Link), which switches over without touching
//! the bridge (on stays on, nothing is let go of or grabbed anew).
//!
//! A search round asks for the board's MAC address (discovery; the first
//! matching answer ends the wait), then looks up its name. An address from
//! a name is only a candidate: DNS and mDNS caches may still give the old
//! address after the board got a new one. The old address is skipped (it
//! does not answer), any other must answer discovery with the board's MAC
//! address. After a switch the board has [`CONFIRM`] to answer an encrypted
//! ping there, else the search goes on.
//!
//! It searches without a pause, round after round, until the board answers
//! again. In the app the rounds look into the list of the network search
//! that runs all the time ([`crate::discover::Watcher`]); elsewhere (the
//! CLI) each round asks the network itself ([`NetFinder`]).

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::discover;
use crate::protocol::DISCOVERY_PORT;

/// How the board was found at its new address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveReason {
    /// It said so itself (MOVED, sent from the old address).
    Notice,
    /// It answered discovery with its MAC address.
    Discovery,
    /// Its name points there now (and it answered discovery there).
    Name,
}

impl MoveReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notice => "notice",
            Self::Discovery => "discovery",
            Self::Name => "name",
        }
    }
}

/// The board moved to `to`; `seq` counts the moves of a link (1, 2, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardMove {
    pub to: SocketAddr,
    pub reason: MoveReason,
    pub seq: u64,
}

/// No answer for this long: search (the app pings every second, so one
/// answer is missing), with the bridge on or off.
pub const LOST: Duration = Duration::from_millis(1500);
/// The name is looked up at most this often while the search goes on (each
/// lookup of a `.local` name may ask the network).
pub const NAME_EVERY: Duration = Duration::from_secs(1);
/// After a switch the board must answer there within this.
pub const CONFIRM: Duration = Duration::from_secs(3);

/// What the search asks the network (replaced in tests).
pub trait Finder: Send + Sync {
    /// The board with this MAC address among those answering discovery:
    /// its address and input port.
    fn find_mac(&self, mac: &str) -> Option<SocketAddr>;
    /// What answers discovery at `ip`: its MAC address and name.
    fn probe(&self, ip: Ipv4Addr) -> Option<(String, String)>;
    /// The addresses `host` resolves to now (maybe from a cache).
    fn resolve(&self, host: &str) -> Vec<IpAddr>;
}

/// The real network, asked anew each round: discovery to the broadcast
/// addresses of every IPv4 network of this PC (every address of the small
/// ones every [`discover::SWEEP_EVERY`]), or to `targets`; names through
/// the system's resolver.
pub struct NetFinder {
    /// Where discovery requests go; None = the networks of this PC.
    targets: Option<Vec<Ipv4Addr>>,
    port: u16,
    last_sweep: Mutex<Option<Instant>>,
}

impl NetFinder {
    pub fn new(targets: Option<Vec<Ipv4Addr>>, port: u16) -> Self {
        Self {
            targets,
            port,
            last_sweep: Mutex::new(None),
        }
    }
}

impl Default for NetFinder {
    fn default() -> Self {
        Self::new(None, DISCOVERY_PORT)
    }
}

impl Finder for NetFinder {
    fn find_mac(&self, mac: &str) -> Option<SocketAddr> {
        let targets = match &self.targets {
            Some(t) => t.clone(),
            None => {
                let nets = discover::ipv4_networks();
                let mut last = self.last_sweep.lock().unwrap();
                if last.is_none_or(|t| t.elapsed() >= discover::SWEEP_EVERY) {
                    *last = Some(Instant::now());
                    discover::targets(&nets)
                } else {
                    discover::broadcast_targets(&nets)
                }
            }
        };
        discover::find_mac(mac, &targets, self.port, discover::DISCOVER_TIMEOUT)
            .ok()
            .flatten()
            .map(|b| SocketAddr::new(IpAddr::V4(b.ip), b.udp_port))
    }

    fn probe(&self, ip: Ipv4Addr) -> Option<(String, String)> {
        discover::probe(ip, self.port, discover::DISCOVER_TIMEOUT)
            .ok()
            .flatten()
            .map(|b| (b.mac_string(), b.name))
    }

    fn resolve(&self, host: &str) -> Vec<IpAddr> {
        (host, 0)
            .to_socket_addrs()
            .map(|a| a.map(|a| a.ip()).collect())
            .unwrap_or_default()
    }
}

/// What the link's search knows.
#[derive(Clone)]
pub struct FollowConfig {
    /// A host name to look up ("ps-hid-bridge.local"): the board setting if
    /// it is one, or the board's network name.
    pub name: Option<String>,
    /// The board's MAC address ("02:68:9d:96:c3:3d"); None: the link learns
    /// it from the board's discovery answer once it answers.
    pub mac: Option<String>,
    pub finder: Arc<dyn Finder>,
}

impl FollowConfig {
    pub fn new(name: Option<String>, mac: Option<String>) -> Self {
        Self {
            name,
            mac,
            finder: Arc::new(NetFinder::default()),
        }
    }
}

/// The host name in a board setting ("name.local", "name.local:45050"),
/// None for an IP address.
pub fn host_name(board: &str) -> Option<String> {
    let bare = board
        .strip_prefix('[')
        .and_then(|b| b.strip_suffix(']'))
        .unwrap_or(board);
    if bare.parse::<IpAddr>().is_ok() || board.parse::<SocketAddr>().is_ok() || board.is_empty() {
        return None;
    }
    let host = match board.rsplit_once(':') {
        Some((h, port)) if port.parse::<u16>().is_ok() => h,
        _ => board,
    };
    Some(host.to_string())
}

/// One search round: where the board is now, if not at `current`.
pub fn search(
    finder: &dyn Finder,
    current: SocketAddr,
    mac: Option<&str>,
    name: Option<&str>,
) -> Option<(SocketAddr, MoveReason)> {
    // The socket is bound to the family of the first address.
    let same_family = |a: &IpAddr| a.is_ipv4() == current.is_ipv4();
    if let Some(mac) = mac
        && let Some(found) = finder.find_mac(mac)
        && same_family(&found.ip())
    {
        // At the same address it is there and only did not answer: nothing
        // to move (the name would not know better).
        return (found != current).then_some((found, MoveReason::Discovery));
    }
    let name = name?;
    for ip in finder.resolve(name) {
        // The old address (a cache may still give it) does not answer.
        if ip == current.ip() || !same_family(&ip) {
            continue;
        }
        let IpAddr::V4(v4) = ip else { continue };
        let answers = match (finder.probe(v4), mac) {
            (Some((found, _)), Some(mac)) => found.eq_ignore_ascii_case(mac),
            // Not known yet: a board answers there; the encrypted ping decides.
            (Some(_), None) => true,
            (None, _) => false,
        };
        if answers {
            return Some((SocketAddr::new(ip, current.port()), MoveReason::Name));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const MAC: &str = "02:68:9d:96:c3:3d";
    const OTHER_MAC: &str = "02:00:00:00:00:01";

    /// A network: which board answers discovery where, what names resolve to.
    #[derive(Default)]
    struct FakeNet {
        boards: HashMap<Ipv4Addr, &'static str>,
        names: HashMap<&'static str, Vec<IpAddr>>,
        /// Discovery broadcasts do not get through (firewall).
        no_broadcast: bool,
    }

    impl Finder for FakeNet {
        fn find_mac(&self, mac: &str) -> Option<SocketAddr> {
            if self.no_broadcast {
                return None;
            }
            self.boards
                .iter()
                .find(|(_, m)| m.eq_ignore_ascii_case(mac))
                .map(|(ip, _)| SocketAddr::new(IpAddr::V4(*ip), 45045))
        }
        fn probe(&self, ip: Ipv4Addr) -> Option<(String, String)> {
            self.boards.get(&ip).map(|m| (m.to_string(), "board".into()))
        }
        fn resolve(&self, host: &str) -> Vec<IpAddr> {
            self.names.get(host).cloned().unwrap_or_default()
        }
    }

    fn ip(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 1, last)
    }
    fn at(last: u8) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(ip(last)), 45045)
    }

    #[test]
    fn found_by_its_mac_address() {
        let mut net = FakeNet::default();
        net.boards.insert(ip(60), MAC);
        net.boards.insert(ip(70), OTHER_MAC);
        assert_eq!(
            search(&net, at(50), Some(MAC), None),
            Some((at(60), MoveReason::Discovery))
        );
        // Still at its address (it only missed a ping): no move.
        assert_eq!(search(&net, at(60), Some(MAC), None), None);
        // Gone: nothing.
        net.boards.remove(&ip(60));
        assert_eq!(search(&net, at(50), Some(MAC), None), None);
    }

    #[test]
    fn a_name_with_the_old_address_from_a_cache_is_skipped() {
        let mut net = FakeNet::default();
        net.boards.insert(ip(60), MAC);
        // The cache still says .50 (the old address).
        net.names.insert("board.local", vec![IpAddr::V4(ip(50))]);
        net.no_broadcast = true;
        assert_eq!(search(&net, at(50), Some(MAC), Some("board.local")), None);
        // With discovery the board is found anyway.
        net.no_broadcast = false;
        assert_eq!(
            search(&net, at(50), Some(MAC), Some("board.local")),
            Some((at(60), MoveReason::Discovery))
        );
    }

    #[test]
    fn a_name_counts_only_where_the_board_answers() {
        let mut net = FakeNet {
            no_broadcast: true,
            ..Default::default()
        };
        net.boards.insert(ip(60), MAC);
        net.boards.insert(ip(70), OTHER_MAC);
        // Old address first (cache), then another board, then the board.
        net.names.insert(
            "board.local",
            vec![IpAddr::V4(ip(50)), IpAddr::V4(ip(70)), IpAddr::V4(ip(60))],
        );
        assert_eq!(
            search(&net, at(50), Some(MAC), Some("board.local")),
            Some((at(60), MoveReason::Name))
        );
        // Only another board there: no move.
        net.names.insert("board.local", vec![IpAddr::V4(ip(70))]);
        assert_eq!(search(&net, at(50), Some(MAC), Some("board.local")), None);
        // MAC address not known yet: any board there is a candidate (the
        // encrypted ping decides).
        assert_eq!(
            search(&net, at(50), None, Some("board.local")),
            Some((at(70), MoveReason::Name))
        );
        // Nothing answers there.
        net.names.insert("board.local", vec![IpAddr::V4(ip(80))]);
        assert_eq!(search(&net, at(50), Some(MAC), Some("board.local")), None);
    }

    #[test]
    fn the_port_set_by_hand_stays_with_a_name() {
        let mut net = FakeNet {
            no_broadcast: true,
            ..Default::default()
        };
        net.boards.insert(ip(60), MAC);
        net.names.insert("board.local", vec![IpAddr::V4(ip(60))]);
        let current = SocketAddr::new(IpAddr::V4(ip(50)), 45050);
        assert_eq!(
            search(&net, current, Some(MAC), Some("board.local")),
            Some((SocketAddr::new(IpAddr::V4(ip(60)), 45050), MoveReason::Name))
        );
    }

    #[test]
    fn host_names_in_board_settings() {
        assert_eq!(host_name("192.168.1.50"), None);
        assert_eq!(host_name("192.168.1.50:45050"), None);
        assert_eq!(host_name("[fe80::1]:45045"), None);
        assert_eq!(host_name("fe80::1"), None);
        assert_eq!(host_name(""), None);
        assert_eq!(host_name("board.local").as_deref(), Some("board.local"));
        assert_eq!(host_name("board.local:45050").as_deref(), Some("board.local"));
    }
}
