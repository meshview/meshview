// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The daily jobs, and backing up.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use meshview::db::sqlite;
use meshview::jobs::{self, Settings, TimeOfDay};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use tempfile::TempDir;
use tokio::sync::watch;

const DAY_US: i64 = 86_400 * 1_000_000;

fn now_us() -> i64 {
    Timestamp::now().as_microsecond()
}

async fn database(dir: &TempDir) -> SqlitePool {
    sqlite::open_for_writing(&dir.path().join("packets.db"))
        .await
        .unwrap()
}

async fn count(pool: &SqlitePool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

async fn packet(pool: &SqlitePool, id: i64, from: i64, imported_us: i64) {
    sqlx::query(
        "INSERT INTO packet (id, portnum, from_node_id, import_time_us) VALUES (?, 1, ?, ?)",
    )
    .bind(id)
    .bind(from)
    .bind(imported_us)
    .execute(pool)
    .await
    .unwrap();
}

async fn seen(pool: &SqlitePool, packet: i64, gateway: i64, imported_us: i64) {
    sqlx::query(
        "INSERT INTO packet_seen (packet_id, node_id, rx_time, import_time_us) VALUES (?, ?, 0, ?)",
    )
    .bind(packet)
    .bind(gateway)
    .bind(imported_us)
    .execute(pool)
    .await
    .unwrap();
}

async fn node(pool: &SqlitePool, node_id: i64, last_seen_us: Option<i64>) {
    sqlx::query("INSERT INTO node (id, node_id, last_seen_us) VALUES (?, ?, ?)")
        .bind(format!("!{node_id:08x}"))
        .bind(node_id)
        .bind(last_seen_us)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn cleanup_deletes_what_is_older_than_the_days_kept() {
    let dir = TempDir::new().unwrap();
    let pool = database(&dir).await;
    let now = now_us();
    let old = now - 15 * DAY_US;
    let recent = now - DAY_US;

    packet(&pool, 1, 10, old).await;
    packet(&pool, 2, 10, recent).await;
    seen(&pool, 1, 20, old).await;
    seen(&pool, 2, 20, recent).await;
    // A sighting today of an old packet stays: each table goes by its own time.
    seen(&pool, 1, 21, recent).await;
    sqlx::query(
        "INSERT INTO traceroute (packet_id, gateway_node_id, import_time_us) VALUES (1, 20, ?), \
         (2, 20, ?)",
    )
    .bind(old)
    .bind(recent)
    .execute(&pool)
    .await
    .unwrap();
    node(&pool, 10, Some(recent)).await;
    node(&pool, 11, Some(old)).await;
    // Never seen at all, as Python leaves some: not older than anything.
    node(&pool, 12, None).await;

    let details = jobs::cleanup(&pool, 14, true).await.unwrap();

    assert_eq!(details["rows_deleted"]["packet"], 1);
    assert_eq!(details["rows_deleted"]["packet_seen"], 1);
    assert_eq!(details["rows_deleted"]["traceroute"], 1);
    assert_eq!(details["rows_deleted"]["node"], 1);
    assert_eq!(details["vacuum_completed"], true);
    assert_eq!(count(&pool, "SELECT count(*) FROM packet").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM packet_seen").await, 2);
    assert_eq!(count(&pool, "SELECT count(*) FROM traceroute").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM node").await, 2);

    let run = sqlx::query("SELECT job, status, finished_us, details FROM job_run")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(run.get::<String, _>("job"), jobs::CLEANUP);
    assert_eq!(run.get::<String, _>("status"), "ok");
    assert!(run.get::<Option<i64>, _>("finished_us").is_some());
    let recorded: serde_json::Value =
        serde_json::from_str(&run.get::<String, _>("details")).unwrap();
    assert_eq!(recorded, details);
}

#[tokio::test]
async fn snapshot_records_the_previous_utc_day_once() {
    let dir = TempDir::new().unwrap();
    let pool = database(&dir).await;

    let today = Timestamp::now().to_zoned(TimeZone::UTC).date();
    let midnight = today
        .at(0, 0, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
        .as_microsecond();
    let yesterday = midnight - DAY_US / 2;
    let before = midnight - 2 * DAY_US;

    // Yesterday: two packets from node 10, heard by gateways 20 and 21.
    packet(&pool, 1, 10, yesterday).await;
    packet(&pool, 2, 10, yesterday).await;
    seen(&pool, 1, 20, yesterday).await;
    seen(&pool, 2, 21, yesterday).await;
    // Node 11's packet is older, but was heard again yesterday: it was active.
    packet(&pool, 3, 11, before).await;
    seen(&pool, 3, 20, yesterday).await;
    // Today counts for tomorrow's snapshot, not this one.
    packet(&pool, 4, 12, midnight + 1).await;
    seen(&pool, 4, 22, midnight + 1).await;

    let details = jobs::snapshot(&pool).await.unwrap();
    assert_eq!(
        details["snapshot_date"],
        today.yesterday().unwrap().to_string()
    );
    assert_eq!(details["node_count"], 2);
    assert_eq!(details["packet_count"], 2);
    assert_eq!(details["gateway_count"], 2);

    jobs::snapshot(&pool).await.unwrap();
    let rows = sqlx::query("SELECT * FROM daily_snapshot")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "a second run replaces the first");
    assert_eq!(
        rows[0].get::<String, _>("snapshot_date"),
        today.yesterday().unwrap().to_string()
    );
    assert_eq!(rows[0].get::<i64, _>("node_count"), 2);
    assert_eq!(count(&pool, "SELECT count(*) FROM job_run").await, 2);
}

#[tokio::test]
async fn only_the_latest_runs_are_kept() {
    let dir = TempDir::new().unwrap();
    let pool = database(&dir).await;

    for _ in 0..95 {
        jobs::snapshot(&pool).await.unwrap();
    }

    assert_eq!(count(&pool, "SELECT count(*) FROM job_run").await, 90);
}

#[tokio::test]
async fn runs_a_previous_start_left_running_are_marked_interrupted() {
    let dir = TempDir::new().unwrap();
    let pool = database(&dir).await;
    sqlx::query("INSERT INTO job_run (job, status, started_us) VALUES ('cleanup', 'running', 1)")
        .execute(&pool)
        .await
        .unwrap();

    let (_stop, stopping) = watch::channel(true);
    let settings = Settings {
        cleanup: None,
        snapshot_at: TimeOfDay { hour: 1, minute: 0 },
    };
    jobs::run(settings, pool.clone(), stopping).await;

    let status: String = sqlx::query_scalar("SELECT status FROM job_run")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "interrupted");
}

fn side_files(database: &Path) -> Vec<String> {
    ["-wal", "-shm"]
        .iter()
        .map(|suffix| format!("{}{suffix}", database.display()))
        .filter(|file| Path::new(file).exists())
        .collect()
}

#[tokio::test]
async fn a_backup_is_whole_while_the_database_is_written() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("packets.db");
    let writer = sqlite::open_for_writing(&database).await.unwrap();
    packet(&writer, 1, 10, now_us()).await;
    let output = dir.path().join("copy.db");

    // With the writer still open, and what it wrote still in the WAL.
    let size = sqlite::backup(&database, &output).await.unwrap();

    assert!(size > 0);
    assert!(!dir.path().join("copy.db.partial").exists());
    let copy = sqlite::open_for_writing(&output).await.unwrap();
    assert_eq!(count(&copy, "SELECT count(*) FROM packet").await, 1);
    sqlite::close_writer(copy).await;

    // Never over an existing file.
    assert!(sqlite::backup(&database, &output).await.is_err());

    sqlite::close_writer(writer).await;
}

#[tokio::test]
async fn backing_up_a_stopped_database_leaves_it_as_it_was() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("packets.db");
    sqlite::close_writer(sqlite::open_for_writing(&database).await.unwrap()).await;

    sqlite::backup(&database, &dir.path().join("copy.db"))
        .await
        .unwrap();

    assert_eq!(side_files(&database), Vec::<String>::new());
    assert!(
        sqlite::backup(&dir.path().join("absent.db"), &dir.path().join("x.db"))
            .await
            .is_err()
    );
}
