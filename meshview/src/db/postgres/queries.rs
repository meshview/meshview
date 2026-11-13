// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The read queries, in PostgreSQL's dialect: SQLite's, one for one, written so
//! that both answer exactly the same.
//!
//! What that takes, beyond the placeholders:
//!
//! - NULLs sort first going up and last going down in SQLite, the other way
//!   round in PostgreSQL, so every ORDER BY on a column that can be NULL says
//!   where they go.
//! - Text sorts by its bytes in SQLite; here by the database's collation unless
//!   told otherwise, so text orderings are `COLLATE "C"`.
//! - Columns Python created as INTEGER are cast to BIGINT, which is what the
//!   rows are read into; SQLite has only the one integer.
//! - Text is looked for in a packet's payload, its bytes, ASCII letters in
//!   either case, as SQLite's instr() over lower() does. PostgreSQL has no
//!   such thing for bytes, and text cannot hold the zero bytes a payload
//!   has, so a regular expression goes over its hex dump instead.

use crate::db::{
    DailySnapshot, Facet, JobRun, Node, NodeFacet, NodeFilter, NodeQuery, PacketFilter, PacketSeen,
    Period, PeriodCount, SightingsOf, TopNode, TopQuery,
};
use crate::db::{PacketQuery, PacketRow, TracerouteHop, TracerouteRow};
use anyhow::{Context, Result};
use sqlx::{PgPool, Postgres, QueryBuilder};

pub async fn channels_since(pool: &PgPool, since_us: i64) -> Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT DISTINCT channel COLLATE \"C\" FROM packet \
         WHERE import_time_us >= $1 AND channel IS NOT NULL ORDER BY 1",
    )
    .bind(since_us)
    .fetch_all(pool)
    .await
    .context("listing the channels")
}

const SNAPSHOT_COLUMNS: &str = "SELECT snapshot_date::text AS snapshot_date, node_count, \
     packet_count, gateway_count, captured_at_us FROM daily_snapshot";

pub async fn daily_snapshots(pool: &PgPool, latest: Option<i64>) -> Result<Vec<DailySnapshot>> {
    let rows = match latest {
        Some(latest) => {
            let mut rows: Vec<DailySnapshot> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "{SNAPSHOT_COLUMNS} ORDER BY snapshot_date DESC LIMIT $1"
            )))
            .bind(latest)
            .fetch_all(pool)
            .await?;
            rows.reverse();
            rows
        }
        None => {
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "{SNAPSHOT_COLUMNS} ORDER BY snapshot_date"
            )))
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows)
}

pub async fn packet_sightings(pool: &PgPool, packet_id: i64) -> Result<Vec<PacketSeen>> {
    sqlx::query_as(
        "SELECT packet_id, node_id, rx_time, hop_limit::bigint AS hop_limit, \
                hop_start::bigint AS hop_start, channel, rx_snr, rx_rssi::bigint AS rx_rssi, \
                topic, import_time_us \
         FROM packet_seen WHERE packet_id = $1 \
         ORDER BY import_time_us DESC NULLS LAST, node_id, rx_time",
    )
    .bind(packet_id)
    .fetch_all(pool)
    .await
    .context("listing the sightings of a packet")
}

pub async fn sightings_of(pool: &PgPool, packet_ids: &[i64]) -> Result<Vec<PacketSeen>> {
    let mut query = QueryBuilder::new(
        "SELECT packet_id, node_id, rx_time, hop_limit::bigint AS hop_limit, \
                hop_start::bigint AS hop_start, channel, rx_snr, rx_rssi::bigint AS rx_rssi, \
                topic, import_time_us \
         FROM packet_seen WHERE packet_id IN (",
    );
    let mut ids = query.separated(", ");
    for id in packet_ids {
        ids.push_bind(*id);
    }
    query.push(")");
    query
        .build_query_as()
        .fetch_all(pool)
        .await
        .context("listing the sightings of packets")
}

