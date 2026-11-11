// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Figures about the mesh: packets over time, totals, and the nodes with the
//! most traffic.

use super::error::{ApiError, ErrorBody};
use super::now_us;
use super::params::{Params, parse_db_int};
use crate::db::{PacketFilter, Period, PeriodCount, SightingsOf, TopQuery};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

fn period(name: &str) -> Option<Period> {
    match name {
        "hour" => Some(Period::Hour),
        "day" => Some(Period::Day),
        _ => None,
    }
}

/// An integer filter that, once given, has to be one: given empty, it is not.
/// This is how the Python version reads these, unlike its other integers.
fn strict_int(
    params: &Params,
    name: &str,
    message: &str,
    bits32: bool,
) -> Result<Option<i64>, ApiError> {
    let Some(raw) = params.get(name) else {
        return Ok(None);
    };
    parse_db_int(raw)
        .filter(|value| !bits32 || i32::try_from(*value).is_ok())
        .map(Some)
        .ok_or_else(|| ApiError::bad_request(message))
}

/// The answer with a series.
#[derive(Debug, Serialize, ToSchema)]
pub struct Series {
    #[schema(example = "hour")]
    pub period_type: String,
    pub length: i64,
    pub channel: Option<String>,
    pub portnum: Option<i64>,
    pub to_node: Option<i64>,
    pub from_node: Option<i64>,
    /// The periods that had packets, oldest first.
    pub data: Vec<PeriodPoint>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PeriodPoint {
    /// `YYYY-MM-DD HH:00` or `YYYY-MM-DD`, in UTC.
    #[schema(example = "2026-11-11 09:00")]
    pub period: String,
    pub count: i64,
}

impl From<PeriodCount> for PeriodPoint {
    fn from(row: PeriodCount) -> Self {
        Self {
            period: row.period,
            count: row.count,
        }
    }
}

/// The answer for one node.
#[derive(Debug, Serialize, ToSchema)]
pub struct NodeTotals {
    pub node_id: i64,
    pub period_type: String,
    pub length: i64,
    /// Packets the node sent in the window.
    pub sent: i64,
    /// Packets sent to the node in the window.
    pub seen: i64,
}

/// The answer is one of the two shapes.
#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum Stats {
    Series(Series),
    Node(NodeTotals),
}

