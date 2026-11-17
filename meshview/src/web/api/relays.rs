// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Who a gateway heard its packets through, and who heard through a node.
//!
//! The firmware tells only the last byte of the id of the node a packet was
//! heard from, so a relay is a byte, and the nodes it may be are those the
//! gateway heard straight from, at no hops, whose id ends in it: whoever a
//! gateway hears from it must be able to hear directly.

use super::error::{ApiError, ErrorBody};
use super::params::{Params, parse_db_int};
use crate::db::{DirectCount, Direction, RelayedCount};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct Relays {
    /// The gateway asked about.
    #[schema(example = 825542997)]
    pub node_id: i64,
    /// One for each relay, the one heard through most first.
    pub relays: Vec<Relay>,
}

/// A relay a gateway heard packets through, with the signal they arrived
/// with and the nodes it may be.
#[derive(Debug, PartialEq, Serialize, ToSchema)]
pub struct Relay {
    /// The last byte of the relay's id, from 1 to 255.
    #[schema(example = 162)]
    pub relay_node: i64,
    /// How many packets the gateway heard through it.
    #[schema(example = 512)]
    pub packets: i64,
    /// Received signal strength, in dBm: the mean, the weakest and the
    /// strongest; null when none was a reading.
    #[schema(example = -98.4)]
    pub rssi_avg: Option<f64>,
    #[schema(example = -121)]
    pub rssi_min: Option<i64>,
    #[schema(example = -80)]
    pub rssi_max: Option<i64>,
    /// Signal to noise ratio, in dB, likewise.
    #[schema(example = 4.5)]
    pub snr_avg: Option<f64>,
    #[schema(example = -11.25)]
    pub snr_min: Option<f64>,
    #[schema(example = 9.5)]
    pub snr_max: Option<f64>,
    /// The nodes the gateway heard straight from in the same time whose id
    /// ends in the relay's byte, the one heard most first. None when it heard
    /// none of them; more than one when the byte is shared.
    pub candidates: Vec<Candidate>,
}

/// A node a relay may be.
#[derive(Debug, PartialEq, Serialize, ToSchema)]
pub struct Candidate {
    #[schema(example = 2925605860_i64)]
    pub node_id: i64,
    #[schema(example = "KMesh3")]
    pub long_name: Option<String>,
    #[schema(example = "KM3")]
    pub short_name: Option<String>,
    /// How many packets the gateway heard straight from it.
    #[schema(example = 31)]
    pub direct_packets: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Relayed {
    /// The node asked about.
    #[schema(example = 2925605860_i64)]
    pub node_id: i64,
    /// The last byte of its id, the relay it is to the gateways.
    #[schema(example = 228)]
    pub relay_node: i64,
    /// One for each gateway that heard packets through it, the one that
    /// heard most first.
    pub gateways: Vec<RelayGateway>,
}

/// A gateway that heard packets through a node, with the signal they arrived
/// with.
#[derive(Debug, PartialEq, Serialize, ToSchema)]
pub struct RelayGateway {
    #[schema(example = 825542997)]
    pub node_id: i64,
    #[schema(example = "Usuario = Problema")]
    pub long_name: Option<String>,
    #[schema(example = "ART1")]
    pub short_name: Option<String>,
    /// How many packets it heard through the node.
    #[schema(example = 188)]
    pub packets: i64,
    /// Received signal strength, in dBm: the mean, the weakest and the
    /// strongest; null when none was a reading.
    #[schema(example = -95.2)]
    pub rssi_avg: Option<f64>,
    #[schema(example = -118)]
    pub rssi_min: Option<i64>,
    #[schema(example = -77)]
    pub rssi_max: Option<i64>,
    /// Signal to noise ratio, in dB, likewise.
    #[schema(example = 6.0)]
    pub snr_avg: Option<f64>,
    #[schema(example = -7.5)]
    pub snr_min: Option<f64>,
    #[schema(example = 11.0)]
    pub snr_max: Option<f64>,
    /// How many packets it heard straight from the node.
    #[schema(example = 40)]
    pub direct_packets: i64,
}

/// The last byte of a node's id, which is how relays are told.
fn relay_byte(node_id: i64) -> i64 {
    node_id & 0xff
}

/// A mean to the tenth, as signal is read; none for what was not a number.
fn round(value: Option<f64>) -> Option<f64> {
    value
        .filter(|value| value.is_finite())
        .map(|value| (value * 10.0).round() / 10.0)
}

fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
}

