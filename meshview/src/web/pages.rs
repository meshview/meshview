// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The web pages: the Python version's, served as it served them.
//!
//! Its templates take nothing from the request: everything a page shows it
//! fetches from the API itself. What they do take, the site's language and
//! texts and the map's tiles, is known once the configuration is read, so every
//! page is rendered once, at start, and answered from memory after that. The
//! one exception is the node page's link preview: the apps that show one do not
//! run the page's script, so the node's name and details go into the page as it
//! is answered, in the places left for them at start.
//!
//! The templates are Tera 2 ones, under `templates/`, and the files they use
//! under `assets/static/`; their strings are translated in the `ui` section of
//! the catalogs under `assets/lang/`. All of it is built into the binary.

use super::api::lang;
use super::api::site_config;
use crate::config::Config;
use crate::db::Node;
use crate::state::AppState;
use anyhow::{Context as _, anyhow};
use axum::Router;
use axum::extract::{Path, Request, State as AxumState};
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

/// Where each page is served, as in the Python version; the node page,
/// `/node/{id}`, is [`node_page`].
const ROUTES: [(&str, &str); 15] = [
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

/// The pictures of link previews: the site's, on every page, and a node's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preview {
    Site,
    Node,
}

/// Where link previews take a picture from: the server's own, at the
/// configured domain (over HTTPS unless the domain names its scheme), or,
/// without a domain, the same picture from the project's website, as previews
/// need an absolute address.
fn preview_image(domain: &str, preview: Preview) -> String {
    let (own, project) = match preview {
        Preview::Site => ("og.png", "og-instance.png"),
        Preview::Node => ("og-node.png", "og-instance-node.png"),
    };
    let domain = domain.trim().trim_end_matches('/');
    if domain.is_empty() {
        format!("https://meshview.github.io/{project}")
    } else if domain.starts_with("https://") || domain.starts_with("http://") {
        format!("{domain}/static/{own}")
    } else {
        format!("https://{domain}/static/{own}")
    }
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
    globals.insert("site_title", config.site.title.trim());
    // What a link to a page shows where it is pasted: the site, by default.
    let site_name = Some(config.site.title.trim())
        .filter(|title| !title.is_empty())
        .unwrap_or("Meshview");
    globals.insert("preview_site", site_name);
    globals.insert("preview_title", site_name);
    globals.insert("preview_description", config.site.message.trim());
    globals.insert(
        "preview_image",
        &preview_image(&config.site.domain, Preview::Site),
    );
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
    node: NodePage,
    not_found: Arc<str>,
}

/// Stand-ins for a node's preview, which the node page is rendered with once
/// more to learn where the node's own goes. No site text can hold them.
const TITLE_MARK: &str = "\u{1}preview-title\u{1}";
const DESCRIPTION_MARK: &str = "\u{1}preview-description\u{1}";

/// The node page, as rendered for any node, and in the pieces that go around
/// one node's preview title and description.
struct NodePage {
    plain: Arc<str>,
    around: [String; 3],
}

impl NodePage {
    fn new(plain: Arc<str>, marked: &str) -> anyhow::Result<Self> {
        let (head, rest) = marked
            .split_once(TITLE_MARK)
            .context("the node page has no place for its preview title")?;
        let (middle, tail) = rest
            .split_once(DESCRIPTION_MARK)
            .context("the node page has no place for its preview description")?;
        anyhow::ensure!(
            !tail.contains(TITLE_MARK) && !tail.contains(DESCRIPTION_MARK),
            "the node page has more than one place for its preview"
        );
        Ok(Self {
            plain,
            around: [head.into(), middle.into(), tail.into()],
        })
    }

    /// The page, with this preview in it.
    fn with(&self, title: &str, description: &str) -> String {
        let [head, middle, tail] = &self.around;
        [
            head.as_str(),
            &escape(title),
            middle,
            &escape(description),
            tail,
        ]
        .concat()
    }
}

