//! Settings file (~/.config/ps-hid-bridge/config.toml) and the key file.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::crypto::parse_key_hex;
use crate::engine::{EngineConfig, EngineProfile, STANDARD_PROFILE};
use crate::keymap::{self, Keymap};
use crate::mouse::MouseCurve;
use crate::protocol::{KEY_LEN, UDP_PORT_DEFAULT};
use crate::touch::TouchSettings;
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
    /// `profiles`: the profile names, for "profile:<name>" actions.
    pub fn to_trigger(&self, profiles: &[&str]) -> Result<Trigger, String> {
        let action = Action::parse_with_profiles(&self.action, profiles)?;
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
    /// Up and down reversed.
    pub invert_y: bool,
    /// Vertical sensitivity relative to the horizontal one.
    pub y_ratio: f64,
    /// Factor on the wheel's notches.
    pub wheel_speed: f64,
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
        if !ok(self.y_ratio, true) || !ok(self.wheel_speed, true) {
            return Err("Vertical ratio and wheel speed must be above 0".into());
        }
        Ok(())
    }

    pub fn curve(&self) -> MouseCurve {
        MouseCurve {
            sensitivity: self.sensitivity,
            accel: self.accel,
            threshold: self.threshold,
            invert_y: self.invert_y,
            y_ratio: self.y_ratio,
            wheel: self.wheel_speed,
            ..MouseCurve::default()
        }
    }
}

impl Default for MouseConfig {
    fn default() -> Self {
        let c = MouseCurve::default();
        Self {
            sensitivity: c.sensitivity,
            accel: c.accel,
            threshold: c.threshold,
            invert_y: c.invert_y,
            y_ratio: c.y_ratio,
            wheel_speed: c.wheel,
            extra: toml::Table::new(),
        }
    }
}

/// One remapped key or button: `from` (physical) reaches the PlayStation as
/// `to`; `to` = "none" sends nothing for it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemapConfig {
    /// e.g. "Mouse4", "CAPSLOCK", "Ctrl" (both Ctrl keys)
    pub from: String,
    /// e.g. "F", "LEFTCTRL", "MouseMiddle", "none"
    pub to: String,
    /// Settings this version does not know (written by a newer one): kept
    /// and written back as they are.
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl RemapConfig {
    /// Physical codes and what they become (None: nothing).
    pub fn parse(&self, keymap: Keymap) -> Result<(Vec<u16>, Option<u16>), String> {
        let from = remap_codes(&self.from)?;
        let to = match self.to.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "off" => None,
            _ => {
                let code = remap_codes(&self.to)?[0];
                let forwarded = keymap::modifier_bit(code).is_some()
                    || keymap::button_bit(code).is_some()
                    || keymap.usage(code).is_some();
                if !forwarded {
                    return Err(format!(
                        "\"{}\" cannot be sent to the PlayStation",
                        self.to.trim()
                    ));
                }
                Some(code)
            }
        };
        Ok((from, to))
    }
}

/// Codes of a single key or button name ("Ctrl" stands for both Ctrl keys).
fn remap_codes(name: &str) -> Result<Vec<u16>, String> {
    let c = Combo::parse(name.trim())?;
    match c.0.as_slice() {
        [one] => Ok(one.0.clone()),
        _ => Err(format!("\"{}\" is more than one key", name.trim())),
    }
}

/// A profile besides "Standard" (whose settings are `[mouse]` and `[[remap]]`
/// at the top of the file, as before profiles existed).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileConfig {
    pub name: String,
    /// Applies while a window matching this has the focus ("" = only when
    /// chosen by hand or by a trigger).
    pub window: String,
    pub mouse: MouseConfig,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub remap: Vec<RemapConfig>,
    /// Settings this version does not know (written by a newer one): kept
    /// and written back as they are.
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// Touchpads and touchscreens (see [`crate::touch`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TouchpadConfig {
    /// Factor on the motion (1.0 = as designed).
    pub speed: f64,
    /// A short tap clicks.
    pub tap_to_click: bool,
    /// Two-finger scrolling moves the content with the fingers.
    pub natural_scroll: bool,
    /// Settings this version does not know (written by a newer one): kept
    /// and written back as they are.
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl TouchpadConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.speed.is_finite() && self.speed > 0.0) {
            return Err(format!("Touchpad speed must be above 0 (is {})", self.speed));
        }
        Ok(())
    }

    pub fn settings(&self) -> TouchSettings {
        TouchSettings {
            speed: self.speed,
            tap: self.tap_to_click,
            natural_scroll: self.natural_scroll,
        }
    }
}

