// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use super::error::{ApiError, ErrorBody};
use super::params::{Params, must_be_integer, parse_python_int};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct Snapshots {
    /// The days, oldest first; none when nothing has been recorded.
    pub data: Vec<Snapshot>,
}

/// One day's figures, as the daily snapshot recorded them.
#[derive(Debug, Serialize, ToSchema)]
pub struct Snapshot {
    /// The UTC day.
    #[schema(example = "2026-11-10")]
    pub date: String,
    /// Nodes that sent a packet that day, or whose older packet was heard again.
    #[schema(example = 143)]
    pub node_count: i64,
    /// Packets imported that day, each once however many gateways heard it.
    #[schema(example = 28465)]
    pub packet_count: i64,
    /// Gateways that relayed anything that day.
    #[schema(example = 17)]
    pub gateway_count: i64,
    /// When the snapshot was taken, in microseconds since the epoch: usually
    /// the day after `date`.
    #[schema(example = 1792886400000000_i64)]
    pub captured_at_us: i64,
}

/// Daily snapshots
///
/// The figures recorded each day, oldest first, which outlive the data they
/// were counted from.
///
/// Every day just after midnight UTC the day before is summed up: from 00:00
/// to the next midnight, by the time packets and sightings were imported, not
/// the time the devices say. They are counts of what was active, not of what
/// the database holds; a node that was only sent to does not count. A
/// snapshot keeps the figures it was taken with, and counts only what was
/// still kept when it was taken.
///
/// `length` is a number of records, not of calendar days: a day without a
/// snapshot is not filled in.
#[utoipa::path(
    get,
    path = "/snapshots/daily",
    tag = "stats",
    params(
        ("length" = Option<i64>, Query, description = "Only the latest this many days, 1 to 3650. Default: every day recorded."),
    ),
    responses(
        (status = 200, description = "The snapshots", body = Snapshots),
        (status = 400, description = "`length` is not an integer", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn daily(
    State(state): State<AppState>,
    params: Params,
) -> Result<Json<Snapshots>, ApiError> {
    // Present at all, even empty, it has to be a number: unlike the other
    // integer parameters, the Python version reads this one with a plain int().
    let latest = match params.get("length") {
        None => None,
        Some(raw) => {
            let value = parse_python_int(raw).ok_or_else(|| must_be_integer("length"))?;
            Some(value.clamp(1, 3650) as i64)
        }
    };

    let rows = state
        .db()?
        .daily_snapshots(latest)
        .await
        .map_err(|e| ApiError::internal("Failed to fetch daily snapshots", &e))?;

    Ok(Json(Snapshots {
        data: rows
            .into_iter()
            .map(|row| Snapshot {
                date: row.snapshot_date,
                node_count: row.node_count,
                packet_count: row.packet_count,
                gateway_count: row.gateway_count,
                captured_at_us: row.captured_at_us,
            })
            .collect(),
    }))
}
