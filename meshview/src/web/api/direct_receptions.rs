// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use super::error::{ApiError, ErrorBody};
use super::params::{Params, parse_db_int};
use crate::db::{DirectReception, Direction};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;

/// The most receptions one answer carries: the latest, when there are more.
const MAX_RECEPTIONS: i64 = 20_000;

#[derive(Debug, Serialize, ToSchema)]
pub struct DirectReceptions {
    /// The node asked about.
    #[schema(example = 825542997)]
    pub node_id: i64,
    /// `received` or `transmitted`, as asked.
    #[schema(example = "received")]
    pub direction: &'static str,
    /// Whether there were more receptions than an answer carries, so that
    /// only the latest are here.
    pub truncated: bool,
    /// One for each node on the other side, the one heard most first.
    pub nodes: Vec<DirectPeer>,
}

/// A node on the other side of the direct receptions, with what it was heard
/// with.
#[derive(Debug, PartialEq, Serialize, ToSchema)]
pub struct DirectPeer {
    /// The sender of the received packets, or the gateway that heard the
    /// transmitted ones.
    #[schema(example = 1125685839)]
    pub node_id: i64,
    #[schema(example = "ground1")]
    pub long_name: Option<String>,
    #[schema(example = "gnd1")]
    pub short_name: Option<String>,
    /// How many receptions there are.
    #[schema(example = 42)]
    pub packets: i64,
    /// Received signal strength, in dBm: the mean, the weakest and the
    /// strongest.
    #[schema(example = -114.8)]
    pub rssi_avg: f64,
    #[schema(example = -123)]
    pub rssi_min: i64,
    #[schema(example = -104)]
    pub rssi_max: i64,
    /// Signal to noise ratio, in dB, likewise; null when none was a number.
    #[schema(example = -1.2)]
    pub snr_avg: Option<f64>,
    #[schema(example = -10.0)]
    pub snr_min: Option<f64>,
    #[schema(example = 7.25)]
    pub snr_max: Option<f64>,
    /// The receptions, the earliest first.
    pub receptions: Vec<Reception>,
}

/// One packet heard straight from the node that sent it.
#[derive(Debug, PartialEq, Serialize, ToSchema)]
pub struct Reception {
    /// The packet's stored id.
    #[schema(example = 1218311)]
    pub packet_id: i64,
    /// When it was stored, in microseconds since the epoch.
    #[schema(example = 1792271863787589_i64)]
    pub time_us: i64,
    /// Received signal strength, in dBm.
    #[schema(example = -116)]
    pub rssi: i64,
    /// Signal to noise ratio, in dB; null when it was not a number.
    #[schema(example = -3.25)]
    pub snr: Option<f64>,
}

/// The receptions, latest first as the database gives them, gathered by the
/// node on the other side, the one heard most first.
fn by_peer(rows: Vec<DirectReception>) -> Vec<DirectPeer> {
    let mut order: Vec<i64> = Vec::new();
    let mut peers: HashMap<i64, DirectPeer> = HashMap::new();
    for row in rows.into_iter().rev() {
        let snr = row.snr.filter(|snr| snr.is_finite());
        let peer = peers.entry(row.peer).or_insert_with(|| {
            order.push(row.peer);
            DirectPeer {
                node_id: row.peer,
                long_name: None,
                short_name: None,
                packets: 0,
                rssi_avg: 0.0,
                rssi_min: row.rssi,
                rssi_max: row.rssi,
                snr_avg: None,
                snr_min: None,
                snr_max: None,
                receptions: Vec::new(),
            }
        });
        peer.long_name = row.long_name.or(peer.long_name.take());
        peer.short_name = row.short_name.or(peer.short_name.take());
        peer.rssi_min = peer.rssi_min.min(row.rssi);
        peer.rssi_max = peer.rssi_max.max(row.rssi);
        if let Some(snr) = snr {
            peer.snr_min = Some(peer.snr_min.map_or(snr, |low| low.min(snr)));
            peer.snr_max = Some(peer.snr_max.map_or(snr, |high| high.max(snr)));
        }
        peer.receptions.push(Reception {
            packet_id: row.packet_id,
            time_us: row.time_us,
            rssi: row.rssi,
            snr,
        });
    }
    let mut peers: Vec<DirectPeer> = order
        .into_iter()
        .filter_map(|id| peers.remove(&id))
        .map(|mut peer| {
            let count = peer.receptions.len();
            peer.packets = count as i64;
            peer.rssi_avg =
                round(peer.receptions.iter().map(|r| r.rssi as f64).sum::<f64>() / count as f64);
            let snrs: Vec<f64> = peer.receptions.iter().filter_map(|r| r.snr).collect();
            peer.snr_avg =
                (!snrs.is_empty()).then(|| round(snrs.iter().sum::<f64>() / snrs.len() as f64));
            peer
        })
        .collect();
    peers.sort_by(|a, b| b.packets.cmp(&a.packets).then(a.node_id.cmp(&b.node_id)));
    peers
}