/// Each relay with the nodes heard straight from that end in its byte.
fn with_candidates(counts: Vec<RelayedCount>, direct: &[DirectCount]) -> Vec<Relay> {
    counts
        .into_iter()
        .map(|count| Relay {
            relay_node: count.relay_node,
            packets: count.packets,
            rssi_avg: round(count.rssi_avg),
            rssi_min: count.rssi_min,
            rssi_max: count.rssi_max,
            snr_avg: round(count.snr_avg),
            snr_min: finite(count.snr_min),
            snr_max: finite(count.snr_max),
            // Already the one heard most first.
            candidates: direct
                .iter()
                .filter(|peer| relay_byte(peer.peer) == count.relay_node)
                .map(|peer| Candidate {
                    node_id: peer.peer,
                    long_name: peer.long_name.clone(),
                    short_name: peer.short_name.clone(),
                    direct_packets: peer.packets,
                })
                .collect(),
        })
        .collect()
}

/// Each gateway that heard through the node, among those that heard it
/// straight.
fn gateways(counts: Vec<RelayedCount>, direct: &[DirectCount]) -> Vec<RelayGateway> {
    counts
        .into_iter()
        .filter_map(|count| {
            let gateway = direct.iter().find(|peer| peer.peer == count.gateway)?;
            Some(RelayGateway {
                node_id: count.gateway,
                long_name: gateway.long_name.clone(),
                short_name: gateway.short_name.clone(),
                packets: count.packets,
                rssi_avg: round(count.rssi_avg),
                rssi_min: count.rssi_min,
                rssi_max: count.rssi_max,
                snr_avg: round(count.snr_avg),
                snr_min: finite(count.snr_min),
                snr_max: finite(count.snr_max),
                direct_packets: gateway.packets,
            })
        })
        .collect()
}

fn node_and_since(node_id: &str, params: &Params) -> Result<(i64, i64), ApiError> {
    let node_id =
        parse_db_int(node_id).ok_or_else(|| ApiError::bad_request("Invalid or missing node_id"))?;
    let since_us = params.int("since", Some(0), Some(0), None)?.unwrap_or(0);
    Ok((node_id, since_us))
}

