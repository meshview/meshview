// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use super::error::{ApiError, ErrorBody};
use super::params::parse_db_int;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct Sightings {
    /// The sightings, latest first; none for a packet nobody heard, or that
    /// is not there.
    pub seen: Vec<Sighting>,
}

/// One gateway hearing the packet.
#[derive(Debug, Serialize, ToSchema)]
pub struct Sighting {
    /// The packet's stored id.
    #[schema(example = 123)]
    pub packet_id: i64,
    /// The gateway that relayed it to the broker.
    #[schema(example = 456)]
    pub node_id: i64,
    /// When the gateway received it, in seconds since the epoch by the
    /// gateway's own clock.
    #[schema(example = 1753188300)]
    pub rx_time: i64,
    /// The hops it had left when the gateway heard it.
    #[schema(example = 5)]
    pub hop_limit: Option<i64>,
    /// The hops it started with: `hop_start - hop_limit` is how many it had
    /// taken, when both are known.
    #[schema(example = 7)]
    pub hop_start: Option<i64>,
    /// The channel the gateway heard it on.
    #[schema(example = "LongFast")]
    pub channel: Option<String>,
    /// Signal to noise ratio, in dB; null when it was not a number.
    #[schema(example = 5.0)]
    pub rx_snr: Option<f64>,
    /// Received signal strength, in dBm.
    #[schema(example = -90)]
    pub rx_rssi: Option<i64>,
    /// The MQTT topic it came in on.
    #[schema(example = "msh/EU_868/2/e/LongFast/!000001c8")]
    pub topic: Option<String>,
    /// When it was stored, in microseconds since the epoch.
    #[schema(example = 1736370123456789_i64)]
    pub import_time_us: Option<i64>,
}

/// Packet sightings
///
/// Every gateway that heard a packet, latest first.
///
/// A gateway can appear more than once, having heard the packet by more than
/// one way: count them once each to know how many heard it. A gateway hearing
/// a packet does not mean it heard it straight from the node that sent it;
/// the hops tell.
#[utoipa::path(
    get,
    path = "/packets_seen/{packet_id}",
    tag = "packets",
    params(("packet_id" = i64, Path, description = "The packet's id, in decimal.")),
    responses(
        (status = 200, description = "The sightings, maybe none", body = Sightings),
        (status = 400, description = "`packet_id` is not an integer", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn packets_seen(
    State(state): State<AppState>,
    Path(packet_id): Path<String>,
) -> Result<Json<Sightings>, ApiError> {
    let packet_id = parse_db_int(&packet_id)
        .ok_or_else(|| ApiError::bad_request("Invalid or missing packet_id"))?;

    let rows = state
        .db()?
        .packet_sightings(packet_id)
        .await
        .map_err(|e| ApiError::internal("Internal server error", &e))?;

    Ok(Json(Sightings {
        seen: rows
            .into_iter()
            .map(|row| Sighting {
                packet_id: row.packet_id,
                node_id: row.node_id,
                rx_time: row.rx_time,
                hop_limit: row.hop_limit,
                hop_start: row.hop_start,
                channel: row.channel,
                rx_snr: row.rx_snr.filter(|snr| snr.is_finite()),
                rx_rssi: row.rx_rssi,
                topic: row.topic,
                import_time_us: row.import_time_us,
            })
            .collect(),
    }))
}
