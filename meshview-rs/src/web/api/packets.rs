// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The packets, latest first, as the firehose, the chat and the node pages
//! show them.

use super::error::{ApiError, ErrorBody};
use super::params::{Params, parse_db_int, parse_db_int_any_base, parse_python_int};
use crate::db::{PacketQuery, PacketRow};
use crate::decode::{self, python_strip};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 1000;

/// A node's long name as the pages get it: the name, null when the node has
/// none, and empty when there is no such node at all.
fn long_name(known: bool, name: Option<String>) -> Value {
    if known { json!(name) } else { json!("") }
}

/// A packet, as the pages show it.
// Only for the reference: the answer is built field by field.
#[derive(ToSchema)]
#[schema(as = Packet)]
#[allow(dead_code)]
pub struct PacketDoc {
    /// Its stored id: what `packet_id` parameters take.
    #[schema(example = 123)]
    id: i64,
    /// The same stored id, by another name.
    #[schema(example = 123)]
    storage_id: i64,
    /// The id it was sent with over the air; null when it cannot be read.
    #[schema(example = 3314808102_i64)]
    packet_id: Option<i64>,
    /// When it was imported, in microseconds.
    #[schema(example = 1736370123456789_i64)]
    import_time_us: Option<i64>,
    /// The channel it was heard on.
    #[schema(example = "LongFast")]
    channel: Option<String>,
    /// The node that sent it.
    #[schema(example = 5678)]
    from_node_id: Option<i64>,
    /// The node it was sent to; 4294967295 (`!ffffffff`) is everyone.
    #[schema(example = 4294967295_i64)]
    to_node_id: Option<i64>,
    /// Its port: 1 is a text message, 3 a position, 4 node info, 67
    /// telemetry, 70 a traceroute, 71 neighbour info.
    #[schema(example = 1)]
    portnum: Option<i64>,
    /// The sender's long name: null when it has none, empty when the node is
    /// not known at all.
    #[schema(example = "Alice")]
    long_name: Option<String>,
    /// What it says: a text message's text; for the other ports understood,
    /// the decoded message in the protobuf text format; `<redacted>` for a
    /// text message, a waypoint or a message replayed from history sent to
    /// one node; `Did not decode` for what cannot be read.
    #[schema(example = "Hello, mesh!")]
    payload: String,
    /// The recipient's long name, as `long_name`. Not when asked for by
    /// `packet_id`.
    #[schema(example = json!(null))]
    to_long_name: Option<String>,
    /// For a reply, the over-the-air id of the packet it answers. Not when
    /// asked for by `packet_id`.
    #[schema(example = 3314808101_i64)]
    reply_id: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Packets {
    /// The packets, latest first.
    #[schema(value_type = Vec<PacketDoc>)]
    pub packets: Vec<Value>,
    /// The latest import time among them, when there are any: `since` for
    /// the next poll. Not when asked for by `packet_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_import_time: Option<i64>,
    /// When the page is full, so older packets may follow: `before` for the
    /// next page. It is the time of the oldest packet read, which with
    /// `portnum=1` can be older than the last one answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_before: Option<i64>,
}

/// Packets
///
/// The latest packets, with what each says as the pages show it: decoded and in
/// the protobuf text format for the ports that are understood, the text of a
/// text message sent to everyone, `<redacted>` for one sent to someone, as for
/// a waypoint or a message replayed from history, and `Did not decode` for
/// what cannot be read.
///
/// With `packet_id`, that one packet, whatever the other filters say. Otherwise
/// pages of up to `limit`, older ones by passing `next_before` as `before`. Text
/// messages that only carry a link test (`seq 123`) are left out, which can make
/// a page shorter than `limit`.
///
/// To follow new packets, poll with `since` set to the `latest_import_time` of
/// the previous answer, and drop those whose `id` you have already: it is the
/// latest `limit` that are answered, so when more than that arrive between
/// two polls the older ones are missed. This is a feed to watch, not a way to
/// copy the database.
///
/// `contains` ignores case. With `portnum=1` it is looked for in the text as
/// shown, `<redacted>` included, among the packets read, so a page can come
/// back short; otherwise in the stored bytes read as text, which is not a
/// search of the decoded fields.
///
/// A `packet_id` that is not there answers no packets, with 200.
#[utoipa::path(
    get,
    path = "/packets",
    tag = "packets",
    params(
        ("packet_id" = Option<i64>, Query, description = "This packet, by its stored id, in decimal. The other filters do not apply."),
        ("limit" = Option<i64>, Query, description = "Up to this many, 1 to 1000. Default 50, which is also what anything not a number gets."),
        ("before" = Option<i64>, Query, description = "Only packets imported before this time, in microseconds: the page cursor."),
        ("since" = Option<i64>, Query, description = "Only packets imported after this time, in microseconds. Ignored if not a number."),
        ("portnum" = Option<i64>, Query, description = "Only packets of this port. Ignored if not a number."),
        ("from_node_id" = Option<i64>, Query, description = "Only packets from this node, in decimal or 0x hex. Ignored if not a number."),
        ("to_node_id" = Option<i64>, Query, description = "Only packets to this node, in decimal or 0x hex. Ignored if not a number."),
        ("node_id" = Option<i64>, Query, description = "Only packets from or to this node, in decimal or 0x hex. Ignored if not a number."),
        ("channel" = Option<String>, Query, description = "Only packets on this channel, exactly."),
        ("contains" = Option<String>, Query, description = "Only packets with this text, ignoring case. For text messages, in what is shown; otherwise in the stored bytes."),
    ),
    responses(
        (status = 200, description = "The packets", body = Packets, example = json!({
            "packets": [{
                "id": 123,
                "storage_id": 123,
                "packet_id": 3314808102_i64,
                "import_time_us": 1736370123456789_i64,
                "channel": "LongFast",
                "from_node_id": 5678,
                "to_node_id": 4294967295_i64,
                "portnum": 1,
                "long_name": "Alice",
                "payload": "Hello, mesh!",
                "to_long_name": null,
                "reply_id": 3314808101_i64,
            }],
            "latest_import_time": 1736370123456789_i64,
            "next_before": 1736370023456789_i64,
        })),
        (status = 400, description = "`before` or `packet_id` is not a number", body = ErrorBody),
        (status = 500, description = "They could not be read", body = ErrorBody),
    )
)]
pub async fn packets(
    State(state): State<AppState>,
    params: Params,
) -> Result<Json<Packets>, ApiError> {
    let before = params.int("before", None, Some(0), None)?;
    let db = state.db()?;
    let failed = |e| ApiError::internal("Failed to fetch packets", &e);

    if let Some(raw) = params.text("packet_id") {
        let id =
            parse_db_int(raw).ok_or_else(|| ApiError::bad_request("Invalid packet_id format"))?;
        let packets = match db.packet(id).await.map_err(failed)? {
            None => Vec::new(),
            Some(row) => vec![single(row)],
        };
        return Ok(Json(Packets {
            packets,
            latest_import_time: None,
            next_before: None,
        }));
    }

    // Read with plain int(), as in the Python version: what is not a number
    // gets the default rather than an error.
    let limit = params
        .get("limit")
        .and_then(parse_python_int)
        .map_or(DEFAULT_LIMIT, |limit| {
            limit.clamp(1, i128::from(MAX_LIMIT)) as i64
        });
    let since = params.text("since").and_then(parse_db_int);
    let portnum = params
        .text("portnum")
        .and_then(parse_db_int)
        .filter(|portnum| i32::try_from(*portnum).is_ok());
    let node = |name| params.text(name).and_then(parse_db_int_any_base);
    let contains = params.text("contains");
    let texts = portnum == Some(1);

    let rows = db
        .packets(&PacketQuery {
            from_node_id: node("from_node_id"),
            to_node_id: node("to_node_id"),
            node_id: node("node_id"),
            channel: params.text("channel").map(str::to_string),
            portnum,
            after_us: since,
            before_us: before,
            // Text messages are searched in what is shown, below, rather than
            // in their bytes.
            contains: contains.filter(|_| !texts).map(str::to_string),
            limit,
        })
        .await
        .map_err(failed)?;

    // From the rows read, before any are left out, so that the next page
    // starts where this one ended.
    let next_before = if rows.len() as i64 == limit {
        rows.last().and_then(|row| row.import_time_us)
    } else {
        None
    };

    let mut items: Vec<(Option<i64>, Value)> = Vec::with_capacity(rows.len());
    for row in rows {
        let shown = decode::show(row.payload.as_deref(), row.portnum, row.to_node_id);
        if texts {
            if shown.text.is_empty() || decode::is_sequence_only(&shown.text) {
                continue;
            }
            if let Some(contains) = contains
                && !shown.text.to_lowercase().contains(&contains.to_lowercase())
            {
                continue;
            }
        }
        let mut item = json!({
            "id": row.id,
            "storage_id": row.id,
            "packet_id": shown.packet_id(),
            "import_time_us": row.import_time_us,
            "channel": row.channel,
            "from_node_id": row.from_node_id,
            "to_node_id": row.to_node_id,
            "portnum": row.portnum,
            "long_name": long_name(row.from_known, row.from_long_name),
            "payload": python_strip(&shown.text),
            "to_long_name": long_name(row.to_known, row.to_long_name),
        });
        if let Some(reply_id) = shown.reply_id() {
            item["reply_id"] = json!(reply_id);
        }
        items.push((row.import_time_us, item));
    }
    // Latest first, those with no time last; the order is the database's
    // already, but the Python version sorted again, and so does this.
    items.sort_by_key(|(time, _)| std::cmp::Reverse((time.is_some(), time.unwrap_or(0))));
    items.truncate(limit as usize);

    let latest_import_time = items
        .iter()
        .filter_map(|(time, _)| *time)
        .filter(|&time| time > 0)
        .max();

    Ok(Json(Packets {
        packets: items.into_iter().map(|(_, item)| item).collect(),
        latest_import_time,
        next_before,
    }))
}

/// One packet, as asked for by its id: fewer fields than in a list, as in the
/// Python version.
fn single(row: PacketRow) -> Value {
    let shown = decode::show(row.payload.as_deref(), row.portnum, row.to_node_id);
    json!({
        "id": row.id,
        "storage_id": row.id,
        "packet_id": shown.packet_id(),
        "from_node_id": row.from_node_id,
        "to_node_id": row.to_node_id,
        "portnum": row.portnum,
        "payload": python_strip(&shown.text),
        "import_time_us": row.import_time_us,
        "channel": row.channel,
        "long_name": long_name(row.from_known, row.from_long_name),
    })
}
