// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The web pages: the Python version's, served as it served them.
//!
//! Its templates take nothing from the request: everything a page shows it
//! fetches from the API itself. What they do take, the site's language and
//! texts and the map's tiles, is known once the configuration is read, so every
//! page is rendered once, at start, and answered from memory after that.
//!
//! The templates are Tera 2 ones, under `templates/`, and the files they use
//! under `assets/static/`; their strings are translated in the `ui` section of
//! the catalogs under `assets/lang/`. All of it is built into the binary.

use super::api::lang;
use super::api::site_config;
use crate::config::Config;
use anyhow::{Context as _, anyhow};
use axum::Router;
use axum::extract::{Path, Request};
use axum::http::StatusCode;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_TYPE, ETAG, HeaderMap, HeaderValue, IF_NONE_MATCH, LOCATION,
};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tera::{Context, Kwargs, State, Tera};

#[derive(RustEmbed)]
#[folder = "templates/"]
struct Templates;

#[derive(RustEmbed)]
#[folder = "assets/static/"]
struct Static;

/// The templates of an embedded folder, by name.
fn templates_of<E: RustEmbed>() -> anyhow::Result<Vec<(String, String)>> {
    E::iter()
        .map(|name| {
            let file = E::get(&name).expect("listed");
            let text = String::from_utf8(file.data.into_owned())
                .map_err(|_| anyhow!("template {name} is not UTF-8"))?;
            Ok((name.to_string(), text))
        })
        .collect()
}

/// The pages, by template, and whether each is the dashboard: the map with
/// the node explorer beside it.
const PAGES: [(&str, bool); 13] = [
    ("index.html", true),
    ("map.html", false),
    ("net.html", false),
    ("reliability.html", false),
    ("nodelist.html", false),
    ("firehose.html", false),
    ("chat.html", false),
    ("packet.html", false),
    ("node.html", false),
    ("nodegraph.html", false),
    ("top.html", false),
    ("stats.html", false),
    ("traceroute.html", false),
];

/// Where each page is served, as in the Python version.
const ROUTES: [(&str, &str); 16] = [
    ("/", "index.html"),
    ("/index", "index.html"),
    ("/index.html", "index.html"),
    ("/map", "map.html"),
    ("/map.html", "map.html"),
    ("/net", "net.html"),
    ("/reliability", "reliability.html"),
    ("/nodelist", "nodelist.html"),
    ("/firehose", "firehose.html"),
    ("/chat", "chat.html"),
    ("/nodegraph", "nodegraph.html"),
    ("/top", "top.html"),
    ("/stats", "stats.html"),
    ("/packet/{id}", "packet.html"),
    ("/node/{id}", "node.html"),
    ("/traceroute/{id}", "traceroute.html"),
];

/// The Python version's API pages, which the reference replaces.
const OLD_API_PAGES: [&str; 2] = ["/api.html", "/api-chat.html"];

/// The bundled libraries live in folders named after their version
/// (`leaflet-1.9.4/`), so what such a path holds never changes.
fn is_versioned_vendor(path: &str) -> bool {
    path.strip_prefix("vendor/")
        .and_then(|rest| rest.split_once('/'))
        .and_then(|(folder, _)| folder.rsplit_once('-'))
        .is_some_and(|(name, version)| {
            !name.is_empty()
                && !version.is_empty()
                && version
                    .split('.')
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        })
}

/// A value as JSON that is safe inside a `<script>` element and an HTML
/// attribute, as Jinja's `tojson` writes it.
fn to_json(value: &tera::Value) -> String {
    let json = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\'', "\\u0027")
}