/// Relays
///
/// The relays a node, as an MQTT gateway, heard packets through: those that
/// had taken a hop at least, by the last byte of the id of the node it heard
/// them from, which is all the firmware tells of it, with the signal they
/// arrived with.
///
/// The nodes each relay may be are those the gateway heard straight from, at
/// no hops, in the same time, whose id ends in that byte: a relay a gateway
/// hears is one it can hear directly. A byte two nodes share has both; one no
/// such node ends in has none.
///
/// Only sightings stored by Meshview 5 or later, from firmware that tells the
/// relay, count.
#[utoipa::path(
    get,
    path = "/node/{node_id}/relays",
    tag = "nodes",
    params(
        ("node_id" = i64, Path, description = "The node's number, in decimal."),
        ("since" = Option<i64>, Query, description = "Only packets heard at or after this time, in microseconds since the epoch. All that are kept when absent."),
    ),
    responses(
        (status = 200, description = "The relays, maybe none", body = Relays),
        (status = 400, description = "`node_id` or `since` is not an integer", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn relays(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    params: Params,
) -> Result<Json<Relays>, ApiError> {
    let (node_id, since_us) = node_and_since(&node_id, &params)?;
    let db = state.db()?;
    let internal = |e| ApiError::internal("Internal server error", &e);

    let counts = db
        .relayed_counts(&[node_id], None, since_us)
        .await
        .map_err(internal)?;
    let direct = if counts.is_empty() {
        Vec::new()
    } else {
        db.direct_counts(node_id, Direction::Received, since_us)
            .await
            .map_err(internal)?
    };

    Ok(Json(Relays {
        node_id,
        relays: with_candidates(counts, &direct),
    }))
}

/// Gateways heard through a node
///
/// The gateways that heard packets through a node: those that had taken a
/// hop at least and that they heard from a node whose id ends in the same
/// byte as this one's, with the signal they arrived with.
///
/// Only gateways that heard the node straight, at no hops, in the same time
/// count, as one it relays to must be able to hear it; even so, another node
/// they hear straight whose id ends in the same byte cannot be told apart
/// from it.
///
/// Only sightings stored by Meshview 5 or later, from firmware that tells the
/// relay, count.
#[utoipa::path(
    get,
    path = "/node/{node_id}/relayed",
    tag = "nodes",
    params(
        ("node_id" = i64, Path, description = "The node's number, in decimal."),
        ("since" = Option<i64>, Query, description = "Only packets heard at or after this time, in microseconds since the epoch. All that are kept when absent."),
    ),
    responses(
        (status = 200, description = "The gateways, maybe none", body = Relayed),
        (status = 400, description = "`node_id` or `since` is not an integer", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn relayed(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    params: Params,
) -> Result<Json<Relayed>, ApiError> {
    let (node_id, since_us) = node_and_since(&node_id, &params)?;
    let db = state.db()?;
    let internal = |e| ApiError::internal("Internal server error", &e);
    let relay_node = relay_byte(node_id);

    let direct = db
        .direct_counts(node_id, Direction::Transmitted, since_us)
        .await
        .map_err(internal)?;
    let heard_by: Vec<i64> = direct.iter().map(|peer| peer.peer).collect();
    let counts = db
        .relayed_counts(&heard_by, Some(relay_node), since_us)
        .await
        .map_err(internal)?;

    Ok(Json(Relayed {
        node_id,
        relay_node,
        gateways: gateways(counts, &direct),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct(peer: i64, packets: i64) -> DirectCount {
        DirectCount {
            peer,
            long_name: Some(format!("Node {peer}")),
            short_name: None,
            packets,
        }
    }

    fn relayed(gateway: i64, relay_node: i64, packets: i64) -> RelayedCount {
        RelayedCount {
            gateway,
            relay_node,
            packets,
            rssi_avg: Some(-98.44),
            rssi_min: Some(-120),
            rssi_max: Some(-80),
            snr_avg: Some(f64::NAN),
            snr_min: None,
            snr_max: None,
        }
    }

    #[test]
    fn a_relay_may_be_every_node_heard_straight_that_ends_in_its_byte() {
        // 0x1a2 and 0x2a2 share 0xa2; 0x3c ends in 0x3c, which nothing came
        // through.
        let direct = [direct(0x2a2, 9), direct(0x1a2, 4), direct(0x3c, 7)];
        let relays = with_candidates(vec![relayed(1, 0xa2, 50), relayed(1, 0x7f, 3)], &direct);

        assert_eq!(relays.len(), 2);
        let shared = &relays[0];
        assert_eq!((shared.relay_node, shared.packets), (0xa2, 50));
        let candidates: Vec<(i64, i64)> = shared
            .candidates
            .iter()
            .map(|c| (c.node_id, c.direct_packets))
            .collect();
        assert_eq!(candidates, [(0x2a2, 9), (0x1a2, 4)]);
        // Signal to the tenth, and what was not a number is none.
        assert_eq!(shared.rssi_avg, Some(-98.4));
        assert_eq!(shared.snr_avg, None);

        assert!(relays[1].candidates.is_empty());
    }

    #[test]
    fn only_gateways_that_heard_the_node_straight_count() {
        let direct = [direct(10, 40), direct(20, 2)];
        // 30 heard something through the same byte, but never the node.
        let counts = vec![relayed(30, 0xe4, 99), relayed(10, 0xe4, 12)];
        let gateways = gateways(counts, &direct);

        assert_eq!(gateways.len(), 1);
        assert_eq!(
            (
                gateways[0].node_id,
                gateways[0].packets,
                gateways[0].direct_packets
            ),
            (10, 12, 40)
        );
        assert_eq!(gateways[0].long_name.as_deref(), Some("Node 10"));
    }
}
