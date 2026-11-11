// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use super::error::{ApiError, ErrorBody};
use super::params::{Params, parse_db_int_any_base};
use crate::coverage::{self, MODEL, Radio};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use meshview_proto::text_format::python_float;
use serde::Serialize;
use utoipa::ToSchema;

/// The most steps out to the radius: beyond this a prediction is too costly
/// to make on request.
const MAX_STEPS: f64 = 4000.0;

const DEFAULT_MIN_DBM: f64 = -130.0;
const DEFAULT_MAX_DBM: f64 = -80.0;

/// The parameters, in the order they are checked: name, default, lowest and
/// highest accepted.
const LIMITS: [(&str, f64, f64, f64); 8] = [
    ("freq_mhz", 907.0, 1.0, 10000.0),
    ("tx_dbm", 20.0, -50.0, 60.0),
    ("tx_height_m", 5.0, 0.0, 1000.0),
    ("rx_height_m", 1.5, 0.0, 1000.0),
    ("radius_km", 40.0, 1.0, 200.0),
    ("step_km", 0.25, 0.05, 50.0),
    ("reliability", 0.5, 0.01, 0.99),
    ("threshold_dbm", -120.0, -200.0, 0.0),
];

/// A number as Python's `float()` reads one: surrounding space, a sign,
/// underscores between digits, and `inf` and `nan` in any case.
fn python_float_parse(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.contains('_') {
        let bytes = text.as_bytes();
        let ok = bytes.iter().enumerate().all(|(i, &b)| {
            b != b'_'
                || (i > 0
                    && i + 1 < bytes.len()
                    && bytes[i - 1].is_ascii_digit()
                    && bytes[i + 1].is_ascii_digit())
        });
        if !ok {
            return None;
        }
    }
    let cleaned = text.replace('_', "");
    // Rust takes "infinity" and "nan" as Python does; it also takes a few
    // spellings Python does not, such as a bare "." being refused by both.
    cleaned.parse().ok()
}