impl Default for TouchpadConfig {
    fn default() -> Self {
        let t = TouchSettings::default();
        Self {
            speed: t.speed,
            tap_to_click: t.tap,
            natural_scroll: t.natural_scroll,
            extra: toml::Table::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Board address, "192.168.1.50" or "192.168.1.50:45045".
    pub board: String,
    /// MAC address of the board at `board` ("02:68:9d:96:c3:3d"; empty =
    /// not known yet), learned from its discovery answer: the app finds the
    /// board again when it got another address (crate::discover).
    pub board_mac: String,
    /// Device selectors (path, "vvvv:pppp" or part of the name).
    pub devices: Vec<String>,
    pub triggers: Vec<TriggerConfig>,
    /// Window rule (KDE Plasma): activate while a window whose class, title
    /// or desktop file contains `window_rule` has the focus. Off by default.
    pub window_rule_enabled: bool,
    pub window_rule: String,
    pub mouse: MouseConfig,
    /// Remapped keys of the profile "Standard".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub remap: Vec<RemapConfig>,
    /// Further profiles.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<ProfileConfig>,
    /// The profile chosen by hand or by a trigger ("" = "Standard").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub active_profile: String,
    pub touchpad: TouchpadConfig,
    /// ISO keyboard (German): `#'` key sends HID 0x32.
    pub iso_keyboard: bool,
    /// The first-run wizard has been completed (GUI).
    pub setup_done: bool,
    /// Look for new releases on GitHub (GUI: at start and every 6 hours).
    pub update_check: bool,
    /// Look of the app: "system" (follow the desktop), "light" or "dark".
    pub appearance: String,
    /// Language of the app: "en" or "de".
    pub language: String,
    /// Keyboard language set on the PlayStation, for typing text there
    /// (crate::typing): "de", "us", "uk" …; "" = from the system language.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ps_layout: String,
    /// Keep the CPU out of deep sleep states while the bridge is on (faster
    /// reaction to input, a few watts more; see crate::lowlat::CpuWake).
    pub keep_cpu_awake: bool,
    /// Settings this version does not know (written by a newer one): kept
    /// and written back as they are.
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            board: String::new(),
            board_mac: String::new(),
            devices: Vec::new(),
            triggers: vec![TriggerConfig {
                keys: "Ctrl+Alt+P".into(),
                ..Default::default()
            }],
            window_rule_enabled: false,
            window_rule: DEFAULT_WINDOW_RULE.into(),
            mouse: MouseConfig::default(),
            remap: Vec::new(),
            profiles: Vec::new(),
            active_profile: String::new(),
            touchpad: TouchpadConfig::default(),
            iso_keyboard: true,
            setup_done: false,
            update_check: true,
            appearance: "system".into(),
            language: "en".into(),
            ps_layout: String::new(),
            keep_cpu_awake: true,
            extra: toml::Table::new(),
        }
    }
}

/// Remote Play client the window rule looks for by default.
pub const DEFAULT_WINDOW_RULE: &str = "chiaki-ng";

/// What the app's sliders cover, and so what `pshb-cli config set` takes:
/// a value outside would be shown wrong in the app and replaced by it.
pub const SENSITIVITY_RANGE: (f64, f64) = (0.1, 8.0);
pub const ACCEL_RANGE: (f64, f64) = (0.0, 1.0);
pub const TOUCH_SPEED_RANGE: (f64, f64) = (0.1, 8.0);
pub const Y_RATIO_RANGE: (f64, f64) = (0.25, 4.0);
pub const WHEEL_SPEED_RANGE: (f64, f64) = (0.25, 4.0);

