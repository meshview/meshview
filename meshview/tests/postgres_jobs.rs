// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The daily jobs on PostgreSQL do what they do on SQLite, to the same rows.
//! Needs a server: see `common::postgres`.

#![cfg(feature = "postgres")]

mod common;

use common::postgres::database;
use meshview::config::DatabaseUrl;
use meshview::db::write::{self, Arg};
use meshview::db::{self, Pool, Reader};
use meshview::jobs;
use tempfile::TempDir;

const DAY_US: i64 = 86_400 * 1_000_000;

fn now_us() -> i64 {
    jiff::Timestamp::now().as_microsecond()
}

/// The same rows, written through the same statements, into either database:
/// old and recent packets, an old one heard again today, their sightings and
/// traceroutes, and nodes seen long ago, lately and never.
async fn fill(pool: &Pool) {
    let now = now_us();
    let old = now - 15 * DAY_US;
    let recent = now - DAY_US;
    let yesterday = now - DAY_US / 2;
    let mut tx = write::begin(pool).await.unwrap();
    for (id, at) in [(1, old), (2, recent), (3, old), (4, yesterday)] {
        tx.execute(
            "INSERT INTO packet (id, portnum, from_node_id, to_node_id, import_time_us) \
             VALUES (?, 1, 10, 4294967295, ?)",
            &[Arg::Int(id), Arg::Int(at)],
        )
        .await
        .unwrap();
    }
    for (packet, gateway, at) in [
        (1, 20, old),
        (2, 20, recent),
        (3, 20, old),
        (1, 21, recent),
        (4, 22, yesterday),
    ] {
        tx.execute(
            "INSERT INTO packet_seen (packet_id, node_id, rx_time, import_time_us) VALUES (?, ?, 0, ?)",
            &[Arg::Int(packet), Arg::Int(gateway), Arg::Int(at)],
        )
        .await
        .unwrap();
    }
    for (packet, at) in [(1, old), (2, recent)] {
        tx.execute(
            "INSERT INTO traceroute (packet_id, gateway_node_id, import_time_us) VALUES (?, 20, ?)",
            &[Arg::Int(packet), Arg::Int(at)],
        )
        .await
        .unwrap();
    }
    for (node, seen) in [(10, Some(recent)), (11, Some(old)), (12, None)] {
        tx.execute(
            "INSERT INTO node (id, node_id, last_seen_us) VALUES (?, ?, ?)",
            &[
                Arg::Text(&format!("!{node:08x}")),
                Arg::Int(node),
                Arg::OptInt(seen),
            ],
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}

/// What is left, the packets by id and the other tables by count.
async fn left(pool: &Pool) -> Vec<i64> {
    let mut out = write::integers(pool, "SELECT id FROM packet ORDER BY id", &[])
        .await
        .unwrap();
    for table in ["packet_seen", "traceroute", "node"] {
        let sql: &'static str = match table {
            "packet_seen" => "SELECT count(*) FROM packet_seen",
            "traceroute" => "SELECT count(*) FROM traceroute",
            _ => "SELECT count(*) FROM node",
        };
        out.extend(write::integers(pool, sql, &[]).await.unwrap());
    }
    out
}

/// Runs both jobs, the snapshot twice, and answers what they said and left.
async fn run_jobs(pool: &Pool) -> (serde_json::Value, serde_json::Value, Vec<i64>, Vec<i64>) {
    fill(pool).await;
    let cleanup = jobs::cleanup(pool, 14, true).await.unwrap();
    jobs::snapshot(pool).await.unwrap();
    let mut snapshot = jobs::snapshot(pool).await.unwrap();
    snapshot.as_object_mut().unwrap().remove("captured_at_us");
    let snapshots = write::integers(
        pool,
        "SELECT node_count * 1000000 + packet_count * 1000 + gateway_count FROM daily_snapshot",
        &[],
    )
    .await
    .unwrap();
    (cleanup, snapshot, left(pool).await, snapshots)
}

#[tokio::test]
async fn the_jobs_do_the_same_on_either_database() {
    let Some(postgres) = database(None).await else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let sqlite = db::open_for_writing(&DatabaseUrl::Sqlite(dir.path().join("packets.db")))
        .await
        .unwrap();
    let pg = db::open_for_writing(&DatabaseUrl::Postgres(postgres.url.clone()))
        .await
        .unwrap();

    let (mut sqlite_cleanup, sqlite_snapshot, sqlite_left, sqlite_snapshots) =
        run_jobs(&sqlite).await;
    let (mut pg_cleanup, pg_snapshot, pg_left, pg_snapshots) = run_jobs(&pg).await;

    // The cutoff is whenever each ran.
    for details in [&mut sqlite_cleanup, &mut pg_cleanup] {
        let details = details.as_object_mut().unwrap();
        details.remove("cutoff_us");
        details.remove("cutoff_at");
    }
    assert_eq!(sqlite_cleanup, pg_cleanup);
    assert_eq!(pg_cleanup["vacuum_completed"], true);
    assert_eq!(sqlite_snapshot, pg_snapshot);
    assert_eq!(sqlite_left, pg_left);
    // Packets 1, heard again lately, 2 and 4; the old 3 is gone.
    assert_eq!(&pg_left[..3], &[1, 2, 4]);
    // One row for the day, however many times it was taken.
    assert_eq!(sqlite_snapshots, pg_snapshots);
    assert_eq!(pg_snapshots.len(), 1);

    // The runs are recorded where the health check reads them.
    let reader = Reader::new(pg.clone());
    let run = reader
        .latest_job_run(jobs::SNAPSHOT)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "ok");
    assert!(run.finished_us.is_some());
    let run = reader.latest_job_run(jobs::CLEANUP).await.unwrap().unwrap();
    assert_eq!(run.status, "ok");

    db::close_writer(pg).await;
    db::close_writer(sqlite).await;
    postgres.drop().await;
}
