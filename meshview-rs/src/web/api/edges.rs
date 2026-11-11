// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The links between nodes heard in the latest hours, for the node graph and
//! the map: those traceroutes went through, and those nodes reported having.

use super::error::{ApiError, ErrorBody};
use super::now_us;
use super::params::{Params, parse_python_int};
use crate::db::{Edge, PacketQuery, Reader};
use crate::decode;
use crate::state::AppState;
use crate::traceroute;
use axum::Json;
use axum::extract::State;
use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
use meshview_proto::meshtastic::{MeshPacket, NeighborInfo, PortNum};
use meshview_proto::wire;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use utoipa::ToSchema;

/// How far back links are looked for.
const WINDOW_US: i64 = 12 * 3600 * 1_000_000;

/// How long a built list of links is answered from before it is built again.
/// Building it reads half a day of traceroutes and neighbour reports.
const CACHE_FOR: Duration = Duration::from_secs(60);

const TRACEROUTE: &str = "traceroute";
const NEIGHBOR: &str = "neighbor";

#[derive(Debug, Serialize, ToSchema)]
pub struct Edges {
    /// The links, each once.
    pub edges: Vec<Edge>,
}

/// The links, in the order they were first heard: traceroutes' first, the
/// oldest first, then neighbours', the latest report first. A link known both
/// ways is listed once, as the first.
async fn build(db: &Reader, kind: Option<&'static str>) -> anyhow::Result<Vec<Edge>> {
    let since_us = now_us() - WINDOW_US;
    let mut edges = Vec::new();
    let mut known = HashSet::new();
    let mut add = |from: i64, to: i64, kind: &'static str| {
        if known.insert((from, to)) {
            edges.push(Edge { from, to, kind });
        }
    };

    if kind.is_none_or(|kind| kind == TRACEROUTE) {
        for row in db.traceroutes_since(since_us).await? {
            let Some(route) = row.route.as_deref().and_then(decode::route) else {
                continue;
            };
            if !row.has_packet {
                continue;
            }
            let hops: Vec<i64> = route.route.iter().map(|&hop| i64::from(hop)).collect();
            let path = traceroute::forward_path(
                row.from_node_id,
                row.to_node_id,
                &hops,
                row.done == Some(true),
                row.gateway_node_id,
            );
            for (a, b) in traceroute::edges(&path) {
                add(a, b, TRACEROUTE);
            }
        }
    }

    if kind.is_none_or(|kind| kind == NEIGHBOR) {
        let reports = db
            .packets(&PacketQuery {
                portnum: Some(PortNum::NeighborinfoApp as i64),
                after_us: Some(since_us),
                limit: -1,
                ..PacketQuery::default()
            })
            .await?;
        // Only each node's latest report counts.
        let mut reported = HashSet::new();
        for report in reports {
            let Some(reporter) = report.from_node_id else {
                continue;
            };
            if reported.contains(&reporter) {
                continue;
            }
            let Some(info) = report
                .payload
                .as_deref()
                .and_then(|bytes| wire::decode::<MeshPacket>("meshtastic.MeshPacket", bytes))
                .and_then(|mesh| match mesh.payload_variant {
                    Some(PayloadVariant::Decoded(data))
                        if data.portnum == PortNum::NeighborinfoApp as i32 =>
                    {
                        wire::decode::<NeighborInfo>("meshtastic.NeighborInfo", &data.payload)
                    }
                    _ => None,
                })
            else {
                continue;
            };
            reported.insert(reporter);
            for neighbor in info.neighbors {
                add(i64::from(neighbor.node_id), reporter, NEIGHBOR);
            }
        }
    }

    Ok(edges)
}

/// Links
///
/// The links between nodes in the latest 12 hours: those a traceroute went
/// through, from the node that started it, and those a node reported having
/// with its neighbours, from the neighbour to it. Built at most once a minute, so a new
/// link can take a minute to show.
///
/// A finished traceroute is heard as its answer, which goes the other way, so
/// its links run from the answer's `to`, through its route, to its `from`.
/// Hops the mesh could not name come as `0xFFFFFFFF`: the links on either side
/// of one are left out, because the nodes around an unknown hop are not
/// neighbours.
///
/// Neighbour links come from each node's latest neighbour info report (port
/// 71) in the window, which lists its neighbours of the moment.
///
/// A node with no links, or that is not there, gets none, with 200.
#[utoipa::path(
    get,
    path = "/edges",
    tag = "nodes",
    params(
        ("type" = Option<String>, Query, description = "Only `traceroute` or only `neighbor` links. Anything else gets none."),
        ("node_id" = Option<i64>, Query, description = "Only links from or to this node."),
    ),
    responses(
        (status = 200, description = "The links", body = Edges, example = json!({
            "edges": [
                {"from": 12345678, "to": 87654321, "type": "traceroute"},
                {"from": 11111111, "to": 22222222, "type": "neighbor"},
            ],
        })),
        (status = 400, description = "`node_id` is not an integer", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn edges(State(state): State<AppState>, params: Params) -> Result<Json<Edges>, ApiError> {
    let kind = match params.text("type") {
        None => None,
        Some(TRACEROUTE) => Some(TRACEROUTE),
        Some(NEIGHBOR) => Some(NEIGHBOR),
        Some(_) => return Ok(Json(Edges { edges: Vec::new() })),
    };
    let node = match params.text("node_id") {
        None => None,
        Some(raw) => Some(
            parse_python_int(raw)
                .ok_or_else(|| ApiError::bad_request("node_id must be integer"))?,
        ),
    };

    let cached = {
        let cache = state.caches.edges.lock().expect("never poisoned");
        cache
            .get(&kind)
            .filter(|(expiry, _)| *expiry > Instant::now())
            .map(|(_, edges)| Arc::clone(edges))
    };
    let all = match cached {
        Some(edges) => edges,
        None => {
            let built = Arc::new(
                build(state.db()?, kind)
                    .await
                    .map_err(|e| ApiError::internal("Failed to fetch edges", &e))?,
            );
            state
                .caches
                .edges
                .lock()
                .expect("never poisoned")
                .insert(kind, (Instant::now() + CACHE_FOR, Arc::clone(&built)));
            built
        }
    };

    let edges = match node {
        None => all.as_ref().clone(),
        Some(node) => all
            .iter()
            .filter(|edge| i128::from(edge.from) == node || i128::from(edge.to) == node)
            .cloned()
            .collect(),
    };
    Ok(Json(Edges { edges }))
}
