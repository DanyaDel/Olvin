//! Localisation: string tables keyed by name, with a fallback language.
//!
//! Tables are RON maps, one file per language in `assets/locales/`
//! (`en.ron`, `ru.ron`, `pt-BR.ron`):
//!
//! ```ron
//! {
//!     "menu.play": "Play",
//!     "hello": "Hello, {name}!",
//!     "coins.one": "{n} coin",
//!     "coins.other": "{n} coins",
//! }
//! ```
//!
//! A key is looked up in the current language, then in the fallback language,
//! then the key itself is returned (so a missing string shows up on screen and
//! is logged once). `{name}` placeholders are filled by [`Locale::tr_with`];
//! plural forms are keys with a `.one` / `.few` / `.many` / `.other` suffix,
//! picked by [`Locale::plural`] with the rules of the language.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use olvin_core::{App, Plugin, ProjectPaths};

/// One language's strings.
pub type Table = BTreeMap<String, String>;

/// Resource: every loaded language and which one is shown.
#[derive(Clone, Debug)]
pub struct Locale {
    tables: BTreeMap<String, Table>,
    current: String,
    fallback: String,
    /// Keys already reported as missing, so the log gets each one once.
    reported: std::cell::RefCell<BTreeSet<String>>,
}

impl Default for Locale {
    fn default() -> Self {
        Self::new("en")
    }
}

impl Locale {
    /// No tables yet; `fallback` is also the starting language.
    pub fn new(fallback: &str) -> Self {
        Self {
            tables: BTreeMap::new(),
            current: fallback.to_owned(),
            fallback: fallback.to_owned(),
            reported: Default::default(),
        }
    }

    /// Adds (or replaces) a language from RON text.
    pub fn add_ron(&mut self, language: &str, text: &str) -> Result<(), String> {
        let table: Table = ron::from_str(text).map_err(|e| format!("{language}: {e}"))?;
        self.tables.insert(language.to_owned(), table);
        Ok(())
    }

    /// Adds a language from a ready table (tests, strings built in code).
    pub fn add_table(&mut self, language: &str, table: Table) {
        self.tables.insert(language.to_owned(), table);
    }

    /// Loads every `*.ron` in `dir`; the file stem is the language. A broken
    /// file is logged and skipped. Returns how many languages were loaded.
    pub fn load_dir(&mut self, dir: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let mut loaded = 0;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("ron") {
                continue;
            }
            let Some(language) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let result = std::fs::read_to_string(&path)
                .map_err(|e| format!("{}: {e}", path.display()))
                .and_then(|text| self.add_ron(language, &text));
            match result {
                Ok(()) => loaded += 1,
                Err(e) => log::warn!("locale file skipped: {e}"),
            }
        }
        loaded
    }

    /// Languages with a table, sorted.
    pub fn languages(&self) -> Vec<&str> {
        self.tables.keys().map(String::as_str).collect()
    }

    pub fn language(&self) -> &str {
        &self.current
    }

    pub fn fallback(&self) -> &str {
        &self.fallback
    }

    /// Switches to `tag` (`ru`, `pt-BR`, `ru_RU.UTF-8`). A regional tag with no
    /// table of its own uses the base language (`pt-BR` → `pt`). Returns false
    /// and keeps the current language if nothing matches.
    pub fn set_language(&mut self, tag: &str) -> bool {
        match self.resolve(tag) {
            Some(found) => {
                self.current = found;
                true
            }
            None => false,
        }
    }

    /// The loaded language that `tag` selects, if any.
    pub fn resolve(&self, tag: &str) -> Option<String> {
        let tag = normalize(tag);
        if self.tables.contains_key(&tag) {
            return Some(tag);
        }
        let base = tag.split('-').next().unwrap_or(&tag);
        self.tables
            .keys()
            .find(|k| k.eq_ignore_ascii_case(&tag))
            .or_else(|| self.tables.keys().find(|k| k.eq_ignore_ascii_case(base)))
            .cloned()
    }

    /// The string for `key`: current language, fallback, then the key itself.
    pub fn tr(&self, key: &str) -> String {
        self.lookup(key).map_or_else(
            || {
                self.report_missing(key);
                key.to_owned()
            },
            str::to_owned,
        )
    }

    /// Like [`Locale::tr`], with `{name}` placeholders filled from `args`.
    /// An unknown placeholder stays as written.
    pub fn tr_with(&self, key: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
        fill(&self.tr(key), args)
    }

    /// The plural form of `key` for `count` (`coins` → `coins.one`, `coins.few`,
    /// ...), with `{n}` set to the count and `args` filled in. Falls back to
    /// `.other`, then to the bare key.
    pub fn plural(&self, key: &str, count: i64, args: &[(&str, &dyn std::fmt::Display)]) -> String {
        let form = plural_form(&self.current, count);
        let text = self
            .lookup(&format!("{key}.{form}"))
            .or_else(|| self.lookup(&format!("{key}.other")))
            .or_else(|| self.lookup(key));
        let Some(text) = text else {
            self.report_missing(&format!("{key}.{form}"));
            return key.to_owned();
        };
        let mut all: Vec<(&str, &dyn std::fmt::Display)> = vec![("n", &count)];
        all.extend_from_slice(args);
        fill(text, &all)
    }

    /// Whether `key` has a string in the current or fallback language.
    pub fn has(&self, key: &str) -> bool {
        self.lookup(key).is_some()
    }

    /// Keys that `language` lacks compared with the fallback: the checklist of
    /// a translator. Empty for the fallback itself.
    pub fn missing_in(&self, language: &str) -> Vec<&str> {
        let (Some(base), Some(other)) =
            (self.tables.get(&self.fallback), self.tables.get(language))
        else {
            return Vec::new();
        };
        base.keys()
            .filter(|k| !other.contains_key(*k))
            .map(String::as_str)
            .collect()
    }

    fn lookup(&self, key: &str) -> Option<&str> {
        [&self.current, &self.fallback]
            .into_iter()
            .find_map(|lang| self.tables.get(lang)?.get(key))
            .map(String::as_str)
    }

    fn report_missing(&self, key: &str) {
        if self.reported.borrow_mut().insert(key.to_owned()) {
            log::warn!(
                "no string for `{key}` in `{}` or `{}`",
                self.current,
                self.fallback
            );
        }
    }
}

