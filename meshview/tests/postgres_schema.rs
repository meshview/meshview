// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Taking a PostgreSQL database over from the Python Meshview, and refusing the
//! ones that should not be, as in SQLite. Needs a server: see
//! `common::postgres`.

#![cfg(feature = "postgres")]

mod common;

use common::postgres::{PYTHON_HEAD, TestDatabase, database, shape};
use meshview::config::DatabaseUrl;
use meshview::db::{self, PYTHON_HEAD_REVISION, Schema};
use sqlx::{Connection, Executor};

fn url(database: &TestDatabase) -> DatabaseUrl {
    DatabaseUrl::Postgres(database.url.clone())
}

async fn open_and_close(database: &TestDatabase) {
    db::close_writer(db::open_for_writing(&url(database)).await.unwrap()).await;
}

#[tokio::test]
async fn an_empty_database_gets_the_whole_schema() {
    let Some(new) = database(None).await else {
        return;
    };

    let pool = db::open_for_writing(&url(&new)).await.unwrap();

    assert_eq!(
        db::inspect(&pool).await.unwrap(),
        Schema::Migrated {
            applied: db::known_versions(&url(&new))
        }
    );
    db::close_writer(pool).await;
    new.drop().await;
}

#[tokio::test]
async fn a_python_database_is_taken_over_and_ends_up_like_a_new_one() {
    let Some(python) = database(Some(PYTHON_HEAD)).await else {
        return;
    };
    let new = database(None).await.unwrap();

    open_and_close(&python).await;
    open_and_close(&new).await;

    // The same tables, columns, constraints and indexes either way, and none
    // of Python's bookkeeping left.
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
    let mut conn = python.connect().await;
    for table in [
        "node",
        "packet",
        "packet_seen",
        "traceroute",
        "daily_snapshot",
    ] {
        let rows: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&mut conn)
                .await
                .unwrap();
        assert_eq!(rows, 1, "{table} lost its row");
    }
    conn.close().await.unwrap();
    python.drop().await;
    new.drop().await;
}

#[tokio::test]
async fn taking_over_twice_changes_nothing() {
    let Some(python) = database(Some(PYTHON_HEAD)).await else {
        return;
    };

    open_and_close(&python).await;
    let once = shape(&python).await;
    open_and_close(&python).await;

    assert_eq!(once, shape(&python).await);
    python.drop().await;
}

#[tokio::test]
async fn an_older_python_revision_is_refused_untouched() {
    let older = PYTHON_HEAD.replace(
        &format!("INSERT INTO public.alembic_version VALUES ('{PYTHON_HEAD_REVISION}')"),
        "INSERT INTO public.alembic_version VALUES ('4d2a7c9e1b30')",
    );
    assert_ne!(
        older, PYTHON_HEAD,
        "the fixture no longer has the revision row"
    );
    let Some(python) = database(Some(&older)).await else {
        return;
    };
    let before = shape(&python).await;

    let error = db::open_for_writing(&url(&python)).await.unwrap_err();

    assert!(format!("{error:#}").contains("4d2a7c9e1b30"));
    assert_eq!(before, shape(&python).await);
    python.drop().await;
}

#[tokio::test]
async fn a_database_that_is_not_meshview_is_refused() {
    let Some(other) = database(Some("CREATE TABLE something (id INTEGER);")).await else {
        return;
    };

    assert!(db::open_for_writing(&url(&other)).await.is_err());
    other.drop().await;
}

#[tokio::test]
async fn a_database_from_a_newer_build_is_refused() {
    let Some(newer) = database(None).await else {
        return;
    };
    open_and_close(&newer).await;

    let mut conn = newer.connect().await;
    conn.execute(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (9999, 'from the future', true, '\\x00', 0)",
    )
    .await
    .unwrap();
    conn.close().await.unwrap();

    let error = db::open_for_writing(&url(&newer)).await.unwrap_err();
    assert!(format!("{error:#}").contains("newer Meshview"));
    newer.drop().await;
}

#[tokio::test]
async fn both_databases_carry_the_same_steps() {
    assert_eq!(
        db::known_versions(&DatabaseUrl::Sqlite("x.db".into())),
        db::known_versions(&DatabaseUrl::Postgres(String::new()))
    );
}
