// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The nodes: listed, filtered, sorted and paged, and counted by role, channel,
//! hardware and firmware.

use super::error::{ApiError, ErrorBody};
use super::now_us;
use super::params::{Params, parse_db_int, parse_python_int};
use crate::db::{Node, NodeFacet, NodeFilter, NodeQuery, NodeSort};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
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

/// A page of nodes, or all of them.
#[derive(Debug, Serialize, ToSchema)]
pub struct Nodes {
    /// The nodes, with the fields asked for; `node_id` always.
    #[schema(value_type = Vec<Object>)]
    pub nodes: Vec<Value>,
    /// With `limit` or `offset`: how many match before paging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
    /// With `limit` or `offset`: the page size, or null when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<i64>)]
    pub limit: Option<Option<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}

/// Nodes
///
/// The nodes ever seen, filtered, sorted and optionally paged. All filters
/// combine. Without `limit` or `offset` every matching node is answered; with
/// either, a page, with how many there are in all.
#[utoipa::path(
    get,
    path = "/nodes",
    tag = "nodes",
    params(
        ("node_id" = Option<i64>, Query, description = "Only this node."),
        ("role" = Option<String>, Query, description = "Only nodes with this role, in any case."),
        ("channel" = Option<String>, Query, description = "Only nodes on this channel, exactly."),
        ("hw_model" = Option<String>, Query, description = "Only nodes of this hardware, exactly."),
        ("firmware" = Option<String>, Query, description = "Only nodes with this firmware, exactly."),
        ("days_active" = Option<i64>, Query, description = "Only nodes seen in the latest this many days. Ignored if not a number."),
        ("search" = Option<String>, Query, description = "Only nodes with this text in their names, id or number, ignoring case. At most 100 characters."),
        ("ids" = Option<String>, Query, description = "Only these nodes, comma separated numbers, at most 500. Empty matches no node."),
        ("is_mqtt_gateway" = Option<bool>, Query, description = "Only gateways, or only nodes that are not."),
        ("has_position" = Option<bool>, Query, description = "Only nodes with a position, or only those without."),
        ("sort" = Option<String>, Query, description = "The field to sort by, `-` in front for descending; text fields ignore case and nodes without the field go last. Default: by short name."),
        ("limit" = Option<i64>, Query, description = "Page size, 1 to 1000."),
        ("offset" = Option<i64>, Query, description = "Nodes to skip."),
        ("fields" = Option<String>, Query, description = "Only these fields, comma separated; `node_id` always comes."),
    ),
    responses(
        (status = 200, description = "The nodes", body = Nodes),
        (status = 400, description = "A parameter is not valid", body = ErrorBody),
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
            if items.iter().any(|item| !FIELDS.contains(item)) {
                return Err(ApiError::bad_request(format!(
                    "fields must be a comma-separated list of: {}",
                    FIELDS.join(", ")
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

    let nodes = rows
        .into_iter()
        .map(|node| {
            let wanted = |key: &str| {
                key == "node_id"
                    || fields
                        .as_ref()
                        .is_none_or(|fields| fields.iter().any(|f| f == key))
            };
            Value::Object(
                node_fields(node)
                    .into_iter()
                    .filter(|(key, _)| wanted(key))
                    .map(|(key, value)| (key.to_string(), value))
                    .collect::<Map<_, _>>(),
            )
        })
        .collect();

    Ok(Json(Nodes {
        nodes,
        total,
        limit: paged.then_some(limit),
        offset: paged.then_some(offset),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Facets {
    /// How many nodes match.
    pub total: i64,
    /// For each of `role`, `channel`, `hw_model` and `firmware`, its values
    /// among them, the most common first.
    #[schema(value_type = Object)]
    pub facets: BTreeMap<&'static str, Vec<FacetValue>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FacetValue {
    pub value: Option<String>,
    pub count: i64,
}

/// Node facets
///
/// How many nodes match the filters of `/nodes`, and how many of them have each
/// role, channel, hardware and firmware, the most common first: what a list of
/// nodes needs to offer its filters.
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
        (status = 200, description = "The facets", body = Facets),
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
