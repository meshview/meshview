// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! HTTP: the API, its reference, and the probes.
//!
//! The API lives under `/api/v1`, and every route is declared through utoipa's
//! router, so the reference at `/api/docs` is built from the same calls that
//! register the handlers and cannot drift from them.
//!
//! The Python version served the same API without a version, at `/api/...`.
//! Those addresses keep working for a while as permanent redirects to their
//! `/api/v1` counterparts, marked deprecated, so that anything pointed at an
//! existing installation keeps working while it is moved over. They are one
//! route, [`legacy_api`], and removing them is removing it.
//!
//! Outside the version: the reference itself, which documents whatever versions
//! there are, and the probes, whose contract has nothing in it that could change.

pub mod api;
mod health;
mod reference;

use crate::app::Role;
use crate::state::AppState;
use crate::web::api::params::Params;
use axum::Router;
use axum::extract::Path;
use axum::http::header::{CONTENT_TYPE, HeaderMap, HeaderValue, LINK, LOCATION};
use axum::http::{Extensions, StatusCode, Uri, Version};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use std::collections::BTreeMap;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{Predicate, SizeAbove};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_scalar::{Scalar, Servable};

/// Where the reference is served.
const DOCS_PATH: &str = "/api/docs";

/// Where the document behind it is served, for clients and code generators.
const OPENAPI_PATH: &str = "/api/openapi.json";

/// The smallest body worth compressing, in bytes.
const COMPRESS_FROM: u64 = 1024;

/// The version the unversioned addresses stand for.
const CURRENT_VERSION: &str = "v1";

/// When the unversioned addresses were deprecated, for the `Deprecation` header
/// (RFC 9745), as seconds since the epoch: 2026-11-11T00:00:00Z.
const LEGACY_DEPRECATED_AT: i64 = 1_794_355_200;

/// The language the reference is written in, and answered in when no other is
/// asked for or there is no catalog for the one asked for.
const REFERENCE_LANGUAGE: &str = "en";

/// Scalar's page with a title of our own, the sidebar listing paths rather
/// than summaries, as a reference is navigated by URL, and Scalar's own words
/// in the site's language. Scalar is pinned, so that the page does not change
/// under us from one day to the next.
const SCALAR_HTML: &str = r#"<!doctype html>
<html>
  <head>
    <title>meshview-rs API</title>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
  </head>
  <body>
    <script
      id="api-reference"
      type="application/json"
      data-configuration='{"operationTitleSource":"path","localization":{"locale":"$locale"}}'
    >$spec</script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference@1.76.4"></script>
  </body>
</html>
"#;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "meshview-rs",
        description = include_str!("../../assets/api/intro.md"),
        license(name = "AGPL-3.0-or-later", identifier = "AGPL-3.0-or-later"),
    ),
    tags(
        (name = "health", description = "Whether the process can serve"),
        (name = "meta", description = "What is answering"),
        (name = "channels", description = "The channels the mesh is heard on"),
        (name = "nodes", description = "The nodes of the mesh"),
        (name = "packets", description = "The packets, and who heard them"),
        (name = "stats", description = "Figures about the mesh"),
    ),
)]
struct Reference;

/// The routes for a process with this role.
///
/// Every role answers the probes. Only one that serves the web also has the API
/// and its reference: an ingester listens only so that it can be probed.
pub fn router(role: Role, state: AppState) -> Router {
    let (router, reference) = assemble(role);

    let router = if role.serves() {
        let documents = Arc::new(documents(&reference));
        let site_language = state.config.site.language.as_str();
        let language = if documents.contains_key(site_language) {
            site_language
        } else {
            REFERENCE_LANGUAGE
        };
        let page: serde_json::Value = serde_json::from_slice(&documents[language])
            .expect("the reference was just serialized");
        let html = SCALAR_HTML.replace("$locale", language);
        let default = language.to_string();
        router
            .merge(Scalar::with_url(DOCS_PATH, page).custom_html(html))
            .route(
                OPENAPI_PATH,
                get(move |params: Params| async move {
                    let asked = params.get("lang").filter(|code| !code.is_empty());
                    let body = asked
                        .map_or(documents.get(&default), |code| documents.get(code))
                        .unwrap_or(&documents[REFERENCE_LANGUAGE])
                        .clone();
                    ([(CONTENT_TYPE, "application/json")], body)
                }),
            )
            .route("/api/{*rest}", get(legacy_api))
            .layer(compression())
    } else {
        router
    };

    router.with_state(state)
}

