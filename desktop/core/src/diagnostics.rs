//! What helps to find a problem, in one text file (the app's "Save
//! diagnostics", `pshb-cli diag`), and a watch on the board's counters of
//! refused packets ([`BoardHealth`]).
//!
//! The report holds versions, the system, the settings, the devices, what
//! the board says about itself and the recent log lines (crate::log). It
//! never holds the key: that lives in its own file, which is not read here.

use std::fmt::Write as _;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::{self, Config};
use crate::discover;
use crate::input;
use crate::protocol::{self, Pong};
use crate::update::BoardApi;

/// What the caller knows besides the settings.
pub struct ReportInput<'a> {
    pub config: &'a Config,
    /// "desktop app" or "pshb-cli".
    pub program: &'a str,
    /// The window backend's name, or why there is none.
    pub windows: String,
    /// The bridge service's last status (the app), as text.
    pub status: Option<String>,
    /// The key, to ask the board for its status and diagnostics (signed).
    pub key: Option<[u8; protocol::KEY_LEN]>,
    /// The board's web interface port (80; the simulator's differs).
    pub http_port: u16,
}

/// The report (Markdown-like plain text). Takes a few seconds: it asks the
/// board.
pub fn report(i: &ReportInput) -> String {
    let mut out = String::new();
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(out, "# PS HID Bridge diagnostics\n");
    let _ = writeln!(out, "- {} {}", i.program, crate::VERSION);
    let _ = writeln!(out, "- made at Unix time {secs}");
    let _ = writeln!(out, "- processor: {}", std::env::consts::ARCH);

    section(&mut out, "System");
    let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let pretty = os
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| "unknown".into());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let _ = writeln!(out, "- system: {pretty}");
    let _ = writeln!(out, "- kernel: {}", kernel.trim());
    let _ = writeln!(out, "- session: {}", env("XDG_SESSION_TYPE"));
    let _ = writeln!(out, "- desktop: {}", env("XDG_CURRENT_DESKTOP"));
    let _ = writeln!(out, "- windows: {}", i.windows);
    let _ = writeln!(
        out,
        "- device access rule: {}, latency rule: {}",
        yes_no(crate::access::rule_installed()),
        yes_no(crate::access::latency_rule_installed())
    );

    section(&mut out, "Settings");
    let _ = writeln!(
        out,
        "(The key is not part of the settings and not in this report.)\n"
    );
    let _ = writeln!(out, "```toml");
    out += &toml::to_string_pretty(i.config).unwrap_or_else(|e| format!("({e})"));
    let _ = writeln!(out, "```");
    let _ = writeln!(out, "- key file present: {}", yes_no(config::psk_path().exists()));

    section(&mut out, "Devices");
    match input::list_devices() {
        Ok(list) => {
            let selected = input::select(&list.devices, &i.config.devices);
            for d in list.devices.iter().filter(|d| d.keyboard || d.pointer()) {
                let _ = writeln!(
                    out,
                    "- {} {} [{}] {} ({}){}",
                    if selected.iter().any(|s| s.path == d.path) {
                        "*"
                    } else {
                        " "
                    },
                    d.path.display(),
                    d.id_string(),
                    d.name,
                    d.kind(),
                    if d.accessible { "" } else { ", no access" }
                );
            }
            let _ = writeln!(
                out,
                "\n(* = selected; {} devices without access)",
                list.inaccessible
            );
        }
        Err(e) => {
            let _ = writeln!(out, "- device list: {e}");
        }
    }

    section(&mut out, "Board");
    let _ = writeln!(out, "- address: {}", i.config.board);
    match discover::board_ipv4(&i.config.board) {
        Some(ip) => match discover::probe(ip, protocol::DISCOVERY_PORT, discover::DISCOVER_TIMEOUT) {
            Ok(Some(b)) => {
                let _ = writeln!(
                    out,
                    "- answers the search: {}, firmware {}, MAC {}, key set: {}, static address: {}",
                    b.name,
                    b.firmware(),
                    b.mac_string(),
                    yes_no(b.key_set),
                    yes_no(b.static_ip)
                );
            }
            Ok(None) => {
                let _ = writeln!(out, "- does not answer the search");
            }
            Err(e) => {
                let _ = writeln!(out, "- search: {e}");
            }
        },
        None => {
            let _ = writeln!(out, "- (no IPv4 address: not searched)");
        }
    }
    if let Some(key) = &i.key {
        let host = config::board_socket_addr(&i.config.board)
            .map(|a| a.ip().to_string())
            .unwrap_or_else(|_| i.config.board.clone());
        let api = BoardApi::new(&host, i.http_port, key);
        for (what, result) in [("status", api.status()), ("latency", api.diag())] {
            match result {
                Ok(v) => {
                    let _ = writeln!(out, "\nBoard {what}:\n```json");
                    out += &serde_json::to_string_pretty(&v).unwrap_or_default();
                    let _ = writeln!(out, "\n```");
                }
                Err(e) => {
                    let _ = writeln!(out, "- board {what}: {e}");
                }
            }
        }
    } else {
        let _ = writeln!(out, "- no key: the board was not asked for its status");
    }

    if let Some(st) = &i.status {
        section(&mut out, "Bridge");
        let _ = writeln!(out, "```\n{st}\n```");
    }

    section(&mut out, "Log");
    for line in crate::log::recent() {
        let _ = writeln!(out, "{line}");
    }
    out
}

