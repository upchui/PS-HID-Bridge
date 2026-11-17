//! Typing text on the PlayStation: characters -> the key presses a real
//! keyboard would send.
//!
//! The board sends key positions; the PlayStation turns them into characters
//! with its keyboard language setting. So the text is planned for that
//! language ([`Layout`]): `@` is Shift+2 on a US keyboard and AltGr+Q on a
//! German one. Accented letters a layout has no key for are typed with a
//! dead key (German `´` then `e` gives `é`); a dead key's own character is
//! the dead key followed by Space.
//!
//! [`plan`] makes the sequence of key states; the service sends them one
//! after the other, [`EDGE`] apart (crate::service). Text is never logged.

use std::time::Duration;

use crate::transport::KeyState;

/// Time between two key states: the PlayStation fetches the keyboard once
/// per millisecond, but text fields drop keys that come too fast.
pub const EDGE: Duration = Duration::from_millis(15);

const SHIFT: u8 = 0x02;
const ALTGR: u8 = 0x40;
const SPACE: u8 = 0x2C;

/// A keyboard language of the PlayStation.
pub struct Layout {
    pub id: &'static str,
    pub name: &'static str,
    /// Character -> (usage, modifiers).
    keys: &'static [(char, u8, u8)],
    /// Dead keys: their character -> (usage, modifiers).
    dead: &'static [(char, u8, u8)],
    /// Composed character -> (dead key's character, base character).
    compose: &'static [(char, char, char)],
}

/// The keyboard languages text can be typed for.
pub const LAYOUTS: [&Layout; 3] = [&US, &UK, &DE];

/// The layout with this id ("us", "uk", "de").
pub fn layout(id: &str) -> Option<&'static Layout> {
    LAYOUTS.iter().copied().find(|l| l.id == id)
}

/// The layout that fits a locale such as "de_DE.UTF-8" (LANG): German for
/// German-speaking locales, UK English for en_GB, else US English.
pub fn layout_for_locale(locale: &str) -> &'static Layout {
    let l = locale.to_ascii_lowercase();
    if l.starts_with("de") {
        &DE
    } else if l.starts_with("en_gb") || l.starts_with("en-gb") {
        &UK
    } else {
        &US
    }
}

/// The layout for the setting `id` ("" = from the system language).
pub fn layout_or_default(id: &str) -> &'static Layout {
    layout(id).unwrap_or_else(|| {
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
            .unwrap_or_default();
        layout_for_locale(&locale)
    })
}

impl Layout {
    fn key(&self, c: char) -> Option<(u8, u8)> {
        let c = match c {
            '\r' => return None,
            c => c,
        };
        self.keys
            .iter()
            .find(|(k, _, _)| *k == c)
            .map(|(_, u, m)| (*u, *m))
    }

    fn dead_key(&self, c: char) -> Option<(u8, u8)> {
        self.dead
            .iter()
            .find(|(k, _, _)| *k == c)
            .map(|(_, u, m)| (*u, *m))
    }

    /// Key presses for one character: [(usage, modifiers)], or None.
    fn strokes(&self, c: char) -> Option<Vec<(u8, u8)>> {
        if let Some(k) = self.key(c) {
            return Some(vec![k]);
        }
        if let Some(d) = self.dead_key(c) {
            return Some(vec![d, (SPACE, 0)]);
        }
        let (_, dead, base) = self.compose.iter().find(|(k, _, _)| *k == c)?;
        Some(vec![self.dead_key(*dead)?, self.key(*base)?])
    }

    /// Characters of `text` this layout cannot type (each once).
    pub fn unsupported(&self, text: &str) -> Vec<char> {
        let mut out = Vec::new();
        for c in text.chars() {
            if c != '\r' && self.strokes(c).is_none() && !out.contains(&c) {
                out.push(c);
            }
        }
        out
    }
}

/// The key states that type `text` (each to be held for [`EDGE`]), ending
/// with everything released. Modifiers go down before their key and up
/// after it. Err: the characters the layout cannot type.
pub fn plan(text: &str, layout: &Layout) -> Result<Vec<KeyState>, Vec<char>> {
    let missing = layout.unsupported(text);
    if !missing.is_empty() {
        return Err(missing);
    }
    let mut states = Vec::new();
    let mut push = |mods: u8, key: u8| {
        let s = KeyState {
            mods,
            keys: [key, 0, 0, 0, 0, 0],
            buttons: 0,
        };
        // (Nothing is held before the first key.)
        if states.last().copied().unwrap_or_default() != s {
            states.push(s);
        }
    };
    for c in text.chars() {
        for (usage, mods) in layout.strokes(c).unwrap_or_default() {
            push(mods, 0);
            push(mods, usage);
            push(mods, 0);
            push(0, 0);
        }
    }
    Ok(states)
}