pub async fn last_import_us(pool: &PgPool) -> Result<Option<i64>> {
    sqlx::query_scalar("SELECT max(import_time_us) FROM packet")
        .fetch_one(pool)
        .await
        .context("finding the latest import")
}

pub async fn latest_job_run(pool: &PgPool, job: &str) -> Result<Option<JobRun>> {
    sqlx::query_as(
        "SELECT job, status, started_us, finished_us, details, error FROM job_run \
         WHERE job = $1 ORDER BY started_us DESC, id DESC LIMIT 1",
    )
    .bind(job)
    .fetch_optional(pool)
    .await
    .context("reading the latest job run")
}

/// Adds a filter's conditions, each only when it is set, after a WHERE that
/// is already there, on a query whose packet table is `packet`.
fn push_filter(query: &mut QueryBuilder<Postgres>, filter: &PacketFilter) {
    query
        .push(" packet.import_time_us >= ")
        .push_bind(filter.since_us);
    if let Some(channel) = &filter.channel {
        query
            .push(" AND lower(packet.channel) = ")
            .push_bind(channel.to_lowercase());
    }
    if let Some(portnum) = filter.portnum {
        query.push(" AND packet.portnum = ").push_bind(portnum);
    }
    if let Some(to_node) = filter.to_node {
        query.push(" AND packet.to_node_id = ").push_bind(to_node);
    }
    if let Some(from_node) = filter.from_node {
        query
            .push(" AND packet.from_node_id = ")
            .push_bind(from_node);
    }
}

pub async fn packets_per_period(
    pool: &PgPool,
    period: Period,
    filter: &PacketFilter,
) -> Result<Vec<PeriodCount>> {
    let format = match period {
        Period::Hour => "YYYY-MM-DD HH24:00",
        Period::Day => "YYYY-MM-DD",
    };
    let mut query = QueryBuilder::new(
        "SELECT to_char(to_timestamp(packet.import_time_us / 1000000.0) AT TIME ZONE 'UTC', ",
    );
    query.push_bind(format).push(
        ") AS period, count(*) AS count FROM packet WHERE packet.import_time_us IS NOT NULL AND",
    );
    push_filter(&mut query, filter);
    // Digits and punctuation only: no collation sorts them differently.
    query.push(" GROUP BY period ORDER BY period");
    query
        .build_query_as()
        .fetch_all(pool)
        .await
        .context("counting packets per period")
}

pub async fn count_packets(pool: &PgPool, filter: Option<&PacketFilter>) -> Result<i64> {
    let mut query = QueryBuilder::new("SELECT count(packet.id) FROM packet");
    if let Some(filter) = filter {
        query.push(" WHERE");
        push_filter(&mut query, filter);
    }
    query
        .build_query_scalar()
        .fetch_one(pool)
        .await
        .context("counting packets")
}

pub async fn count_sightings(pool: &PgPool, of: SightingsOf<'_>) -> Result<i64> {
    let mut query = QueryBuilder::new("SELECT count(packet_seen.packet_id) FROM packet_seen");
    match of {
        SightingsOf::All => {}
        SightingsOf::Packet(id) => {
            query.push(" WHERE packet_seen.packet_id = ").push_bind(id);
        }
        SightingsOf::Packets(filter) => {
            query.push(" JOIN packet ON packet.id = packet_seen.packet_id WHERE");
            push_filter(&mut query, filter);
        }
    }
    query
        .build_query_scalar()
        .fetch_one(pool)
        .await
        .context("counting sightings")
}