fn section(out: &mut String, title: &str) {
    let _ = writeln!(out, "\n## {title}\n");
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// A rise in the board's counters of refused packets, while this PC sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthWarning {
    /// Packets with a wrong key (another PC with an old key, or a stranger).
    WrongKey,
    /// Packets repeated or too old: a second sender with the same key.
    Replayed,
}

/// Watches the counters in the board's PONGs. A board restart (new session)
/// starts them at 0; the first seconds after a (re)connect are not counted:
/// a key just changed may still meet packets sent with the old one.
#[derive(Debug, Default)]
pub struct BoardHealth {
    session: Option<u64>,
    auth_fail: u32,
    replay: u32,
    quiet_until: Option<Instant>,
}

/// Not counted after a (re)connect.
pub const HEALTH_QUIET: Duration = Duration::from_secs(5);

impl BoardHealth {
    /// Start over (service restart, other key).
    pub fn reset(&mut self, now: Instant) {
        self.session = None;
        self.quiet_until = Some(now + HEALTH_QUIET);
    }

    /// The latest PONG; a warning if a counter rose since the last one.
    pub fn update(&mut self, p: &Pong, now: Instant) -> Option<HealthWarning> {
        let quiet = self.quiet_until.is_some_and(|t| now < t);
        let fresh = self.session != Some(p.session_id);
        let warning = if fresh || quiet {
            None
        } else if p.rx_auth_fail > self.auth_fail {
            Some(HealthWarning::WrongKey)
        } else if p.rx_replay > self.replay {
            Some(HealthWarning::Replayed)
        } else {
            None
        };
        self.session = Some(p.session_id);
        self.auth_fail = p.rx_auth_fail;
        self.replay = p.rx_replay;
        warning
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pong(session_id: u64, auth: u32, replay: u32) -> Pong {
        Pong {
            session_id,
            rx_auth_fail: auth,
            rx_replay: replay,
            ..Default::default()
        }
    }

    #[test]
    fn warns_when_refused_packets_rise() {
        let t0 = Instant::now();
        let mut h = BoardHealth::default();
        h.reset(t0);
        // Counts before the connection and during the first seconds do not count.
        assert_eq!(h.update(&pong(1, 7, 3), t0), None);
        assert_eq!(h.update(&pong(1, 9, 3), t0 + Duration::from_secs(1)), None);
        let later = t0 + HEALTH_QUIET + Duration::from_secs(1);
        assert_eq!(h.update(&pong(1, 9, 3), later), None);
        assert_eq!(h.update(&pong(1, 10, 3), later), Some(HealthWarning::WrongKey));
        assert_eq!(h.update(&pong(1, 10, 3), later), None);
        assert_eq!(h.update(&pong(1, 10, 4), later), Some(HealthWarning::Replayed));
        // The board restarted: its counters start over.
        assert_eq!(h.update(&pong(2, 0, 0), later), None);
        assert_eq!(h.update(&pong(2, 1, 0), later), Some(HealthWarning::WrongKey));
    }

    #[test]
    fn the_report_never_holds_the_key() {
        let key = [0xabu8; protocol::KEY_LEN];
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        let config = Config {
            // No address: nothing is asked in the network.
            board: String::new(),
            ..Default::default()
        };
        let text = report(&ReportInput {
            config: &config,
            program: "test",
            windows: "none".into(),
            status: Some("connected: false".into()),
            key: Some(key),
            http_port: 9,
        });
        assert!(!text.contains(&hex));
        assert!(text.contains("## Settings") && text.contains("## Log") && text.contains("connected: false"));
    }
}
