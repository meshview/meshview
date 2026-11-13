// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use super::error::{ApiError, ErrorBody};
use super::params::{Params, parse_db_int};
use crate::db::PacketSeen;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
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

/// The most packets one summary request can ask for.
const MAX_SUMMARIES: usize = 100;

#[derive(Debug, Serialize, ToSchema)]
pub struct SightingSummaries {
    /// One for each packet asked for that somebody heard, in the order asked.
    pub packets: Vec<SightingSummary>,
}

/// How a packet was heard, summed up over every gateway that heard it.
///
/// The ranges leave out what a gateway did not report: the signal when its
/// strength is 0, as for a packet a gateway sent itself rather than heard, a
/// signal to noise ratio that was not a number, and the hops when the packet
/// did not say how many it started with.
#[derive(Debug, PartialEq, Serialize, ToSchema)]
pub struct SightingSummary {
    /// The packet's stored id.
    #[schema(example = 123)]
    pub packet_id: i64,
    /// How many gateways heard it, each once however many times it did.
    #[schema(example = 5)]
    pub gateways: i64,
    /// The weakest signal strength heard, in dBm.
    #[schema(example = -124)]
    pub rssi_min: Option<i64>,
    /// The strongest signal strength heard, in dBm.
    #[schema(example = -64)]
    pub rssi_max: Option<i64>,
    /// The lowest signal to noise ratio heard, in dB.
    #[schema(example = -7.5)]
    pub snr_min: Option<f64>,
    /// The highest signal to noise ratio heard, in dB.
    #[schema(example = 12.5)]
    pub snr_max: Option<f64>,
    /// The fewest hops it had taken when heard.
    #[schema(example = 0)]
    pub hops_min: Option<i64>,
    /// The most hops it had taken when heard.
    #[schema(example = 5)]
    pub hops_max: Option<i64>,
}

/// The hops a sighting says the packet had taken, when it can tell: a packet
/// that did not say how many it started with (`hop_start` 0, from firmware
/// before 2.3) cannot.
fn hops_taken(row: &PacketSeen) -> Option<i64> {
    let start = row.hop_start.filter(|start| *start > 0)?;
    let left = row.hop_limit?;
    (left <= start).then_some(start - left)
}

/// The sightings of each packet in `packet_ids`, summed up, in that order;
/// those with none are left out.
fn summarize(packet_ids: &[i64], rows: &[PacketSeen]) -> Vec<SightingSummary> {
    fn widen<T: PartialOrd + Copy>(range: &mut Option<(T, T)>, value: T) {
        *range = Some(match *range {
            None => (value, value),
            Some((low, high)) => (
                if value < low { value } else { low },
                if value > high { value } else { high },
            ),
        });
    }
    let mut by_packet: BTreeMap<i64, Vec<&PacketSeen>> = BTreeMap::new();
    for row in rows {
        by_packet.entry(row.packet_id).or_default().push(row);
    }
    packet_ids
        .iter()
        .filter_map(|id| {
            let rows = by_packet.get(id)?;
            let (mut rssi, mut snr, mut hops) = (None, None, None);
            for row in rows {
                // No strength, no signal: what a gateway uploads of its own
                // has 0 for both, which is not a reading.
                if let Some(value) = row.rx_rssi.filter(|rssi| *rssi != 0) {
                    widen(&mut rssi, value);
                    if let Some(value) = row.rx_snr.filter(|snr| snr.is_finite()) {
                        widen(&mut snr, value);
                    }
                }
                if let Some(value) = hops_taken(row) {
                    widen(&mut hops, value);
                }
            }
            let gateways = rows.iter().map(|row| row.node_id).collect::<BTreeSet<_>>();
            Some(SightingSummary {
                packet_id: *id,
                gateways: gateways.len() as i64,
                rssi_min: rssi.map(|(low, _)| low),
                rssi_max: rssi.map(|(_, high)| high),
                snr_min: snr.map(|(low, _)| low),
                snr_max: snr.map(|(_, high)| high),
                hops_min: hops.map(|(low, _)| low),
                hops_max: hops.map(|(_, high)| high),
            })
        })
        .collect()
}