/// Rounded to `digits` decimals as Python's `round(x, digits)` rounds a float.
fn python_round(value: f64, digits: usize) -> f64 {
    format!("{value:.digits$}").parse().unwrap_or(value)
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum Coverage {
    Perimeter {
        #[schema(example = "perimeter")]
        mode: &'static str,
        threshold_dbm: f64,
        /// `[latitude, longitude]`, a point a bearing, every 5°.
        #[schema(value_type = Vec<Vec<f64>>)]
        perimeter: Vec<(f64, f64)>,
    },
    Heatmap {
        #[schema(example = "heatmap")]
        mode: &'static str,
        min_dbm: f64,
        max_dbm: f64,
        /// `[latitude, longitude, received_power_dbm]`.
        #[schema(value_type = Vec<Vec<f64>>)]
        points: Vec<(f64, f64, f64)>,
    },
}

/// Predicted coverage
///
/// Where a node is predicted to be heard, from the ITS Irregular Terrain Model
/// in area mode over average, hilly ground: by default the outline, the
/// furthest point every 5° where the power received still reaches
/// `threshold_dbm`; with any other `mode`, the power received at every point
/// sampled.
#[utoipa::path(
    get,
    path = "/coverage/{node_id}",
    tag = "nodes",
    params(
        ("node_id" = i64, Path, description = "The node's number, in decimal or 0x hex. It needs a position."),
        ("mode" = Option<String>, Query, description = "`perimeter` (the default), or anything else for every point."),
        ("freq_mhz" = Option<f64>, Query, description = "Frequency, 1 to 10000 MHz. Default 907."),
        ("tx_dbm" = Option<f64>, Query, description = "Transmitted power, -50 to 60 dBm. Default 20."),
        ("tx_height_m" = Option<f64>, Query, description = "Height of the node's antenna, 0 to 1000 m. Default 5."),
        ("rx_height_m" = Option<f64>, Query, description = "Height of a receiving antenna, 0 to 1000 m. Default 1.5."),
        ("radius_km" = Option<f64>, Query, description = "How far out to look, 1 to 200 km. Default 40."),
        ("step_km" = Option<f64>, Query, description = "The step out, 0.05 to 50 km, at most 4000 to the radius. Default 0.25."),
        ("reliability" = Option<f64>, Query, description = "The fraction of time and locations to be heard in, 0.01 to 0.99. Default 0.5."),
        ("threshold_dbm" = Option<f64>, Query, description = "The weakest power that counts as heard, -200 to 0 dBm. Default -120."),
    ),
    responses(
        (status = 200, description = "The prediction", body = Coverage),
        (status = 400, description = "A parameter is not valid", body = ErrorBody),
        (status = 404, description = "No such node, or it has no position", body = ErrorBody),
    )
)]
pub async fn coverage(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    params: Params,
) -> Result<Json<Coverage>, ApiError> {
    let node_id =
        parse_db_int_any_base(&node_id).ok_or_else(|| ApiError::bad_request("Invalid node_id"))?;

    let mut values = [0.0; LIMITS.len()];
    for (value, (name, default, low, high)) in values.iter_mut().zip(LIMITS) {
        *value = match params.get(name) {
            None => default,
            Some(raw) => {
                let parsed = python_float_parse(raw)
                    .ok_or_else(|| ApiError::bad_request(format!("{name} must be a number")))?;
                if !parsed.is_finite() || !(low..=high).contains(&parsed) {
                    return Err(ApiError::bad_request(format!(
                        "{name} must be between {} and {}",
                        python_float(low),
                        python_float(high)
                    )));
                }
                parsed
            }
        };
    }
    let [
        freq_mhz,
        tx_dbm,
        tx_height_m,
        rx_height_m,
        radius_km,
        step_km,
        reliability,
        threshold_dbm,
    ] = values;

    if radius_km / step_km > MAX_STEPS {
        return Err(ApiError::bad_request(
            "radius_km / step_km must not exceed 4000",
        ));
    }

    let node = state
        .db()?
        .node(node_id)
        .await
        .map_err(|e| ApiError::internal("Failed to compute coverage", &e))?;
    let Some((lat, lon)) = node
        .and_then(|node| Some((node.last_lat?, node.last_long?)))
        .filter(|&(lat, lon)| lat != 0 && lon != 0)
    else {
        return Err(ApiError::not_found("Node not found or missing location"));
    };

    // Rounded as the Python version rounded them, which is what it computed
    // with, so the points are the same.
    let radio = Radio {
        lat: python_round(lat as f64 * 1e-7, 7),
        lon: python_round(lon as f64 * 1e-7, 7),
        freq_mhz: python_round(freq_mhz, 3),
        tx_dbm: python_round(tx_dbm, 2),
        tx_height_m: python_round(tx_height_m, 2),
        rx_height_m: python_round(rx_height_m, 2),
        radius_km: python_round(radius_km, 2),
        step_km: python_round(step_km, 3),
        reliability: python_round(reliability, 3),
    };
    let model = *MODEL;
    let perimeter_mode = params.get("mode").unwrap_or("perimeter") == "perimeter";

    // Pure computation, thousands of model runs: off the threads serving
    // requests.
    let answer = tokio::task::spawn_blocking(move || {
        if perimeter_mode {
            Coverage::Perimeter {
                mode: "perimeter",
                threshold_dbm,
                perimeter: coverage::perimeter(model, &radio, python_round(threshold_dbm, 1)),
            }
        } else {
            let points = coverage::heatmap(model, &radio);
            Coverage::Heatmap {
                mode: "heatmap",
                min_dbm: points.iter().map(|p| p.2).fold(DEFAULT_MIN_DBM, f64::min),
                max_dbm: points.iter().map(|p| p.2).fold(DEFAULT_MAX_DBM, f64::max),
                points,
            }
        }
    })
    .await
    .map_err(|e| ApiError::internal("Failed to compute coverage", &anyhow::anyhow!(e)))?;

    Ok(Json(answer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_python_reads_them() {
        assert_eq!(python_float_parse(" 868 "), Some(868.0));
        assert_eq!(python_float_parse("1_000.5"), Some(1000.5));
        assert_eq!(python_float_parse("-2.5e1"), Some(-25.0));
        assert!(python_float_parse("inf").unwrap().is_infinite());
        assert!(python_float_parse("NaN").unwrap().is_nan());
        for bad in ["", "abc", "1__0", "_1", "1_", "1,5"] {
            assert_eq!(python_float_parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn rounding_is_python_rounding() {
        assert_eq!(python_round(0.125, 2), 0.12);
        assert_eq!(python_round(42.123_456_789_9, 7), 42.123_456_8);
    }
}