/// The traffic of each node in the window, as SQLite's.
fn push_top_traffic(query: &mut QueryBuilder<Postgres>, window_us: i64) {
    query
        .push(
            "WITH sent AS ( \
               SELECT packet.from_node_id AS node_id, count(*) AS sent FROM packet \
                WHERE packet.import_time_us >= (SELECT max(import_time_us) FROM packet) - ",
        )
        .push_bind(window_us)
        .push(
            " GROUP BY packet.from_node_id), \
             seen AS ( \
               SELECT packet.from_node_id AS node_id, count(*) AS seen \
                 FROM packet_seen JOIN packet ON packet.id = packet_seen.packet_id \
                WHERE packet_seen.import_time_us >= \
                      (SELECT max(import_time_us) FROM packet_seen) - ",
        )
        .push_bind(window_us)
        .push(" GROUP BY packet.from_node_id) ");
}

fn push_top_conditions(query: &mut QueryBuilder<Postgres>, top: &TopQuery) {
    let mut first = true;
    let mut condition = |query: &mut QueryBuilder<Postgres>| {
        query.push(if first { " WHERE " } else { " AND " });
        first = false;
    };
    if let Some(channel) = &top.channel {
        condition(query);
        query.push("node.channel = ").push_bind(channel.clone());
    }
    if top.active {
        condition(query);
        query.push("(sent.sent IS NOT NULL OR seen.seen IS NOT NULL)");
    }
}

pub async fn top_nodes(pool: &PgPool, top: &TopQuery) -> Result<(i64, Vec<TopNode>)> {
    let joins = " FROM node LEFT OUTER JOIN sent ON sent.node_id = node.node_id \
                 LEFT OUTER JOIN seen ON seen.node_id = node.node_id";

    let mut list = QueryBuilder::new("");
    push_top_traffic(&mut list, top.window_us);
    list.push(
        "SELECT node.node_id, node.long_name, node.short_name, node.channel, \
         coalesce(sent.sent, 0) AS sent, coalesce(seen.seen, 0) AS seen",
    )
    .push(joins);
    push_top_conditions(&mut list, top);
    list.push(
        " ORDER BY coalesce(seen.seen, 0) DESC, coalesce(sent.sent, 0) DESC, \
         node.node_id NULLS FIRST LIMIT ",
    )
    .push_bind(top.limit)
    .push(" OFFSET ")
    .push_bind(top.offset);
    let nodes = list
        .build_query_as()
        .fetch_all(pool)
        .await
        .context("listing the top nodes")?;

    let mut count = QueryBuilder::new("");
    push_top_traffic(&mut count, top.window_us);
    count.push("SELECT count(*)").push(joins);
    push_top_conditions(&mut count, top);
    let total = count
        .build_query_scalar()
        .fetch_one(pool)
        .await
        .context("counting the top nodes")?;

    Ok((total, nodes))
}

/// `text` with LIKE's wildcards, and the escape itself, escaped with `/`.
fn like_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '/' | '%' | '_') {
            out.push('/');
        }
        out.push(c);
    }
    out
}

/// A pattern for the hex dump of the bytes that hold `text`: its bytes, at a
/// byte boundary, each ASCII letter in either case, as SQLite's lower() folds
/// only those.
fn hex_pattern(text: &str) -> String {
    let mut pattern = String::from("^(?:..)*?");
    for byte in text.bytes() {
        if byte.is_ascii_lowercase() {
            pattern.push_str(&format!("(?:{byte:02x}|{:02x})", byte.to_ascii_uppercase()));
        } else {
            pattern.push_str(&format!("{byte:02x}"));
        }
    }
    pattern
}

