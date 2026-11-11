// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Errors from the API, always as JSON: `{"error": "..."}`.
//!
//! The Python version answered its own errors that way, and everything else, an
//! unknown address or a failure it had not foreseen, as plain text. Here every
//! answer under `/api/v1` is JSON, so a client has one shape to handle.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use tracing::error;
use utoipa::ToSchema;

/// The body of every error the API answers.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    /// What was wrong.
    #[schema(example = "length must be an integer")]
    pub error: String,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    /// A failure on our side. What went wrong is logged; the client is told
    /// only `message`, as the Python version told it.
    pub fn internal(message: impl Into<String>, cause: &anyhow::Error) -> Self {
        let message = message.into();
        error!("{message}: {cause:#}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    /// The database is not at the schema this build knows yet.
    pub fn not_ready() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "The database is not ready yet",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}
