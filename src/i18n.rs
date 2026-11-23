//! Tiny i18n: English source strings, JSON tables per language, system language by default.
use std::collections::HashMap;
use std::sync::RwLock;

pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("ru", "Русский"),
    ("uk", "Українська"),
    ("de", "Deutsch"),
    ("es", "Español"),
    ("fr", "Français"),
    ("zh", "中文"),
    ("ja", "日本語"),
    ("ko", "한국어"),
];

fn table_src(code: &str) -> Option<&'static str> {
    Some(match code {
        "ru" => include_str!("../locales/ru.json"),
        "uk" => include_str!("../locales/uk.json"),
        "de" => include_str!("../locales/de.json"),
        "es" => include_str!("../locales/es.json"),
        "fr" => include_str!("../locales/fr.json"),
        "zh" => include_str!("../locales/zh.json"),
        "ja" => include_str!("../locales/ja.json"),
        "ko" => include_str!("../locales/ko.json"),
        _ => return None,
    })
}

struct State {
    code: &'static str,
    table: HashMap<String, &'static str>,
}

static STATE: RwLock<Option<State>> = RwLock::new(None);

/// First language named by the usual POSIX variables; unsupported ones fall back to English.
pub fn detect() -> &'static str {
    for var in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        let v = std::env::var(var).unwrap_or_default();
        for item in v.split(':') {
            let code = item.split('.').next().unwrap_or("").split('@').next().unwrap_or("");
            let code = code.split('_').next().unwrap_or("").to_lowercase();
            if code.is_empty() || code == "c" || code == "posix" {
                continue;
            }
            return LANGUAGES.iter().find(|(c, _)| *c == code).map_or("en", |(c, _)| c);
        }
    }
    "en"
}

pub fn name_of(code: &str) -> &'static str {
    LANGUAGES.iter().find(|(c, _)| *c == code).map_or("English", |(_, n)| n)
}

pub fn setup(lang: &str) -> &'static str {
    let code = if lang.is_empty() || lang == "auto" {
        detect()
    } else {
        lang
    };
    let code = LANGUAGES.iter().find(|(c, _)| *c == code).map_or("en", |(c, _)| *c);
    let mut table = HashMap::new();
    if let Some(src) = table_src(code) {
        // leaked once per language switch: a few KB, and &'static str keeps tr() allocation-free
        let src: &'static str = src;
        if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(src) {
            for (k, v) in map {
                table.insert(k, &*Box::leak(v.into_boxed_str()));
            }
        }
    }
    *STATE.write().unwrap() = Some(State { code, table });
    code
}

pub fn current() -> &'static str {
    STATE.read().unwrap().as_ref().map_or("en", |s| s.code)
}

/// Translate an English source string (returns it unchanged when there is no entry).
pub fn tr(text: &str) -> String {
    let g = STATE.read().unwrap();
    match g.as_ref().and_then(|s| s.table.get(text)) {
        Some(t) => (*t).to_string(),
        None => text.to_string(),
    }
}

/// tr() + "{name}" substitution.
pub fn trf(text: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut s = tr(text);
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), &v.to_string());
    }
    s
}
