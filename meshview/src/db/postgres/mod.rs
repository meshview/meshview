// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! PostgreSQL: how it is opened, what its catalogue says, and its migrations.
//!
//! The same two ways in as SQLite, the writer for the ingester, which is the only
//! one that migrates, and readers for the web server, but without SQLite's
//! reasons to hold back: PostgreSQL takes concurrent writers and readers, so the
//! writer gets a few connections, and the daily jobs need not wait for the
//! ingester's turn. A database that is not there cannot be created from here,
//! only its tables: the server, the role and the database are the
//! administrator's.

pub mod queries;

use super::{Compatibility, Schema};
use anyhow::{Context, Result, bail};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::{ConnectOptions, Row};
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

/// The migrations, compiled into the binary. Numbered as SQLite's, one for one:
/// a number is the same step of the schema on either database.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations/postgres");

/// From how long a write is worth a warning; see SQLite's.
const SLOW_WRITE: Duration = Duration::from_secs(30);

/// Connections for the writer: the ingester, and a daily job beside it.
const WRITE_CONNECTIONS: u32 = 4;

/// Read connections per process.
const READ_CONNECTIONS: u32 = 8;

/// The versions of the migrations this build carries, in order.
pub fn known_versions() -> Vec<i64> {
    MIGRATOR.iter().map(|migration| migration.version).collect()
}

fn options(url: &str) -> Result<PgConnectOptions> {
    Ok(PgConnectOptions::from_str(url)
        .context("reading the PostgreSQL connection string")?
        // The notices are things like "relation already exists, skipping", one
        // for each IF NOT EXISTS of a migration over a Python database.
        .options([("client_min_messages", "warning")]))
}

/// Opens the database for writing and brings its schema up to date, refusing
/// one it should not touch, as SQLite's does.
pub async fn open_for_writing(url: &str, shown: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(WRITE_CONNECTIONS)
        .connect_with(options(url)?.log_slow_statements(log::LevelFilter::Warn, SLOW_WRITE))
        .await
        .with_context(|| format!("connecting to {shown} to write"))?;

    let schema = inspect(&pool).await?;
    match schema.compatibility(&known_versions()) {
        Compatibility::Current => {}
        Compatibility::Behind => {
            info!("database is {schema}, migrating");
            MIGRATOR
                .run(&pool)
                .await
                .with_context(|| format!("migrating {shown}"))?;
            info!("database migrated to {}", inspect(&pool).await?);
        }
        Compatibility::Ahead {
            latest_known,
            found,
        } => bail!(
            "{shown} has been migrated to {found} by a newer Meshview, and this one only knows \
             up to {latest_known}: run the newer version"
        ),
        Compatibility::Incompatible(why) => bail!("{shown}: {why}"),
    }

    Ok(pool)
}

/// Opens the database for reading. Nothing stops a reader writing here, as a
/// read only file does in SQLite; nothing on the web side writes.
pub async fn open_for_reading(url: &str, shown: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(READ_CONNECTIONS)
        .connect_with(options(url)?)
        .await
        .with_context(|| format!("connecting to {shown} to read"))
}

/// Opens one connection that does not warn about slow statements, for reading
/// through the database on purpose.
pub async fn open_quietly_for_reading(url: &str, shown: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options(url)?.log_slow_statements(log::LevelFilter::Off, Duration::MAX))
        .await
        .with_context(|| format!("connecting to {shown} to read"))
}

/// What the database holds, read from its catalogue: the tables of the schema
/// the connection works in, `public` unless its search path says otherwise.
pub async fn inspect(pool: &PgPool) -> Result<Schema> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'",
    )
    .fetch_all(pool)
    .await
    .context("reading the list of tables")?;

    let has = |name: &str| tables.iter().any(|table| table == name);

    if has("_sqlx_migrations") {
        let rows = sqlx::query("SELECT version, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .context("reading the applied migrations")?;

        let mut applied = Vec::with_capacity(rows.len());
        for row in rows {
            let version: i64 = row.try_get("version")?;
            if !row.try_get::<bool, _>("success")? {
                bail!(
                    "migration {version} failed partway through and the database needs to be \
                     restored from a backup"
                );
            }
            applied.push(version);
        }
        return Ok(Schema::Migrated { applied });
    }

    if has("alembic_version") {
        let revision: Option<String> =
            sqlx::query_scalar("SELECT version_num::text FROM alembic_version LIMIT 1")
                .fetch_optional(pool)
                .await
                .context("reading the Alembic revision")?;
        return Ok(Schema::Python {
            revision: revision.unwrap_or_default(),
        });
    }

    if tables.is_empty() {
        Ok(Schema::Empty)
    } else {
        Ok(Schema::Unknown)
    }
}
