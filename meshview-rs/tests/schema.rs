// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Taking a database over from the Python meshview, and refusing the ones that
//! should not be.

mod common;

use common::{PYTHON_HEAD, connect, database_from, shape};
use meshview_rs::db::sqlite::{self, known_versions};
use meshview_rs::db::{PYTHON_HEAD_REVISION, Schema};
use sqlx::{Connection, Executor};
use tempfile::TempDir;

#[tokio::test]
async fn an_empty_file_gets_the_whole_schema() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("new.db");

    let pool = sqlite::open_for_writing(&path).await.unwrap();

    assert_eq!(
        sqlite::inspect(&pool).await.unwrap(),
        Schema::Migrated {
            applied: known_versions()
        }
    );
}

#[tokio::test]
async fn a_python_database_is_taken_over_and_ends_up_like_a_new_one() {
    let dir = TempDir::new().unwrap();
    let python = database_from(&dir, "python.db", PYTHON_HEAD).await;
    let new = dir.path().join("new.db");

    sqlite::open_for_writing(&python)
        .await
        .unwrap()
        .close()
        .await;
    sqlite::open_for_writing(&new).await.unwrap().close().await;

    // The same tables, columns and indexes either way, and none of Python's
    // bookkeeping left.
    let adopted = shape(&python).await;
    assert_eq!(adopted, shape(&new).await);
    for gone in ["alembic_version", "migration_status", "example"] {
        assert!(
            !adopted
                .iter()
                .any(|line| line.starts_with(&format!("{gone}."))),
            "{gone} is still there"
        );
    }

    // And the data is still there.
    let mut conn = connect(&python, false).await;
    for (table, count) in [
        ("node", "SELECT count(*) FROM node"),
        ("packet", "SELECT count(*) FROM packet"),
        ("packet_seen", "SELECT count(*) FROM packet_seen"),
        ("traceroute", "SELECT count(*) FROM traceroute"),
        ("daily_snapshot", "SELECT count(*) FROM daily_snapshot"),
    ] {
        let rows: i64 = sqlx::query_scalar(count)
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(rows, 1, "{table} lost its row");
    }
}

#[tokio::test]
async fn taking_over_twice_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let python = database_from(&dir, "python.db", PYTHON_HEAD).await;

    sqlite::open_for_writing(&python)
        .await
        .unwrap()
        .close()
        .await;
    let once = shape(&python).await;
    sqlite::open_for_writing(&python)
        .await
        .unwrap()
        .close()
        .await;

    assert_eq!(once, shape(&python).await);
}

#[tokio::test]
async fn an_older_python_revision_is_refused_untouched() {
    let dir = TempDir::new().unwrap();
    let older = PYTHON_HEAD.replace(
        &format!("INSERT INTO alembic_version VALUES('{PYTHON_HEAD_REVISION}')"),
        "INSERT INTO alembic_version VALUES('4d2a7c9e1b30')",
    );
    assert_ne!(
        older, PYTHON_HEAD,
        "the fixture no longer has the revision row"
    );
    let path = database_from(&dir, "older.db", &older).await;
    let before = shape(&path).await;

    let error = sqlite::open_for_writing(&path).await.unwrap_err();

    assert!(format!("{error:#}").contains("4d2a7c9e1b30"));
    assert_eq!(before, shape(&path).await);
}

#[tokio::test]
async fn a_database_that_is_not_meshview_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = database_from(&dir, "other.db", "CREATE TABLE something (id INTEGER);").await;

    assert!(sqlite::open_for_writing(&path).await.is_err());
}

#[tokio::test]
async fn a_database_from_a_newer_build_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("newer.db");
    sqlite::open_for_writing(&path).await.unwrap().close().await;

    let mut conn = connect(&path, false).await;
    conn.execute(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (9999, 'from the future', 1, x'00', 0)",
    )
    .await
    .unwrap();
    conn.close().await.unwrap();

    let error = sqlite::open_for_writing(&path).await.unwrap_err();
    assert!(format!("{error:#}").contains("newer meshview-rs"));
}

#[tokio::test]
async fn reading_never_creates_the_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("absent.db");

    assert!(sqlite::open_for_reading(&path).await.unwrap().is_none());
    assert!(!path.exists());
}
