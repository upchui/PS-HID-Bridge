//! The desktop app's texts and their German catalog (ui/i18n/de.json).
//!
//! English is written in the sources: `I18n.tr("…")` / `I18n.trf("…", …)`
//! in the QML (ui/qml), `tr("…")` / `trf("…", &[…])` in the app's Rust code
//! (ui/src). The catalog maps each English text to its German one. Checked
//! here, every problem with file:line and the text:
//!
//! 1. every text has a German entry (not empty);
//! 2. the catalog has no entry that no source uses any more;
//! 3. placeholders "%1", "%2" … are the same in a text and its translation,
//!    and a `trf` call passes as many arguments as its text uses;
//! 4. the text is a string literal (in QML literals joined with `+` count as
//!    one): `tr(name)` would hide its texts from this test;
//! 5. QML: no text property (`text: "…"`, `title:`, `help:`,
//!    `placeholderText:`, `ToolTip.text:`, `Accessible.name:` …) shows a
//!    plain string that does not go through I18n, apart from the technical
//!    names in [`ALLOWED`]. A heuristic: it reads property bindings (also
//!    in JS object literals such as `{ value: "x", text: "…" }`), not every
//!    string in the code;
//! 6. the catalog's keys are sorted and unique.
//!
//! The scan knows comments, strings with their escapes, JS template strings
//! and, roughly, JS regular expressions (a backslash outside a string
//! escapes the next character). I18n.qml and i18n.rs, which implement
//! tr/trf, are not scanned.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Plain strings a QML text property may show as they are, and why.
const ALLOWED: [(&str, &str); 8] = [
    ("PS HID Bridge", "the app's name"),
    ("PlayStation", "the console's name, the same in every language"),
    (
        "Ctrl+Alt+Shift+Esc",
        "the emergency stop in the trigger syntax (the keys as typed in a trigger)",
    ),
    ("chiaki-ng", "a program's name (the window rule's example)"),
    ("EN", "the language switch: a language's name in that language"),
    ("DE", "the language switch: a language's name in that language"),
    (
        "English",
        "the language switch: a language's name in that language",
    ),
    (
        "Deutsch",
        "the language switch: a language's name in that language",
    ),
];

/// QML properties that show text: the last part of the name ...
const TEXT_PROPS: [&str; 9] = [
    "text",
    "title",
    "subtitle",
    "help",
    "detail",
    "label",
    "placeholderText",
    "tip",
    "pillText",
];
/// ... or the whole name.
const TEXT_PATHS: [&str; 2] = ["Accessible.name", "Accessible.description"];

/// Implement tr/trf themselves: not scanned.
const SKIPPED: [&str; 2] = ["I18n.qml", "i18n.rs"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Qml,
    Rust,
}

/// A string literal: `start..end` covers the quotes.
struct Str {
    start: usize,
    end: usize,
    value: String,
    /// A JS template string; `value` holds its text with "${…}" for the code.
    template: bool,
}

struct Source {
    /// For messages, e.g. "ui/qml/BoardPage.qml".
    name: String,
    text: String,
    lang: Lang,
    /// `text` with comments and the contents of strings blanked out (same
    /// length, line breaks kept): only the code.
    code: Vec<u8>,
    strings: Vec<Str>,
    line_starts: Vec<usize>,
}

/// A call of tr/trf.
struct Call {
    source: usize,
    /// Where the function's name starts.
    at: usize,
    trf: bool,
    /// The text, if the first argument is a string literal.
    text: Option<String>,
    /// Number of arguments after the text (None: not countable, e.g. a Rust
    /// slice in a variable).
    args: Option<usize>,
    /// The parentheses.
    open: usize,
    close: usize,
}

struct Ui {
    sources: Vec<Source>,
    calls: Vec<Call>,
    /// The catalog in file order.
    catalog: Vec<(String, String)>,
}