// ------------------------------------------------------------ layouts
// Usages (HID keyboard page): a..z 0x04..0x1D, 1..9 0x1E..0x26, 0 0x27,
// Enter 0x28, Tab 0x2B, Space 0x2C, then the punctuation keys by position.

macro_rules! letters {
    ($($c:literal $u:literal),* $(,)?) => {
        [$(($c, $u, 0)),*]
    };
}

const fn upper(c: char) -> char {
    (c as u8 - b'a' + b'A') as char
}

/// a..z and A..Z, with y and z where `qwertz` puts them.
const fn alphabet(qwertz: bool) -> [(char, u8, u8); 52] {
    let mut out = [('a', 0u8, 0u8); 52];
    let mut i = 0;
    while i < 26 {
        let c = (b'a' + i as u8) as char;
        let mut u = 0x04 + i as u8;
        if qwertz && c == 'y' {
            u = 0x1D;
        } else if qwertz && c == 'z' {
            u = 0x1C;
        }
        out[i] = (c, u, 0);
        out[26 + i] = (upper(c), u, SHIFT);
        i += 1;
    }
    out
}

const ALPHA_QWERTY: [(char, u8, u8); 52] = alphabet(false);
const ALPHA_QWERTZ: [(char, u8, u8); 52] = alphabet(true);

const DIGITS: [(char, u8, u8); 10] = letters![
    '1' 0x1E, '2' 0x1F, '3' 0x20, '4' 0x21, '5' 0x22, '6' 0x23, '7' 0x24, '8' 0x25, '9' 0x26, '0' 0x27,
];

const COMMON: [(char, u8, u8); 3] = [('\n', 0x28, 0), ('\t', 0x2B, 0), (' ', 0x2C, 0)];

const US_PUNCT: [(char, u8, u8); 32] = [
    ('!', 0x1E, SHIFT),
    ('@', 0x1F, SHIFT),
    ('#', 0x20, SHIFT),
    ('$', 0x21, SHIFT),
    ('%', 0x22, SHIFT),
    ('^', 0x23, SHIFT),
    ('&', 0x24, SHIFT),
    ('*', 0x25, SHIFT),
    ('(', 0x26, SHIFT),
    (')', 0x27, SHIFT),
    ('-', 0x2D, 0),
    ('_', 0x2D, SHIFT),
    ('=', 0x2E, 0),
    ('+', 0x2E, SHIFT),
    ('[', 0x2F, 0),
    ('{', 0x2F, SHIFT),
    (']', 0x30, 0),
    ('}', 0x30, SHIFT),
    ('\\', 0x31, 0),
    ('|', 0x31, SHIFT),
    (';', 0x33, 0),
    (':', 0x33, SHIFT),
    ('\'', 0x34, 0),
    ('"', 0x34, SHIFT),
    ('`', 0x35, 0),
    ('~', 0x35, SHIFT),
    (',', 0x36, 0),
    ('<', 0x36, SHIFT),
    ('.', 0x37, 0),
    ('>', 0x37, SHIFT),
    ('/', 0x38, 0),
    ('?', 0x38, SHIFT),
];

const UK_PUNCT: [(char, u8, u8); 35] = [
    ('!', 0x1E, SHIFT),
    ('"', 0x1F, SHIFT),
    ('£', 0x20, SHIFT),
    ('$', 0x21, SHIFT),
    ('€', 0x21, ALTGR),
    ('%', 0x22, SHIFT),
    ('^', 0x23, SHIFT),
    ('&', 0x24, SHIFT),
    ('*', 0x25, SHIFT),
    ('(', 0x26, SHIFT),
    (')', 0x27, SHIFT),
    ('-', 0x2D, 0),
    ('_', 0x2D, SHIFT),
    ('=', 0x2E, 0),
    ('+', 0x2E, SHIFT),
    ('[', 0x2F, 0),
    ('{', 0x2F, SHIFT),
    (']', 0x30, 0),
    ('}', 0x30, SHIFT),
    ('#', 0x32, 0),
    ('~', 0x32, SHIFT),
    (';', 0x33, 0),
    (':', 0x33, SHIFT),
    ('\'', 0x34, 0),
    ('@', 0x34, SHIFT),
    ('`', 0x35, 0),
    ('¬', 0x35, SHIFT),
    (',', 0x36, 0),
    ('<', 0x36, SHIFT),
    ('.', 0x37, 0),
    ('>', 0x37, SHIFT),
    ('/', 0x38, 0),
    ('?', 0x38, SHIFT),
    ('\\', 0x64, 0),
    ('|', 0x64, SHIFT),
];

