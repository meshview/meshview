// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The routes each role answers, the probes, and the redirects from the
//! unversioned API.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use meshview_rs::app::Role;
use meshview_rs::config::Config;
use meshview_rs::state::{AppState, CHECK_DATABASE, Readiness};
use meshview_rs::web;
use std::sync::Arc;
use tower::ServiceExt;

fn app(role: Role) -> (Router, AppState) {
    let state = AppState::new(
        Arc::new(Readiness::new([CHECK_DATABASE])),
        Config::default(),
    );
    (web::router(role, state.clone()), state)
}

async fn get(app: &Router, uri: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn alive_at_once_and_ready_once_the_database_is() {
    let (app, state) = app(Role::Serve);

    assert_eq!(get(&app, "/livez").await.status(), StatusCode::OK);

    let waiting = get(&app, "/readyz").await;
    assert_eq!(waiting.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json(waiting).await["checks"]["database"], false);

    state.readiness.set(CHECK_DATABASE, true);
    let ready = get(&app, "/readyz").await;
    assert_eq!(ready.status(), StatusCode::OK);
    assert_eq!(json(ready).await["status"], "ok");
}

#[tokio::test]
async fn the_unversioned_api_redirects_to_v1() {
    let (app, _) = app(Role::Serve);

    let response = get(&app, "/api/stats/top?limit=5&channel=LongFast").await;

    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    let headers = response.headers();
    assert_eq!(
        headers[header::LOCATION],
        "/api/v1/stats/top?limit=5&channel=LongFast"
    );
    assert_eq!(
        headers[header::LINK],
        "</api/v1/stats/top>; rel=\"successor-version\""
    );
    assert!(headers["deprecation"].to_str().unwrap().starts_with('@'));
}

#[tokio::test]
async fn an_unknown_v1_address_is_not_redirected_again() {
    let (app, _) = app(Role::Serve);

    assert_eq!(
        get(&app, "/api/v1/nothing-here").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_reference_describes_what_is_served() {
    let (app, _) = app(Role::Serve);

    assert_eq!(get(&app, "/api/docs").await.status(), StatusCode::OK);

    let document = json(get(&app, "/api/openapi.json").await).await;
    assert!(document["paths"]["/livez"].is_object());
    assert!(document["paths"]["/readyz"].is_object());
}

#[tokio::test]
async fn an_ingester_only_answers_the_probes() {
    let (app, _) = app(Role::Ingest);

    assert_eq!(get(&app, "/livez").await.status(), StatusCode::OK);
    assert_eq!(get(&app, "/api/docs").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        get(&app, "/api/stats").await.status(),
        StatusCode::NOT_FOUND
    );
}
