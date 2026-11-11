// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The two questions an orchestrator asks: is the process alive, and should it
//! be sent traffic. Kept apart because the answers differ while it starts — a
//! web server waiting for the ingester to migrate the database is alive and must
//! not be restarted, but is not ready.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;

/// The answer to either question.
#[derive(Debug, Serialize, ToSchema)]
pub struct Status {
    /// `ok`, or `unavailable` when the process cannot serve yet.
    #[schema(example = "ok")]
    pub status: &'static str,
    /// Each readiness check and whether it passes. Absent from liveness, which
    /// checks nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"database": true}))]
    pub checks: Option<BTreeMap<String, bool>>,
}

/// Liveness
///
/// Answers 200 for as long as the process can answer at all. It looks at
/// nothing else on purpose: a liveness probe that fails restarts the process,
/// and nothing a restart cannot fix belongs here.
#[utoipa::path(
    get,
    path = "/livez",
    tag = "health",
    responses((status = 200, description = "The process is running", body = Status))
)]
pub async fn livez() -> Json<Status> {
    Json(Status {
        status: "ok",
        checks: None,
    })
}

/// Readiness
///
/// Answers 200 once every check this process depends on passes, and 503 with
/// the list of them until then.
#[utoipa::path(
    get,
    path = "/readyz",
    tag = "health",
    responses(
        (status = 200, description = "Ready to serve", body = Status),
        (status = 503, description = "Not ready yet", body = Status),
    )
)]
pub async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<Status>) {
    let ready = state.readiness.is_ready();
    let body = Status {
        status: if ready { "ok" } else { "unavailable" },
        checks: Some(state.readiness.report()),
    };
    let code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(body))
}
