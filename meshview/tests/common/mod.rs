// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What more than one integration test needs: the Python fixture, and ways to
//! build a database from SQL and to compare two schemas.

// Each test crate uses its own part of this.
#![allow(dead_code)]

#[cfg(feature = "postgres")]
pub mod postgres;

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Row, SqliteConnection};
use std::path::Path;
use tempfile::TempDir;

/// A database at the Python version's last revision, as it leaves it.
pub const PYTHON_HEAD: &str = include_str!("../fixtures/python-44612e74a798.sql");

/// What a schema is made of, in a form two databases can be compared by: every
/// table with its columns, and every index with its definition.
pub async fn shape(path: &Path) -> Vec<String> {
    let mut conn = connect(path, false).await;
    let mut shape = Vec::new();

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations' ORDER BY name",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();

    for table in tables {
        let columns = sqlx::query("SELECT name, type, \"notnull\", pk FROM pragma_table_info(?)")
            .bind(&table)
            .fetch_all(&mut conn)
            .await
            .unwrap();
        for column in columns {
            shape.push(format!(
                "{table}.{} {} notnull={} pk={}",
                column.get::<String, _>("name"),
                column.get::<String, _>("type"),
                column.get::<i64, _>("notnull"),
                column.get::<i64, _>("pk"),
            ));
        }
    }

    let indexes = sqlx::query(
        "SELECT name, sql FROM sqlite_master WHERE type = 'index' AND sql IS NOT NULL \
         ORDER BY name",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    for index in indexes {
        // Whitespace and IF NOT EXISTS are how a statement was written, not what
        // it made.
        let sql = index
            .get::<String, _>("sql")
            .replace("IF NOT EXISTS ", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        shape.push(sql);
    }

    shape
}

pub async fn connect(path: &Path, create: bool) -> SqliteConnection {
    SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(create),
    )
    .await
    .unwrap()
}

/// A file holding what `sql` builds.
pub async fn database_from(dir: &TempDir, name: &str, sql: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    let mut conn = connect(&path, true).await;
    // Fixtures written here, not input from anywhere.
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();
    path
}