/// Text made safe for an HTML attribute.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// What a link to a node shows: its names, and its id in hex with its role,
/// hardware and channel, whichever it has told. A node with no name yet is its
/// hex id, as everywhere else.
fn node_preview(node: &Node) -> (String, String) {
    let told = |field: &Option<String>| {
        field
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let hex = node.node_id.map(|id| format!("!{:08x}", id as u32));
    let title = match (told(&node.long_name), told(&node.short_name)) {
        (Some(long), Some(short)) if long != short => format!("{long} ({short})"),
        (Some(name), _) | (None, Some(name)) => name,
        (None, None) => hex.clone().unwrap_or_else(|| node.id.clone()),
    };
    let description = [
        hex,
        told(&node.role),
        told(&node.hw_model),
        told(&node.channel),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    (title, description)
}

/// `/node/{id}`: the node page, with the node's preview when it is known.
async fn node_page(state: &AppState, page: &NodePage, id: &str) -> String {
    let (Ok(node_id), Some(reader)) = (id.parse::<i64>(), state.reader.get()) else {
        return page.plain.to_string();
    };
    match reader.node(node_id).await {
        Ok(Some(node)) => {
            let (title, description) = node_preview(&node);
            page.with(&title, &description)
        }
        Ok(None) => page.plain.to_string(),
        Err(error) => {
            tracing::warn!("looking up node {node_id} for its page's preview: {error:#}");
            page.plain.to_string()
        }
    }
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
    context.insert("preview_title", TITLE_MARK);
    context.insert(
        "preview_image",
        &preview_image(&config.site.domain, Preview::Node),
    );
    context.insert("preview_description", DESCRIPTION_MARK);
    let marked = tera
        .render("node.html", &context)
        .map_err(|e| anyhow!("{e}"))
        .context("rendering node.html for a node's preview")?;
    let node = NodePage::new(Arc::clone(&pages["node.html"]), &marked)?;
    let mut context = Context::new();
    context.insert("error_message", "Page not found");
    let not_found = tera
        .render("error.html", &context)
        .map_err(|e| anyhow!("{e}"))
        .context("rendering error.html")?;
    Ok(Rendered {
        pages,
        node,
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
pub fn router(config: &Config) -> anyhow::Result<Router<AppState>> {
    let rendered = Arc::new(render(config)?);
    let mut router = Router::new();

    for (route, template) in ROUTES {
        let page = Arc::clone(&rendered.pages[template]);
        router = router.route(route, get(move || async move { Html(page.to_string()) }));
    }
    let pages = Arc::clone(&rendered);
    router = router.route(
        "/node/{id}",
        get(
            move |AxumState(state): AxumState<AppState>, Path(id): Path<String>| async move {
                Html(node_page(&state, &pages.node, &id).await)
            },
        ),
    );

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

    #[test]
    fn link_previews_take_the_picture_from_the_domain_or_the_project() {
        assert_eq!(
            preview_image("", Preview::Site),
            "https://meshview.github.io/og-instance.png"
        );
        assert_eq!(
            preview_image("  ", Preview::Node),
            "https://meshview.github.io/og-instance-node.png"
        );
        assert_eq!(
            preview_image("mesh.example.org", Preview::Site),
            "https://mesh.example.org/static/og.png"
        );
        assert_eq!(
            preview_image("http://mesh.example.org/", Preview::Node),
            "http://mesh.example.org/static/og-node.png"
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

    fn node(long: Option<&str>, short: Option<&str>) -> Node {
        Node {
            id: "!02ec7b48".into(),
            node_id: Some(0x02ec7b48),
            long_name: long.map(Into::into),
            short_name: short.map(Into::into),
            hw_model: Some("HELTEC_V3".into()),
            firmware: None,
            role: Some("ROUTER".into()),
            last_lat: None,
            last_long: None,
            last_altitude: None,
            channel: Some("".into()),
            is_mqtt_gateway: None,
            first_seen_us: None,
            last_seen_us: None,
        }
    }

    #[test]
    fn a_node_previews_with_its_names_and_details() {
        let (title, description) = node_preview(&node(Some("Refugio 103"), Some("R103")));
        assert_eq!(title, "Refugio 103 (R103)");
        assert_eq!(description, "!02ec7b48 · ROUTER · HELTEC_V3");
        assert_eq!(node_preview(&node(Some("R103"), Some("R103"))).0, "R103");
        assert_eq!(node_preview(&node(None, Some(" R103 "))).0, "R103");
        assert_eq!(node_preview(&node(None, Some(" "))).0, "!02ec7b48");
    }

    #[test]
    fn the_node_page_takes_a_node_preview_in_place_of_the_site_one() {
        let mut config = Config::default();
        config.site.title = "Galicia Mesh".into();
        config.site.message = "Live data".into();
        let rendered = render(&config).unwrap();
        let plain = &rendered.node.plain;
        assert!(plain.contains(r#"<meta property="og:title" content="Galicia Mesh">"#));
        assert!(plain.contains(r#"<meta property="og:description" content="Live data">"#));
        let page = rendered
            .node
            .with("<Refugio> \"103\"", "!02ec7b48 · ROUTER");
        assert!(
            page.contains(
                r#"<meta property="og:title" content="&lt;Refugio&gt; &quot;103&quot;">"#
            )
        );
        assert!(page.contains(r#"<meta property="og:description" content="!02ec7b48 · ROUTER">"#));
        assert!(page.contains(r#"<meta property="og:site_name" content="Galicia Mesh">"#));
        assert!(plain.contains("/og-instance.png"));
        assert!(page.contains("/og-instance-node.png"));
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
