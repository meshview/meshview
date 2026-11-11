// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The web pages' translations.
//!
//! The catalogs are the Python version's, under `assets/lang/`, and are built
//! into the binary. A language's catalog is English with that language's strings
//! laid over it, so that a string not translated yet shows in English rather
//! than not at all.

use super::error::{ApiError, ErrorBody};
use super::params::Params;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use rust_embed::RustEmbed;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[derive(RustEmbed)]
#[folder = "assets/lang/"]
struct Catalogs;

/// Merged catalogs, by language, built the first time they are asked for.
static MERGED: LazyLock<Mutex<HashMap<String, Value>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Two or three letters, then optionally a region or script, as `es`, `pt_BR` or
/// `zh-Hant`. Anything else is taken as English.
pub(crate) fn is_language_code(code: &str) -> bool {
    // A single trailing newline passes too, as with the Python version's regex.
    let code = code.strip_suffix('\n').unwrap_or(code);
    let (language, rest) = match code.find(['-', '_']) {
        Some(at) => (&code[..at], Some(&code[at + 1..])),
        None => (code, None),
    };
    (2..=3).contains(&language.len())
        && language.bytes().all(|b| b.is_ascii_alphabetic())
        && rest.is_none_or(|rest| {
            (2..=8).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

fn read(code: &str) -> Option<Map<String, Value>> {
    let file = Catalogs::get(&format!("{code}.json"))?;
    match serde_json::from_slice(&file.data) {
        Ok(Value::Object(catalog)) => Some(catalog),
        _ => None,
    }
}

/// English, with `code`'s strings laid over it, section by section; sections
/// only `code` has are added as they are.
pub(crate) fn merged(code: &str) -> Value {
    if let Some(catalog) = MERGED.lock().expect("never poisoned").get(code) {
        return catalog.clone();
    }

    let english = read("en").unwrap_or_default();
    let catalog = match (code, read(code)) {
        ("en", _) | (_, None) => english,
        (_, Some(selected)) => {
            let mut out = Map::new();
            for (section, english_values) in &english {
                let mut values = english_values.as_object().cloned().unwrap_or_default();
                if let Some(Value::Object(translated)) = selected.get(section) {
                    values.extend(translated.clone());
                }
                out.insert(section.clone(), Value::Object(values));
            }
            for (section, values) in selected {
                out.entry(section).or_insert(values);
            }
            out
        }
    };

    let catalog = Value::Object(catalog);
    MERGED
        .lock()
        .expect("never poisoned")
        .insert(code.to_string(), catalog.clone());
    catalog
}

/// Translations
///
/// The catalog of the web pages' strings in one language, or one section of
/// it. Strings the language has no translation for are in English.
///
/// The answer is the catalog itself, or the section, with nothing around it:
/// an object of sections, each an object of strings by key.
#[utoipa::path(
    get,
    path = "/lang",
    tag = "meta",
    params(
        ("lang" = Option<String>, Query, description = "A language code, such as `es`. Default: `[site] language`. One there is no catalog for gets English."),
        ("section" = Option<String>, Query, description = "Only this section of the catalog, such as `top`."),
    ),
    responses(
        (status = 200, description = "The catalog, or the section asked for", body = Object, example = json!({
            "title": "Meshview",
            "search_placeholder": "Search...",
        })),
        (status = 404, description = "No such section", body = ErrorBody),
    )
)]
pub async fn lang(State(state): State<AppState>, params: Params) -> Result<Json<Value>, ApiError> {
    let requested = match params.get("lang") {
        None | Some("") => state.config.site.language.as_str(),
        Some(code) => code,
    };
    let code = if is_language_code(requested) {
        requested
    } else {
        "en"
    };
    // The catalog served is English when there is none for the code, but an
    // error names the code that was asked for, as in the Python version.
    let served = if Catalogs::get(&format!("{code}.json")).is_some() {
        code
    } else {
        "en"
    };
    let catalog = merged(served);

    match params.get("section").filter(|section| !section.is_empty()) {
        None => Ok(Json(catalog)),
        Some(section) => {
            let section = section.to_lowercase();
            catalog.get(&section).cloned().map(Json).ok_or_else(|| {
                ApiError::not_found(format!("Section '{section}' not found in {code}"))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_are_checked_as_python_checks_them() {
        for good in ["en", "es", "pt_BR", "zh-Hant", "ast", "en\n"] {
            assert!(is_language_code(good), "{good:?}");
        }
        for bad in ["e", "engl", "es-", "es-x", "../etc", "es_ES_x", "", "e1"] {
            assert!(!is_language_code(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_translation_falls_back_to_english_string_by_string() {
        let english = merged("en");
        let spanish = merged("es");
        assert_eq!(
            english.as_object().unwrap().keys().collect::<Vec<_>>(),
            spanish
                .as_object()
                .unwrap()
                .keys()
                .filter(|key| english.get(key.as_str()).is_some())
                .collect::<Vec<_>>()
        );
        for (section, values) in english.as_object().unwrap() {
            for key in values.as_object().unwrap().keys() {
                assert!(spanish[section].get(key).is_some(), "{section}.{key}");
            }
        }
    }
}
