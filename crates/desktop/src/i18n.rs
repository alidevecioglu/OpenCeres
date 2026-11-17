//! Translations of the app's own text.
//!
//! Each language is a plain UTF-8 file in `locales/` (`en.txt`, …) that a
//! translator can edit without knowing Rust: `key = text` lines, `#`
//! comments, `{name}` placeholders and `\n` for a line break (see
//! `TRANSLATING.md`). The files are compiled into the app, so nothing is read
//! from disk at run time and a release cannot lose them.
//!
//! [`tr`] looks a key up in the current language, then in English; a key
//! English does not have comes back as the key itself, so a mistake shows in
//! the UI instead of crashing, and the tests below fail on it. The language
//! is global (set once from the preferences at start and again when the user
//! picks another), because every drawing function needs it and egui passes no
//! context of ours around.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Language {
    /// BCP 47 code, also the file name and the value saved in the preferences.
    pub code: &'static str,
    /// The language's name in itself, as the list shows it.
    pub endonym: &'static str,
    /// Its English name, for people who cannot read the endonym.
    pub english: &'static str,
    text: &'static str,
}

/// Languages the app ships, English first (the reference and the fallback).
pub static LANGUAGES: &[Language] = &[Language {
    code: "en",
    endonym: "English",
    english: "English",
    text: include_str!("../locales/en.txt"),
}];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Makes `code` the language of the UI; an unknown code means English.
pub fn set_language(code: &str) {
    CURRENT.store(index(code).unwrap_or(0), Ordering::Relaxed);
}

pub fn current() -> &'static Language {
    &LANGUAGES[CURRENT.load(Ordering::Relaxed).min(LANGUAGES.len() - 1)]
}

/// The language with this code, if the app has it.
pub fn find(code: &str) -> Option<&'static Language> {
    index(code).map(|i| &LANGUAGES[i])
}

fn index(code: &str) -> Option<usize> {
    LANGUAGES.iter().position(|l| l.code == code)
}

/// The text of `key` in the current language (English where it has none).
pub fn tr(key: &'static str) -> &'static str {
    get(key).unwrap_or(key)
}

/// [`tr`] with each `{name}` replaced by its value.
pub fn tr_args(key: &'static str, args: &[(&str, &str)]) -> String {
    fill(tr(key), args)
}

/// Like [`tr`] for keys made at run time; `None` if English lacks it too.
pub fn get(key: &str) -> Option<&'static str> {
    let tables = tables();
    let i = CURRENT.load(Ordering::Relaxed);
    tables
        .get(i)
        .and_then(|t| t.get(key))
        .or_else(|| tables[0].get(key))
        .map(String::as_str)
}

/// Share of English's keys that `language` translates, 0–1.
pub fn completeness(language: &Language) -> f32 {
    let tables = tables();
    let i = index(language.code).unwrap_or(0);
    let english = &tables[0];
    let done = english
        .keys()
        .filter(|k| tables[i].contains_key(*k))
        .count();
    done as f32 / english.len().max(1) as f32
}

fn tables() -> &'static [HashMap<&'static str, String>] {
    static TABLES: OnceLock<Vec<HashMap<&'static str, String>>> = OnceLock::new();
    TABLES.get_or_init(|| {
        LANGUAGES
            .iter()
            // A broken line is left out (the tests catch it) rather than
            // costing the user the whole language.
            .map(|l| parse(l.text).0.into_iter().collect())
            .collect()
    })
}

fn fill(text: &str, args: &[(&str, &str)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// The entries of a locale file, and a message for each line that is not
/// one (`line N: …`, 1-based).
fn parse(text: &'static str) -> (Vec<(&'static str, String)>, Vec<String>) {
    let mut entries: Vec<(&str, String)> = Vec::new();
    let mut errors = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            errors.push(format!("line {}: no `=`", n + 1));
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let key_ok = !key.is_empty()
            && key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._".contains(c));
        if !key_ok {
            errors.push(format!(
                "line {}: bad key `{key}` (a–z, 0–9, `.` and `_`)",
                n + 1
            ));
        } else if value.is_empty() {
            errors.push(format!("line {}: `{key}` has no text", n + 1));
        } else if entries.iter().any(|(k, _)| *k == key) {
            errors.push(format!("line {}: `{key}` appears twice", n + 1));
        } else {
            entries.push((key, unescape(value)));
        }
    }
    (entries, errors)
}

