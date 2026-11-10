//! Linux evdev key codes -> USB HID usages (Keyboard/Keypad page 0x07).
//!
//! The board sends *positions*, not characters: the PS5 must use the same
//! layout as the physical keyboard (e.g. German). ISO specifics:
//! * KEY_102ND (`<>|` left of Y) -> 0x64 "Non-US \ and |"
//! * KEY_BACKSLASH (`#'` left of Enter on ISO boards) -> 0x32 "Non-US # and ~",
//!   which is what a real German USB keyboard sends. ANSI boards use 0x31;
//!   see [`Keymap::iso`].

/// Modifier keys and their bit in the HID report's modifier byte.
pub const MODIFIERS: [(u16, u8); 8] = [
    (KEY_LEFTCTRL, 0x01),
    (KEY_LEFTSHIFT, 0x02),
    (KEY_LEFTALT, 0x04),
    (KEY_LEFTMETA, 0x08),
    (KEY_RIGHTCTRL, 0x10),
    (KEY_RIGHTSHIFT, 0x20),
    (KEY_RIGHTALT, 0x40),
    (KEY_RIGHTMETA, 0x80),
];

/// Mouse buttons and their bit in the HID mouse report.
pub const BUTTONS: [(u16, u8); 7] = [
    (BTN_LEFT, 0x01),
    (BTN_RIGHT, 0x02),
    (BTN_MIDDLE, 0x04),
    (BTN_SIDE, 0x08),
    (BTN_EXTRA, 0x10),
    (BTN_BACK, 0x08),
    (BTN_FORWARD, 0x10),
];

pub const KEY_ESC: u16 = 1;
pub const KEY_BACKSLASH: u16 = 43;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_LEFTALT: u16 = 56;
pub const KEY_RIGHTSHIFT: u16 = 54;
pub const KEY_RIGHTCTRL: u16 = 97;
pub const KEY_RIGHTALT: u16 = 100;
pub const KEY_LEFTMETA: u16 = 125;
pub const KEY_RIGHTMETA: u16 = 126;
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
pub const BTN_SIDE: u16 = 0x113;
pub const BTN_EXTRA: u16 = 0x114;
pub const BTN_FORWARD: u16 = 0x115;
pub const BTN_BACK: u16 = 0x116;