/// `value` in `range`, as a message for the option `name` otherwise.
pub fn check_range(name: &str, value: f64, range: (f64, f64)) -> Result<f64, String> {
    if value.is_finite() && value >= range.0 && value <= range.1 {
        Ok(value)
    } else {
        Err(format!(
            "{name} must be between {} and {} (is {value})",
            range.0, range.1
        ))
    }
}

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
            crate::logln!(
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
        for r in &self.remap {
            keys.extend(r.extra.keys().map(|k| format!("remap.{k}")));
        }
        for p in &self.profiles {
            keys.extend(p.extra.keys().map(|k| format!("profiles.{k}")));
            keys.extend(p.mouse.extra.keys().map(|k| format!("profiles.mouse.{k}")));
            for r in &p.remap {
                keys.extend(r.extra.keys().map(|k| format!("profiles.remap.{k}")));
            }
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
        if !["en", "de"].contains(&self.language.as_str()) {
            self.language = "en".into();
        }
        for t in &mut self.triggers {
            if let Ok(combo) = Combo::parse(&t.keys) {
                t.keys = combo.to_text();
            }
            // ("profile:<name>" stays as written: the name is not in the action.)
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
        self.touchpad.validate()?;
        let names = self.profile_names();
        Ok(EngineConfig {
            triggers: self
                .triggers
                .iter()
                .map(|t| t.to_trigger(&names))
                .collect::<Result<_, _>>()?,
            window_rule: self.window_rule_active(),
            window_pattern: self.window_rule.trim().to_string(),
            keymap: self.keymap(),
            profiles: self.engine_profiles()?,
            profile: self.active_profile_index(),
        })
    }

    pub fn keymap(&self) -> Keymap {
        Keymap {
            iso: self.iso_keyboard,
        }
    }

    /// "Standard", then the further profiles.
    pub fn profile_names(&self) -> Vec<&str> {
        std::iter::once(STANDARD_PROFILE)
            .chain(self.profiles.iter().map(|p| p.name.as_str()))
            .collect()
    }

    /// Index of `active_profile` (0, "Standard", if there is no such profile).
    pub fn active_profile_index(&self) -> usize {
        self.profile_index(&self.active_profile).unwrap_or(0)
    }

    /// Index of the profile called `name` (ignoring case; "" = "Standard").
    pub fn profile_index(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        if name.is_empty() {
            return Some(0);
        }
        self.profile_names()
            .iter()
            .position(|n| n.to_lowercase() == name.to_lowercase())
    }

    /// Mouse settings of the profile with this index (0 = "Standard").
    pub fn profile_mouse(&self, index: usize) -> &MouseConfig {
        match index.checked_sub(1).and_then(|i| self.profiles.get(i)) {
            Some(p) => &p.mouse,
            None => &self.mouse,
        }
    }

    pub fn profile_mouse_mut(&mut self, index: usize) -> &mut MouseConfig {
        match index.checked_sub(1) {
            Some(i) if i < self.profiles.len() => &mut self.profiles[i].mouse,
            _ => &mut self.mouse,
        }
    }

    pub fn profile_remap(&self, index: usize) -> &[RemapConfig] {
        match index.checked_sub(1).and_then(|i| self.profiles.get(i)) {
            Some(p) => &p.remap,
            None => &self.remap,
        }
    }

    pub fn profile_remap_mut(&mut self, index: usize) -> &mut Vec<RemapConfig> {
        match index.checked_sub(1) {
            Some(i) if i < self.profiles.len() => &mut self.profiles[i].remap,
            _ => &mut self.remap,
        }
    }

    /// Names must be set and differ (ignoring case), "Standard" is taken.
    pub fn check_profile_name(&self, name: &str, except: Option<usize>) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A profile needs a name".into());
        }
        if name.len() > 40 {
            return Err("Profile names have at most 40 characters".into());
        }
        let taken = self
            .profile_names()
            .iter()
            .enumerate()
            .any(|(i, n)| Some(i) != except && n.to_lowercase() == name.to_lowercase());
        if taken {
            return Err(format!("There is already a profile named \"{name}\""));
        }
        Ok(())
    }

    /// Every profile for the engine; also checks names and remapped keys.
    pub fn engine_profiles(&self) -> Result<Vec<EngineProfile>, String> {
        let keymap = self.keymap();
        let remap = |list: &[RemapConfig], profile: &str| -> Result<_, String> {
            let mut map = std::collections::BTreeMap::new();
            for r in list {
                let (from, to) = r
                    .parse(keymap)
                    .map_err(|e| format!("Profile \"{profile}\": {e}"))?;
                for f in from {
                    map.insert(f, to);
                }
            }
            Ok(map)
        };
        self.mouse.validate()?;
        let mut out = vec![EngineProfile {
            name: STANDARD_PROFILE.into(),
            window: String::new(),
            mouse: self.mouse.curve(),
            remap: remap(&self.remap, STANDARD_PROFILE)?,
        }];
        for (i, p) in self.profiles.iter().enumerate() {
            self.check_profile_name(&p.name, Some(i + 1))?;
            p.mouse
                .validate()
                .map_err(|e| format!("Profile \"{}\": {e}", p.name))?;
            out.push(EngineProfile {
                name: p.name.trim().to_string(),
                window: p.window.trim().to_string(),
                mouse: p.mouse.curve(),
                remap: remap(&p.remap, &p.name)?,
            });
        }
        Ok(out)
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

/// Notes a network adapter the app switched to "send at once", so that it
/// is put back after a crash (see crate::lowlat::AdapterTimer).
pub fn adapter_state_path() -> PathBuf {
    state_dir().join("adapter-timer")
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
            board_mac: "02:68:9d:96:c3:3d".into(),
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
        c.mouse.wheel_speed = 0.0;
        assert!(c.engine_config().is_err());
    }

    #[test]
    fn older_settings_load_unchanged_and_mean_one_profile() {
        let text = "board = \"192.168.1.50\"\n[mouse]\nsensitivity = 1.5\naccel = 0.0\nthreshold = 2.0\n";
        let c = Config::parse(text).unwrap();
        assert!(c.unknown_keys().is_empty());
        assert_eq!(c.mouse.y_ratio, 1.0);
        assert!(!c.mouse.invert_y);
        let ec = c.engine_config().unwrap();
        assert_eq!(ec.profiles.len(), 1);
        assert_eq!(ec.profiles[0].name, STANDARD_PROFILE);
        assert_eq!(ec.profiles[0].mouse.sensitivity, 1.5);
        assert_eq!(ec.profile, 0);
        // Nothing about profiles is written while there are none.
        let saved = toml::to_string_pretty(&c).unwrap();
        assert!(!saved.contains("profiles") && !saved.contains("remap") && !saved.contains("active_profile"));
    }

    #[test]
    fn profiles_and_remapping() {
        let dir = temp("profiles");
        let path = dir.join("config.toml");
        let mut c = Config::default();
        c.remap.push(RemapConfig {
            from: "CAPSLOCK".into(),
            to: "Ctrl".into(),
            ..Default::default()
        });
        c.profiles.push(ProfileConfig {
            name: "Shooter".into(),
            window: "chiaki".into(),
            mouse: MouseConfig {
                invert_y: true,
                ..Default::default()
            },
            remap: vec![
                RemapConfig {
                    from: "Mouse4".into(),
                    to: "F".into(),
                    ..Default::default()
                },
                RemapConfig {
                    from: "Ctrl".into(),
                    to: "none".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        c.active_profile = "shooter".into();
        c.triggers.push(TriggerConfig {
            keys: "F9".into(),
            action: "profile:Shooter".into(),
            ..Default::default()
        });
        c.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert_eq!(back, c);
        assert!(back.unknown_keys().is_empty());
        // "profile:<name>" is kept as written.
        assert_eq!(back.triggers[1].action, "profile:Shooter");

        let ec = back.engine_config().unwrap();
        assert_eq!(ec.profiles.len(), 2);
        assert_eq!(ec.profile, 1);
        assert_eq!(ec.triggers[1].action, Action::Profile(1));
        assert_eq!(
            ec.profiles[0].remap.get(&58),
            Some(&Some(crate::trigger::KEY_LEFTCTRL))
        );
        let shooter = &ec.profiles[1];
        assert_eq!(shooter.window, "chiaki");
        assert!(shooter.mouse.invert_y);
        assert_eq!(shooter.remap.get(&keymap::BTN_SIDE), Some(&Some(33))); // KEY_F
        // "Ctrl" stands for both Ctrl keys.
        assert_eq!(shooter.remap.get(&crate::trigger::KEY_LEFTCTRL), Some(&None));
        assert_eq!(shooter.remap.get(&crate::trigger::KEY_RIGHTCTRL), Some(&None));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn profile_mistakes_are_refused() {
        let mut c = Config::default();
        c.triggers[0].action = "profile:Racing".into();
        assert!(c.engine_config().unwrap_err().contains("Racing"));
        c.triggers[0].action = "toggle".into();

        c.profiles.push(ProfileConfig {
            name: "standard".into(),
            ..Default::default()
        });
        assert!(
            c.engine_config().is_err(),
            "the name of the first profile is taken"
        );
        c.profiles[0].name = " ".into();
        assert!(c.engine_config().is_err());
        c.profiles[0].name = "Racing".into();
        assert!(c.engine_config().is_ok());
        assert!(c.check_profile_name("RACING", None).is_err());
        assert!(c.check_profile_name("RACING", Some(1)).is_ok(), "its own name");

        c.remap.push(RemapConfig {
            from: "Ctrl+A".into(),
            to: "B".into(),
            ..Default::default()
        });
        assert!(c.engine_config().is_err(), "one key only");
        c.remap[0].from = "A".into();
        c.remap[0].to = "Nonsense".into();
        assert!(c.engine_config().is_err());
        c.remap[0].to = "#164".into(); // play/pause: no key the PlayStation gets
        assert!(c.engine_config().unwrap_err().contains("cannot be sent"));
        c.remap[0].to = "".into();
        assert_eq!(c.engine_config().unwrap().profiles[0].remap.get(&30), Some(&None));
    }

    #[test]
    fn unknown_active_profile_means_standard() {
        let c = Config {
            active_profile: "gone".into(),
            ..Default::default()
        };
        assert_eq!(c.active_profile_index(), 0);
        assert_eq!(c.engine_config().unwrap().profile, 0);
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
