//! Read access to the selected input devices via a udev rule.
//!
//! The rule tags the chosen devices (by USB vendor/product ID) with
//! `uaccess`, so logind grants the logged-in user access. It must sort before
//! 73-seat-late.rules, hence the "70-" prefix.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::input::DeviceInfo;

pub const RULE_PATH: &str = "/etc/udev/rules.d/70-ps5-hid-bridge.rules";

/// Rule text for the given devices (one line per vendor:product pair).
pub fn udev_rule(devices: &[DeviceInfo]) -> String {
    let mut ids: Vec<(u16, u16, String)> = devices
        .iter()
        .map(|d| (d.vendor, d.product, d.name.clone()))
        .collect();
    ids.sort();
    ids.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    let mut out = String::from(
        "# PS5 HID Bridge: read access to the selected keyboards and mice\n\
         # (written by the app; remove this file to revoke)\n",
    );
    for (v, p, name) in ids {
        let name: String = name.chars().filter(|c| !c.is_control() && *c != '"').collect();
        out += &format!(
            "# {name}\nSUBSYSTEM==\"input\", KERNEL==\"event*\", ATTRS{{id/vendor}}==\"{v:04x}\", ATTRS{{id/product}}==\"{p:04x}\", TAG+=\"uaccess\"\n"
        );
    }
    out
}

/// Install the rule as root (polkit asks for the password) and apply it.
/// `pkexec` runs /bin/sh from the host: inside an AppImage our own files live
/// on a FUSE mount that root may not be able to read, so the rule is passed
/// on stdin instead of as a file.
pub fn install_rule(rule: &str) -> Result<(), String> {
    let script = format!(
        "set -e; cat > {RULE_PATH}.tmp && mv {RULE_PATH}.tmp {RULE_PATH} && \
         udevadm control --reload && udevadm trigger --subsystem-match=input --action=change"
    );
    let mut child = Command::new("pkexec")
        .args(["/bin/sh", "-c", &script])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("pkexec konnte nicht gestartet werden: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("stdin")?
        .write_all(rule.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    match out.status.code() {
        Some(0) => Ok(()),
        Some(126) | Some(127) => Err("Abgebrochen oder keine Berechtigung (polkit).".into()),
        _ => Err(format!(
            "Einrichten fehlgeschlagen: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dev(v: u16, p: u16, name: &str) -> DeviceInfo {
        DeviceInfo {
            path: PathBuf::from("/dev/input/event1"),
            name: name.into(),
            phys: String::new(),
            uniq: String::new(),
            vendor: v,
            product: p,
            keyboard: true,
            mouse: false,
            accessible: false,
        }
    }

    #[test]
    fn one_line_per_device_id() {
        let rule = udev_rule(&[
            dev(0x1532, 0x00ab, "Razer Basilisk"),
            dev(0x1532, 0x00ab, "Razer Basilisk Keyboard"),
            dev(0x046d, 0xc52b, "Logitech \"K400\""),
        ]);
        assert_eq!(rule.matches("TAG+=\"uaccess\"").count(), 2);
        assert!(rule.contains("ATTRS{id/vendor}==\"046d\", ATTRS{id/product}==\"c52b\""));
        assert!(rule.contains("# Logitech K400\n"));
    }
}
