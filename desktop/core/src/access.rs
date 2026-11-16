//! Read access to the input devices via a udev rule.
//!
//! The rule tags the chosen devices (by USB vendor/product ID) and every
//! keyboard (triggers and the emergency stop work on all of them, see
//! [`crate::input::watch_keyboards`]) with `uaccess`, so logind grants the
//! logged-in user access. It must sort before 73-seat-late.rules, hence the
//! "70-" prefix; ID_INPUT_KEYBOARD comes from 60-input-id.rules.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::input::DeviceInfo;

pub const RULE_PATH: &str = "/etc/udev/rules.d/70-ps-hid-bridge.rules";
/// Rule file of the old name (PS5 HID Bridge), replaced by RULE_PATH.
const OLD_RULE_PATH: &str = "/etc/udev/rules.d/70-ps5-hid-bridge.rules";

/// Every keyboard: only watched for triggers and the emergency stop unless
/// it is selected.
const ALL_KEYBOARDS: &str = "# Every keyboard: triggers (e.g. Ctrl+Alt+P) and the emergency stop work on all of them\n\
     SUBSYSTEM==\"input\", KERNEL==\"event*\", ENV{ID_INPUT_KEYBOARD}==\"1\", TAG+=\"uaccess\"\n";

/// Rule text for the given devices (one line per vendor:product pair) and
/// every keyboard.
pub fn udev_rule(devices: &[DeviceInfo]) -> String {
    let mut ids: Vec<(u16, u16, String)> = devices
        .iter()
        .map(|d| (d.vendor, d.product, d.name.clone()))
        .collect();
    ids.sort();
    ids.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    let mut out = String::from(
        "# PS HID Bridge: read access to the selected keyboards and mice, and to every keyboard\n\
         # (written by the app; remove this file to revoke)\n",
    );
    out += ALL_KEYBOARDS;
    for (v, p, name) in ids {
        let name: String = name.chars().filter(|c| !c.is_control() && *c != '"').collect();
        out += &format!(
            "# {name}\nSUBSYSTEM==\"input\", KERNEL==\"event*\", ATTRS{{id/vendor}}==\"{v:04x}\", ATTRS{{id/product}}==\"{p:04x}\", TAG+=\"uaccess\"\n"
        );
    }
    out
}

/// Install the rule as root (polkit asks for the password) and apply it.
pub fn install_rule(rule: &str) -> Result<(), String> {
    // Also removes the rule file of the old name (PS5 HID Bridge).
    let script = format!(
        "set -e; cat > {RULE_PATH}.tmp && mv {RULE_PATH}.tmp {RULE_PATH} && rm -f {OLD_RULE_PATH} && \
         udevadm control --reload && udevadm trigger --subsystem-match=input --action=change"
    );
    run_as_root(&script, rule, "Setting up access")
}

/// Rule file for the low-latency settings (kept apart from [`RULE_PATH`],
/// which is rewritten whenever the device selection changes).
pub const LATENCY_RULE_PATH: &str = "/etc/udev/rules.d/70-ps-hid-bridge-latency.rules";

/// Low latency outside the app (see [`crate::lowlat`]), set up once:
/// user `uid` may switch the packet bundling of USB network adapters with
/// the cdc_ncm driver (the app does so while it runs, without
/// root), and with `cpu_access` the logged-in user may keep the CPU out of
/// deep sleep states (only needed where the system does not allow it).
pub fn latency_rule(uid: u32, cpu_access: bool) -> String {
    let mut out = format!(
        "# PS HID Bridge: low latency (written by the app; remove this file to revoke)\n\
         # USB network adapters (cdc_ncm): user {uid} may switch packet bundling off; the app does so while it runs\n\
         ACTION==\"add\", SUBSYSTEM==\"net\", TEST==\"{NCM_TIMER}\", RUN+=\"/bin/chown {uid} %S%p/{NCM_TIMER}\"\n"
    );
    if cpu_access {
        out += "# The logged-in user may keep the CPU out of deep sleep states while the bridge is on\n\
                SUBSYSTEM==\"misc\", KERNEL==\"cpu_dma_latency\", TAG+=\"uaccess\"\n";
    }
    out
}

const NCM_TIMER: &str = "cdc_ncm/tx_timer_usecs";

/// Install [`latency_rule`] as root and apply it right away (no synthetic
/// "add" event for network devices): adapters plugged in now go back to
/// the driver's default if something set them to 0 for good (the app
/// switches them from now on) and belong to the user; /dev/cpu_dma_latency
/// gets its permissions (an ACL from an earlier rule that is no longer
/// needed is taken back).
///
/// For the user running the app; the CPU line only if that user may not use
/// /dev/cpu_dma_latency yet.
pub fn install_latency_rule() -> Result<(), String> {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    // An earlier rule's line stays: the access may come from it.
    let cpu_access = crate::lowlat::cpu_wake_needs_access()
        || std::fs::read_to_string(LATENCY_RULE_PATH).is_ok_and(|r| r.contains("cpu_dma_latency"));
    run_as_root(
        &install_latency_script(uid),
        &latency_rule(uid, cpu_access),
        "Setting up low latency",
    )
}

fn install_latency_script(uid: u32) -> String {
    format!(
        "set -e; {REVOKE_FN}; cat > {LATENCY_RULE_PATH}.tmp && mv {LATENCY_RULE_PATH}.tmp {LATENCY_RULE_PATH} && \
         udevadm control --reload && \
         for f in /sys/class/net/*/{NCM_TIMER}; do if [ -e \"$f\" ]; then \
         if [ \"$(cat \"$f\")\" = 0 ]; then echo {NCM_DEFAULT_TIMER_US} > \"$f\"; fi; chown {uid} \"$f\"; fi; done && \
         udevadm trigger --action=change --subsystem-match=misc --sysname-match=cpu_dma_latency && udevadm settle && \
         revoke /dev/cpu_dma_latency"
    )
}