impl Ui {
    fn load() -> Ui {
        let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui");
        let mut sources = Vec::new();
        for (dir, ext, lang) in [("qml", "qml", Lang::Qml), ("src", "rs", Lang::Rust)] {
            let mut paths: Vec<PathBuf> = fs::read_dir(ui.join(dir))
                .unwrap_or_else(|e| panic!("ui/{dir}: {e}"))
                .map(|e| e.expect("directory entry").path())
                .filter(|p| p.extension().is_some_and(|x| x == ext))
                .filter(|p| !SKIPPED.iter().any(|s| p.file_name().is_some_and(|n| n == *s)))
                .collect();
            paths.sort();
            for p in paths {
                let text = fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                let name = format!("ui/{dir}/{}", p.file_name().expect("file name").to_string_lossy());
                sources.push(Source::new(name, text, lang));
            }
        }
        let calls = sources.iter().enumerate().flat_map(|(i, s)| s.calls(i)).collect();
        let json = fs::read_to_string(ui.join("i18n/de.json")).expect("ui/i18n/de.json");
        let catalog = serde_json::from_str::<Entries>(&json)
            .unwrap_or_else(|e| panic!("ui/i18n/de.json: {e}"))
            .0;
        Ui {
            sources,
            calls,
            catalog,
        }
    }

    fn at(&self, source: usize, offset: usize) -> String {
        let s = &self.sources[source];
        format!("{}:{}", s.name, s.line(offset))
    }

    fn german(&self) -> HashMap<&str, &str> {
        self.catalog
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }
}

/// The catalog's entries in file order (a map would hide duplicates).
struct Entries(Vec<(String, String)>);

impl<'de> serde::Deserialize<'de> for Entries {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Entries;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object of strings")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Entries, A::Error> {
                let mut entries = Vec::new();
                while let Some(e) = map.next_entry()? {
                    entries.push(e);
                }
                Ok(Entries(entries))
            }
        }
        d.deserialize_map(Visitor)
    }
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn has_letter(s: &str) -> bool {
    s.chars().any(char::is_alphabetic)
}

/// The numbers of "%1", "%2" … in `s`, sorted.
fn placeholders(s: &str) -> Vec<u32> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let digits = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            if digits > 0 {
                out.push(s[i + 1..i + 1 + digits].parse().expect("digits"));
            }
            i += digits;
        }
        i += 1;
    }
    out.sort_unstable();
    out
}

/// Fails with all `problems` listed under `what`.
fn report(what: &str, problems: Vec<String>) {
    if !problems.is_empty() {
        panic!("{what} ({}):\n  {}\n", problems.len(), problems.join("\n  "));
    }
}

// ------------------------------------------------------------------ scanning

struct Scanner<'a> {
    src: &'a [u8],
    lang: Lang,
    code: Vec<u8>,
    strings: Vec<Str>,
}

