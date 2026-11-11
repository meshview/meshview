// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use super::error::{ApiError, ErrorBody};
use super::now_us;
use super::params::Params;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

const HOUR_US: i64 = 3600 * 1_000_000;
const DAY_US: i64 = 24 * HOUR_US;

#[derive(Debug, Serialize, ToSchema)]
pub struct Channels {
    /// The channels, in byte order.
    #[schema(example = json!(["LongFast", "MediumFast"]))]
    pub channels: Vec<String>,
    /// Why there are none, when it is because of the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Channels
///
/// The channels packets have been heard on in the latest hours or days, by
/// the time they were imported.
///
/// An unknown `period_type` is answered with 200, no channels and an `error`,
/// as the Python version answers it.
#[utoipa::path(
    get,
    path = "/channels",
    tag = "channels",
    params(
        ("period_type" = Option<String>, Query, description = "`hour` or `day`, exactly. Default `hour`."),
        ("length" = Option<i64>, Query, description = "How many of them, 1 to 3650; beyond those, the nearest. Default 24."),
    ),
    responses(
        (status = 200, description = "The channels", body = Channels),
        (status = 400, description = "`length` is not an integer", body = ErrorBody),
    )
)]
pub async fn channels(
    State(state): State<AppState>,
    params: Params,
) -> Result<Json<Channels>, ApiError> {
    let length = params
        .int("length", Some(24), Some(1), Some(3650))?
        .unwrap_or(24);
    let unit = match params.get("period_type").unwrap_or("hour") {
        "hour" => HOUR_US,
        "day" => DAY_US,
        _ => {
            return Ok(Json(Channels {
                channels: Vec::new(),
                error: Some("period_type must be 'hour' or 'day'".into()),
            }));
        }
    };

    // From now. The Python version took its local time for UTC here, which put
    // the window off by the server's offset from UTC.
    let since_us = now_us() - length * unit;
    match state.db()?.channels_since(since_us).await {
        Ok(channels) => Ok(Json(Channels {
            channels,
            error: None,
        })),
        Err(e) => Ok(Json(Channels {
            channels: Vec::new(),
            error: Some(format!("{e:#}")),
        })),
    }
}
