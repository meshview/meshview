// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use super::error::{ApiError, ErrorBody};
use super::params::{Params, parse_db_int};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use utoipa::ToSchema;

/// The longest span the counts go back.
const MAX_DAYS: i64 = 30;
const HOUR_US: i64 = 3_600_000_000;

#[derive(Debug, Serialize, ToSchema)]
pub struct NodePacketCounts {
    /// The node asked about.
    #[schema(example = 825542997)]
    pub node_id: i64,
    /// The hours that had packets, the earliest first, one entry for each
    /// port in each.
    pub counts: Vec<HourCount>,
}

/// How many packets of one port were stored in one hour.
#[derive(Debug, Serialize, ToSchema)]
pub struct HourCount {
    /// The start of the hour, in microseconds since the epoch.
    #[schema(example = 1794312000000000_i64)]
    pub hour_us: i64,
    /// The port, null for a packet that did not say.
    #[schema(example = 67)]
    pub portnum: Option<i64>,
    #[schema(example = 12)]
    pub packets: i64,
}

/// Packets of a node by hour
///
/// How many packets from or to a node were stored in each hour, by port,
/// counted by the server: for charting a node's traffic without fetching its
/// packets. Hours are UTC; a client can add them up into its own days.
///
/// The counts go back at most 30 days, as far as `since` asks.
#[utoipa::path(
    get,
    path = "/node/{node_id}/packet_counts",
    tag = "nodes",
    params(
        ("node_id" = i64, Path, description = "The node's number, in decimal."),
        ("since" = Option<i64>, Query, description = "Only packets stored at or after this time, in microseconds since the epoch; no earlier than 30 days ago, which is also the default."),
    ),
    responses(
        (status = 200, description = "The counts, maybe none", body = NodePacketCounts),
        (status = 400, description = "`node_id` or `since` is not an integer", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn packet_counts(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    params: Params,
) -> Result<Json<NodePacketCounts>, ApiError> {
    let node_id = parse_db_int(&node_id)
        .ok_or_else(|| ApiError::bad_request("Invalid or missing node_id"))?;
    let earliest = jiff::Timestamp::now().as_microsecond() - MAX_DAYS * 24 * HOUR_US;
    let since_us = params
        .int("since", Some(earliest), Some(earliest), None)?
        .unwrap_or(earliest);

    let rows = state
        .db()?
        .node_packets_per_hour(node_id, since_us)
        .await
        .map_err(|e| ApiError::internal("Internal server error", &e))?;

    Ok(Json(NodePacketCounts {
        node_id,
        counts: rows
            .into_iter()
            .map(|row| HourCount {
                hour_us: row.hour * HOUR_US,
                portnum: row.portnum,
                packets: row.packets,
            })
            .collect(),
    }))
}