/// A node filter's conditions, after a WHERE already written.
fn push_node_filter(query: &mut QueryBuilder<Postgres>, filter: &NodeFilter) {
    query.push(" node.last_seen_us IS NOT NULL");
    match filter.node_id {
        None => {}
        Some(Some(node_id)) => {
            query.push(" AND node.node_id = ").push_bind(node_id);
        }
        // Asked for by something that is not a node number: no node is that.
        Some(None) => {
            query.push(" AND false");
        }
    }
    if let Some(role) = &filter.role {
        query
            .push(" AND node.role = ")
            .push_bind(role.to_uppercase());
    }
    for (column, value) in [
        ("channel", &filter.channel),
        ("hw_model", &filter.hw_model),
        ("firmware", &filter.firmware),
    ] {
        if let Some(value) = value {
            query
                .push(format!(" AND node.{column} = "))
                .push_bind(value.clone());
        }
    }
    if let Some(since) = filter.seen_since_us {
        query.push(" AND node.last_seen_us > ").push_bind(since);
    }
    if let Some(ids) = &filter.ids {
        if ids.is_empty() {
            query.push(" AND false");
        } else {
            query.push(" AND node.node_id IN (");
            let mut separated = query.separated(", ");
            for id in ids {
                separated.push_bind(*id);
            }
            query.push(")");
        }
    }
    match filter.is_mqtt_gateway {
        None => {}
        Some(true) => {
            query.push(" AND node.is_mqtt_gateway IS TRUE");
        }
        Some(false) => {
            query.push(" AND node.is_mqtt_gateway IS NOT TRUE");
        }
    }
    if let Some(has_position) = filter.has_position {
        query
            .push(if has_position { " AND " } else { " AND NOT " })
            .push(
                "(node.last_lat IS NOT NULL AND node.last_long IS NOT NULL \
              AND node.last_lat != 0 AND node.last_long != 0)",
            );
    }
    if let Some(search) = &filter.search {
        let term = like_literal(&search.to_lowercase());
        query.push(" AND (");
        for (i, column) in [
            "lower(node.long_name)",
            "lower(node.short_name)",
            "lower(node.id)",
            "CAST(node.node_id AS VARCHAR)",
        ]
        .iter()
        .enumerate()
        {
            if i > 0 {
                query.push(" OR ");
            }
            query
                .push(format!("{column} LIKE '%' || "))
                .push_bind(term.clone())
                .push(" || '%' ESCAPE '/'");
        }
        query.push(")");
    }
}

const NODE_COLUMNS: &str = "SELECT id, node_id, long_name, short_name, hw_model, firmware, role, \
     last_lat, last_long, last_altitude, channel, is_mqtt_gateway, first_seen_us, last_seen_us \
     FROM node";

pub async fn nodes(pool: &PgPool, node_query: &NodeQuery) -> Result<Vec<Node>> {
    let mut query = QueryBuilder::new(NODE_COLUMNS);
    query.push(" WHERE");
    push_node_filter(&mut query, &node_query.filter);
    match node_query.sort {
        // As it always was: by short name, exactly, those without one first.
        None => query.push(" ORDER BY node.short_name COLLATE \"C\" ASC NULLS FIRST"),
        Some((sort, descending)) => {
            let column = if sort.is_text() {
                format!("lower(node.{}) COLLATE \"C\"", sort.column())
            } else {
                format!("node.{}", sort.column())
            };
            let direction = if descending { "DESC" } else { "ASC" };
            query.push(format!(" ORDER BY {column} {direction} NULLS LAST"))
        }
    };
    query.push(", node.node_id ASC NULLS FIRST");
    if node_query.limit.is_some() || node_query.offset != 0 {
        // A NULL limit is no limit.
        query
            .push(" LIMIT ")
            .push_bind(node_query.limit)
            .push(" OFFSET ")
            .push_bind(node_query.offset);
    }
    query
        .build_query_as()
        .fetch_all(pool)
        .await
        .context("listing nodes")
}

pub async fn count_nodes(pool: &PgPool, filter: &NodeFilter) -> Result<i64> {
    let mut query = QueryBuilder::new("SELECT count(*) FROM node WHERE");
    push_node_filter(&mut query, filter);
    query
        .build_query_scalar()
        .fetch_one(pool)
        .await
        .context("counting nodes")
}

