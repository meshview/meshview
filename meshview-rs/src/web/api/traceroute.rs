// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use super::error::{ApiError, ErrorBody};
use super::params::parse_db_int;
use crate::decode;
use crate::state::AppState;
use crate::traceroute::{endpoints, forward_path, return_is_complete, return_path};
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use utoipa::ToSchema;

/// The most traceroute rows read for one packet: one per gateway that heard it.
const MAX_ROWS: i64 = 200;

#[derive(Debug, Serialize, ToSchema)]
pub struct Traceroute {
    pub packet: TraceroutePacket,
    /// What each gateway reported, in the order they were heard, leaving out
    /// those that could not be read; `index` counts them all.
    pub traceroute_packets: Vec<Report>,
    /// Every forward route reported, once, with how many reported it and
    /// whether any of those was a finished traceroute.
    pub unique_forward_paths: Vec<UniquePath>,
    /// Every return route reported, once.
    pub unique_reverse_paths: Vec<Vec<i64>>,
    /// The whole paths of the finished traceroutes, endpoints included.
    pub winning_paths: WinningPaths,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceroutePacket {
    pub id: i64,
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// The node that started the traceroute.
    pub initiator: Option<i64>,
    /// The node it traced.
    pub target: Option<i64>,
    pub channel: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Report {
    pub index: usize,
    pub gateway_node_id: Option<i64>,
    /// Whether this was the finished traceroute, its answer.
    pub done: Option<bool>,
    pub forward_hops: Vec<i64>,
    pub reverse_hops: Vec<i64>,
    /// Whether the answer was seen back at the node that started it.
    pub reverse_complete: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UniquePath {
    pub path: Vec<i64>,
    pub count: usize,
    pub complete: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WinningPaths {
    pub forward: Vec<Vec<i64>>,
    pub reverse: Vec<Vec<i64>>,
}

/// Traceroute
///
/// A traceroute as the gateways reported it: the hops each saw, the different
/// routes among them, and the whole paths of the finished traceroutes from the
/// node that started it to the one it traced and back.
#[utoipa::path(
    get,
    path = "/traceroute/{packet_id}",
    tag = "packets",
    params(("packet_id" = i64, Path, description = "The traceroute packet's stored id.")),
    responses(
        (status = 200, description = "The traceroute", body = Traceroute),
        (status = 400, description = "`packet_id` is not an integer", body = ErrorBody),
        (status = 404, description = "There is no such packet", body = ErrorBody),
        (status = 500, description = "It could not be read", body = ErrorBody),
    )
)]
pub async fn traceroute(
    State(state): State<AppState>,
    Path(packet_id): Path<String>,
) -> Result<Json<Traceroute>, ApiError> {
    let packet_id = parse_db_int(&packet_id)
        .ok_or_else(|| ApiError::bad_request("packet_id must be an integer"))?;
    let db = state.db()?;
    let failed = |e| ApiError::internal("Failed to fetch traceroute", &e);

    let rows = db.traceroutes(packet_id, MAX_ROWS).await.map_err(failed)?;
    let packet = db
        .packet(packet_id)
        .await
        .map_err(failed)?
        .ok_or_else(|| ApiError::not_found("Packet not found"))?;
    let (from, to) = (packet.from_node_id, packet.to_node_id);

    let reports: Vec<Report> = rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            // A row without a route is one that cannot be read, as one whose
            // route does not decode: the Python version failed on it instead.
            let route = decode::route(row.route.as_deref()?)?;
            Some(Report {
                index,
                gateway_node_id: row.gateway_node_id,
                done: row.done,
                reverse_complete: return_is_complete(route.snr_back.len(), route.route_back.len()),
                forward_hops: route.route.iter().map(|&hop| i64::from(hop)).collect(),
                reverse_hops: route.route_back.iter().map(|&hop| i64::from(hop)).collect(),
            })
        })
        .collect();

    let mut forward_counts: BTreeMap<Vec<i64>, usize> = BTreeMap::new();
    let mut reverse_paths: BTreeSet<Vec<i64>> = BTreeSet::new();
    let mut complete_forward: BTreeSet<Vec<i64>> = BTreeSet::new();
    let mut winning_forward: Vec<Vec<i64>> = Vec::new();
    let mut winning_reverse: Vec<(Vec<i64>, bool)> = Vec::new();

    for report in &reports {
        if !report.forward_hops.is_empty() {
            *forward_counts
                .entry(report.forward_hops.clone())
                .or_default() += 1;
        }
        if !report.reverse_hops.is_empty() {
            reverse_paths.insert(report.reverse_hops.clone());
        }
        if report.done == Some(true) {
            complete_forward.insert(report.forward_hops.clone());
            winning_forward.push(report.forward_hops.clone());
            winning_reverse.push((report.reverse_hops.clone(), report.reverse_complete));
        }
    }

    // Each finished route once, in the order they were first reported.
    let mut forward: Vec<Vec<i64>> = Vec::new();
    let mut seen_hops: Vec<Vec<i64>> = Vec::new();
    for hops in winning_forward {
        if seen_hops.contains(&hops) {
            continue;
        }
        let path = forward_path(from, to, &hops, true, None);
        seen_hops.push(hops);
        if path.len() > 1 {
            forward.push(path);
        }
    }
    let mut reverse: Vec<Vec<i64>> = Vec::new();
    for (hops, complete) in winning_reverse {
        let path = return_path(from, to, &hops, complete);
        if path.len() > 1 && !reverse.contains(&path) {
            reverse.push(path);
        }
    }

    let done = reports.iter().any(|report| report.done == Some(true));
    let (initiator, target) = endpoints(from, to, done);

    Ok(Json(Traceroute {
        packet: TraceroutePacket {
            id: packet.id,
            from,
            to,
            initiator,
            target,
            channel: packet.channel,
        },
        unique_forward_paths: forward_counts
            .into_iter()
            .map(|(path, count)| UniquePath {
                complete: complete_forward.contains(&path),
                path,
                count,
            })
            .collect(),
        unique_reverse_paths: reverse_paths.into_iter().collect(),
        winning_paths: WinningPaths { forward, reverse },
        traceroute_packets: reports,
    }))
}