impl Scanner<'_> {
    fn blank(&mut self, from: usize, to: usize) {
        for b in &mut self.code[from..to.min(self.src.len())] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    }

    fn get(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    /// Code from `i` to the end, or in a template's "${…}" to the closing
    /// brace; returns the position after it.
    fn code_until(&mut self, mut i: usize, in_template: bool) -> usize {
        let mut depth = 0usize;
        while i < self.src.len() {
            let b = self.src[i];
            let next = self.get(i + 1);
            i = match b {
                b'/' if next == b'/' => {
                    let end = self.src[i..]
                        .iter()
                        .position(|&c| c == b'\n')
                        .map_or(self.src.len(), |n| i + n);
                    self.blank(i, end);
                    end
                }
                b'/' if next == b'*' => self.block_comment(i),
                b'"' => self.string(i, b'"'),
                b'\'' if self.lang == Lang::Qml => self.string(i, b'\''),
                b'\'' => self.rust_char(i),
                b'`' if self.lang == Lang::Qml => self.template(i),
                b'\\' => i + 2,
                b'r' if self.lang == Lang::Rust => self.raw_string(i).unwrap_or(i + 1),
                b'{' => {
                    depth += 1;
                    i + 1
                }
                b'}' if in_template && depth == 0 => return i + 1,
                b'}' => {
                    depth = depth.saturating_sub(1);
                    i + 1
                }
                _ => i + 1,
            };
        }
        i
    }

    /// Rust comments nest, JS ones do not.
    fn block_comment(&mut self, start: usize) -> usize {
        let mut depth = 0;
        let mut i = start;
        while i < self.src.len() {
            if self.src[i] == b'/' && self.get(i + 1) == b'*' && (depth == 0 || self.lang == Lang::Rust) {
                depth += 1;
                i += 2;
            } else if self.src[i] == b'*' && self.get(i + 1) == b'/' {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    break;
                }
            } else {
                i += 1;
            }
        }
        self.blank(start, i);
        i
    }

    /// The escape at `i` (a backslash): its text and the position after it.
    fn escape(&self, i: usize) -> (String, usize) {
        let hex = |from: usize, to: usize| {
            std::str::from_utf8(&self.src[from..to.min(self.src.len())])
                .ok()
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .and_then(char::from_u32)
                .map(String::from)
                .unwrap_or_default()
        };
        let c = self.get(i + 1);
        match c {
            b'n' => ("\n".into(), i + 2),
            b't' => ("\t".into(), i + 2),
            b'r' => ("\r".into(), i + 2),
            b'0' => ("\0".into(), i + 2),
            b'x' => (hex(i + 2, i + 4), i + 4),
            b'u' if self.get(i + 2) == b'{' => {
                let end = self.src[i..]
                    .iter()
                    .position(|&b| b == b'}')
                    .map_or(self.src.len(), |n| i + n);
                (hex(i + 3, end), end + 1)
            }
            b'u' => (hex(i + 2, i + 6), i + 6),
            b'\n' if self.lang == Lang::Rust => {
                // Line continuation: the next line's indentation goes too.
                let skip = self.src[i + 1..]
                    .iter()
                    .take_while(|b| b.is_ascii_whitespace())
                    .count();
                (String::new(), i + 1 + skip)
            }
            b'\n' => (String::new(), i + 2),
            _ => {
                // Anything else stands for itself (\" \' \\ ...), also a
                // character of several bytes.
                let len = utf8_len(c);
                (
                    String::from_utf8_lossy(&self.src[i + 1..(i + 1 + len).min(self.src.len())]).into(),
                    i + 1 + len,
                )
            }
        }
    }

    fn string(&mut self, start: usize, quote: u8) -> usize {
        let mut value = String::new();
        let mut i = start + 1;
        let mut run = i;
        while i < self.src.len() && self.src[i] != quote {
            if self.src[i] == b'\\' {
                value.push_str(&String::from_utf8_lossy(&self.src[run..i]));
                let (text, next) = self.escape(i);
                value.push_str(&text);
                i = next;
                run = i;
            } else if self.src[i] == b'\n' && self.lang == Lang::Qml {
                break; // not closed
            } else {
                i += 1;
            }
        }
        let i = i.min(self.src.len());
        value.push_str(&String::from_utf8_lossy(&self.src[run..i]));
        self.blank(start + 1, i);
        let end = (i + 1).min(self.src.len());
        self.strings.push(Str {
            start,
            end,
            value,
            template: false,
        });
        end
    }

    fn template(&mut self, start: usize) -> usize {
        let mut value = String::new();
        let mut i = start + 1;
        while i < self.src.len() && self.src[i] != b'`' {
            if self.src[i] == b'\\' {
                let (text, next) = self.escape(i);
                value.push_str(&text);
                self.blank(i, next);
                i = next;
            } else if self.src[i] == b'$' && self.get(i + 1) == b'{' {
                value.push_str("${…}");
                i = self.code_until(i + 2, true);
            } else {
                let len = utf8_len(self.src[i]);
                value.push_str(&String::from_utf8_lossy(
                    &self.src[i..(i + len).min(self.src.len())],
                ));
                self.blank(i, i + len);
                i += len;
            }
        }
        let end = (i + 1).min(self.src.len());
        self.strings.push(Str {
            start,
            end,
            value,
            template: true,
        });
        end
    }

    /// A character literal ('x', '\n', '"'), or a lifetime ('a).
    fn rust_char(&mut self, start: usize) -> usize {
        if self.get(start + 1) == b'\\' {
            let close = self
                .src
                .get(start + 3..)
                .and_then(|rest| rest.iter().position(|&b| b == b'\''));
            let end = close.map_or(self.src.len(), |n| start + 3 + n);
            self.blank(start + 1, end);
            return (end + 1).min(self.src.len());
        }
        let len = utf8_len(self.get(start + 1));
        if self.get(start + 1 + len) == b'\'' {
            self.blank(start + 1, start + 1 + len);
            start + 2 + len
        } else {
            start + 1
        }
    }

    /// r"…", r#"…"#, br"…" (not used for texts, but they must not confuse
    /// the scan).
    fn raw_string(&mut self, start: usize) -> Option<usize> {
        let before = |n: usize| start.checked_sub(n).map(|i| self.src[i]);
        let word_start = match before(1) {
            Some(b'b') => before(2).is_none_or(|b| !is_ident(b)),
            Some(b) => !is_ident(b),
            None => true,
        };
        let hashes = self.src[start + 1..].iter().take_while(|&&b| b == b'#').count();
        if !word_start || self.get(start + 1 + hashes) != b'"' {
            return None;
        }
        let from = start + 2 + hashes;
        let mut close = vec![b'"'];
        close.extend(std::iter::repeat_n(b'#', hashes));
        let end = self.src[from..]
            .windows(close.len())
            .position(|w| w == close.as_slice())?
            + from;
        let value = String::from_utf8_lossy(&self.src[from..end]).into_owned();
        self.blank(from, end);
        let end = end + close.len();
        self.strings.push(Str {
            start,
            end,
            value,
            template: false,
        });
        Some(end)
    }
}

