//! Settings file (~/.config/ps5-hid-bridge/config.toml) and the key file.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::crypto::parse_key_hex;
use crate::engine::EngineConfig;
use crate::keymap::Keymap;
use crate::mouse::MouseCurve;
use crate::protocol::KEY_LEN;
use crate::trigger::{Action, Combo, KeyAlt, Trigger, TriggerKind};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TriggerConfig {
    /// "combo", "hold" or "double-tap"
    pub kind: String,
    /// e.g. "Strg+Alt+P", "Maus5", "LEFTCTRL"
    pub keys: String,
    /// "toggle", "on", "off", "pause", "release-all"
    pub action: String,
    /// double tap: maximum time between the taps
    pub window_ms: u64,
}

impl Default for TriggerConfig {
    fn default() -> Self {
        Self {
            kind: "combo".into(),
            keys: String::new(),
            action: "toggle".into(),
            window_ms: 300,
        }
    }
}

impl TriggerConfig {
    pub fn to_trigger(&self) -> Result<Trigger, String> {
        let action = Action::parse(&self.action)?;
        let kind = match self.kind.as_str() {
            "combo" => TriggerKind::Combo(Combo::parse(&self.keys)?),
            "hold" => TriggerKind::Hold(Combo::parse(&self.keys)?),
            "double-tap" => {
                let c = Combo::parse(&self.keys)?;
                if c.0.len() != 1 {
                    return Err(format!("Doppeltipp braucht genau eine Taste: „{}“", self.keys));
                }
                TriggerKind::DoubleTap {
                    key: KeyAlt(c.0[0].0.clone()),
                    window_ms: self.window_ms,
                }
            }
            k => return Err(format!("Unbekannte Auslöser-Art „{k}“")),
        };
        Ok(Trigger { kind, action })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MouseConfig {
    pub sensitivity: f64,
    pub accel: f64,
    pub threshold: f64,
}

impl Default for MouseConfig {
    fn default() -> Self {
        let c = MouseCurve::default();
        Self {
            sensitivity: c.sensitivity,
            accel: c.accel,
            threshold: c.threshold,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Board address, "192.168.1.50" or "192.168.1.50:45045".
    pub board: String,
    /// Device selectors (path, "vvvv:pppp" or part of the name).
    pub devices: Vec<String>,
    pub triggers: Vec<TriggerConfig>,
    /// Activate while a window whose class/title contains this text has the
    /// focus (KDE Plasma); empty = no window rule.
    pub window_rule: String,
    pub mouse: MouseConfig,
    /// ISO keyboard (German): `#'` key sends HID 0x32.
    pub iso_keyboard: bool,
    /// The first-run wizard has been completed (GUI).
    pub setup_done: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            board: String::new(),
            devices: Vec::new(),
            triggers: vec![TriggerConfig {
                keys: "Strg+Alt+P".into(),
                ..Default::default()
            }],
            window_rule: String::new(),
            mouse: MouseConfig::default(),
            iso_keyboard: true,
            setup_done: false,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        write_atomic(path, text.as_bytes(), 0o644).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn engine_config(&self) -> Result<EngineConfig, String> {
        Ok(EngineConfig {
            triggers: self
                .triggers
                .iter()
                .map(TriggerConfig::to_trigger)
                .collect::<Result<_, _>>()?,
            window_rule: !self.window_rule.trim().is_empty(),
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

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
        .join("ps5-hid-bridge")
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

pub fn load_psk(path: &Path) -> Result<[u8; KEY_LEN], String> {
    let text = fs::read_to_string(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => format!(
            "Kein Schlüssel gespeichert ({}). Mit „pshb-cli config set-psk“ hinterlegen.",
            path.display()
        ),
        _ => format!("{}: {e}", path.display()),
    })?;
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
    fs::rename(&tmp, path)
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
        });
        c.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert_eq!(back, c);
        let ec = back.engine_config().unwrap();
        assert_eq!(ec.triggers.len(), 2);
        assert!(!ec.window_rule);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let c = Config::load(Path::new("/nonexistent/pshb.toml")).unwrap();
        assert_eq!(c.triggers[0].keys, "Strg+Alt+P");
    }

    #[test]
    fn bad_trigger_is_reported() {
        let c = Config {
            triggers: vec![TriggerConfig {
                keys: "Strg+Banane".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(c.engine_config().unwrap_err().contains("Banane"));
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
