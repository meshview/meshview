// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The API reference in the languages the site is in.
//!
//! Scalar translates its own page, but not the document it shows, so the
//! document is translated here: every `summary` and `description` in it, piece
//! by piece, from a catalog under `assets/api-lang/` that maps the English text
//! to its translation. A piece is a paragraph, a heading or an item of a list,
//! with its lines joined by spaces, which is how Markdown reads them anyway;
//! code blocks are left as they are. What a catalog lacks stays in English,
//! and a test fails while any piece lacks a translation or a catalog holds one
//! for a piece there no longer is.

use rust_embed::RustEmbed;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

#[derive(RustEmbed)]
#[folder = "assets/api-lang/"]
struct Catalogs;

/// English text to its translation.
pub type Catalog = HashMap<String, String>;

/// The languages there is a catalog for, by code, each with its catalog.
/// English, the document's own language, is not among them.
pub fn catalogs() -> BTreeMap<String, Catalog> {
    Catalogs::iter()
        .filter_map(|name| {
            let code = name.strip_suffix(".json")?.to_string();
            let file = Catalogs::get(&name)?;
            let catalog = serde_json::from_slice(&file.data)
                .unwrap_or_else(|e| panic!("assets/api-lang/{name} is not valid: {e}"));
            Some((code, catalog))
        })
        .collect()
}

/// `text` with each of its pieces replaced by what `translate` gives for it,
/// or left as it is when that is `None`.
fn pieces(text: &str, translate: &mut impl FnMut(&str) -> Option<String>) -> String {
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let trimmed = block.trim();
        if trimmed.is_empty() || trimmed.starts_with("```") {
            out.push(block.to_string());
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            let level = 1 + heading.len() - heading.trim_start_matches('#').len();
            let title = trimmed[level..].trim();
            let title = translate(title).unwrap_or_else(|| title.to_string());
            out.push(format!("{} {title}", "#".repeat(level)));
            continue;
        }
        if trimmed.starts_with("- ") {
            let mut items: Vec<Vec<&str>> = Vec::new();
            for line in trimmed.lines() {
                match line.strip_prefix("- ") {
                    Some(item) => items.push(vec![item.trim()]),
                    None => match items.last_mut() {
                        Some(item) => item.push(line.trim()),
                        None => items.push(vec![line.trim()]),
                    },
                }
            }
            let items: Vec<String> = items
                .into_iter()
                .map(|item| {
                    let item = item.join(" ");
                    format!("- {}", translate(&item).unwrap_or(item))
                })
                .collect();
            out.push(items.join("\n"));
            continue;
        }
        let paragraph = trimmed.lines().map(str::trim).collect::<Vec<_>>().join(" ");
        out.push(translate(&paragraph).unwrap_or(paragraph));
    }
    out.join("\n\n")
}

/// Every `summary` and `description` string in `document`, through `each`.
/// Examples are left alone: they are data, whatever their keys are called.
fn texts_of(document: &mut Value, each: &mut impl FnMut(&mut String)) {
    match document {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                match (key.as_str(), value) {
                    ("example" | "examples", _) => {}
                    ("summary" | "description", Value::String(text)) => each(text),
                    (_, value) => texts_of(value, each),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                texts_of(item, each);
            }
        }
        _ => {}
    }
}

/// `document` in the language of `catalog`.
pub fn translate(document: &Value, catalog: &Catalog) -> Value {
    let mut document = document.clone();
    texts_of(&mut document, &mut |text| {
        *text = pieces(text, &mut |piece| catalog.get(piece).cloned());
    });
    document
}

/// Every piece of `document` a catalog has to translate.
#[cfg(test)]
fn pieces_of(document: &Value) -> std::collections::BTreeSet<String> {
    let mut found = std::collections::BTreeSet::new();
    let mut document = document.clone();
    texts_of(&mut document, &mut |text| {
        pieces(text, &mut |piece| {
            found.insert(piece.to_string());
            None
        });
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Role;
    use serde_json::json;

    #[test]
    fn pieces_are_paragraphs_headings_and_items() {
        let text = "A first\nparagraph.\n\n## A heading\n\n- One item\n  going on.\n- Another.\n\n```sh\ncurl x\n```";
        let mut seen = Vec::new();
        let out = pieces(text, &mut |piece| {
            seen.push(piece.to_string());
            Some(piece.to_uppercase())
        });
        assert_eq!(
            seen,
            [
                "A first paragraph.",
                "A heading",
                "One item going on.",
                "Another."
            ]
        );
        assert_eq!(
            out,
            "A FIRST PARAGRAPH.\n\n## A HEADING\n\n- ONE ITEM GOING ON.\n- ANOTHER.\n\n```sh\ncurl x\n```"
        );
    }

    #[test]
    fn only_summaries_and_descriptions_are_translated() {
        let document = json!({
            "summary": "Nodes",
            "description": "The nodes.",
            "properties": {"description": {"description": "The nodes.", "example": {"description": "Nodes"}}},
            "title": "Nodes",
        });
        let catalog = Catalog::from([
            ("Nodes".into(), "Nodos".into()),
            ("The nodes.".into(), "Los nodos.".into()),
        ]);
        assert_eq!(
            translate(&document, &catalog),
            json!({
                "summary": "Nodos",
                "description": "Los nodos.",
                "properties": {"description": {"description": "Los nodos.", "example": {"description": "Nodes"}}},
                "title": "Nodes",
            })
        );
    }

    /// Every catalog translates every piece of the reference, and nothing
    /// else. On failure, what is missing is printed as JSON, to translate and
    /// add.
    #[test]
    fn catalogs_translate_the_whole_reference() {
        let (_, reference) = crate::web::assemble(Role::Run);
        let needed = pieces_of(&serde_json::to_value(reference).unwrap());
        let catalogs = catalogs();
        assert!(!catalogs.is_empty());
        for (code, catalog) in catalogs {
            let missing: BTreeMap<&String, &str> = needed
                .iter()
                .filter(|piece| !catalog.contains_key(*piece))
                .map(|piece| (piece, ""))
                .collect();
            let stale: Vec<&String> = catalog
                .keys()
                .filter(|piece| !needed.contains(*piece))
                .collect();
            let empty: Vec<&String> = catalog
                .iter()
                .filter(|(_, translation)| translation.trim().is_empty())
                .map(|(piece, _)| piece)
                .collect();
            assert!(
                missing.is_empty() && stale.is_empty() && empty.is_empty(),
                "assets/api-lang/{code}.json: {} missing, {} stale, {} empty\nmissing: {}\nstale: {stale:#?}\nempty: {empty:#?}",
                missing.len(),
                stale.len(),
                empty.len(),
                serde_json::to_string_pretty(&missing).unwrap(),
            );
        }
    }
}