/// The templates, with what they need from the site: its language, the
/// translation of each string, its tagline and the map's tiles.
fn tera(config: &Config) -> anyhow::Result<Tera> {
    let language = config.site.language.trim();
    let code = if lang::is_language_code(language) {
        language
    } else {
        "en"
    };
    let ui = lang::merged(code)
        .get("ui")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let translations: Arc<HashMap<String, String>> = Arc::new(
        ui.as_object()
            .into_iter()
            .flatten()
            .filter_map(|(source, text)| Some((source.clone(), text.as_str()?.to_string())))
            .collect(),
    );

    let site = site_config::build(config).site;
    let map_tiles = json!({
        "url": site.map_tile_url,
        "attribution": site.map_tile_attribution,
        "max_zoom": site.map_tile_max_zoom,
        "name": site.map_tile_name,
        "custom": site.map_tile_custom,
    });

    let mut tera = Tera::new();
    tera.register_filter("tr", move |source: &str, _: Kwargs, _: &State| {
        translations
            .get(source)
            .cloned()
            .unwrap_or_else(|| source.to_string())
    });
    tera.register_filter("tojson", |value: tera::Value, _: Kwargs, _: &State| {
        tera::Value::safe_string(&to_json(&value))
    });

    tera.add_raw_templates(templates_of::<Templates>()?)
        .map_err(|e| anyhow!("{e}"))?;

    let globals = tera.global_context();
    globals.insert("ui_translations", &ui);
    globals.insert("ui_language", &config.site.language);
    globals.insert("site_description", config.site.message.trim());
    globals.insert("map_tiles", &map_tiles);
    let build = super::api::version::current();
    globals.insert("build_version", build.version);
    // Nothing to show when it was built from outside a Git checkout.
    globals.insert(
        "build_revision",
        &Some(build.git_revision_short).filter(|revision| *revision != "unknown"),
    );
    globals.insert("dashboard", &false);
    globals.insert("error_message", "");
    globals.insert("error_details", "");
    Ok(tera)
}

/// Every page, rendered, by template.
struct Rendered {
    pages: HashMap<&'static str, Arc<str>>,
    not_found: Arc<str>,
}

fn render(config: &Config) -> anyhow::Result<Rendered> {
    let tera = tera(config)?;
    let mut pages = HashMap::new();
    for (template, dashboard) in PAGES {
        let mut context = Context::new();
        context.insert("dashboard", &dashboard);
        let html = tera
            .render(template, &context)
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("rendering {template}"))?;
        pages.insert(template, Arc::from(html));
    }
    let mut context = Context::new();
    context.insert("error_message", "Page not found");
    let not_found = tera
        .render("error.html", &context)
        .map_err(|e| anyhow!("{e}"))
        .context("rendering error.html")?;
    Ok(Rendered {
        pages,
        not_found: Arc::from(not_found),
    })
}

fn not_found(page: &Arc<str>) -> Response {
    (StatusCode::NOT_FOUND, Html(page.to_string())).into_response()
}

