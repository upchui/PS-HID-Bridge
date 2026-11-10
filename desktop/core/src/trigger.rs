//! Trigger definitions: which input switches the bridge on and off.
//!
//! Textual form (config file, CLI, GUI recorder):
//!   "Ctrl+Alt+P"            key combination (modifiers match left or right)
//!   "Mouse5" / "BTN_EXTRA"  mouse button (optionally with modifiers)
//!   "LEFTCTRL"              single key (e.g. for double tap)

use crate::keymap::{self, BTN_EXTRA, BTN_MIDDLE, BTN_SIDE};

pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_RIGHTCTRL: u16 = 97;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_RIGHTSHIFT: u16 = 54;
pub const KEY_LEFTALT: u16 = 56;
pub const KEY_RIGHTALT: u16 = 100;
pub const KEY_LEFTMETA: u16 = 125;
pub const KEY_RIGHTMETA: u16 = 126;

/// One element of a combination: any of these codes satisfies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyAlt(pub Vec<u16>);

impl KeyAlt {
    pub fn matches(&self, code: u16) -> bool {
        self.0.contains(&code)
    }
}

/// All elements must be pressed at the same time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Combo(pub Vec<KeyAlt>);

impl Combo {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut parts = Vec::new();
        for raw in text.split('+') {
            let name = raw.trim();
            if name.is_empty() {
                return Err(format!("Empty part in \"{text}\""));
            }
            let alt = match name.to_ascii_lowercase().as_str() {
                "strg" | "ctrl" | "control" => vec![KEY_LEFTCTRL, KEY_RIGHTCTRL],
                "shift" | "umschalt" => vec![KEY_LEFTSHIFT, KEY_RIGHTSHIFT],
                "alt" => vec![KEY_LEFTALT],
                "altgr" => vec![KEY_RIGHTALT],
                "super" | "meta" | "win" => vec![KEY_LEFTMETA, KEY_RIGHTMETA],
                // German names are still accepted (configs from earlier versions).
                "mouse4" | "maus4" | "seitentaste4" => vec![BTN_SIDE],
                "mouse5" | "maus5" | "seitentaste5" => vec![BTN_EXTRA],
                "mousemiddle" | "mausmitte" => vec![BTN_MIDDLE],
                _ => vec![keymap::code_by_name(name).ok_or_else(|| format!("Unknown key \"{name}\""))?],
            };
            parts.push(KeyAlt(alt));
        }
        Ok(Combo(parts))
    }

    /// Satisfied by the set of pressed codes.
    pub fn satisfied(&self, pressed: &[u16]) -> bool {
        !self.0.is_empty() && self.0.iter().all(|alt| pressed.iter().any(|c| alt.matches(*c)))
    }

    pub fn contains(&self, code: u16) -> bool {
        self.0.iter().any(|alt| alt.matches(code))
    }

    /// Canonical text form, e.g. "Ctrl+Alt+P".
    pub fn to_text(&self) -> String {
        self.0
            .iter()
            .map(|alt| {
                let c = alt.0[0];
                match c {
                    KEY_LEFTCTRL if alt.0.len() > 1 => "Ctrl".to_string(),
                    KEY_LEFTSHIFT if alt.0.len() > 1 => "Shift".to_string(),
                    KEY_LEFTMETA if alt.0.len() > 1 => "Super".to_string(),
                    KEY_LEFTALT => "Alt".to_string(),
                    KEY_RIGHTALT => "AltGr".to_string(),
                    BTN_SIDE => "Mouse4".to_string(),
                    BTN_EXTRA => "Mouse5".to_string(),
                    BTN_MIDDLE => "MouseMiddle".to_string(),
                    _ => keymap::name_of(c),
                }
            })
            .collect::<Vec<_>>()
            .join("+")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Toggle,
    On,
    Off,
    /// While paused nothing is forwarded, even if the window rule matches.
    Pause,
    ReleaseAll,
}

impl Action {
    pub fn parse(s: &str) -> Result<Self, String> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "toggle" | "umschalten" => Self::Toggle,
            "on" | "an" => Self::On,
            "off" | "aus" => Self::Off,
            "pause" | "pausieren" => Self::Pause,
            "release" | "release-all" | "loslassen" => Self::ReleaseAll,
            _ => return Err(format!("Unknown action \"{s}\"")),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::On => "on",
            Self::Off => "off",
            Self::Pause => "pause",
            Self::ReleaseAll => "release-all",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    /// Fires when the combination becomes complete.
    Combo(Combo),
    /// Fires on the second tap of a single key within `window_ms`.
    DoubleTap { key: KeyAlt, window_ms: u64 },
    /// Bridge is active while the combination is held (action is ignored).
    Hold(Combo),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub kind: TriggerKind,
    pub action: Action,
}

impl Trigger {
    pub fn combo(text: &str, action: Action) -> Result<Self, String> {
        Ok(Self {
            kind: TriggerKind::Combo(Combo::parse(text)?),
            action,
        })
    }
}

/// The fixed emergency combination: always releases the devices.
pub fn emergency_combo() -> Combo {
    Combo::parse("Ctrl+Alt+Shift+Esc").unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_print() {
        let c = Combo::parse("ctrl+Alt+p").unwrap();
        assert_eq!(c.to_text(), "Ctrl+Alt+P");
        assert!(c.satisfied(&[KEY_RIGHTCTRL, KEY_LEFTALT, 25]));
        assert!(!c.satisfied(&[KEY_RIGHTCTRL, 25]));
        assert_eq!(Combo::parse("Mouse5").unwrap().to_text(), "Mouse5");
        assert_eq!(Combo::parse("Ctrl+BTN_SIDE").unwrap().to_text(), "Ctrl+Mouse4");
        assert!(Combo::parse("Ctrl+Nonsense").is_err());
        assert!(Combo::parse("Ctrl++P").is_err());
    }

    #[test]
    fn german_aliases_still_work() {
        assert_eq!(Combo::parse("Strg+Alt+P").unwrap().to_text(), "Ctrl+Alt+P");
        assert_eq!(Combo::parse("Umschalt+Maus4").unwrap().to_text(), "Shift+Mouse4");
        assert_eq!(Combo::parse("MausMitte").unwrap().to_text(), "MouseMiddle");
        assert_eq!(Action::parse("umschalten").unwrap(), Action::Toggle);
    }

    #[test]
    fn actions() {
        assert_eq!(Action::parse("Toggle").unwrap(), Action::Toggle);
        assert_eq!(Action::parse(Action::Pause.as_str()).unwrap(), Action::Pause);
        assert!(Action::parse("x").is_err());
    }
}