pub async fn node_facet(
    pool: &PgPool,
    field: NodeFacet,
    filter: &NodeFilter,
) -> Result<Vec<Facet>> {
    let column = field.column();
    let mut query = QueryBuilder::new(format!(
        "SELECT node.{column} AS value, count(*) AS count FROM node WHERE"
    ));
    push_node_filter(&mut query, filter);
    query.push(format!(
        " GROUP BY node.{column} \
          ORDER BY count(*) DESC, node.{column} COLLATE \"C\" ASC NULLS FIRST"
    ));
    query
        .build_query_as()
        .fetch_all(pool)
        .await
        .with_context(|| format!("counting nodes by {column}"))
}

const PACKET_COLUMNS: &str = "SELECT packet.id, packet.portnum::bigint AS portnum, \
     packet.from_node_id, packet.to_node_id, packet.payload, packet.import_time_us, \
     packet.channel, \
     sender.node_id IS NOT NULL AS from_known, sender.long_name AS from_long_name, \
     recipient.node_id IS NOT NULL AS to_known, recipient.long_name AS to_long_name \
     FROM packet \
     LEFT OUTER JOIN node AS sender ON packet.from_node_id = sender.node_id \
     LEFT OUTER JOIN node AS recipient ON packet.to_node_id = recipient.node_id";

pub async fn packet(pool: &PgPool, id: i64) -> Result<Option<PacketRow>> {
    let mut query = QueryBuilder::new(PACKET_COLUMNS);
    query.push(" WHERE packet.id = ").push_bind(id);
    query
        .build_query_as()
        .fetch_optional(pool)
        .await
        .context("reading a packet")
}

pub async fn packets(pool: &PgPool, packets: &PacketQuery) -> Result<Vec<PacketRow>> {
    let mut query = QueryBuilder::new(PACKET_COLUMNS);
    query.push(" WHERE true");
    if let Some(from) = packets.from_node_id {
        query.push(" AND packet.from_node_id = ").push_bind(from);
    }
    if let Some(to) = packets.to_node_id {
        query.push(" AND packet.to_node_id = ").push_bind(to);
    }
    if let Some(node) = packets.node_id {
        query
            .push(" AND (packet.from_node_id = ")
            .push_bind(node)
            .push(" OR packet.to_node_id = ")
            .push_bind(node)
            .push(")");
    }
    if let Some(channel) = &packets.channel {
        query
            .push(" AND packet.channel = ")
            .push_bind(channel.clone());
    }
    if let Some(portnum) = packets.portnum {
        query.push(" AND packet.portnum = ").push_bind(portnum);
    }
    if let Some(after) = packets.after_us {
        query.push(" AND packet.import_time_us > ").push_bind(after);
    }
    if let Some(before) = packets.before_us {
        query
            .push(" AND packet.import_time_us < ")
            .push_bind(before);
    }
    if let Some(contains) = &packets.contains {
        query
            .push(" AND encode(packet.payload, 'hex') ~ ")
            .push_bind(hex_pattern(&contains.to_lowercase()))
            .push(" AND (packet.portnum != 1 OR packet.to_node_id = ")
            .push_bind(crate::decode::BROADCAST)
            .push(")");
    }
    query
        .push(" ORDER BY packet.import_time_us DESC NULLS LAST, packet.id DESC LIMIT ")
        // A negative limit is none, as in SQLite; here that is a NULL one.
        .push_bind((packets.limit >= 0).then_some(packets.limit));
    query
        .build_query_as()
        .fetch_all(pool)
        .await
        .context("listing packets")
}