/// The reference serialized in each language there is a catalog for, and in
/// its own.
fn documents(reference: &utoipa::openapi::OpenApi) -> BTreeMap<String, Bytes> {
    let english = serde_json::to_value(reference).expect("the reference serializes");
    let mut documents: BTreeMap<String, Bytes> = reference::catalogs()
        .into_iter()
        .map(|(code, catalog)| {
            let translated = reference::translate(&english, &catalog);
            (code, Bytes::from(translated.to_string()))
        })
        .collect();
    documents.insert(
        REFERENCE_LANGUAGE.to_string(),
        Bytes::from(english.to_string()),
    );
    documents
}

/// Gzip for what is worth compressing, as the Python version did: JSON, SVG,
/// HTML, Markdown and plain text, from a kilobyte up. Unlike it, the client's
/// `Accept-Encoding` is read properly, quality values included.
fn compression() -> CompressionLayer<impl Predicate> {
    const TYPES: &[&str] = &[
        "application/json",
        "image/svg+xml",
        "text/html",
        "text/markdown",
        "text/plain",
    ];
    let worth_it = |_: StatusCode, _: Version, headers: &HeaderMap, _: &Extensions| {
        headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or("").trim())
            .is_some_and(|mime| TYPES.contains(&mime))
    };
    CompressionLayer::new()
        .gzip(true)
        .compress_when(SizeAbove::new(COMPRESS_FROM).and(worth_it))
}

/// The routes and the document describing them, built by the same calls.
pub fn assemble(role: Role) -> (Router<AppState>, utoipa::openapi::OpenApi) {
    let router = OpenApiRouter::with_openapi(Reference::openapi()).routes(routes!(health::livez));
    let router = router.routes(routes!(health::readyz));

    let router = if role.serves() {
        router
            .merge(api::unversioned())
            .nest(&format!("/api/{CURRENT_VERSION}"), v1())
    } else {
        router
    };

    router.split_for_parts()
}

/// The first version of the API. Paths are relative to it.
fn v1() -> OpenApiRouter<AppState> {
    api::v1()
}

/// An unversioned API address, sent on to the current version of itself.
///
/// 308 rather than 301 so that the method and body survive, and with the query
/// string intact. `Deprecation` says since when this address is on its way out
/// and `Link` names its successor, which is what lets a client notice without
/// anybody having to read a changelog.
async fn legacy_api(Path(rest): Path<String>, uri: Uri) -> Response {
    // Anything already under a version that got here matched nothing, and
    // redirecting it would only add another version to the front, for ever.
    if rest == CURRENT_VERSION || rest.starts_with(&format!("{CURRENT_VERSION}/")) {
        return api::error::ApiError::not_found("Not found").into_response();
    }

    let mut target = format!("/api/{CURRENT_VERSION}/{rest}");
    if let Some(query) = uri.query() {
        target.push('?');
        target.push_str(query);
    }

    let mut headers = HeaderMap::new();
    let Ok(location) = HeaderValue::from_str(&target) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let path_only = target.split('?').next().unwrap_or(&target);
    if let Ok(link) = HeaderValue::from_str(&format!("<{path_only}>; rel=\"successor-version\"")) {
        headers.insert(LINK, link);
    }
    headers.insert(LOCATION, location);
    headers.insert(
        "deprecation",
        HeaderValue::from_str(&format!("@{LEGACY_DEPRECATED_AT}")).expect("ASCII digits"),
    );

    (StatusCode::PERMANENT_REDIRECT, headers).into_response()
}
