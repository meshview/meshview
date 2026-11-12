// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! PostgreSQL for the tests: a database of its own for each, on the server
//! `MESHVIEW_TEST_POSTGRES` points at, such as
//! `postgresql://postgres@127.0.0.1:5433/postgres`. The role there has to be
//! able to create databases. Without the variable these tests pass without
//! doing anything, and say so, so that `cargo test` needs no server.

use sqlx::{Connection, Executor, PgConnection, Row};
use std::sync::atomic::{AtomicU32, Ordering};

/// A database at the Python version's last revision, as it leaves it.
pub const PYTHON_HEAD: &str = include_str!("../fixtures/python-postgres-44612e74a798.sql");

const VARIABLE: &str = "MESHVIEW_TEST_POSTGRES";

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A database made for one test, dropped by [`TestDatabase::drop`].
pub struct TestDatabase {
    /// Its URL, as the driver takes it.
    pub url: String,
    admin: String,
    name: String,
}

/// A new, empty database, built from `sql` if there is any; `None`, after
/// saying so, when there is no server to test against.
pub async fn database(sql: Option<&str>) -> Option<TestDatabase> {
    let Ok(admin) = std::env::var(VARIABLE) else {
        eprintln!("{VARIABLE} is not set: skipping a PostgreSQL test");
        return None;
    };
    let name = format!(
        "meshview_test_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let mut conn = PgConnection::connect(&admin).await.unwrap();
    conn.execute(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {name}"
    )))
    .await
    .unwrap();
    conn.execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .await
        .unwrap();
    conn.close().await.unwrap();

    let url = with_database(&admin, &name);
    if let Some(sql) = sql {
        let mut conn = PgConnection::connect(&url).await.unwrap();
        // Fixtures written here, not input from anywhere.
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
    }
    Some(TestDatabase { url, admin, name })
}

impl TestDatabase {
    /// A connection of its own.
    pub async fn connect(&self) -> PgConnection {
        PgConnection::connect(&self.url).await.unwrap()
    }

    /// Drops the database, whoever is still connected to it.
    pub async fn drop(self) {
        let mut conn = PgConnection::connect(&self.admin).await.unwrap();
        conn.execute(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.name
        )))
        .await
        .unwrap();
    }
}

/// The admin URL with another database in its path.
fn with_database(admin: &str, name: &str) -> String {
    let (base, query) = match admin.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (admin, None),
    };
    let (scheme, rest) = base.split_once("://").unwrap();
    let host = rest.split('/').next().unwrap();
    let mut url = format!("{scheme}://{host}/{name}");
    if let Some(query) = query {
        url.push('?');
        url.push_str(query);
    }
    url
}

/// What a schema is made of, in a form two databases can be compared by: every
/// table with its columns, every constraint and every index, by name and
/// definition.
pub async fn shape(database: &TestDatabase) -> Vec<String> {
    let mut conn = database.connect().await;
    let mut shape = Vec::new();

    let columns = sqlx::query(
        "SELECT table_name::text, column_name::text, data_type::text, is_nullable::text, \
                coalesce(column_default, '') AS column_default \
         FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name <> '_sqlx_migrations' \
         ORDER BY table_name, ordinal_position",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    for column in columns {
        shape.push(format!(
            "{}.{} {} nullable={} default={}",
            column.get::<String, _>("table_name"),
            column.get::<String, _>("column_name"),
            column.get::<String, _>("data_type"),
            column.get::<String, _>("is_nullable"),
            column.get::<String, _>("column_default"),
        ));
    }

    let constraints = sqlx::query(
        "SELECT conrelid::regclass::text AS table_name, conname::text, \
                pg_get_constraintdef(oid) AS definition \
         FROM pg_constraint \
         WHERE connamespace = 'public'::regnamespace AND conrelid::regclass::text <> '_sqlx_migrations' \
         ORDER BY 1, 2",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    for constraint in constraints {
        shape.push(format!(
            "{} {} {}",
            constraint.get::<String, _>("table_name"),
            constraint.get::<String, _>("conname"),
            constraint.get::<String, _>("definition"),
        ));
    }

    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes \
         WHERE schemaname = 'public' AND tablename <> '_sqlx_migrations' ORDER BY indexname",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    shape.extend(indexes);

    conn.close().await.unwrap();
    shape
}