/// Packets over time
///
/// How many packets were heard in each hour or day of the latest ones, oldest
/// first, leaving out the periods with none; optionally only those on a channel,
/// of a port, or to or from a node.
///
/// With `node`, instead, how many packets that node sent, and how many were sent
/// to it, in the whole window. (The Python version answered 0 for both, always.)
#[utoipa::path(
    get,
    path = "/stats",
    tag = "stats",
    params(
        ("period_type" = Option<String>, Query, description = "`hour` or `day`, in any case. Default `hour`."),
        ("length" = Option<i64>, Query, description = "How many of them, 1 to 3650. Default 24."),
        ("channel" = Option<String>, Query, description = "Only packets on this channel, ignoring case."),
        ("portnum" = Option<i64>, Query, description = "Only packets of this port."),
        ("to_node" = Option<i64>, Query, description = "Only packets to this node."),
        ("from_node" = Option<i64>, Query, description = "Only packets from this node."),
        ("node" = Option<i64>, Query, description = "Totals for this node instead of a series. The other filters do not apply."),
    ),
    responses(
        (status = 200, description = "The series, or the node's totals", body = Stats),
        (status = 400, description = "A parameter is not valid", body = ErrorBody),
    )
)]
pub async fn stats(State(state): State<AppState>, params: Params) -> Result<Json<Stats>, ApiError> {
    let period_type = params.get("period_type").unwrap_or("hour").to_lowercase();
    let Some(unit) = period(&period_type) else {
        // In a fixed order: the Python version printed a set, in whatever order
        // its process happened to hash it.
        return Err(ApiError::bad_request(
            "Invalid period_type. Must be one of {'hour', 'day'}",
        ));
    };
    let length = params
        .int("length", Some(24), Some(1), Some(3650))?
        .unwrap_or(24);
    let since_us = now_us() - length * unit.micros();
    let db = state.db()?;
    let failed = |e| ApiError::internal("Failed to fetch stats", &e);

    if let Some(node) = params.text("node") {
        let node_id =
            parse_db_int(node).ok_or_else(|| ApiError::bad_request("node must be an integer"))?;
        let total = |filter: PacketFilter| async move {
            db.packets_per_period(unit, &filter)
                .await
                .map(|rows| rows.iter().map(|row| row.count).sum::<i64>())
        };
        let sent = total(PacketFilter {
            since_us,
            from_node: Some(node_id),
            ..PacketFilter::default()
        })
        .await
        .map_err(failed)?;
        let seen = total(PacketFilter {
            since_us,
            to_node: Some(node_id),
            ..PacketFilter::default()
        })
        .await
        .map_err(failed)?;
        return Ok(Json(Stats::Node(NodeTotals {
            node_id,
            period_type,
            length,
            sent,
            seen,
        })));
    }

    let channel = params.get("channel").map(str::to_string);
    let portnum = strict_int(&params, "portnum", "portnum must be an integer", true)?;
    let to_node = strict_int(&params, "to_node", "to_node must be an integer", false)?;
    let from_node = strict_int(&params, "from_node", "from_node must be an integer", false)?;

    let filter = PacketFilter {
        since_us,
        channel: channel.clone().filter(|c| !c.is_empty()),
        portnum,
        to_node,
        from_node,
    };
    let data = db.packets_per_period(unit, &filter).await.map_err(failed)?;

    Ok(Json(Stats::Series(Series {
        period_type,
        length,
        channel,
        portnum,
        to_node,
        from_node,
        data: data.into_iter().map(PeriodPoint::from).collect(),
    })))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Counts {
    pub total_packets: i64,
    pub total_seen: i64,
}

/// Totals
///
/// How many packets and sightings there are: all of them, or those of the
/// packets imported in the latest hours or days (by default the latest day),
/// optionally only on a channel or to or from a node. With `packet_id`, the
/// sightings are those of that packet.
#[utoipa::path(
    get,
    path = "/stats/count",
    tag = "stats",
    params(
        ("period_type" = Option<String>, Query, description = "`hour` or `day`. Default `day` once any filter is given."),
        ("length" = Option<i64>, Query, description = "How many of them, 1 to 3650. Default 1 once any filter is given."),
        ("channel" = Option<String>, Query, description = "Only packets on this channel, ignoring case."),
        ("from_node" = Option<i64>, Query, description = "Only packets from this node."),
        ("to_node" = Option<i64>, Query, description = "Only packets to this node."),
        ("packet_id" = Option<i64>, Query, description = "Count the sightings of this packet."),
    ),
    responses(
        (status = 200, description = "The totals", body = Counts),
        (status = 400, description = "A parameter is not valid", body = ErrorBody),
    )
)]
pub async fn count(
    State(state): State<AppState>,
    params: Params,
) -> Result<Json<Counts>, ApiError> {
    let packet_id = match params.text("packet_id") {
        None => None,
        Some(raw) => Some(
            parse_db_int(raw).ok_or_else(|| ApiError::bad_request("packet_id must be integer"))?,
        ),
    };
    let period_type = params.text("period_type");
    let length = params.int("length", None, Some(1), Some(3650))?;
    let channel = params.text("channel");
    let from_node = strict_int(&params, "from_node", "from_node must be integer", false)?;
    let to_node = strict_int(&params, "to_node", "to_node must be integer", false)?;

    let db = state.db()?;
    let failed = |e| ApiError::internal("Failed to count packets", &e);

    let windowed = period_type.is_some()
        || length.is_some()
        || channel.is_some()
        || from_node.is_some()
        || to_node.is_some();
    let filter = if windowed {
        let unit = period(period_type.unwrap_or("day"))
            .ok_or_else(|| ApiError::bad_request("period_type must be 'hour' or 'day'"))?;
        Some(PacketFilter {
            since_us: now_us() - length.unwrap_or(1) * unit.micros(),
            channel: channel.map(str::to_string),
            portnum: None,
            // Node 0 is no node: the Python version ignored a filter on it.
            to_node: to_node.filter(|&node| node != 0),
            from_node: from_node.filter(|&node| node != 0),
        })
    } else {
        None
    };

    let total_packets = db.count_packets(filter.as_ref()).await.map_err(failed)?;
    let total_seen = db
        .count_sightings(match (packet_id, &filter) {
            (Some(packet_id), _) => SightingsOf::Packet(packet_id),
            (None, Some(filter)) => SightingsOf::Packets(filter),
            (None, None) => SightingsOf::All,
        })
        .await
        .map_err(failed)?;

    Ok(Json(Counts {
        total_packets,
        total_seen,
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Top {
    /// How many nodes there are in all, before paging.
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    pub nodes: Vec<TopEntry>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TopEntry {
    pub node_id: Option<i64>,
    pub long_name: Option<String>,
    pub short_name: Option<String>,
    pub channel: Option<String>,
    /// Packets it sent in the window.
    pub sent: i64,
    /// Times its packets were heard by a gateway in the window.
    pub seen: i64,
    /// Sightings per packet sent, to two decimals.
    #[schema(example = 2.4)]
    pub avg: f64,
}

/// Top nodes
///
/// The nodes by how often their packets were heard, then by how many they sent,
/// in the latest hours or days. The window ends at the latest packet, and the
/// latest sighting, rather than now, so a mesh gone quiet still has a list.
#[utoipa::path(
    get,
    path = "/stats/top",
    tag = "stats",
    params(
        ("period_type" = Option<String>, Query, description = "`hour`, or anything else for days. Default `day`."),
        ("length" = Option<i64>, Query, description = "How many of them, 1 to 3650. Default 1."),
        ("channel" = Option<String>, Query, description = "Only nodes on this channel, exactly."),
        ("limit" = Option<i64>, Query, description = "Nodes per page, 1 to 100. Default 20."),
        ("offset" = Option<i64>, Query, description = "Nodes to skip. Default 0."),
        ("active" = Option<bool>, Query, description = "Only nodes that sent or were heard in the window. Default false."),
    ),
    responses(
        (status = 200, description = "A page of nodes", body = Top),
        (status = 400, description = "A parameter is not valid", body = ErrorBody),
    )
)]
pub async fn top(State(state): State<AppState>, params: Params) -> Result<Json<Top>, ApiError> {
    let unit = if params.get("period_type").unwrap_or("day") == "hour" {
        Period::Hour
    } else {
        Period::Day
    };
    let length = params
        .int("length", Some(1), Some(1), Some(3650))?
        .unwrap_or(1);
    let channel = params.text("channel").map(str::to_string);
    let limit = params
        .int("limit", Some(20), Some(1), Some(100))?
        .unwrap_or(20);
    let offset = params.int("offset", Some(0), Some(0), None)?.unwrap_or(0);
    let active = params.bool("active")?.unwrap_or(false);

    let (total, nodes) = state
        .db()?
        .top_nodes(&TopQuery {
            window_us: length * unit.micros(),
            channel,
            active,
            limit,
            offset,
        })
        .await
        .map_err(|e| ApiError::internal("Failed to fetch top nodes", &e))?;

    Ok(Json(Top {
        total,
        limit,
        offset,
        nodes: nodes
            .into_iter()
            .map(|node| TopEntry {
                avg: two_decimals(node.seen as f64 / node.sent.max(1) as f64),
                node_id: node.node_id,
                long_name: node.long_name,
                short_name: node.short_name,
                channel: node.channel,
                sent: node.sent,
                seen: node.seen,
            })
            .collect(),
    }))
}

/// Rounded to two decimals as Python's `round(x, 2)` rounds: on the exact value
/// of the float, half to even. Rust's formatting does exactly that, which
/// multiplying by 100 and rounding does not.
fn two_decimals(value: f64) -> f64 {
    format!("{value:.2}").parse().unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_round_as_python_rounds_them() {
        assert_eq!(two_decimals(1.0 / 8.0), 0.12);
        assert_eq!(two_decimals(3.0 / 8.0), 0.38);
        assert_eq!(two_decimals(2.0 / 3.0), 0.67);
        assert_eq!(two_decimals(12.0 / 5.0), 2.4);
    }
}
