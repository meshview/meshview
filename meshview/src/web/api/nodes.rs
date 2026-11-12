// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The nodes: listed, filtered, sorted and paged, and counted by role, channel,
//! hardware and firmware.

use super::error::{ApiError, ErrorBody};
use super::now_us;
use super::params::{Params, parse_db_int, parse_python_int};
use crate::db::{Node, NodeFacet, NodeFilter, NodeQuery, NodeSort, Reader};
use crate::decode;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;

/// The fields a node has, in the order they are answered in.
const FIELDS: [&str; 14] = [
    "id",
    "node_id",
    "long_name",
    "short_name",
    "hw_model",
    "firmware",
    "role",
    "last_lat",
    "last_long",
    "last_altitude",
    "channel",
    "is_mqtt_gateway",
    "first_seen_us",
    "last_seen_us",
];

/// Fields worked out from the node's packets rather than kept with it, so only
/// answered when `fields` asks for them.
const PRECISION: &str = "position_precision";
const CHANNEL_UTILIZATION: &str = "channel_utilization";
const AIR_UTIL_TX: &str = "air_util_tx";
const WORKED_OUT: [&str; 3] = [PRECISION, CHANNEL_UTILIZATION, AIR_UTIL_TX];

/// How far back the utilization is looked for: nodes send their device
/// telemetry every half hour unless set otherwise.
const UTILIZATION_WINDOW_US: i64 = 2 * 3_600 * 1_000_000;

const PAGE_MAX: i64 = 1000;
const IDS_MAX: usize = 500;
const SEARCH_MAX: usize = 100;
const DAY_US: i128 = 86_400 * 1_000_000;

/// A comma separated list, as the Python version's `_query_list` reads one:
/// absent is `None`, present and empty is an empty list.
fn list<'a>(
    params: &'a Params,
    name: &str,
    maximum: Option<usize>,
) -> Result<Option<Vec<&'a str>>, ApiError> {
    let Some(raw) = params.get(name) else {
        return Ok(None);
    };
    let items: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect();
    if let Some(maximum) = maximum
        && items.len() > maximum
    {
        return Err(ApiError::bad_request(format!(
            "{name} accepts at most {maximum} values"
        )));
    }
    Ok(Some(items))
}

/// The filters `/nodes` and `/nodes/facets` share, read in the Python version's
/// order, so that of two mistakes the same one is reported.
fn filter(params: &Params) -> Result<NodeFilter, ApiError> {
    // Ignored rather than refused when it is not a number, as in Python.
    let seen_since_us = params
        .text("days_active")
        .and_then(parse_python_int)
        .map(|days| {
            let cutoff = i128::from(now_us()) - days.saturating_mul(DAY_US);
            cutoff.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
        });

    let search = params.get("search").unwrap_or("").trim();
    if search.chars().count() > SEARCH_MAX {
        return Err(ApiError::bad_request(format!(
            "search accepts at most {SEARCH_MAX} characters"
        )));
    }

    let ids = match list(params, "ids", Some(IDS_MAX))? {
        None => None,
        Some(items) => Some(
            items
                .into_iter()
                .map(parse_db_int)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    ApiError::bad_request("ids must be a comma-separated list of integers")
                })?,
        ),
    };

    Ok(NodeFilter {
        // Not a number, it matches no node: there is none with that id.
        node_id: params.text("node_id").map(parse_db_int),
        role: params.text("role").map(str::to_string),
        channel: params.text("channel").map(str::to_string),
        hw_model: params.text("hw_model").map(str::to_string),
        firmware: params.text("firmware").map(str::to_string),
        seen_since_us,
        search: Some(search.to_string()).filter(|search| !search.is_empty()),
        ids,
        is_mqtt_gateway: params.bool("is_mqtt_gateway")?,
        has_position: params.bool("has_position")?,
    })
}