fn utf8_len(lead: u8) -> usize {
    match lead {
        0..0xc0 => 1,
        0xc0..0xe0 => 2,
        0xe0..0xf0 => 3,
        _ => 4,
    }
}

impl Source {
    fn new(name: String, text: String, lang: Lang) -> Source {
        let mut scanner = Scanner {
            src: text.as_bytes(),
            lang,
            code: text.as_bytes().to_vec(),
            strings: Vec::new(),
        };
        scanner.code_until(0, false);
        let Scanner {
            code, mut strings, ..
        } = scanner;
        strings.sort_by_key(|s| s.start);
        let line_starts = std::iter::once(0)
            .chain(
                text.bytes()
                    .enumerate()
                    .filter(|(_, b)| *b == b'\n')
                    .map(|(i, _)| i + 1),
            )
            .collect();
        Source {
            name,
            text,
            lang,
            code,
            strings,
            line_starts,
        }
    }

    /// 1-based line of a byte offset.
    fn line(&self, at: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= at)
    }

    fn skip_space(&self, mut i: usize) -> usize {
        while i < self.code.len() && self.code[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    }

    fn string_at(&self, at: usize) -> Option<&Str> {
        let i = self.strings.binary_search_by_key(&at, |s| s.start).ok()?;
        Some(&self.strings[i])
    }

    /// The bracket that closes the one at `open`.
    fn closing(&self, open: usize) -> usize {
        let mut depth = 0;
        for (i, &b) in self.code.iter().enumerate().skip(open) {
            match b {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            }
        }
        self.code.len()
    }

    /// Arguments between `from` (a comma or an opening bracket) and `close`,
    /// counted by the commas on their level (a trailing comma does not count).
    fn count_args(&self, from: usize, close: usize) -> usize {
        let mut depth = 0i32;
        let mut commas = 0;
        let mut last = b',';
        for &b in &self.code[from + 1..close] {
            match b {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => commas += 1,
                _ => {}
            }
            if !b.is_ascii_whitespace() {
                last = b;
            }
        }
        // "a, b" has one comma and two arguments; "a, b," too.
        if last == b',' { commas } else { commas + 1 }
    }

    /// The calls of `I18n.tr` / `I18n.trf` (QML) or `tr` / `trf` (Rust).
    fn calls(&self, source: usize) -> Vec<Call> {
        let code = &self.code;
        let mut calls = Vec::new();
        let mut i = 0;
        while i < code.len() {
            if !is_ident(code[i]) || (i > 0 && is_ident(code[i - 1])) {
                i += 1;
                continue;
            }
            let word_end = i + code[i..].iter().take_while(|&&b| is_ident(b)).count();
            let (name_at, name_end) = match self.lang {
                Lang::Qml if &code[i..word_end] == b"I18n" && code.get(word_end) == Some(&b'.') => {
                    let end =
                        word_end + 1 + code[word_end + 1..].iter().take_while(|&&b| is_ident(b)).count();
                    (word_end + 1, end)
                }
                Lang::Qml => {
                    i = word_end;
                    continue;
                }
                Lang::Rust => (i, word_end),
            };
            let name = &self.text[name_at..name_end];
            let open = self.skip_space(name_end);
            if !matches!(name, "tr" | "trf")
                || code.get(open) != Some(&b'(')
                || (self.lang == Lang::Rust && self.method_or_definition(i))
            {
                i = name_end;
                continue;
            }
            let close = self.closing(open);
            let (text, args) = self.arguments(open, close);
            calls.push(Call {
                source,
                at: i,
                trf: name == "trf",
                text,
                args,
                open,
                close,
            });
            i = open + 1;
        }
        calls
    }

    /// Rust: `x.tr(` (a method) or `fn tr(` before `i`, not a call of ours.
    fn method_or_definition(&self, i: usize) -> bool {
        let code = &self.code;
        let j = code[..i]
            .iter()
            .rposition(|b| !b.is_ascii_whitespace())
            .map_or(0, |n| n + 1);
        (j > 0 && code[j - 1] == b'.')
            || (j >= 2 && &code[j - 2..j] == b"fn" && (j == 2 || !is_ident(code[j - 3])))
    }

    /// The text (a string literal, or literals joined with "+") and the
    /// number of further arguments of the call in `open..=close`.
    fn arguments(&self, open: usize, close: usize) -> (Option<String>, Option<usize>) {
        let mut text = String::new();
        let mut at = self.skip_space(open + 1);
        loop {
            match self.string_at(at) {
                Some(s) if !s.template => text.push_str(&s.value),
                _ => return (None, None),
            }
            at = self.skip_space(self.string_at(at).expect("string").end);
            if self.code.get(at) == Some(&b'+') {
                at = self.skip_space(at + 1);
            } else {
                break;
            }
        }
        if at >= close {
            return (Some(text), Some(0));
        }
        if self.code[at] != b',' {
            return (None, None);
        }
        let args = match self.lang {
            Lang::Qml => Some(self.count_args(at, close)),
            Lang::Rust => {
                // trf("…", &[a, b]): the slice's elements.
                let slice = self.skip_space(at + 1);
                (self.code.get(slice) == Some(&b'&') && self.code.get(slice + 1) == Some(&b'[')).then(|| {
                    let end = self.closing(slice + 1);
                    let empty = self.skip_space(slice + 2) >= end;
                    if empty { 0 } else { self.count_args(slice + 1, end) }
                })
            }
        };
        (Some(text), args)
    }

    /// QML: plain strings in text properties that are not inside an I18n
    /// call (`spans`), as (offset, text).
    fn unwrapped_texts(&self, spans: &[(usize, usize)]) -> Vec<(usize, String)> {
        let code = &self.code;
        let mut found = Vec::new();
        let mut seen = BTreeSet::new();
        let mut i = 0;
        while i < code.len() {
            if !(code[i].is_ascii_alphabetic() || code[i] == b'_')
                || (i > 0 && (is_ident(code[i - 1]) || code[i - 1] == b'.'))
            {
                i += 1;
                continue;
            }
            let end = i + code[i..]
                .iter()
                .take_while(|&&b| is_ident(b) || b == b'.')
                .count();
            let path = &self.text[i..end];
            let binding = code.get(end) == Some(&b':') && code.get(end + 1) != Some(&b':');
            if !binding || !is_text_property(path) || !self.binding_starts(i) {
                i = end;
                continue;
            }
            let stop = self.binding_end(end + 1);
            for s in self.strings.iter().filter(|s| s.start > end && s.start < stop) {
                let wrapped = spans
                    .iter()
                    .any(|&(open, close)| s.start > open && s.start < close);
                if !has_letter(&s.value)
                    || wrapped
                    || self.compared(s)
                    || ALLOWED.iter().any(|(a, _)| *a == s.value)
                    || !seen.insert(s.start)
                {
                    continue;
                }
                found.push((s.start, s.value.clone()));
            }
            i = end + 1;
        }
        found
    }

    /// A binding starts at `i`: first on its line, after "{", "," or ";", or
    /// after "property string" / "property var" (not e.g. "property color").
    fn binding_starts(&self, i: usize) -> bool {
        let line_start = self.text[..i].rfind('\n').map_or(0, |n| n + 1);
        let before = String::from_utf8_lossy(&self.code[line_start..i]).into_owned();
        let before = before.trim_end();
        let words: Vec<&str> = before.split_whitespace().collect();
        match words.as_slice() {
            [.., "property", kind] => matches!(*kind, "string" | "var"),
            _ => before.is_empty() || before.ends_with(['{', ',', ';']),
        }
    }

    /// The end of the binding's expression that starts at `from`: a ",",
    /// ";" or closing bracket on its level, or the end of a line that is not
    /// continued (by an operator at its end or the start of the next one).
    fn binding_end(&self, from: usize) -> usize {
        let code = &self.code;
        let mut depth = 0;
        for (i, &b) in code.iter().enumerate().skip(from) {
            match b {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth == 0 => return i,
                b')' | b']' | b'}' => depth -= 1,
                b',' | b';' if depth == 0 => return i,
                b'\n' if depth == 0 => {
                    let last = code[from..i].iter().rev().find(|b| !b.is_ascii_whitespace());
                    let next = code[i..].iter().find(|b| !b.is_ascii_whitespace());
                    let ends_open = last.is_some_and(|b| b"+-*/?:&|=".contains(b));
                    let goes_on = next.is_some_and(|b| b"?:+-*/.&|".contains(b));
                    if !ends_open && !goes_on {
                        return i;
                    }
                }
                _ => {}
            }
        }
        code.len()
    }

    /// The string is compared (`x === "warn"`) or an index (`a["key"]`):
    /// a value, not a text.
    fn compared(&self, s: &Str) -> bool {
        let code = &self.code;
        let before: &[u8] = match code[..s.start].iter().rposition(|b| !b.is_ascii_whitespace()) {
            Some(b) => &code[b.saturating_sub(1)..=b],
            None => &[],
        };
        let after: &[u8] = match code[s.end..].iter().position(|b| !b.is_ascii_whitespace()) {
            Some(a) => &code[s.end + a..(s.end + a + 2).min(code.len())],
            None => &[],
        };
        let operator = |two: &[u8]| two == b"==" || two == b"!=";
        operator(before) || operator(after) || (before.last() == Some(&b'[') && after.first() == Some(&b']'))
    }
}