const DE_PUNCT: [(char, u8, u8); 43] = [
    ('!', 0x1E, SHIFT),
    ('"', 0x1F, SHIFT),
    ('²', 0x1F, ALTGR),
    ('§', 0x20, SHIFT),
    ('³', 0x20, ALTGR),
    ('$', 0x21, SHIFT),
    ('%', 0x22, SHIFT),
    ('&', 0x23, SHIFT),
    ('/', 0x24, SHIFT),
    ('{', 0x24, ALTGR),
    ('(', 0x25, SHIFT),
    ('[', 0x25, ALTGR),
    (')', 0x26, SHIFT),
    (']', 0x26, ALTGR),
    ('=', 0x27, SHIFT),
    ('}', 0x27, ALTGR),
    ('ß', 0x2D, 0),
    ('?', 0x2D, SHIFT),
    ('\\', 0x2D, ALTGR),
    ('ü', 0x2F, 0),
    ('Ü', 0x2F, SHIFT),
    ('+', 0x30, 0),
    ('*', 0x30, SHIFT),
    ('~', 0x30, ALTGR),
    ('#', 0x32, 0),
    ('\'', 0x32, SHIFT),
    ('ö', 0x33, 0),
    ('Ö', 0x33, SHIFT),
    ('ä', 0x34, 0),
    ('Ä', 0x34, SHIFT),
    ('°', 0x35, SHIFT),
    (',', 0x36, 0),
    (';', 0x36, SHIFT),
    ('.', 0x37, 0),
    (':', 0x37, SHIFT),
    ('-', 0x38, 0),
    ('_', 0x38, SHIFT),
    ('<', 0x64, 0),
    ('>', 0x64, SHIFT),
    ('|', 0x64, ALTGR),
    ('@', 0x14, ALTGR),
    ('€', 0x08, ALTGR),
    ('µ', 0x10, ALTGR),
];

const DE_DEAD: [(char, u8, u8); 3] = [('^', 0x35, 0), ('´', 0x2E, 0), ('`', 0x2E, SHIFT)];

const DE_COMPOSE: [(char, char, char); 33] = [
    ('â', '^', 'a'),
    ('ê', '^', 'e'),
    ('î', '^', 'i'),
    ('ô', '^', 'o'),
    ('û', '^', 'u'),
    ('Â', '^', 'A'),
    ('Ê', '^', 'E'),
    ('Î', '^', 'I'),
    ('Ô', '^', 'O'),
    ('Û', '^', 'U'),
    ('á', '´', 'a'),
    ('é', '´', 'e'),
    ('í', '´', 'i'),
    ('ó', '´', 'o'),
    ('ú', '´', 'u'),
    ('ý', '´', 'y'),
    ('Á', '´', 'A'),
    ('É', '´', 'E'),
    ('Í', '´', 'I'),
    ('Ó', '´', 'O'),
    ('Ú', '´', 'U'),
    ('Ý', '´', 'Y'),
    ('à', '`', 'a'),
    ('è', '`', 'e'),
    ('ì', '`', 'i'),
    ('ò', '`', 'o'),
    ('ù', '`', 'u'),
    ('À', '`', 'A'),
    ('È', '`', 'E'),
    ('Ì', '`', 'I'),
    ('Ò', '`', 'O'),
    ('Ù', '`', 'U'),
    ('ŷ', '^', 'y'),
];

const fn concat<const N: usize>(parts: &[&[(char, u8, u8)]]) -> [(char, u8, u8); N] {
    let mut out = [('\0', 0u8, 0u8); N];
    let mut n = 0;
    let mut p = 0;
    while p < parts.len() {
        let mut i = 0;
        while i < parts[p].len() {
            out[n] = parts[p][i];
            n += 1;
            i += 1;
        }
        p += 1;
    }
    assert!(n == N, "table size");
    out
}

const US_KEYS: [(char, u8, u8); 97] = concat(&[&ALPHA_QWERTY, &DIGITS, &COMMON, &US_PUNCT]);
const UK_KEYS: [(char, u8, u8); 100] = concat(&[&ALPHA_QWERTY, &DIGITS, &COMMON, &UK_PUNCT]);
const DE_KEYS: [(char, u8, u8); 108] = concat(&[&ALPHA_QWERTZ, &DIGITS, &COMMON, &DE_PUNCT]);

pub static US: Layout = Layout {
    id: "us",
    name: "English (US)",
    keys: &US_KEYS,
    dead: &[],
    compose: &[],
};

