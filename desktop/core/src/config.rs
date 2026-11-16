//! Settings file (~/.config/ps-hid-bridge/config.toml) and the key file.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::crypto::parse_key_hex;
use crate::engine::EngineConfig;
use crate::keymap::Keymap;
use crate::mouse::MouseCurve;
use crate::protocol::{KEY_LEN, UDP_PORT_DEFAULT};
use crate::trigger::{Action, Combo, KeyAlt, Trigger, TriggerKind};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TriggerConfig {
    /// "combo", "hold" or "double-tap"
    pub kind: String,
    /// e.g. "Ctrl+Alt+P", "Mouse5", "LEFTCTRL"
    pub keys: String,
    /// "toggle", "on", "off", "pause", "release-all"
    pub action: String,
    /// double tap: maximum time between the taps
    pub window_ms: u64,
    /// Settings this version does not know (written by a newer one): kept
    /// and written back as they are.
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for TriggerConfig {
    fn default() -> Self {
        Self {
            kind: "combo".into(),
            keys: String::new(),
            action: "toggle".into(),
            window_ms: 300,
            extra: toml::Table::new(),
        }
    }
}

impl TriggerConfig {
    pub fn to_trigger(&self) -> Result<Trigger, String> {
        let action = Action::parse(&self.action)?;
        let kind = match self.kind.as_str() {
            "combo" => TriggerKind::Combo(Combo::parse_trigger(&self.keys)?),
            "hold" => TriggerKind::Hold(Combo::parse_trigger(&self.keys)?),
            "double-tap" => {
                let c = Combo::parse_trigger(&self.keys)?;
                if c.0.len() != 1 {
                    return Err(format!("Double tap needs exactly one key: \"{}\"", self.keys));
                }
                TriggerKind::DoubleTap {
                    key: KeyAlt(c.0[0].0.clone()),
                    window_ms: self.window_ms,
                }
            }
            k => return Err(format!("Unknown trigger kind \"{k}\"")),
        };
        Ok(Trigger { kind, action })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MouseConfig {
    pub sensitivity: f64,
    pub accel: f64,
    pub threshold: f64,
    /// Settings this version does not know (written by a newer one): kept
    /// and written back as they are.
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl MouseConfig {
    /// Sensitivity must be positive, acceleration and threshold not
    /// negative (0 or NaN would leave the mouse dead).
    pub fn validate(&self) -> Result<(), String> {
        let ok =
            |v: f64, min_exclusive: bool| v.is_finite() && if min_exclusive { v > 0.0 } else { v >= 0.0 };
        if !ok(self.sensitivity, true) {
            return Err(format!(
                "Mouse sensitivity must be above 0 (is {})",
                self.sensitivity
            ));
        }
        if !ok(self.accel, false) || !ok(self.threshold, false) {
            return Err("Mouse acceleration and threshold must not be negative".into());
        }
        Ok(())
    }
}

impl Default for MouseConfig {
    fn default() -> Self {
        let c = MouseCurve::default();
        Self {
            sensitivity: c.sensitivity,
            accel: c.accel,
            threshold: c.threshold,
            extra: toml::Table::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Board address, "192.168.1.50" or "192.168.1.50:45045".
    pub board: String,
    /// Device selectors (path, "vvvv:pppp" or part of the name).
    pub devices: Vec<String>,
    pub triggers: Vec<TriggerConfig>,
    /// Window rule (KDE Plasma): activate while a window whose class, title
    /// or desktop file contains `window_rule` has the focus. Off by default.
    pub window_rule_enabled: bool,
    pub window_rule: String,
    pub mouse: MouseConfig,
    /// ISO keyboard (German): `#'` key sends HID 0x32.
    pub iso_keyboard: bool,
    /// The first-run wizard has been completed (GUI).
    pub setup_done: bool,
    /// Look for new releases on GitHub (GUI: at start and every 6 hours).
    pub update_check: bool,
    /// Look of the app: "system" (follow the desktop), "light" or "dark".
    pub appearance: String,
    /// Settings this version does not know (written by a newer one): kept
    /// and written back as they are.
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            board: String::new(),
            devices: Vec::new(),
            triggers: vec![TriggerConfig {
                keys: "Ctrl+Alt+P".into(),
                ..Default::default()
            }],
            window_rule_enabled: false,
            window_rule: DEFAULT_WINDOW_RULE.into(),
            mouse: MouseConfig::default(),
            iso_keyboard: true,
            setup_done: false,
            update_check: true,
            appearance: "system".into(),
            extra: toml::Table::new(),
        }
    }
}

/// Remote Play client the window rule looks for by default.
pub const DEFAULT_WINDOW_RULE: &str = "chiaki-ng";

impl Config {
    /// The window rule is switched on and has a non-empty pattern.
    pub fn window_rule_active(&self) -> bool {
        self.window_rule_enabled && !self.window_rule.trim().is_empty()
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let mut config = match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        config.normalize();
        let unknown = config.unknown_keys();
        if !unknown.is_empty() {
            eprintln!(
                "{}: settings this version does not know are kept: {}",
                path.display(),
                unknown.join(", ")
            );
        }
        Ok(config)
    }

    /// Names of the settings this version does not know (from a newer one,
    /// or a typo in a hand-edited file).
    pub fn unknown_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.extra.keys().cloned().collect();
        keys.extend(self.mouse.extra.keys().map(|k| format!("mouse.{k}")));
        for t in &self.triggers {
            keys.extend(t.extra.keys().map(|k| format!("triggers.{k}")));
        }
        keys
    }

    fn parse(text: &str) -> Result<Self, String> {
        let mut config: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        // Settings from before the window rule had its own switch: a
        // pattern that was set meant the rule was on.
        let table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        if !table.contains_key("window_rule_enabled")
            && table
                .get("window_rule")
                .and_then(|v| v.as_str())
                .is_some_and(|r| !r.trim().is_empty())
        {
            config.window_rule_enabled = true;
        }
        Ok(config)
    }

    /// Rewrite triggers in canonical form, e.g. the German names of earlier
    /// versions ("Strg+Alt+P" -> "Ctrl+Alt+P"). Entries that do not parse are
    /// kept as they are, so the error is reported where they are used.
    fn normalize(&mut self) {
        if !["system", "light", "dark"].contains(&self.appearance.as_str()) {
            self.appearance = "system".into();
        }
        for t in &mut self.triggers {
            if let Ok(combo) = Combo::parse(&t.keys) {
                t.keys = combo.to_text();
            }
            if let Ok(action) = Action::parse(&t.action) {
                t.action = action.as_str().into();
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        write_atomic(path, text.as_bytes(), 0o644).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn engine_config(&self) -> Result<EngineConfig, String> {
        self.mouse.validate()?;
        Ok(EngineConfig {
            triggers: self
                .triggers
                .iter()
                .map(TriggerConfig::to_trigger)
                .collect::<Result<_, _>>()?,
            window_rule: self.window_rule_active(),
            keymap: Keymap {
                iso: self.iso_keyboard,
            },
            mouse: self.mouse_curve(),
        })
    }

    pub fn mouse_curve(&self) -> MouseCurve {
        MouseCurve {
            sensitivity: self.mouse.sensitivity,
            accel: self.mouse.accel,
            threshold: self.mouse.threshold,
            ..MouseCurve::default()
        }
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

const APP_DIR: &str = "ps-hid-bridge";
/// Directory name before the project was renamed (PS5 HID Bridge).
const OLD_APP_DIR: &str = "ps5-hid-bridge";

fn xdg(var: &str, fallback: &str) -> PathBuf {
    let base = std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback));
    app_dir(&base)
}

/// `base/ps-hid-bridge`; settings, key and packet counter under the old
/// name move there once. The counter must move along: a fresh one could
/// repeat values already used with the same key.
fn app_dir(base: &Path) -> PathBuf {
    let dir = base.join(APP_DIR);
    let old = base.join(OLD_APP_DIR);
    if !dir.exists() && old.is_dir() {
        let _ = fs::rename(&old, &dir);
    }
    dir
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn psk_path() -> PathBuf {
    config_dir().join("psk")
}

pub fn counter_path() -> PathBuf {
    state_dir().join("counter")
}

/// The board's UDP address from the setting: "192.168.1.50", a host name,
/// either with ":port", an IPv6 address bare or as "[fe80::1]:45045". Host
/// names are looked up (that may take a while).
pub fn board_socket_addr(board: &str) -> io::Result<std::net::SocketAddr> {
    use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
    let bare = board
        .strip_prefix('[')
        .and_then(|b| b.strip_suffix(']'))
        .unwrap_or(board);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, UDP_PORT_DEFAULT));
    }
    let with_port = if board.contains(':') {
        board.to_string()
    } else {
        format!("{board}:{UDP_PORT_DEFAULT}")
    };
    with_port
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no address"))
}

pub fn load_psk(path: &Path) -> Result<[u8; KEY_LEN], String> {
    let text = fs::read_to_string(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => format!(
            "No key stored ({}). Store one with \"pshb-cli config set-psk\".",
            path.display()
        ),
        _ => format!("{}: {e}", path.display()),
    })?;
    // A key others may read (copied by hand, restored from a backup): for
    // the owner only, as it is stored.
    if let Ok(meta) = fs::metadata(path)
        && meta.permissions().mode() & 0o077 != 0
    {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    parse_key_hex(&text)
}

/// Store the key readable for the owner only (0600).
pub fn save_psk(path: &Path, key: &[u8; KEY_LEN]) -> Result<(), String> {
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    write_atomic(path, format!("{hex}\n").as_bytes(), 0o600).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_atomic(path: &Path, data: &[u8], mode: u32) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&tmp)?;
    f.set_permissions(fs::Permissions::from_mode(mode))?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(&tmp, path)?;
    sync_dir(path);
    Ok(())
}

/// After a rename: the directory entry itself reaches the disk (otherwise a
/// power cut can leave the old file).
pub(crate) fn sync_dir(path: &Path) {
    if let Some(dir) = path.parent()
        && let Ok(d) = fs::File::open(dir)
    {
        let _ = d.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pshb-cfg-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn default_roundtrip_and_engine() {
        let dir = temp("rt");
        let path = dir.join("config.toml");
        let mut c = Config {
            board: "192.168.1.50".into(),
            devices: vec!["K400".into()],
            ..Default::default()
        };
        c.triggers.push(TriggerConfig {
            kind: "double-tap".into(),
            keys: "LEFTCTRL".into(),
            action: "pause".into(),
            window_ms: 250,
            ..Default::default()
        });
        c.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert_eq!(back, c);
        let ec = back.engine_config().unwrap();
        assert_eq!(ec.triggers.len(), 2);
        assert!(!ec.window_rule);
        assert_eq!(back.window_rule, "chiaki-ng");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn window_rule_is_off_by_default_and_needs_a_pattern() {
        let mut c = Config::default();
        assert!(!c.window_rule_active());
        c.window_rule_enabled = true;
        assert!(c.window_rule_active());
        assert!(c.engine_config().unwrap().window_rule);
        c.window_rule = "  ".into();
        assert!(!c.window_rule_active());
    }

    #[test]
    fn window_rule_of_older_settings_stays_on() {
        // Before the switch existed, a pattern meant "on".
        let c = Config::parse("window_rule = \"chiaki\"\n").unwrap();
        assert!(c.window_rule_active());
        let c = Config::parse("window_rule = \"\"\n").unwrap();
        assert!(!c.window_rule_enabled);
        let c = Config::parse("window_rule_enabled = false\nwindow_rule = \"chiaki\"\n").unwrap();
        assert!(!c.window_rule_enabled);
        assert!(!Config::parse("").unwrap().window_rule_enabled);
    }

    #[test]
    fn mouse_values_are_checked() {
        let mut c = Config::default();
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            c.mouse.sensitivity = bad;
            assert!(c.engine_config().is_err(), "{bad}");
        }
        c.mouse.sensitivity = 1.5;
        c.mouse.accel = -0.1;
        assert!(c.engine_config().is_err());
        c.mouse.accel = 0.0;
        assert!(c.engine_config().is_ok());
    }

    #[test]
    fn board_addresses() {
        let a = |s: &str| board_socket_addr(s).unwrap().to_string();
        assert_eq!(a("192.168.1.50"), "192.168.1.50:45045");
        assert_eq!(a("192.168.1.50:5000"), "192.168.1.50:5000");
        assert_eq!(a("fe80::1"), "[fe80::1]:45045");
        assert_eq!(a("[fe80::1]"), "[fe80::1]:45045");
        assert_eq!(a("[fe80::1]:5000"), "[fe80::1]:5000");
        assert_eq!(a("localhost:5000").rsplit_once(':').unwrap().1, "5000");
        assert!(board_socket_addr("no such host.invalid").is_err());
    }

    #[test]
    fn settings_of_a_newer_version_are_kept() {
        let dir = temp("newer");
        let path = dir.join("config.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            "board = \"10.0.0.2\"\nfuture_flag = true\n\n[mouse]\nsensitivity = 2.0\nfuture_mouse = 3\n\n\
             [future_table]\nx = \"y\"\n\n[[triggers]]\nkeys = \"Ctrl+Alt+P\"\nfuture_trigger = 1\n",
        )
        .unwrap();
        let mut c = Config::load(&path).unwrap();
        assert_eq!(c.board, "10.0.0.2");
        assert_eq!(c.mouse.sensitivity, 2.0);
        let mut unknown = c.unknown_keys();
        unknown.sort();
        assert_eq!(
            unknown,
            [
                "future_flag",
                "future_table",
                "mouse.future_mouse",
                "triggers.future_trigger"
            ]
        );
        // An older version that saves its own change keeps the newer settings.
        c.iso_keyboard = false;
        c.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert_eq!(back, c);
        assert!(!back.iso_keyboard);
        assert_eq!(back.extra["future_table"]["x"].as_str(), Some("y"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn keyboard_type_roundtrip() {
        let dir = temp("kbd");
        let path = dir.join("config.toml");
        let c = Config {
            iso_keyboard: false,
            ..Default::default()
        };
        c.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert!(!back.iso_keyboard);
        assert!(!back.engine_config().unwrap().keymap.iso);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn german_trigger_names_are_normalized_on_load() {
        let dir = temp("norm");
        let path = dir.join("config.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            "[[triggers]]\nkeys = \"Strg+Alt+P\"\naction = \"umschalten\"\n\n\
             [[triggers]]\nkind = \"hold\"\nkeys = \"Maus5\"\n\n\
             [[triggers]]\nkeys = \"Strg+Banana\"\n",
        )
        .unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(c.triggers[0].keys, "Ctrl+Alt+P");
        assert_eq!(c.triggers[0].action, "toggle");
        assert_eq!(c.triggers[1].keys, "Mouse5");
        assert_eq!(c.triggers[2].keys, "Strg+Banana");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn appearance_is_kept_or_falls_back_to_system() {
        let dir = temp("appearance");
        let path = dir.join("config.toml");
        fs::create_dir_all(&dir).unwrap();
        for (stored, loaded) in [("dark", "dark"), ("light", "light"), ("purple", "system")] {
            fs::write(&path, format!("appearance = \"{stored}\"\n")).unwrap();
            assert_eq!(Config::load(&path).unwrap().appearance, loaded);
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let c = Config::load(Path::new("/nonexistent/pshb.toml")).unwrap();
        assert_eq!(c.triggers[0].keys, "Ctrl+Alt+P");
        assert_eq!(c.appearance, "system");
    }

    #[test]
    fn bad_trigger_is_reported() {
        let c = Config {
            triggers: vec![TriggerConfig {
                keys: "Ctrl+Banana".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(c.engine_config().unwrap_err().contains("Banana"));
    }

    #[test]
    fn files_under_the_old_name_move_once() {
        let base = temp("rename");
        let old = base.join(OLD_APP_DIR);
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("counter"), "12345\n").unwrap();
        let dir = app_dir(&base);
        assert_eq!(dir, base.join(APP_DIR));
        assert_eq!(fs::read_to_string(dir.join("counter")).unwrap(), "12345\n");
        assert!(!old.exists());
        // An old directory that shows up later is left alone.
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("counter"), "1\n").unwrap();
        app_dir(&base);
        assert_eq!(fs::read_to_string(dir.join("counter")).unwrap(), "12345\n");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn psk_file_is_private() {
        let dir = temp("psk");
        let path = dir.join("psk");
        save_psk(&path, &[0xAB; 32]).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(load_psk(&path).unwrap(), [0xAB; 32]);
        let _ = fs::remove_dir_all(dir);
    }
}
