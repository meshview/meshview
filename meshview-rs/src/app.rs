// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What a process does, and putting it together.
//!
//! The Python meshview was two programs sharing a database: one reading MQTT
//! into it and one serving it. Here they are roles of one binary, so that they
//! can run together in one process, which is the simple case, or apart, which is
//! what lets one ingester feed any number of web servers.

use crate::config::{Config, DatabaseUrl};
use crate::db::{self, Compatibility};
use crate::state::{AppState, CHECK_DATABASE, Readiness};
use crate::web;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;
use tracing::{error, info, warn};

/// How often a web server waiting for the database to be ready looks again.
const SCHEMA_POLL: Duration = Duration::from_secs(2);

/// What this process is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Ingests and serves, in one process.
    Run,
    /// Ingests only: the one process that writes and migrates the database.
    Ingest,
    /// Serves only, reading a database something else writes.
    Serve,
    /// Brings the database's schema up to date and exits.
    Migrate,
}

impl Role {
    pub fn ingests(self) -> bool {
        matches!(self, Self::Run | Self::Ingest)
    }

    pub fn serves(self) -> bool {
        matches!(self, Self::Run | Self::Serve)
    }
}

/// Runs a process in this role until it is told to stop.
pub async fn run(role: Role, config: Config) -> Result<()> {
    let DatabaseUrl::Sqlite(database) = config.database()?;

    if role == Role::Migrate {
        let pool = db::sqlite::open_for_writing(&database).await?;
        pool.close().await;
        info!("{} is up to date", database.display());
        return Ok(());
    }

    if config
        .get("server", "tls_cert")
        .is_some_and(|v| !v.trim().is_empty())
    {
        warn!("[server] tls_cert is not supported yet and is ignored: terminate TLS in front");
    }

    let readiness = Arc::new(Readiness::new([CHECK_DATABASE]));
    let state = AppState {
        readiness: readiness.clone(),
        reader: Arc::new(OnceLock::new()),
    };

    let (stop, stopping) = watch::channel(false);
    tokio::spawn(wait_for_signal(stop.clone()));

    // Listening comes first, so that the probes answer while the database is
    // migrated or waited for: a process busy migrating is alive, only not ready.
    let addr = config.listen_addr()?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    info!("listening on http://{addr}");

    let app = web::router(role, state.clone());
    let mut server_stopping = stopping.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = server_stopping.wait_for(|stop| *stop).await;
            })
            .await
    });

    let result = tokio::select! {
        result = prepare(role, database, state) => result,
        _ = wait_until(stopping.clone()) => Ok(()),
    };

    if let Err(e) = result {
        error!("{e:#}");
        let _ = stop.send(true);
        let _ = server.await;
        return Err(e);
    }

    if role.ingests() {
        warn!("ingesting from MQTT is not implemented yet: the database is ready and idle");
    }

    wait_until(stopping).await;
    info!("stopping");
    server.await.context("the web server task")??;
    Ok(())
}

/// Gets the database where this role needs it, and says so through readiness.
async fn prepare(role: Role, database: PathBuf, state: AppState) -> Result<()> {
    // The writer migrates. Doing it before the reader looks is what lets `run`
    // be ready at once instead of waiting on itself.
    let _writer = if role.ingests() {
        Some(db::sqlite::open_for_writing(&database).await?)
    } else {
        None
    };

    if role.serves() {
        let reader = wait_for_schema(&database).await?;
        let _ = state.reader.set(reader);
    }

    state.readiness.set(CHECK_DATABASE, true);
    info!("ready");

    // The writer is kept for the ingester, which does not exist yet, and so is
    // dropped here. Holding it would change nothing but the lock file.
    Ok(())
}

/// Waits until the database is at the schema this build knows, and answers a
/// read pool onto it.
///
/// A web server never migrates, so a database that is not there yet, or is still
/// behind, means the ingester has not got to it and the thing to do is wait. One
/// that is ahead was migrated by a newer build and would be misread, and one that
/// cannot be migrated never will be: both stop the process.
async fn wait_for_schema(database: &std::path::Path) -> Result<sqlx::SqlitePool> {
    let known = db::sqlite::known_versions();
    let mut said = false;

    loop {
        if let Some(pool) = db::sqlite::open_for_reading(database).await? {
            let schema = db::sqlite::inspect(&pool).await?;
            match schema.compatibility(&known) {
                Compatibility::Current => return Ok(pool),
                Compatibility::Behind => {
                    if !said {
                        info!("database is {schema}; waiting for the ingester to migrate it");
                        said = true;
                    }
                }
                Compatibility::Ahead {
                    latest_known,
                    found,
                } => bail!(
                    "{} has been migrated to {found} by a newer meshview-rs, and this one only \
                     knows up to {latest_known}: run the newer version",
                    database.display()
                ),
                Compatibility::Incompatible(why) => bail!("{}: {why}", database.display()),
            }
            pool.close().await;
        } else if !said {
            info!(
                "{} does not exist yet; waiting for the ingester to create it",
                database.display()
            );
            said = true;
        }

        tokio::time::sleep(SCHEMA_POLL).await;
    }
}

async fn wait_until(mut stopping: watch::Receiver<bool>) {
    let _ = stopping.wait_for(|stop| *stop).await;
}

/// Turns SIGINT or SIGTERM into a stop. Either is how a terminal or a container
/// runtime asks, and both mean the same.
async fn wait_for_signal(stop: watch::Sender<bool>) {
    let (Ok(mut interrupt), Ok(mut terminate)) = (
        signal(SignalKind::interrupt()),
        signal(SignalKind::terminate()),
    ) else {
        error!("could not install the signal handlers");
        return;
    };

    tokio::select! {
        _ = interrupt.recv() => {}
        _ = terminate.recv() => {}
    }
    let _ = stop.send(true);
}