pub static UK: Layout = Layout {
    id: "uk",
    name: "English (UK)",
    keys: &UK_KEYS,
    dead: &[],
    compose: &[],
};

pub static DE: Layout = Layout {
    id: "de",
    name: "Deutsch",
    keys: &DE_KEYS,
    dead: &DE_DEAD,
    compose: &DE_COMPOSE,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(states: &[KeyState]) -> Vec<(u8, u8)> {
        states
            .iter()
            .filter(|s| s.keys[0] != 0)
            .map(|s| (s.mods, s.keys[0]))
            .collect()
    }

    #[test]
    fn every_printable_ascii_character_in_every_layout() {
        let ascii: String = (0x20u8..0x7f).map(char::from).collect();
        for l in LAYOUTS {
            assert_eq!(l.unsupported(&ascii), Vec::<char>::new(), "{}", l.id);
            assert!(plan(&ascii, l).is_ok());
        }
    }

    #[test]
    fn no_character_twice_in_a_table() {
        for l in LAYOUTS {
            for (i, (c, _, _)) in l.keys.iter().enumerate() {
                assert!(
                    !l.keys[i + 1..].iter().any(|(d, _, _)| d == c),
                    "{} has {c:?} twice",
                    l.id
                );
            }
        }
    }

    #[test]
    fn modifiers_wrap_their_key() {
        let s = plan("A", &US).unwrap();
        let m = |mods, key| KeyState {
            mods,
            keys: [key, 0, 0, 0, 0, 0],
            buttons: 0,
        };
        assert_eq!(s, vec![m(SHIFT, 0), m(SHIFT, 0x04), m(SHIFT, 0), m(0, 0)]);
        // Without a modifier: down, up.
        assert_eq!(
            plan("aa", &US).unwrap(),
            vec![m(0, 0x04), m(0, 0), m(0, 0x04), m(0, 0)]
        );
        assert_eq!(plan("", &US).unwrap(), vec![]);
    }

    #[test]
    fn the_same_character_differs_by_layout() {
        assert_eq!(keys(&plan("@", &US).unwrap()), vec![(SHIFT, 0x1F)]);
        assert_eq!(keys(&plan("@", &UK).unwrap()), vec![(SHIFT, 0x34)]);
        assert_eq!(keys(&plan("@", &DE).unwrap()), vec![(ALTGR, 0x14)]);
        assert_eq!(keys(&plan("zy", &DE).unwrap()), vec![(0, 0x1C), (0, 0x1D)]);
        assert_eq!(keys(&plan("#", &DE).unwrap()), vec![(0, 0x32)]);
        assert_eq!(keys(&plan("<|", &DE).unwrap()), vec![(0, 0x64), (ALTGR, 0x64)]);
        assert_eq!(keys(&plan("\\", &UK).unwrap()), vec![(0, 0x64)]);
    }

    #[test]
    fn german_umlauts_and_dead_keys() {
        assert!(DE.unsupported("Grüße aus Köln, 30 °C, 5 € — ").contains(&'—'));
        assert_eq!(keys(&plan("ä", &DE).unwrap()), vec![(0, 0x34)]);
        // é: dead acute, then e.
        assert_eq!(keys(&plan("é", &DE).unwrap()), vec![(0, 0x2E), (0, 0x08)]);
        // È: dead grave (Shift), then Shift+E.
        assert_eq!(keys(&plan("È", &DE).unwrap()), vec![(SHIFT, 0x2E), (SHIFT, 0x08)]);
        // ^ alone: the dead key, then Space.
        assert_eq!(keys(&plan("^", &DE).unwrap()), vec![(0, 0x35), (0, 0x2C)]);
        assert_eq!(US.unsupported("é"), vec!['é']);
    }

    #[test]
    fn line_breaks_and_unsupported_characters() {
        assert_eq!(
            keys(&plan("a\r\nb", &US).unwrap()),
            vec![(0, 0x04), (0, 0x28), (0, 0x05)]
        );
        assert_eq!(plan("a😀b😀", &US).unwrap_err(), vec!['😀']);
    }

    #[test]
    fn layout_from_the_system_language() {
        assert_eq!(layout_for_locale("de_DE.UTF-8").id, "de");
        assert_eq!(layout_for_locale("de_AT.UTF-8").id, "de");
        assert_eq!(layout_for_locale("en_GB.UTF-8").id, "uk");
        assert_eq!(layout_for_locale("en_US.UTF-8").id, "us");
        assert_eq!(layout_for_locale("C").id, "us");
        assert_eq!(layout("de").map(|l| l.name), Some("Deutsch"));
        assert!(layout("xx").is_none());
    }
}