/// A mean to the tenth, as signal is read.
fn round(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Direct receptions
///
/// The packets heard straight from the node that sent them, at no hops, with
/// the signal they were heard with, gathered by the node on the other side.
///
/// `received` (the default) is what the node, as an MQTT gateway, heard of
/// other nodes; `transmitted`, what gateways heard of the node. Only what a
/// gateway reported as heard counts: a packet that says how many hops it
/// started with (firmware 2.3 or later) and has taken none, with a signal
/// strength other than 0. A node that is not a gateway has no received ones.
///
/// At most 20 000 receptions come in one answer, the latest; `truncated`
/// says when there were more.
#[utoipa::path(
    get,
    path = "/node/{node_id}/direct_receptions",
    tag = "nodes",
    params(
        ("node_id" = i64, Path, description = "The node's number, in decimal."),
        ("direction" = Option<String>, Query, description = "`received` (the default) or `transmitted`."),
        ("since" = Option<i64>, Query, description = "Only receptions stored at or after this time, in microseconds since the epoch. All that are kept when absent."),
    ),
    responses(
        (status = 200, description = "The receptions, maybe none", body = DirectReceptions),
        (status = 400, description = "`node_id` or `since` is not an integer, or `direction` is neither value", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn direct_receptions(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    params: Params,
) -> Result<Json<DirectReceptions>, ApiError> {
    let node_id = parse_db_int(&node_id)
        .ok_or_else(|| ApiError::bad_request("Invalid or missing node_id"))?;
    let (direction, name) = match params.text("direction") {
        None | Some("received") => (Direction::Received, "received"),
        Some("transmitted") => (Direction::Transmitted, "transmitted"),
        Some(_) => {
            return Err(ApiError::bad_request(
                "direction must be 'received' or 'transmitted'",
            ));
        }
    };
    let since_us = params.int("since", Some(0), Some(0), None)?.unwrap_or(0);

    let mut rows = state
        .db()?
        .direct_receptions(node_id, direction, since_us, MAX_RECEPTIONS + 1)
        .await
        .map_err(|e| ApiError::internal("Internal server error", &e))?;
    let truncated = rows.len() as i64 > MAX_RECEPTIONS;
    rows.truncate(MAX_RECEPTIONS as usize);

    Ok(Json(DirectReceptions {
        node_id,
        direction: name,
        truncated,
        nodes: by_peer(rows),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(peer: i64, packet_id: i64, time_us: i64, rssi: i64, snr: f64) -> DirectReception {
        DirectReception {
            peer,
            long_name: Some(format!("Node {peer}")),
            short_name: None,
            packet_id,
            time_us,
            rssi,
            snr: Some(snr),
        }
    }

    #[test]
    fn receptions_gather_by_node_the_one_heard_most_first() {
        // Latest first, as the database gives them.
        let rows = vec![
            row(2, 5, 500, -100, 4.0),
            row(1, 4, 400, -110, f64::NAN),
            row(1, 3, 300, -120, -6.0),
            row(1, 2, 200, -115, -2.0),
            row(2, 1, 100, -90, 8.0),
        ];
        let peers = by_peer(rows);
        assert_eq!(peers.len(), 2);

        let one = &peers[0];
        assert_eq!((one.node_id, one.packets), (1, 3));
        assert_eq!(
            (one.rssi_min, one.rssi_max, one.rssi_avg),
            (-120, -110, -115.0)
        );
        // The reading that was not a number is left out of the signal to
        // noise ratio, and not the reception.
        assert_eq!(
            (one.snr_min, one.snr_max, one.snr_avg),
            (Some(-6.0), Some(-2.0), Some(-4.0))
        );
        let times: Vec<i64> = one.receptions.iter().map(|r| r.time_us).collect();
        assert_eq!(times, [200, 300, 400]);
        assert_eq!(one.receptions[2].snr, None);
        assert_eq!(one.long_name.as_deref(), Some("Node 1"));

        let two = &peers[1];
        assert_eq!((two.node_id, two.packets, two.rssi_avg), (2, 2, -95.0));
    }
}