/// The device access rule is installed.
pub fn rule_installed() -> bool {
    std::path::Path::new(RULE_PATH).exists() || std::path::Path::new(OLD_RULE_PATH).exists()
}

/// The low-latency rule is installed.
pub fn latency_rule_installed() -> bool {
    std::path::Path::new(LATENCY_RULE_PATH).exists()
}

/// Shell function: take back the access the logged-in user (`$PKEXEC_UID`)
/// got through our rules from the given device nodes. udev does not do that
/// by itself when a rule goes away (the ACL stays until the device is
/// plugged in again or the computer restarts); a device that another rule
/// still tags with "uaccess" keeps it.
const REVOKE_FN: &str = "revoke() { for d in \"$@\"; do [ -e \"$d\" ] || continue; \
     case \"$(udevadm info --query=property --name=\"$d\" 2>/dev/null | grep '^CURRENT_TAGS=')\" in \
     *:uaccess:*) ;; \
     *) if [ -n \"$PKEXEC_UID\" ] && command -v setfacl >/dev/null 2>&1; then setfacl -x \"u:$PKEXEC_UID\" \"$d\" 2>/dev/null || true; fi ;; \
     esac; done; }";

/// Undo [`install_rule`]: remove the rule and take back the read access to
/// the input devices (the app cannot read keyboard and mouse afterwards,
/// unless the user may read them anyway, e.g. as a member of "input").
pub fn remove_rule() -> Result<(), String> {
    run_as_root(&remove_rule_script(), "", "Removing access")
}

fn remove_rule_script() -> String {
    format!(
        "set -e; {REVOKE_FN}; rm -f {RULE_PATH} {OLD_RULE_PATH} && udevadm control --reload && \
         udevadm trigger --subsystem-match=input --action=change && udevadm settle && \
         revoke /dev/input/event*"
    )
}

/// Undo [`install_latency_rule`]: remove the rule, give USB network
/// adapters back to root (at the driver's default if they send at once)
/// and take back the user's access to /dev/cpu_dma_latency.
pub fn remove_latency_rule() -> Result<(), String> {
    run_as_root(&remove_latency_script(), "", "Undoing low latency")
}

fn remove_latency_script() -> String {
    format!(
        "set -e; {REVOKE_FN}; rm -f {LATENCY_RULE_PATH} && udevadm control --reload && \
         for f in /sys/class/net/*/{NCM_TIMER}; do if [ -e \"$f\" ]; then chown 0 \"$f\"; \
         if [ \"$(cat \"$f\")\" = 0 ]; then echo {NCM_DEFAULT_TIMER_US} > \"$f\"; fi; fi; done && \
         udevadm trigger --action=change --subsystem-match=misc --sysname-match=cpu_dma_latency && udevadm settle && \
         revoke /dev/cpu_dma_latency"
    )
}

/// The cdc_ncm driver's own collection timer (CDC_NCM_TIMER_INTERVAL_USEC).
const NCM_DEFAULT_TIMER_US: u32 = 400;

/// Run `script` with /bin/sh as root (polkit asks for the password), with
/// `stdin` as its input. `pkexec` runs /bin/sh from the host: inside an
/// AppImage our own files live on a FUSE mount that root may not be able to
/// read, so rules are passed on stdin instead of as a file.
fn run_as_root(script: &str, stdin: &str, what: &str) -> Result<(), String> {
    let mut child = Command::new("pkexec")
        .args(["/bin/sh", "-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start pkexec: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("stdin")?
        .write_all(stdin.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    match out.status.code() {
        Some(0) => Ok(()),
        Some(126) | Some(127) => Err("Cancelled or not authorized (polkit).".into()),
        _ => Err(format!(
            "{what} failed: {}",
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
        assert_eq!(rule.matches("TAG+=\"uaccess\"").count(), 3);
        assert!(rule.contains(
            "SUBSYSTEM==\"input\", KERNEL==\"event*\", ENV{ID_INPUT_KEYBOARD}==\"1\", TAG+=\"uaccess\"\n"
        ));
        assert!(rule.contains("ATTRS{id/vendor}==\"046d\", ATTRS{id/product}==\"c52b\""));
        assert!(rule.contains("# Logitech K400\n"));
    }

    #[test]
    fn latency_rule_lines() {
        let rules = |rule: String| -> Vec<String> {
            rule.lines()
                .filter(|l| !l.starts_with('#'))
                .map(String::from)
                .collect()
        };
        let adapter = "ACTION==\"add\", SUBSYSTEM==\"net\", TEST==\"cdc_ncm/tx_timer_usecs\", \
                       RUN+=\"/bin/chown 1000 %S%p/cdc_ncm/tx_timer_usecs\"";
        let cpu = "SUBSYSTEM==\"misc\", KERNEL==\"cpu_dma_latency\", TAG+=\"uaccess\"";
        assert_eq!(rules(latency_rule(1000, false)), [adapter]);
        assert_eq!(rules(latency_rule(1000, true)), [adapter, cpu]);
        assert!(latency_rule(1000, true).ends_with('\n'));
        assert!(!latency_rule(1000, false).contains("ATTR{"));
    }

    /// The root scripts are valid shell (checked with `sh -n`, nothing runs).
    #[test]
    fn root_scripts_parse() {
        for script in [
            install_latency_script(1000),
            remove_latency_script(),
            remove_rule_script(),
        ] {
            let out = Command::new("sh").args(["-n", "-c", &script]).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        }
    }
}
