// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The read queries, in SQLite's dialect. Each is the Python meshview's, as
//! SQLAlchemy emitted it, so that both answer the same; where an ordering had no
//! tiebreak, one is added, so that the answer does not depend on the query plan.

use crate::db::{
    DailySnapshot, JobRun, PacketFilter, PacketSeen, Period, PeriodCount, SightingsOf, TopNode,
    TopQuery,
};
use anyhow::{Context, Result};
use sqlx::SqlitePool;
use sqlx::{QueryBuilder, Sqlite};

pub async fn channels_since(pool: &SqlitePool, since_us: i64) -> Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT DISTINCT channel FROM packet \
         WHERE import_time_us >= ? AND channel IS NOT NULL ORDER BY channel",
    )
    .bind(since_us)
    .fetch_all(pool)
    .await
    .context("listing the channels")
}

pub async fn daily_snapshots(pool: &SqlitePool, latest: Option<i64>) -> Result<Vec<DailySnapshot>> {
    let rows = match latest {
        Some(latest) => {
            let mut rows: Vec<DailySnapshot> = sqlx::query_as(
                "SELECT snapshot_date, node_count, packet_count, gateway_count, captured_at_us \
                 FROM daily_snapshot ORDER BY snapshot_date DESC LIMIT ?",
            )
            .bind(latest)
            .fetch_all(pool)
            .await?;
            rows.reverse();
            rows
        }
        None => {
            sqlx::query_as(
                "SELECT snapshot_date, node_count, packet_count, gateway_count, captured_at_us \
                 FROM daily_snapshot ORDER BY snapshot_date",
            )
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows)
}

pub async fn packet_sightings(pool: &SqlitePool, packet_id: i64) -> Result<Vec<PacketSeen>> {
    sqlx::query_as(
        "SELECT packet_id, node_id, rx_time, hop_limit, hop_start, channel, rx_snr, rx_rssi, \
                topic, import_time_us \
         FROM packet_seen WHERE packet_id = ? \
         ORDER BY import_time_us DESC, node_id, rx_time",
    )
    .bind(packet_id)
    .fetch_all(pool)
    .await
    .context("listing the sightings of a packet")
}

pub async fn last_import_us(pool: &SqlitePool) -> Result<Option<i64>> {
    sqlx::query_scalar("SELECT max(import_time_us) FROM packet")
        .fetch_one(pool)
        .await
        .context("finding the latest import")
}

pub async fn latest_job_run(pool: &SqlitePool, job: &str) -> Result<Option<JobRun>> {
    sqlx::query_as(
        "SELECT job, status, started_us, finished_us, details, error FROM job_run \
         WHERE job = ? ORDER BY started_us DESC, id DESC LIMIT 1",
    )
    .bind(job)
    .fetch_optional(pool)
    .await
    .context("reading the latest job run")
}

/// Adds a filter's conditions, each only when it is set, after a WHERE that
/// is already there, on a query whose packet table is `packet`.
fn push_filter(query: &mut QueryBuilder<Sqlite>, filter: &PacketFilter) {
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
    pool: &SqlitePool,
    period: Period,
    filter: &PacketFilter,
) -> Result<Vec<PeriodCount>> {
    let format = match period {
        Period::Hour => "%Y-%m-%d %H:00",
        Period::Day => "%Y-%m-%d",
    };
    let mut query = QueryBuilder::new("SELECT strftime(");
    query.push_bind(format).push(
        ", datetime(packet.import_time_us / (1000000 + 0.0), 'unixepoch')) AS period, \
             count(*) AS count FROM packet WHERE",
    );
    push_filter(&mut query, filter);
    query.push(" GROUP BY period ORDER BY period");
    query
        .build_query_as()
        .fetch_all(pool)
        .await
        .context("counting packets per period")
}

pub async fn count_packets(pool: &SqlitePool, filter: Option<&PacketFilter>) -> Result<i64> {
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

pub async fn count_sightings(pool: &SqlitePool, of: SightingsOf<'_>) -> Result<i64> {
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

/// The traffic of each node in the window, as the `sent` and `seen` common
/// table expressions. The windows are anchored to the latest packet and the
/// latest sighting rather than to now, as in the Python version, so that a mesh
/// that has gone quiet still has a top list.
fn push_top_traffic(query: &mut QueryBuilder<Sqlite>, window_us: i64) {
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

fn push_top_conditions(query: &mut QueryBuilder<Sqlite>, top: &TopQuery) {
    let mut first = true;
    let mut condition = |query: &mut QueryBuilder<Sqlite>| {
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

pub async fn top_nodes(pool: &SqlitePool, top: &TopQuery) -> Result<(i64, Vec<TopNode>)> {
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
        " ORDER BY coalesce(seen.seen, 0) DESC, coalesce(sent.sent, 0) DESC, node.node_id \
         LIMIT ",
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
