// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! SQLite: how it is opened, what its catalogue says, and its migrations.
//!
//! There are two ways in. The writer is the ingester: one connection, because
//! SQLite has one write lock and a second connection would only queue for it, and
//! the only one allowed to create the file or migrate it. Readers are the web
//! server: a pool of read only connections, which is what lets several of them,
//! even in separate processes, share the file with the writer under WAL.

use super::{Compatibility, Schema};
use anyhow::{Context, Result, bail};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::time::Duration;
use tracing::{info, warn};

/// The migrations, compiled into the binary.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations/sqlite");

/// How long a statement waits for a lock before giving up. Generous, because
/// waiting costs nothing when nothing is contended and a failed request costs a
/// lot, and because the readers may be other processes doing whatever they do.
const BUSY_TIMEOUT: Duration = Duration::from_secs(15);

/// Read connections per process. Every request holds one only while its queries
/// run, and SQLite readers do not block each other under WAL.
const READ_CONNECTIONS: u32 = 8;

/// The versions of the migrations this build carries, in order.
pub fn known_versions() -> Vec<i64> {
    MIGRATOR.iter().map(|migration| migration.version).collect()
}

fn options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .busy_timeout(BUSY_TIMEOUT)
        // sqlx turns foreign keys on by default; SQLite and the Python version
        // never did. The data already in these databases was written without
        // them being enforced, and the cleanup deletes in an order that assumes
        // they are not.
        .foreign_keys(false)
}

/// Opens the database for writing, creating it if it is not there, and brings
/// its schema up to date.
///
/// Refuses a database it should not touch: one at an older Python revision, one
/// a newer build of this program has migrated, or one that is not meshview's at
/// all. Each says what to do about it.
pub async fn open_for_writing(path: &Path) -> Result<SqlitePool> {
    let options = options(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // Under WAL this loses nothing on a crash of the process, only possibly
        // the last transactions on a crash of the machine, and saves a sync per
        // commit, which for a stream of small inserts is most of the cost.
        .synchronous(SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| format!("opening {} for writing", path.display()))?;

    let schema = inspect(&pool).await?;
    match schema.compatibility(&known_versions()) {
        Compatibility::Current => {}
        Compatibility::Behind => {
            info!("database is {schema}, migrating");
            MIGRATOR
                .run(&pool)
                .await
                .with_context(|| format!("migrating {}", path.display()))?;
            info!("database migrated to {}", inspect(&pool).await?);
        }
        Compatibility::Ahead {
            latest_known,
            found,
        } => bail!(
            "{} has been migrated to {found} by a newer meshview-rs, and this one only knows \
             up to {latest_known}: run the newer version",
            path.display()
        ),
        Compatibility::Incompatible(why) => bail!("{}: {why}", path.display()),
    }

    Ok(pool)
}

/// Opens the database for reading, without ever creating or changing it.
///
/// Answers `None` while there is no file yet, which for a web server started
/// beside an ingester that has not got going is a reason to wait, not to fail.
pub async fn open_for_reading(path: &Path) -> Result<Option<SqlitePool>> {
    if !path.exists() {
        return Ok(None);
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(READ_CONNECTIONS)
        .connect_with(options(path).read_only(true))
        .await
        .with_context(|| format!("opening {} for reading", path.display()))?;

    Ok(Some(pool))
}

/// Closes the writer so that nothing is left beside the database file.
///
/// Under WAL, committed data can still be in the `-wal` file, with the `-shm`
/// index beside it, and SQLite folds it back in and removes both only when the
/// last connection closes cleanly. A process that just exits leaves them, which
/// is harmless to the next start but untidy, and a backup of the bare file then
/// misses what is in them. So the log is folded back in here first, explicitly,
/// and the connection closed and waited for.
///
/// When another process still has the database open, a web server reading it,
/// the files stay for that one, which is as it should be: the last to close is
/// the one that tidies up, and a read only connection cannot.
pub async fn close_writer(pool: SqlitePool) {
    if let Err(e) = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await
    {
        warn!("could not fold the write-ahead log back into the database: {e}");
    }
    pool.close().await;
}

/// Closes a read pool, waiting for its connections to be returned and closed.
pub async fn close_reader(pool: SqlitePool) {
    pool.close().await;
}

/// What the database holds, read from its catalogue.
pub async fn inspect(pool: &SqlitePool) -> Result<Schema> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
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
            sqlx::query_scalar("SELECT version_num FROM alembic_version LIMIT 1")
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