/// Packet sightings, summed up
///
/// For each of several packets, how many gateways heard it and the range of
/// what they heard: signal strength, signal to noise ratio and hops taken. It
/// is for a table of packets, where every sighting of each would be too much;
/// `/packets_seen/{packet_id}` has those of one.
///
/// Gateways go on reporting a packet for some seconds after it is first
/// stored, so the summary of a packet just in can still grow: ask again for
/// the latest ones.
#[utoipa::path(
    get,
    path = "/packets_seen",
    tag = "packets",
    params(("packet_ids" = String, Query, description = "The packets' stored ids, in decimal, separated by commas: up to 100.")),
    responses(
        (status = 200, description = "The summaries, maybe none", body = SightingSummaries),
        (status = 400, description = "`packet_ids` is missing, has more than 100, or one is not an integer", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn sighting_summaries(
    State(state): State<AppState>,
    params: Params,
) -> Result<Json<SightingSummaries>, ApiError> {
    let invalid = || ApiError::bad_request("Invalid or missing packet_ids");
    let mut packet_ids = Vec::new();
    for id in params.text("packet_ids").ok_or_else(invalid)?.split(',') {
        let id = parse_db_int(id.trim()).ok_or_else(invalid)?;
        if !packet_ids.contains(&id) {
            packet_ids.push(id);
        }
    }
    if packet_ids.len() > MAX_SUMMARIES {
        return Err(ApiError::bad_request("Too many packet_ids, up to 100"));
    }

    let rows = state
        .db()?
        .sightings_of(&packet_ids)
        .await
        .map_err(|e| ApiError::internal("Internal server error", &e))?;

    Ok(Json(SightingSummaries {
        packets: summarize(&packet_ids, &rows),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(packet_id: i64, node_id: i64, rssi: i64, snr: f64, hops: (i64, i64)) -> PacketSeen {
        PacketSeen {
            packet_id,
            node_id,
            rx_time: 0,
            hop_limit: Some(hops.1),
            hop_start: Some(hops.0),
            channel: None,
            rx_snr: Some(snr),
            rx_rssi: Some(rssi),
            topic: None,
            import_time_us: None,
        }
    }

    #[test]
    fn sightings_sum_up_into_ranges_leaving_out_what_was_not_reported() {
        let rows = [
            seen(1, 10, -64, 12.5, (7, 7)),
            seen(1, 11, -124, -7.5, (7, 2)),
            // The same gateway again, by another way: one gateway still.
            seen(1, 11, -100, 1.0, (7, 4)),
            // No signal strength, and so no signal; a packet that did not say
            // how many hops it started with.
            seen(1, 12, 0, 0.0, (0, 0)),
            // A reading that is not a number.
            seen(1, 13, -110, f64::NAN, (7, 6)),
            seen(2, 10, -90, 5.0, (3, 3)),
        ];
        let summaries = summarize(&[2, 3, 1], &rows);
        assert_eq!(
            summaries,
            [
                SightingSummary {
                    packet_id: 2,
                    gateways: 1,
                    rssi_min: Some(-90),
                    rssi_max: Some(-90),
                    snr_min: Some(5.0),
                    snr_max: Some(5.0),
                    hops_min: Some(0),
                    hops_max: Some(0),
                },
                SightingSummary {
                    packet_id: 1,
                    gateways: 4,
                    rssi_min: Some(-124),
                    rssi_max: Some(-64),
                    snr_min: Some(-7.5),
                    snr_max: Some(12.5),
                    hops_min: Some(0),
                    hops_max: Some(5),
                },
            ]
        );
    }

    #[test]
    fn a_packet_with_nothing_reported_has_no_ranges() {
        let summaries = summarize(&[1], &[seen(1, 10, 0, f64::NAN, (0, 3))]);
        assert_eq!(summaries[0].gateways, 1);
        assert_eq!(summaries[0].rssi_min, None);
        assert_eq!(summaries[0].snr_max, None);
        assert_eq!(summaries[0].hops_min, None);
    }
}
