//! The app's texts: English in the source, German from `i18n/de.json`.
//!
//! The catalog maps the English text to its translation; "%1", "%2" … are
//! filled in by [`trf`]. QML uses the same catalog through the singleton
//! `I18n` (qml/I18n.qml), which gets it as JSON from the controller's
//! `translations_json`. `core/tests/ui_i18n.rs` checks that every text has a
//! translation. Messages from pshb-core (errors) stay English.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

const DE: &str = include_str!("../i18n/de.json");

static GERMAN: AtomicBool = AtomicBool::new(false);

fn german() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| serde_json::from_str(DE).unwrap_or_default())
}

/// "en" or "de"; anything else is English.
pub fn set_language(code: &str) {
    GERMAN.store(code == "de", Ordering::Release);
}

pub fn language() -> &'static str {
    if GERMAN.load(Ordering::Acquire) {
        "de"
    } else {
        "en"
    }
}

/// The catalog of the current language as JSON ("{}" for English).
pub fn catalog_json() -> &'static str {
    if GERMAN.load(Ordering::Acquire) { DE } else { "{}" }
}

/// `text` in the current language.
pub fn tr(text: &str) -> String {
    if GERMAN.load(Ordering::Acquire)
        && let Some(t) = german().get(text).filter(|t| !t.is_empty())
    {
        return t.clone();
    }
    text.to_string()
}

/// `text` in the current language with "%1", "%2" … replaced by `args`.
pub fn trf(text: &str, args: &[&str]) -> String {
    fill(&tr(text), args)
}

fn fill(text: &str, args: &[&str]) -> String {
    let mut out = text.to_string();
    // Backwards, so "%1" does not eat the start of "%10".
    for (i, a) in args.iter().enumerate().rev() {
        out = out.replace(&format!("%{}", i + 1), a);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        assert_eq!(fill("%1 of %2", &["3", "5"]), "3 of 5");
        assert_eq!(fill("%2, %1", &["a", "b"]), "b, a");
    }

    #[test]
    fn catalog_parses() {
        assert!(serde_json::from_str::<HashMap<String, String>>(DE).is_ok());
    }
}