pub async fn traceroutes(pool: &PgPool, packet_id: i64, limit: i64) -> Result<Vec<TracerouteRow>> {
    sqlx::query_as(
        "SELECT gateway_node_id, done, route, import_time_us FROM traceroute \
         WHERE packet_id = $1 ORDER BY import_time_us NULLS FIRST, id LIMIT $2",
    )
    .bind(packet_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("listing a packet's traceroutes")
}

pub async fn traceroutes_since(pool: &PgPool, since_us: i64) -> Result<Vec<TracerouteHop>> {
    sqlx::query_as(
        "SELECT traceroute.gateway_node_id, traceroute.done, traceroute.route, \
                packet.id IS NOT NULL AS has_packet, packet.from_node_id, packet.to_node_id \
         FROM traceroute LEFT OUTER JOIN packet ON packet.id = traceroute.packet_id \
         WHERE traceroute.import_time_us > $1 \
         ORDER BY traceroute.import_time_us, traceroute.id",
    )
    .bind(since_us)
    .fetch_all(pool)
    .await
    .context("listing the latest traceroutes")
}

pub async fn node(pool: &PgPool, node_id: i64) -> Result<Option<Node>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{NODE_COLUMNS} WHERE node_id = $1"
    )))
    .bind(node_id)
    .fetch_optional(pool)
    .await
    .context("reading a node")
}

pub async fn nodeinfo_payloads(pool: &PgPool, node_id: i64) -> Result<Vec<Option<Vec<u8>>>> {
    sqlx::query_scalar(
        "SELECT payload FROM packet WHERE from_node_id = $1 AND portnum = 4 \
         ORDER BY import_time_us DESC NULLS LAST, id DESC",
    )
    .bind(node_id)
    .fetch_all(pool)
    .await
    .context("reading a node's node info")
}

/// For each of these nodes, its latest position packet and its latest map
/// report: the node, the port, the time and the stored packet.
pub async fn latest_positions(
    pool: &PgPool,
    node_ids: &[i64],
) -> Result<Vec<(i64, i64, Option<i64>, Option<Vec<u8>>)>> {
    sqlx::query_as(
        "SELECT node.node_id, packet.portnum::bigint, packet.import_time_us, packet.payload \
         FROM node JOIN packet ON packet.id IN (\
           (SELECT id FROM packet WHERE from_node_id = node.node_id AND portnum = 3 \
            ORDER BY import_time_us DESC NULLS LAST LIMIT 1), \
           (SELECT id FROM packet WHERE from_node_id = node.node_id AND portnum = 73 \
            ORDER BY import_time_us DESC NULLS LAST LIMIT 1)) \
         WHERE node.node_id = ANY($1)",
    )
    .bind(node_ids)
    .fetch_all(pool)
    .await
    .context("reading the nodes' latest positions")
}

/// The telemetry packets heard since a time, the latest first.
pub async fn telemetry_since(
    pool: &PgPool,
    since_us: i64,
) -> Result<Vec<(Option<i64>, Option<Vec<u8>>)>> {
    sqlx::query_as(
        "SELECT from_node_id, payload FROM packet \
         WHERE portnum = 67 AND import_time_us > $1 ORDER BY import_time_us DESC, id DESC",
    )
    .bind(since_us)
    .fetch_all(pool)
    .await
    .context("reading the latest telemetry")
}

/// The packets imported since a time, map reports aside, by channel and by
/// whether they went to everyone.
pub async fn packets_by_channel(
    pool: &PgPool,
    since_us: i64,
) -> Result<Vec<(Option<String>, bool, i64)>> {
    sqlx::query_as(
        "SELECT channel, coalesce(to_node_id = $1, false) AS broadcast, count(id) FROM packet \
         WHERE import_time_us >= $2 AND portnum IS DISTINCT FROM 73 \
         GROUP BY channel, broadcast ORDER BY channel COLLATE \"C\" NULLS FIRST, broadcast",
    )
    .bind(crate::decode::BROADCAST)
    .bind(since_us)
    .fetch_all(pool)
    .await
    .context("counting packets by channel")
}

/// What the database takes on disk.
pub async fn size(pool: &PgPool) -> Result<Option<u64>> {
    let bytes: i64 = sqlx::query_scalar("SELECT pg_database_size(current_database())")
        .fetch_one(pool)
        .await
        .context("reading the database's size")?;
    Ok(u64::try_from(bytes).ok())
}