/// (evdev code, name without KEY_ prefix, HID usage). Usage 0 = no mapping.
const TABLE: &[(u16, &str, u8)] = &[
    (1, "ESC", 0x29),
    (2, "1", 0x1E),
    (3, "2", 0x1F),
    (4, "3", 0x20),
    (5, "4", 0x21),
    (6, "5", 0x22),
    (7, "6", 0x23),
    (8, "7", 0x24),
    (9, "8", 0x25),
    (10, "9", 0x26),
    (11, "0", 0x27),
    (12, "MINUS", 0x2D),
    (13, "EQUAL", 0x2E),
    (14, "BACKSPACE", 0x2A),
    (15, "TAB", 0x2B),
    (16, "Q", 0x14),
    (17, "W", 0x1A),
    (18, "E", 0x08),
    (19, "R", 0x15),
    (20, "T", 0x17),
    (21, "Y", 0x1C),
    (22, "U", 0x18),
    (23, "I", 0x0C),
    (24, "O", 0x12),
    (25, "P", 0x13),
    (26, "LEFTBRACE", 0x2F),
    (27, "RIGHTBRACE", 0x30),
    (28, "ENTER", 0x28),
    (29, "LEFTCTRL", 0xE0),
    (30, "A", 0x04),
    (31, "S", 0x16),
    (32, "D", 0x07),
    (33, "F", 0x09),
    (34, "G", 0x0A),
    (35, "H", 0x0B),
    (36, "J", 0x0D),
    (37, "K", 0x0E),
    (38, "L", 0x0F),
    (39, "SEMICOLON", 0x33),
    (40, "APOSTROPHE", 0x34),
    (41, "GRAVE", 0x35),
    (42, "LEFTSHIFT", 0xE1),
    (43, "BACKSLASH", 0x32),
    (44, "Z", 0x1D),
    (45, "X", 0x1B),
    (46, "C", 0x06),
    (47, "V", 0x19),
    (48, "B", 0x05),
    (49, "N", 0x11),
    (50, "M", 0x10),
    (51, "COMMA", 0x36),
    (52, "DOT", 0x37),
    (53, "SLASH", 0x38),
    (54, "RIGHTSHIFT", 0xE5),
    (55, "KPASTERISK", 0x55),
    (56, "LEFTALT", 0xE2),
    (57, "SPACE", 0x2C),
    (58, "CAPSLOCK", 0x39),
    (59, "F1", 0x3A),
    (60, "F2", 0x3B),
    (61, "F3", 0x3C),
    (62, "F4", 0x3D),
    (63, "F5", 0x3E),
    (64, "F6", 0x3F),
    (65, "F7", 0x40),
    (66, "F8", 0x41),
    (67, "F9", 0x42),
    (68, "F10", 0x43),
    (69, "NUMLOCK", 0x53),
    (70, "SCROLLLOCK", 0x47),
    (71, "KP7", 0x5F),
    (72, "KP8", 0x60),
    (73, "KP9", 0x61),
    (74, "KPMINUS", 0x56),
    (75, "KP4", 0x5C),
    (76, "KP5", 0x5D),
    (77, "KP6", 0x5E),
    (78, "KPPLUS", 0x57),
    (79, "KP1", 0x59),
    (80, "KP2", 0x5A),
    (81, "KP3", 0x5B),
    (82, "KP0", 0x62),
    (83, "KPDOT", 0x63),
    (85, "ZENKAKUHANKAKU", 0x94),
    (86, "102ND", 0x64),
    (87, "F11", 0x44),
    (88, "F12", 0x45),
    (89, "RO", 0x87),
    (90, "KATAKANA", 0x92),
    (91, "HIRAGANA", 0x93),
    (92, "HENKAN", 0x8A),
    (93, "KATAKANAHIRAGANA", 0x88),
    (94, "MUHENKAN", 0x8B),
    (95, "KPJPCOMMA", 0x8C),
    (96, "KPENTER", 0x58),
    (97, "RIGHTCTRL", 0xE4),
    (98, "KPSLASH", 0x54),
    (99, "SYSRQ", 0x46),
    (100, "RIGHTALT", 0xE6),
    (102, "HOME", 0x4A),
    (103, "UP", 0x52),
    (104, "PAGEUP", 0x4B),
    (105, "LEFT", 0x50),
    (106, "RIGHT", 0x4F),
    (107, "END", 0x4D),
    (108, "DOWN", 0x51),
    (109, "PAGEDOWN", 0x4E),
    (110, "INSERT", 0x49),
    (111, "DELETE", 0x4C),
    (113, "MUTE", 0x7F),
    (114, "VOLUMEDOWN", 0x81),
    (115, "VOLUMEUP", 0x80),
    (116, "POWER", 0x66),
    (117, "KPEQUAL", 0x67),
    (119, "PAUSE", 0x48),
    (121, "KPCOMMA", 0x85),
    (122, "HANGEUL", 0x90),
    (123, "HANJA", 0x91),
    (124, "YEN", 0x89),
    (125, "LEFTMETA", 0xE3),
    (126, "RIGHTMETA", 0xE7),
    (127, "COMPOSE", 0x65),
    (139, "MENU", 0x65),
    (183, "F13", 0x68),
    (184, "F14", 0x69),
    (185, "F15", 0x6A),
    (186, "F16", 0x6B),
    (187, "F17", 0x6C),
    (188, "F18", 0x6D),
    (189, "F19", 0x6E),
    (190, "F20", 0x6F),
    (191, "F21", 0x70),
    (192, "F22", 0x71),
    (193, "F23", 0x72),
    (194, "F24", 0x73),
];

/// Extra names that only exist for triggers and scripts (mouse buttons).
const BUTTON_NAMES: &[(u16, &str)] = &[
    (BTN_LEFT, "BTN_LEFT"),
    (BTN_RIGHT, "BTN_RIGHT"),
    (BTN_MIDDLE, "BTN_MIDDLE"),
    (BTN_SIDE, "BTN_SIDE"),
    (BTN_EXTRA, "BTN_EXTRA"),
    (BTN_FORWARD, "BTN_FORWARD"),
    (BTN_BACK, "BTN_BACK"),
];

#[derive(Clone, Copy, Debug)]
pub struct Keymap {
    /// ISO keyboard (German): KEY_BACKSLASH is the `#'` key -> 0x32.
    /// false: ANSI, KEY_BACKSLASH -> 0x31.
    pub iso: bool,
}