/// `\n` is a line break and `\\` a backslash; other backslashes stay.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The `{name}` placeholders of `text`, sorted, or an error for a brace
/// that does not make one.
#[cfg(test)]
fn placeholders(text: &str) -> Result<Vec<&str>, String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find(['{', '}']) {
        if rest[open..].starts_with('}') {
            return Err(format!("`}}` without `{{` in “{text}”"));
        }
        let after = &rest[open + 1..];
        let close = after
            .find('}')
            .ok_or_else(|| format!("`{{` without `}}` in “{text}”"))?;
        let name = &after[..close];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("bad placeholder `{{{name}}}` in “{text}”"));
        }
        names.push(name);
        rest = &after[close + 1..];
    }
    names.sort_unstable();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashSet};

    fn english() -> HashMap<&'static str, String> {
        parse(LANGUAGES[0].text).0.into_iter().collect()
    }

    #[test]
    fn every_locale_parses() {
        for l in LANGUAGES {
            let (entries, errors) = parse(l.text);
            assert!(errors.is_empty(), "{}.txt: {errors:#?}", l.code);
            assert!(!entries.is_empty(), "{}.txt is empty", l.code);
        }
    }

    #[test]
    fn languages_are_listed_once_with_english_first() {
        assert_eq!(LANGUAGES[0].code, "en");
        let codes: HashSet<_> = LANGUAGES.iter().map(|l| l.code).collect();
        assert_eq!(codes.len(), LANGUAGES.len());
        for l in LANGUAGES {
            assert!(!l.endonym.is_empty() && !l.english.is_empty());
            assert!(
                l.endonym
                    .chars()
                    .all(|c| c == ' ' || crate::theme::can_draw(c)),
                "the fonts cannot draw {}",
                l.endonym
            );
        }
    }

    #[test]
    fn translations_only_have_english_keys_and_the_same_placeholders() {
        let en = english();
        for l in &LANGUAGES[1..] {
            for (key, text) in parse(l.text).0 {
                let Some(source) = en.get(key) else {
                    panic!("{}.txt: `{key}` is not in en.txt", l.code);
                };
                assert_eq!(
                    placeholders(&text),
                    placeholders(source),
                    "{}.txt: `{key}` must keep the placeholders of en.txt",
                    l.code
                );
            }
        }
        for (key, text) in &en {
            assert!(placeholders(text).is_ok(), "en.txt `{key}`: {text}");
        }
    }

    /// Run with `-- --nocapture` to see it; TRANSLATING.md explains.
    #[test]
    fn completeness_per_language() {
        for l in LANGUAGES {
            let c = completeness(l);
            println!(
                "{:>6} {} ({}): {:.0} %",
                l.code,
                l.endonym,
                l.english,
                c * 100.0
            );
            assert!((0.0..=1.0).contains(&c));
        }
        assert_eq!(completeness(&LANGUAGES[0]), 1.0);
    }

    #[test]
    fn parses_escapes_comments_and_reports_bad_lines() {
        let (entries, errors) =
            parse("# note\n\na.b = One\\nTwo \\\\ {n}\nno equals\nBad = x\nc =\na.b = again\n");
        assert_eq!(entries, [("a.b", "One\nTwo \\ {n}".to_owned())]);
        assert_eq!(errors.len(), 4, "{errors:?}");
        assert!(errors[0].starts_with("line 4:"));
        assert_eq!(
            fill("{n} of {total}", &[("n", "2"), ("total", "5")]),
            "2 of 5"
        );
        assert_eq!(placeholders("{b} and {a}"), Ok(vec!["a", "b"]));
        assert!(placeholders("{oops").is_err() && placeholders("{}").is_err());
    }

    #[test]
    fn falls_back_to_english_then_to_the_key() {
        set_language("xx");
        assert_eq!(current().code, "en");
        assert_eq!(tr("onboarding.next"), "Next");
        assert_eq!(get("no.such.key"), None);
        assert_eq!(tr("no.such.key"), "no.such.key");
        assert_eq!(
            tr_args("onboarding.step", &[("n", "2"), ("total", "5")]),
            "Step 2 of 5"
        );
    }

    /// Every `tr("…")` and `tr_args("…", …)` in the source has its key in
    /// en.txt, and en.txt has no key the source does not use (`setting.*`
    /// keys belong to the settings registry and are checked in `settings`).
    #[test]
    fn english_has_exactly_the_keys_the_code_uses() {
        let mut used = BTreeSet::new();
        let mut stack = vec![std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        ))];
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                for entry in std::fs::read_dir(&path).unwrap() {
                    stack.push(entry.unwrap().path());
                }
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("i18n.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            // Without comments, so doc examples do not count; whole, since
            // rustfmt may put the key on the line after `tr_args(`.
            let code: String = source
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            for call in ["tr(", "tr_args("] {
                for (at, _) in code.match_indices(call) {
                    // `tr(` must not be the end of a longer name.
                    let before = code[..at].chars().next_back();
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let Some(rest) = code[at + call.len()..].trim_start().strip_prefix('"') else {
                        continue; // a key made at run time
                    };
                    used.insert(rest[..rest.find('"').unwrap()].to_owned());
                }
            }
        }
        let en = english();
        let missing: Vec<_> = used
            .iter()
            .filter(|k| !en.contains_key(k.as_str()))
            .collect();
        assert!(missing.is_empty(), "keys missing in en.txt: {missing:?}");
        let unused: Vec<_> = en
            .keys()
            .filter(|k| !k.starts_with("setting.") && !used.contains(**k))
            .collect();
        assert!(unused.is_empty(), "en.txt keys no code uses: {unused:?}");
    }
}