fn node_fields(node: Node) -> [(&'static str, Value); 14] {
    [
        ("id", json!(node.id)),
        ("node_id", json!(node.node_id)),
        ("long_name", json!(node.long_name)),
        ("short_name", json!(node.short_name)),
        ("hw_model", json!(node.hw_model)),
        ("firmware", json!(node.firmware)),
        ("role", json!(node.role)),
        ("last_lat", json!(node.last_lat)),
        ("last_long", json!(node.last_long)),
        ("last_altitude", json!(node.last_altitude)),
        ("channel", json!(node.channel)),
        ("is_mqtt_gateway", json!(node.is_mqtt_gateway)),
        ("first_seen_us", json!(node.first_seen_us)),
        ("last_seen_us", json!(node.last_seen_us)),
    ]
}

/// A node: every field, or only `node_id` and those asked for in `fields`.
// Only for the reference: the answer is built field by field.
#[derive(ToSchema)]
#[schema(as = Node)]
#[allow(dead_code)]
pub struct NodeDoc {
    /// The node number as Meshtastic writes it: `!` and eight hex digits.
    #[schema(example = "!000004d2")]
    id: String,
    /// The node number.
    #[schema(example = 1234)]
    node_id: i64,
    /// The name it gives itself.
    #[schema(example = "Alice")]
    long_name: Option<String>,
    /// Its short name, as the map shows it.
    #[schema(example = "A")]
    short_name: Option<String>,
    /// The hardware, as Meshtastic names it.
    #[schema(example = "HELTEC_V3")]
    hw_model: Option<String>,
    /// Its firmware version, when it has reported one.
    #[schema(example = "2.7.15")]
    firmware: Option<String>,
    /// Its role, as Meshtastic names it.
    #[schema(example = "CLIENT")]
    role: Option<String>,
    /// The latitude of its latest position, in degrees times 10 000 000; 0
    /// when it has none.
    #[schema(example = 377749000)]
    last_lat: Option<i64>,
    /// The longitude of its latest position, in degrees times 10 000 000.
    #[schema(example = -1224194000)]
    last_long: Option<i64>,
    /// Its latest altitude, in metres above sea level. Position packets
    /// without an altitude, and map reports saying 0, leave it as it was.
    #[schema(example = 125)]
    last_altitude: Option<i64>,
    /// The channel it was last heard on.
    #[schema(example = "LongFast")]
    channel: Option<String>,
    /// Whether it forwards what it hears to MQTT.
    is_mqtt_gateway: Option<bool>,
    /// When it was first heard, in microseconds.
    #[schema(example = 1736283723456789_i64)]
    first_seen_us: Option<i64>,
    /// When it was last heard, in microseconds.
    #[schema(example = 1736370123456789_i64)]
    last_seen_us: Option<i64>,
    /// Only when `fields` asks for it: the bits of latitude and longitude its
    /// latest position packet or map report keeps, the latest of the two, up to
    /// 32, the exact position. Below that, the node is somewhere in a cell
    /// around its position: 13 bits, a common choice, is about 2.9 km either
    /// way. Null when neither is stored, or it does not say.
    #[schema(example = 13)]
    position_precision: Option<u32>,
    /// Only when `fields` asks for it: the share of time the node heard the
    /// channel busy, in percent, from its latest device telemetry in the latest
    /// two hours. Null without one.
    #[schema(example = 18.5)]
    channel_utilization: Option<f32>,
    /// Only when `fields` asks for it: the share of time the node transmitted,
    /// in percent, from the same telemetry. Null without it.
    #[schema(example = 2.1)]
    air_util_tx: Option<f32>,
}

/// A page of nodes, or all of them.
#[derive(Debug, Serialize, ToSchema)]
pub struct Nodes {
    /// The nodes, with the fields asked for; `node_id` always.
    #[schema(value_type = Vec<NodeDoc>)]
    pub nodes: Vec<Value>,
    /// With `limit` or `offset`: how many match before paging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
    /// With `limit` or `offset`: the page size, or null when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<i64>)]
    pub limit: Option<Option<i64>>,
    /// With `limit` or `offset`: the nodes skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}

/// Nodes
///
/// The nodes ever seen, filtered, sorted and optionally paged. All filters
/// combine. Without `limit` or `offset` every matching node is answered; with
/// either, a page, with how many there are in all.
///
/// Without `sort`, nodes come by short name as stored, upper case before lower
/// case, as they always have. With it, text compares ignoring case and nodes
/// without the field go last either way. Ties are broken by `node_id`, so the
/// pages of a query neither repeat nor skip a node.
///
/// `role` is compared in upper case; `channel`, `hw_model` and `firmware` must
/// match exactly. `search` looks for the text as it is, `%` and `_` included,
/// in the long and short names, the `!` id and the number.
///
/// Ask for many nodes by `ids` in several requests rather than one: a longer
/// address can be more than the server or a proxy takes.
#[utoipa::path(
    get,
    path = "/nodes",
    tag = "nodes",
    params(
        ("node_id" = Option<i64>, Query, description = "Only this node. Not a number, no node."),
        ("role" = Option<String>, Query, description = "Only nodes with this role, in any case."),
        ("channel" = Option<String>, Query, description = "Only nodes on this channel, exactly."),
        ("hw_model" = Option<String>, Query, description = "Only nodes of this hardware, exactly."),
        ("firmware" = Option<String>, Query, description = "Only nodes with this firmware, exactly."),
        ("days_active" = Option<i64>, Query, description = "Only nodes seen in the latest this many days. Ignored if not a number. Without it, every node ever seen."),
        ("search" = Option<String>, Query, description = "Only nodes with this text in their names, id or number, ignoring case. At most 100 characters."),
        ("ids" = Option<String>, Query, description = "Only these nodes, comma separated numbers, at most 500. Empty matches no node."),
        ("is_mqtt_gateway" = Option<bool>, Query, description = "Only gateways, or only nodes that are not."),
        ("has_position" = Option<bool>, Query, description = "Only nodes with a position, or only those without."),
        ("sort" = Option<String>, Query, description = "The field to sort by, `-` in front for descending: `short_name`, `long_name`, `hw_model`, `firmware`, `role`, `channel`, `last_lat`, `last_long`, `last_altitude`, `is_mqtt_gateway`, `first_seen_us`, `last_seen_us` or `node_id`. Default: by short name."),
        ("limit" = Option<i64>, Query, description = "Page size, 1 to 1000; beyond those, the nearest. Default: every node."),
        ("offset" = Option<i64>, Query, description = "Nodes to skip. Default 0."),
        ("fields" = Option<String>, Query, description = "Only these fields, comma separated; `node_id` always comes. `position_precision`, `channel_utilization` and `air_util_tx` come only when asked for here."),
    ),
    responses(
        (status = 200, description = "The nodes", body = Nodes, example = json!({
            "nodes": [
                {"node_id": 1234, "short_name": "A", "last_seen_us": 1736370123456789_i64},
                {"node_id": 5678, "short_name": "B", "last_seen_us": 1736370023456789_i64},
            ],
            "total": 596,
            "limit": 2,
            "offset": 0,
        })),
        (status = 400, description = "`ids`, `search`, `is_mqtt_gateway`, `has_position`, `sort`, `limit`, `offset` or `fields` is not valid", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn nodes(State(state): State<AppState>, params: Params) -> Result<Json<Nodes>, ApiError> {
    let filter = filter(&params)?;

    let sort = match params.text("sort") {
        None => None,
        Some(raw) => {
            let descending = raw.starts_with('-');
            let name = raw.strip_prefix('-').unwrap_or(raw);
            let sort = NodeSort::ALL
                .iter()
                .find(|(field, _)| *field == name)
                .map(|(_, sort)| *sort)
                .ok_or_else(|| {
                    let names: Vec<&str> = NodeSort::ALL.iter().map(|(field, _)| *field).collect();
                    ApiError::bad_request(format!("sort must be one of: {}", names.join(", ")))
                })?;
            Some((sort, descending))
        }
    };
    let limit = params.int("limit", None, Some(1), Some(PAGE_MAX))?;
    let offset = params.int("offset", Some(0), Some(0), None)?.unwrap_or(0);
    let paged = params.get("limit").is_some() || params.get("offset").is_some();
    let fields = match list(&params, "fields", None)? {
        None => None,
        Some(items) => {
            if items
                .iter()
                .any(|item| !FIELDS.contains(item) && !WORKED_OUT.contains(item))
            {
                return Err(ApiError::bad_request(format!(
                    "fields must be a comma-separated list of: {}, {}",
                    FIELDS.join(", "),
                    WORKED_OUT.join(", ")
                )));
            }
            Some(items.into_iter().map(str::to_string).collect::<Vec<_>>())
        }
    };

    let db = state.db()?;
    let failed = |e| ApiError::internal("Failed to fetch nodes", &e);
    let rows = db
        .nodes(&NodeQuery {
            filter: filter.clone(),
            sort,
            limit,
            offset,
        })
        .await
        .map_err(failed)?;
    let total = if paged {
        Some(db.count_nodes(&filter).await.map_err(failed)?)
    } else {
        None
    };

    let asked = |key: &str| {
        fields
            .as_ref()
            .is_some_and(|fields| fields.iter().any(|f| f == key))
    };
    let precision = if asked(PRECISION) {
        Some(precisions(db, &rows).await.map_err(failed)?)
    } else {
        None
    };
    let utilization = if asked(CHANNEL_UTILIZATION) || asked(AIR_UTIL_TX) {
        Some(utilizations(db).await.map_err(failed)?)
    } else {
        None
    };

    let nodes = rows
        .into_iter()
        .map(|node| {
            let node_id = node.node_id;
            let wanted = |key: &str| {
                key == "node_id"
                    || fields
                        .as_ref()
                        .is_none_or(|fields| fields.iter().any(|f| f == key))
            };
            let mut fields = node_fields(node)
                .into_iter()
                .filter(|(key, _)| wanted(key))
                .map(|(key, value)| (key.to_string(), value))
                .collect::<Map<_, _>>();
            if let Some(precision) = &precision {
                fields.insert(
                    PRECISION.to_string(),
                    json!(node_id.and_then(|id| precision.get(&id))),
                );
            }
            if let Some(utilization) = &utilization {
                let latest = node_id.and_then(|id| utilization.get(&id));
                if asked(CHANNEL_UTILIZATION) {
                    fields.insert(
                        CHANNEL_UTILIZATION.to_string(),
                        json!(latest.map(|(channel, _)| percent(*channel))),
                    );
                }
                if asked(AIR_UTIL_TX) {
                    fields.insert(
                        AIR_UTIL_TX.to_string(),
                        json!(latest.and_then(|(_, air)| air.map(percent))),
                    );
                }
            }
            Value::Object(fields)
        })
        .collect();

    Ok(Json(Nodes {
        nodes,
        total,
        limit: paged.then_some(limit),
        offset: paged.then_some(offset),
    }))
}

/// The position precision of the nodes with a position, from the latest of
/// their position packets and map reports.
async fn precisions(db: &Reader, nodes: &[Node]) -> anyhow::Result<HashMap<i64, u32>> {
    let located: Vec<i64> = nodes
        .iter()
        .filter(|node| node.last_lat.is_some_and(|lat| lat != 0))
        .filter_map(|node| node.node_id)
        .collect();
    let mut latest: HashMap<i64, (Option<i64>, Option<Vec<u8>>)> = HashMap::new();
    for (node_id, _, time, payload) in db.latest_positions(&located).await? {
        let newer = latest.get(&node_id).is_none_or(|(seen, _)| time > *seen);
        if newer {
            latest.insert(node_id, (time, payload));
        }
    }
    Ok(latest
        .into_iter()
        .filter_map(|(node_id, (_, payload))| {
            Some((node_id, decode::position_precision(&payload?)?))
        })
        .collect())
}

/// A percentage as the node sent it, to two decimals: a float32 of 18.3 would
/// otherwise come out as 18.299999237060547.
fn percent(value: f32) -> f64 {
    (f64::from(value) * 100.0).round() / 100.0
}

/// Each node's latest channel and transmit air utilization, from the device
/// telemetry it sent in the window.
async fn utilizations(db: &Reader) -> anyhow::Result<HashMap<i64, (f32, Option<f32>)>> {
    let mut latest = HashMap::new();
    for (node_id, payload) in db.telemetry_since(now_us() - UTILIZATION_WINDOW_US).await? {
        let (Some(node_id), Some(payload)) = (node_id, payload) else {
            continue;
        };
        if latest.contains_key(&node_id) {
            continue;
        }
        if let Some(utilization) = decode::channel_utilization(&payload) {
            latest.insert(node_id, utilization);
        }
    }
    Ok(latest)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Facets {
    /// How many nodes match.
    pub total: i64,
    /// For each of `role`, `channel`, `hw_model` and `firmware`, its values
    /// among them, the most common first.
    #[schema(value_type = std::collections::HashMap<String, Vec<FacetValue>>)]
    pub facets: BTreeMap<&'static str, Vec<FacetValue>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FacetValue {
    /// The value; null for the nodes without one.
    #[schema(example = "ROUTER")]
    pub value: Option<String>,
    /// How many of the nodes have it.
    #[schema(example = 18)]
    pub count: i64,
}

/// Node facets
///
/// How many nodes match the filters of `/nodes`, and how many of them have each
/// role, channel, hardware and firmware, the most common first: what a list of
/// nodes needs to offer its filters, or a chart to sum them up, without
/// fetching every node. Values as common as each other come by value.
#[utoipa::path(
    get,
    path = "/nodes/facets",
    tag = "nodes",
    params(
        ("node_id" = Option<i64>, Query, description = "As in `/nodes`."),
        ("role" = Option<String>, Query, description = "As in `/nodes`."),
        ("channel" = Option<String>, Query, description = "As in `/nodes`."),
        ("hw_model" = Option<String>, Query, description = "As in `/nodes`."),
        ("firmware" = Option<String>, Query, description = "As in `/nodes`."),
        ("days_active" = Option<i64>, Query, description = "As in `/nodes`."),
        ("search" = Option<String>, Query, description = "As in `/nodes`."),
        ("ids" = Option<String>, Query, description = "As in `/nodes`."),
        ("is_mqtt_gateway" = Option<bool>, Query, description = "As in `/nodes`."),
        ("has_position" = Option<bool>, Query, description = "As in `/nodes`."),
    ),
    responses(
        (status = 200, description = "The facets", body = Facets, example = json!({
            "total": 38,
            "facets": {
                "channel": [{"value": "LongFast", "count": 38}],
                "firmware": [{"value": "2.7.15", "count": 30}, {"value": null, "count": 8}],
                "hw_model": [{"value": "HELTEC_V3", "count": 25}, {"value": "RAK4631", "count": 13}],
                "role": [{"value": "CLIENT_BASE", "count": 20}, {"value": "ROUTER", "count": 18}],
            },
        })),
        (status = 400, description = "A parameter is not valid", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn facets(
    State(state): State<AppState>,
    params: Params,
) -> Result<Json<Facets>, ApiError> {
    let filter = filter(&params)?;
    let db = state.db()?;
    let failed = |e| ApiError::internal("Failed to fetch node facets", &e);

    let total = db.count_nodes(&filter).await.map_err(failed)?;
    let mut facets = BTreeMap::new();
    for field in NodeFacet::ALL {
        let values = db.node_facet(field, &filter).await.map_err(failed)?;
        facets.insert(
            field.column(),
            values
                .into_iter()
                .map(|facet| FacetValue {
                    value: facet.value,
                    count: facet.count,
                })
                .collect(),
        );
    }

    Ok(Json(Facets { total, facets }))
}
