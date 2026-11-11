// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The read queries, in SQLite's dialect. Each is the Python meshview's, as
//! SQLAlchemy emitted it, so that both answer the same; where an ordering had no
//! tiebreak, one is added, so that the answer does not depend on the query plan.

use crate::db::{DailySnapshot, PacketSeen};
use anyhow::{Context, Result};
use sqlx::SqlitePool;

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