fn redirect(status: StatusCode, location: String) -> Response {
    match HeaderValue::from_str(&location) {
        Ok(location) => (status, [(LOCATION, location)]).into_response(),
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}

/// A file under `/static/`. Our own files are to be revalidated every time,
/// which their ETag makes cheap; the versioned vendor ones kept for a year.
fn static_file(path: &str, headers: &HeaderMap, not_found_page: &Arc<str>) -> Response {
    let Some(file) = Static::get(path) else {
        return not_found(not_found_page);
    };
    let tag = format!(
        "W/\"{}\"",
        file.metadata
            .sha256_hash()
            .iter()
            .take(16)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let cache = if is_versioned_vendor(path) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let etag = HeaderValue::from_str(&tag).expect("hex");
    let matches = headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|candidate| candidate.trim() == tag || candidate.trim() == "*")
        });
    if matches {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (ETAG, etag),
                (CACHE_CONTROL, HeaderValue::from_static(cache)),
            ],
        )
            .into_response();
    }
    let mime = HeaderValue::from_str(file.metadata.mimetype())
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));
    (
        [
            (CONTENT_TYPE, mime),
            (ETAG, etag),
            (CACHE_CONTROL, HeaderValue::from_static(cache)),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

/// The routes of the pages and their files.
pub fn router<S: Clone + Send + Sync + 'static>(config: &Config) -> anyhow::Result<Router<S>> {
    let rendered = Arc::new(render(config)?);
    let mut router = Router::new();

    for (route, template) in ROUTES {
        let page = Arc::clone(&rendered.pages[template]);
        router = router.route(route, get(move || async move { Html(page.to_string()) }));
    }

    // An old address of the node page.
    router =
        router.route(
            "/packet_list/{id}",
            get(|Path(id): Path<String>| async move {
                redirect(StatusCode::FOUND, format!("/node/{id}"))
            }),
        );
    // The traceroute's graph is not drawn any more; its page shows it.
    router = router.route(
        "/graph/traceroute/{id}",
        get(|Path(id): Path<String>| async move {
            redirect(StatusCode::MOVED_PERMANENTLY, format!("/traceroute/{id}"))
        }),
    );
    for old in OLD_API_PAGES {
        router = router.route(
            old,
            get(|| async { redirect(StatusCode::MOVED_PERMANENTLY, "/api/docs".into()) }),
        );
    }

    let missing = Arc::clone(&rendered.not_found);
    router = router.route(
        "/static/{*path}",
        get(
            move |Path(path): Path<String>, request: Request| async move {
                static_file(&path, request.headers(), &missing)
            },
        ),
    );

    // The pages among the static files, `/kiosk` or `/kiosk.html`, as the
    // Python version served any of them.
    let missing = Arc::clone(&rendered.not_found);
    router = router.route(
        "/{page}",
        get(move |Path(page): Path<String>| async move {
            let name = if page.ends_with(".html") {
                page
            } else {
                format!("{page}.html")
            };
            match Static::get(&name).filter(|_| !name.contains('/')) {
                Some(file) => Html(file.data.into_owned()).into_response(),
                None => not_found(&missing),
            }
        }),
    );

    let missing = Arc::clone(&rendered.not_found);
    Ok(router.fallback(move || async move { not_found(&missing) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioned_vendor_folders_are_recognised() {
        assert!(is_versioned_vendor("vendor/leaflet-1.9.4/leaflet.js"));
        assert!(is_versioned_vendor(
            "vendor/leaflet.heat-0.2.0/leaflet-heat.js"
        ));
        assert!(!is_versioned_vendor("vendor/manifest.json"));
        assert!(!is_versioned_vendor("vendor/leaflet/leaflet.js"));
        assert!(!is_versioned_vendor("theme.js"));
    }

    #[test]
    fn json_in_a_page_cannot_close_its_script() {
        let value = tera::Value::from("</script><b>'&'");
        assert_eq!(
            to_json(&value),
            "\"\\u003c/script\\u003e\\u003cb\\u003e\\u0027\\u0026\\u0027\""
        );
    }

    /// The strings `"…" | tr` translates in a template.
    fn translated_strings(template: &str) -> Vec<&str> {
        template
            .match_indices("\" | tr")
            .filter_map(|(end, _)| {
                let start = template[..end].rfind('"')? + 1;
                Some(&template[start..end])
            })
            .collect()
    }

    /// Every string a template translates is in the `ui` section of every
    /// catalog, English included, so that no page shows a string untranslated.
    #[test]
    fn every_translated_string_is_in_every_catalog() {
        let mut needed = std::collections::BTreeSet::new();
        for name in Templates::iter() {
            let file = Templates::get(&name).unwrap();
            let text = std::str::from_utf8(&file.data).unwrap().to_string();
            needed.extend(translated_strings(&text).into_iter().map(str::to_string));
        }
        assert!(needed.len() > 100);
        for code in ["en", "es", "ru"] {
            let ui = lang::merged(code)["ui"].clone();
            let missing: Vec<_> = needed
                .iter()
                .filter(|s| ui.get(s.as_str()).is_none())
                .collect();
            assert!(
                missing.is_empty(),
                "assets/lang/{code}.json ui lacks {missing:#?}"
            );
        }
    }

    #[test]
    fn the_coverage_help_is_in_the_node_page_in_its_language() {
        let mut config = Config::default();
        config.site.language = "es".into();
        let rendered = render(&config).unwrap();
        let node = &rendered.pages["node.html"];
        assert!(node.contains("<h4>Cómo funciona</h4>"));
        assert!(!node.contains("/docs/COVERAGE"));
    }

    #[test]
    fn every_page_renders_in_every_language() {
        for language in ["en", "es", "ru", "xx"] {
            let mut config = Config::default();
            config.site.language = language.into();
            let rendered = render(&config).unwrap_or_else(|e| panic!("{language}: {e:#}"));
            assert_eq!(rendered.pages.len(), PAGES.len());
        }
    }
}