fn is_text_property(path: &str) -> bool {
    TEXT_PATHS
        .iter()
        .any(|p| path == *p || path.ends_with(&format!(".{p}")))
        || path
            .rsplit('.')
            .next()
            .is_some_and(|last| TEXT_PROPS.contains(&last))
}

// ------------------------------------------------------------------ checks

#[test]
fn every_text_has_a_translation() {
    let ui = Ui::load();
    let german = ui.german();
    let mut problems = Vec::new();
    let mut missing = BTreeSet::new();
    for c in &ui.calls {
        let Some(text) = &c.text else { continue };
        if german.get(text.as_str()).is_none_or(|t| t.is_empty()) {
            problems.push(format!("{}: {text:?}", ui.at(c.source, c.at)));
            missing.insert(text.clone());
        }
    }
    if !missing.is_empty() {
        problems.push("For ui/i18n/de.json (keys sorted):".into());
        for m in &missing {
            problems.push(format!("  {}: \"\",", serde_json::to_string(m).expect("json")));
        }
    }
    report("Texts without a German translation in ui/i18n/de.json", problems);
}

#[test]
fn no_stale_catalog_entries() {
    let ui = Ui::load();
    let used: BTreeSet<&str> = ui.calls.iter().filter_map(|c| c.text.as_deref()).collect();
    let problems = ui
        .catalog
        .iter()
        .filter(|(k, _)| !used.contains(k.as_str()))
        .map(|(k, _)| format!("{k:?}"))
        .collect();
    report(
        "Entries in ui/i18n/de.json that no QML or Rust text uses (remove them)",
        problems,
    );
}

