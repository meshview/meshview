// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The web pages and their files, served as the Python version served them.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use http_body_util::BodyExt;
use meshview::app::Role;
use meshview::config::Config;
use meshview::state::{AppState, CHECK_DATABASE, Readiness};
use meshview::web;
use std::sync::Arc;
use tower::ServiceExt;

fn app(config: Config) -> Router {
    let state = AppState::new(Arc::new(Readiness::new([CHECK_DATABASE])), config);
    web::router(Role::Serve, state)
}

async fn get(app: &Router, uri: &str, headers: &[(header::HeaderName, &str)]) -> Response {
    let mut request = Request::get(uri);
    for (name, value) in headers {
        request = request.header(name, *value);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn text(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn every_page_is_served_where_the_python_version_served_it() {
    let app = app(Config::default());
    for path in [
        "/",
        "/index",
        "/index.html",
        "/map",
        "/map.html",
        "/net",
        "/reliability",
        "/nodelist",
        "/firehose",
        "/chat",
        "/nodegraph",
        "/top",
        "/stats",
        "/packet/123",
        "/node/1234",
        "/traceroute/123",
        "/reliability/123",
        "/radio/123",
    ] {
        let response = get(&app, path, &[]).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert!(
            response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn pages_speak_the_api_version_and_the_visitors_language() {
    let mut config = Config::default();
    config.site.message = "Nuestra malla".into();
    let app = app(config);

    let response = get(&app, "/nodelist", &[(header::ACCEPT_LANGUAGE, "es")]).await;
    assert_eq!(response.headers()[header::VARY], "Cookie, Accept-Language");
    let page = text(response).await;
    assert!(page.contains(r#"<html lang="es""#));
    assert!(page.contains(r#"window.meshLanguage = "es";"#));
    assert!(page.contains(r#"<meta name="description" content="Nuestra malla">"#));
    assert!(page.contains("/api/v1/nodes"));
    assert!(!page.contains("fetch(`/api/nodes"));
    assert!(page.contains(r#"href="/api/docs""#));
    // The footer is Meshview's, not the Python version's.
    assert!(page.contains(r#"href="https://github.com/meshview/meshview""#));
    assert!(page.contains("Meshview en GitHub"));
    // With the version, and the commit it was built from when that is known.
    let build = meshview::web::api::version::current();
    let shown = match build.git_revision_short {
        "unknown" => format!("Meshview Rust {}<", build.version),
        revision => format!("Meshview Rust {} ({revision})<", build.version),
    };
    assert!(page.contains(&shown), "{shown}");

    // The dashboard is the map with the node explorer beside it.
    let dashboard = text(get(&app, "/", &[]).await).await;
    let map = text(get(&app, "/map", &[]).await).await;
    assert!(dashboard.contains("mesh-dashboard"));
    assert!(map.contains("mesh-map-only"));
}

#[tokio::test]
async fn a_visitor_reads_the_pages_in_the_language_they_chose() {
    let app = app(Config::default());
    let spanish = |headers: &'static [(header::HeaderName, &'static str)]| {
        let app = app.clone();
        async move {
            let page = text(get(&app, "/nodelist", headers).await).await;
            page.contains(r#"<html lang="es""#)
        }
    };

    // The cookie of the preferences asks for Spanish.
    assert!(spanish(&[(header::COOKIE, "theme=x; meshview-lang=es")]).await);
    // A language there is no translation for, or none, is the browser's.
    assert!(!spanish(&[(header::COOKIE, "meshview-lang=xx")]).await);
    assert!(!spanish(&[(header::COOKIE, "other=es")]).await);
    // The browser's: the first known, by preference, a region as its language.
    assert!(spanish(&[(header::ACCEPT_LANGUAGE, "es-ES,es;q=0.9,en;q=0.8")]).await);
    assert!(spanish(&[(header::ACCEPT_LANGUAGE, "fr-FR, en;q=0.5, es;q=0.7")]).await);
    assert!(!spanish(&[(header::ACCEPT_LANGUAGE, "fr, de;q=0.8")]).await);
    assert!(!spanish(&[(header::ACCEPT_LANGUAGE, "en, es;q=0.9")]).await);
    assert!(!spanish(&[(header::ACCEPT_LANGUAGE, "es;q=0, en")]).await);
    // The choice in the preferences wins over the browser's.
    assert!(
        !spanish(&[
            (header::COOKIE, "meshview-lang=en"),
            (header::ACCEPT_LANGUAGE, "es")
        ])
        .await
    );
    // The page the menu marks as the current one is the one asked for.
    let page = text(get(&app, "/firehose", &[]).await).await;
    assert!(page.contains(
        r#"<a href="/firehose" data-group="everything" class="active" aria-current="page">"#
    ));
}

#[tokio::test]
async fn a_planet_is_linked_only_when_its_address_is_set() {
    const LINK: &str = r#"<a href="https://planet.example.org" target="_blank""#;
    let page = text(get(&app(Config::default()), "/nodelist", &[]).await).await;
    assert!(!page.contains("Meshview Planet</a>"));

    let mut config = Config::default();
    config.site.planet_url = " https://planet.example.org ".into();
    let page = text(get(&app(config.clone()), "/nodelist", &[]).await).await;
    assert!(page.contains(LINK));

    // Not an address on the web, it is not linked either.
    config.site.planet_url = "javascript:alert(1)".into();
    let page = text(get(&app(config), "/nodelist", &[]).await).await;
    assert!(!page.contains("Meshview Planet</a>"));
}

#[tokio::test]
async fn pages_come_in_their_language_from_the_first_paint() {
    let app = app(Config::default());
    const SPANISH: &[(header::HeaderName, &str)] = &[(header::ACCEPT_LANGUAGE, "es")];
    let page = text(get(&app, "/nodelist", SPANISH).await).await;
    assert!(page.contains(r#"<span id="favorites-label" data-translate-lang="show_favorites">Mostrar favoritos</span>"#));
    // A field: its placeholder, and nothing written after it.
    assert!(page.contains(r#"placeholder="Buscar por nombre o ID...""#));
    assert_eq!(page.matches("Buscar por nombre o ID...").count(), 1);
    let page = text(get(&app, "/top", SPANISH).await).await;
    assert!(
        page.contains(r#"data-translate-lang="top_traffic_nodes">Nodos con mayor tráfico</h1>"#)
    );
}

#[tokio::test]
async fn a_locked_kiosk_keeps_no_way_out() {
    const LOCK: &str = "classList.add('mesh-kiosk-locked')";
    let page = text(get(&app(Config::default()), "/map", &[]).await).await;
    assert!(page.contains(r#"id="kiosk-button""#));
    assert!(!page.contains(LOCK));

    let mut config = Config::default();
    config.site.map_kiosk_lock = true;
    let page = text(get(&app(config), "/map", &[]).await).await;
    assert!(page.contains(LOCK));
}

#[tokio::test]
async fn old_addresses_lead_to_their_new_places() {
    let app = app(Config::default());
    for (path, status, location) in [
        ("/packet_list/42", StatusCode::FOUND, "/node/42"),
        (
            "/graph/traceroute/7",
            StatusCode::MOVED_PERMANENTLY,
            "/traceroute/7",
        ),
        ("/api.html", StatusCode::MOVED_PERMANENTLY, "/api/docs"),
        ("/api-chat.html", StatusCode::MOVED_PERMANENTLY, "/api/docs"),
        (
            "/reliability?node_id=42",
            StatusCode::FOUND,
            "/reliability/42",
        ),
        ("/radio?node_id=42", StatusCode::FOUND, "/radio/42"),
    ] {
        let response = get(&app, path, &[]).await;
        assert_eq!(response.status(), status, "{path}");
        assert_eq!(response.headers()[header::LOCATION], location, "{path}");
    }
}

#[tokio::test]
async fn static_files_are_revalidated_and_vendored_ones_kept() {
    let app = app(Config::default());

    let own = get(&app, "/static/theme.js", &[]).await;
    assert_eq!(own.status(), StatusCode::OK);
    assert_eq!(own.headers()[header::CACHE_CONTROL], "no-cache");
    assert!(
        own.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    let etag = own.headers()[header::ETAG].to_str().unwrap().to_string();
    let again = get(&app, "/static/theme.js", &[(header::IF_NONE_MATCH, &etag)]).await;
    assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

    let vendored = get(&app, "/static/vendor/leaflet-1.9.4/dist/leaflet.js", &[]).await;
    assert_eq!(vendored.status(), StatusCode::OK);
    assert_eq!(
        vendored.headers()[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );

    // Big scripts go compressed to who takes it.
    let compressed = get(
        &app,
        "/static/vendor/echarts-6.1.0/dist/echarts.min.js",
        &[(header::ACCEPT_ENCODING, "gzip")],
    )
    .await;
    assert_eq!(compressed.headers()[header::CONTENT_ENCODING], "gzip");
}

#[tokio::test]
async fn what_is_not_there_is_a_page_saying_so() {
    let app = app(Config::default());
    for path in [
        "/nothing",
        "/static/nothing.js",
        "/docs/nothing.md",
        "/a/b/c",
    ] {
        let response = get(&app, path, &[]).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert!(text(response).await.contains("Page not found"), "{path}");
    }
}

#[tokio::test]
async fn the_coverage_help_comes_with_the_node_page() {
    let app = app(Config::default());

    let page = text(get(&app, "/node/1234", &[(header::ACCEPT_LANGUAGE, "es")]).await).await;
    assert!(page.contains("<h4>Cómo funciona</h4>"));
    // It was fetched from these, which are not served any more.
    assert_eq!(
        get(&app, "/docs/COVERAGE.md", &[]).await.status(),
        StatusCode::NOT_FOUND
    );
}
