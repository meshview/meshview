// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Closing the database leaves only the database: no write-ahead log and no
//! shared memory index beside it.

use meshview::db::sqlite;
use std::path::Path;
use tempfile::TempDir;

fn side_files(database: &Path) -> Vec<String> {
    ["-wal", "-shm", "-journal"]
        .iter()
        .map(|suffix| format!("{}{suffix}", database.display()))
        .filter(|file| Path::new(file).exists())
        .collect()
}

#[tokio::test]
async fn closing_in_order_leaves_nothing_beside_the_file() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("packets.db");

    let writer = sqlite::open_for_writing(&database).await.unwrap();
    sqlx::query("INSERT INTO packet (id, portnum) VALUES (1, 1)")
        .execute(&writer)
        .await
        .unwrap();

    let reader = sqlite::open_for_reading(&database).await.unwrap().unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM packet")
        .fetch_one(&reader)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        !side_files(&database).is_empty(),
        "WAL mode should be in use"
    );

    sqlite::close_reader(reader).await;
    sqlite::close_writer(writer).await;

    assert_eq!(side_files(&database), Vec::<String>::new());

    // And what was written is in the file itself.
    let reopened = sqlite::open_for_writing(&database).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM packet")
        .fetch_one(&reopened)
        .await
        .unwrap();
    assert_eq!(count, 1);
    sqlite::close_writer(reopened).await;
}

#[tokio::test]
async fn migrating_alone_leaves_nothing_either() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("packets.db");

    let writer = sqlite::open_for_writing(&database).await.unwrap();
    sqlite::close_writer(writer).await;

    assert_eq!(side_files(&database), Vec::<String>::new());
}