#[test]
fn placeholders_agree() {
    let ui = Ui::load();
    let mut problems = Vec::new();
    for (en, de) in &ui.catalog {
        if placeholders(en) != placeholders(de) {
            problems.push(format!(
                "ui/i18n/de.json: {en:?} -> {de:?}: not the same %1, %2 …"
            ));
        }
    }
    for c in &ui.calls {
        let (Some(text), Some(args)) = (&c.text, c.args) else {
            continue;
        };
        let used = placeholders(text);
        let max = used.last().copied().unwrap_or(0) as usize;
        let all: BTreeSet<u32> = used.iter().copied().collect();
        let at = ui.at(c.source, c.at);
        if !c.trf && !used.is_empty() {
            problems.push(format!("{at}: {text:?} has placeholders: use trf"));
        } else if c.trf && (args != max || all.len() != max) {
            problems.push(format!(
                "{at}: {text:?} uses %1 … %{max}, the call passes {args} argument(s)"
            ));
        }
    }
    report("Placeholders that do not fit", problems);
}

#[test]
fn texts_are_string_literals() {
    let ui = Ui::load();
    let problems = ui
        .calls
        .iter()
        .filter(|c| c.text.is_none())
        .map(|c| {
            let s = &ui.sources[c.source];
            let call = s.text[c.at..(c.close + 1).min(s.text.len())]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let short: String = call.chars().take(100).collect();
            format!("{}: {short}", ui.at(c.source, c.at))
        })
        .collect();
    report(
        "tr/trf calls whose text is not a string literal (this test cannot see the texts: \
         call tr(\"…\") at each text, e.g. in a match or a lookup function)",
        problems,
    );
}