/// `ru_RU.UTF-8` → `ru-RU`, `en_US` → `en-US`, `C` and `POSIX` → empty.
fn normalize(tag: &str) -> String {
    let tag = tag.split(['.', '@']).next().unwrap_or("").replace('_', "-");
    if tag.eq_ignore_ascii_case("c") || tag.eq_ignore_ascii_case("posix") {
        String::new()
    } else {
        tag
    }
}

/// `{name}` → the value from `args`; `{{` and `}}` are literal braces.
fn fill(text: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '{' if matches!(chars.peek(), Some((_, '{'))) => {
                chars.next();
                out.push('{');
            }
            '}' if matches!(chars.peek(), Some((_, '}'))) => {
                chars.next();
                out.push('}');
            }
            '{' => match text[i + 1..].find('}') {
                Some(len) => {
                    let name = &text[i + 1..i + 1 + len];
                    match args.iter().find(|(k, _)| *k == name) {
                        Some((_, v)) => out.push_str(&v.to_string()),
                        None => out.push_str(&text[i..i + len + 2]),
                    }
                    for _ in 0..len + 1 {
                        chars.next();
                    }
                }
                None => out.push(c),
            },
            _ => out.push(c),
        }
    }
    out
}

/// The CLDR plural category (`zero` is not used) of `count` in `language`.
pub fn plural_form(language: &str, count: i64) -> &'static str {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    let n = count.unsigned_abs();
    match base {
        // One form for everything.
        "ja" | "zh" | "ko" | "vi" | "th" | "id" | "ms" => "other",
        // East Slavic: 1, 21, 31... one; 2-4, 22-24... few; the rest many.
        "ru" | "uk" | "be" => match (n % 10, n % 100) {
            (1, r) if r != 11 => "one",
            (2..=4, r) if !(12..=14).contains(&r) => "few",
            _ => "many",
        },
        "pl" => match (n, n % 10, n % 100) {
            (1, _, _) => "one",
            (_, 2..=4, r) if !(12..=14).contains(&r) => "few",
            _ => "many",
        },
        "cs" | "sk" => match n {
            1 => "one",
            2..=4 => "few",
            _ => "other",
        },
        // 0 and 1 are singular.
        "fr" | "pt" => {
            if n <= 1 {
                "one"
            } else {
                "other"
            }
        }
        _ => {
            if n == 1 {
                "one"
            } else {
                "other"
            }
        }
    }
}

/// The language the system asks for: `OLVIN_LANG`, then `LC_ALL`,
/// `LC_MESSAGES`, `LANG`. `None` when nothing is set or it is `C`/`POSIX`.
pub fn system_language() -> Option<String> {
    ["OLVIN_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .map(|v| normalize(&v))
        .find(|v| !v.is_empty())
}

/// Loads `assets/<dir>/*.ron` and picks the system language.
pub struct LocalePlugin {
    /// Relative to the assets folder.
    pub dir: String,
    pub fallback: String,
    /// Overrides the system language (a setting the player chose).
    pub language: Option<String>,
}

impl LocalePlugin {
    /// Tables in `assets/locales`, fallback `en`.
    pub fn new() -> Self {
        Self {
            dir: "locales".into(),
            fallback: "en".into(),
            language: None,
        }
    }

    pub fn fallback(mut self, language: &str) -> Self {
        self.fallback = language.to_owned();
        self
    }

    pub fn language(mut self, language: &str) -> Self {
        self.language = Some(language.to_owned());
        self
    }

    pub fn dir(mut self, dir: &str) -> Self {
        self.dir = dir.to_owned();
        self
    }
}

impl Default for LocalePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for LocalePlugin {
    fn build(&self, app: &mut App) {
        let assets = app.resources.fetch::<ProjectPaths>().assets.clone();
        let mut locale = Locale::new(&self.fallback);
        let loaded = locale.load_dir(&assets.join(&self.dir));
        let wanted = self.language.clone().or_else(system_language);
        if let Some(tag) = wanted
            && !locale.set_language(&tag)
            && loaded > 0
        {
            log::info!("no table for `{tag}`, showing `{}`", locale.fallback());
        }
        log::info!(
            "locale: {loaded} language(s), showing `{}`",
            locale.language()
        );
        app.insert_resource(locale);
    }
}

#[cfg(test)]
mod tests;