impl Default for Keymap {
    fn default() -> Self {
        Self { iso: true }
    }
}

impl Keymap {
    /// HID usage for an evdev key code, or None if the key is not forwarded.
    pub fn usage(&self, code: u16) -> Option<u8> {
        if code == KEY_BACKSLASH && !self.iso {
            return Some(0x31);
        }
        TABLE
            .iter()
            .find(|(c, _, _)| *c == code)
            .map(|(_, _, u)| *u)
            .filter(|u| *u != 0)
    }
}

pub fn modifier_bit(code: u16) -> Option<u8> {
    MODIFIERS.iter().find(|(c, _)| *c == code).map(|(_, b)| *b)
}

pub fn button_bit(code: u16) -> Option<u8> {
    BUTTONS.iter().find(|(c, _)| *c == code).map(|(_, b)| *b)
}

pub fn is_button(code: u16) -> bool {
    (0x100..0x160).contains(&code)
}

/// Code for a key name: "A", "KEY_A", "f12", "BTN_SIDE" (case-insensitive).
pub fn code_by_name(name: &str) -> Option<u16> {
    let upper = name.trim().to_ascii_uppercase();
    if let Some((c, _)) = BUTTON_NAMES.iter().find(|(_, n)| *n == upper) {
        return Some(*c);
    }
    let bare = upper.strip_prefix("KEY_").unwrap_or(&upper);
    TABLE.iter().find(|(_, n, _)| *n == bare).map(|(c, _, _)| *c)
}

/// Readable name of a HID usage (page 0x07), for the live view.
pub fn usage_name(usage: u8) -> String {
    TABLE
        .iter()
        .find(|(_, _, u)| *u == usage)
        .map(|(_, n, _)| n.to_string())
        .unwrap_or_else(|| format!("0x{usage:02x}"))
}

/// Readable name of a code ("A", "LEFTCTRL", "BTN_SIDE", or the number).
pub fn name_of(code: u16) -> String {
    if let Some((_, n)) = BUTTON_NAMES.iter().find(|(c, _)| *c == code) {
        return n.to_string();
    }
    TABLE
        .iter()
        .find(|(c, _, _)| *c == code)
        .map(|(_, n, _)| n.to_string())
        .unwrap_or_else(|| format!("#{code}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_iso_keys() {
        let k = Keymap::default();
        assert_eq!(k.usage(30), Some(0x04)); // A
        assert_eq!(k.usage(44), Some(0x1D)); // Z position (German Y)
        assert_eq!(k.usage(21), Some(0x1C)); // Y position (German Z)
        assert_eq!(k.usage(11), Some(0x27)); // 0
        assert_eq!(k.usage(86), Some(0x64)); // <>|
        assert_eq!(k.usage(43), Some(0x32)); // #' on ISO
        assert_eq!(Keymap { iso: false }.usage(43), Some(0x31));
        assert_eq!(k.usage(BTN_LEFT), None);
    }

    #[test]
    fn table_is_consistent() {
        let mut codes: Vec<u16> = TABLE.iter().map(|(c, _, _)| *c).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), TABLE.len(), "duplicate evdev codes");
        // Every letter A-Z has a distinct usage 0x04..=0x1D.
        let mut letters: Vec<u8> = TABLE
            .iter()
            .filter(|(_, n, _)| n.len() == 1 && n.as_bytes()[0].is_ascii_alphabetic())
            .map(|(_, _, u)| *u)
            .collect();
        letters.sort_unstable();
        assert_eq!(letters, (0x04..=0x1D).collect::<Vec<u8>>());
        // Modifier usages match the modifier bits.
        for (code, bit) in MODIFIERS {
            let usage = Keymap::default().usage(code).unwrap();
            assert_eq!(1u8 << (usage - 0xE0), bit);
        }
    }

    #[test]
    fn names() {
        assert_eq!(code_by_name("a"), Some(30));
        assert_eq!(code_by_name("KEY_F12"), Some(88));
        assert_eq!(code_by_name("btn_side"), Some(BTN_SIDE));
        assert_eq!(code_by_name("nope"), None);
        assert_eq!(name_of(29), "LEFTCTRL");
        assert_eq!(name_of(BTN_EXTRA), "BTN_EXTRA");
    }
}