#[test]
fn qml_texts_go_through_i18n() {
    let ui = Ui::load();
    let mut problems = Vec::new();
    for (i, s) in ui.sources.iter().enumerate().filter(|(_, s)| s.lang == Lang::Qml) {
        let spans: Vec<(usize, usize)> = ui
            .calls
            .iter()
            .filter(|c| c.source == i)
            .map(|c| (c.open, c.close))
            .collect();
        for (at, text) in s.unwrapped_texts(&spans) {
            problems.push(format!("{}: {text:?}", ui.at(i, at)));
        }
    }
    report(
        "QML texts not passed through I18n.tr/trf (wrap them, or add a technical name to ALLOWED \
         in core/tests/ui_i18n.rs with the reason)",
        problems,
    );
}

#[test]
fn catalog_sorted_and_unique() {
    let ui = Ui::load();
    let problems = ui
        .catalog
        .windows(2)
        .filter(|w| w[0].0 >= w[1].0)
        .map(|w| format!("{:?} comes after {:?}", w[1].0, w[0].0))
        .collect();
    report("ui/i18n/de.json: keys not sorted, or twice", problems);
}

#[test]
fn scanner_finds_calls() {
    let qml = r#"
        // I18n.tr("in a comment")
        Label {
            text: I18n.trf("%1 of %2", a, f(b, c))
            title: I18n.tr("Joined "
                           + "text")
            subtitle: `${x} · ${I18n.tr("in a template")}`
            help: x.replace(/\/\//, "") + I18n.tr('single \'quoted\'')
            other: I18n.tr(name)
            detail: "plain"
            label: x === "compared" ? I18n.tr("A") : "B"
            tip: obj["key"]
            Accessible.name: "PlayStation"
        }
    "#;
    let s = Source::new("test.qml".into(), qml.into(), Lang::Qml);
    let calls = s.calls(0);
    let texts: Vec<Option<&str>> = calls.iter().map(|c| c.text.as_deref()).collect();
    assert_eq!(
        texts,
        [
            Some("%1 of %2"),
            Some("Joined text"),
            Some("in a template"),
            Some("single 'quoted'"),
            None,
            Some("A")
        ]
    );
    assert_eq!(calls[0].args, Some(2));
    let spans: Vec<(usize, usize)> = calls.iter().map(|c| (c.open, c.close)).collect();
    let plain: Vec<String> = s.unwrapped_texts(&spans).into_iter().map(|(_, t)| t).collect();
    assert_eq!(plain, ["plain", "B"]);

    let rust = r#"
        /// tr("doc")
        fn tr(text: &str) -> String { attr(text) }
        let a = tr("Bridge \"on\"");
        let b = trf("%1 (trying \
                     again)", &[&e]);
        let c = x.tr("method");
        let d: &'static str = if q { '"' } else { "x" };
        let e = tr(text);
    "#;
    let s = Source::new("test.rs".into(), rust.into(), Lang::Rust);
    let calls = s.calls(0);
    let texts: Vec<Option<&str>> = calls.iter().map(|c| c.text.as_deref()).collect();
    assert_eq!(texts, [Some("Bridge \"on\""), Some("%1 (trying again)"), None]);
    assert_eq!(calls[1].args, Some(1));
}
