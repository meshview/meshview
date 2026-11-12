// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What a process does, and putting it together.
//!
//! The Python Meshview was two programs sharing a database: one reading MQTT
//! into it and one serving it. Here they are roles of one binary, so that they
//! can run together in one process, which is the simple case, or apart, which is
//! what lets one ingester feed any number of web servers.

use crate::config::{Config, DatabaseUrl};
use crate::db::Pool;
use crate::db::{self, Compatibility};
use crate::ingest::{self, store::Store};
use crate::jobs;
use crate::state::{AppState, CHECK_DATABASE, CHECK_MQTT, Readiness};
use crate::web;
use anyhow::{Context, Result, bail};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

/// How long requests in flight, and messages waiting to be stored, get to finish
/// once the process has been asked to stop. Comfortably under the ten seconds a
/// container runtime waits before it stops asking and kills, because being killed
/// is precisely what leaves the database's side files behind.
pub const DRAIN_GRACE: Duration = Duration::from_secs(5);

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
    let (stop, stopping) = watch::channel(false);
    tokio::spawn(wait_for_signal(stop.clone()));
    run_until(role, config, stop, stopping).await
}

/// Runs a process in this role until `stop` is sent, from a signal or, in a
/// test, from the test.
pub async fn run_until(
    role: Role,
    config: Config,
    stop: watch::Sender<bool>,
    stopping: watch::Receiver<bool>,
) -> Result<()> {
    let database = config.database()?;
    info!("database {database} ({})", config.database_origin());

    if role == Role::Migrate {
        let pool = db::open_for_writing(&database).await?;
        db::close_writer(pool).await;
        info!("{database} is up to date");
        return Ok(());
    }

    // Read before anything starts, so that a mistake in [mqtt] stops the process
    // at once rather than after it has reported itself alive.
    let mqtt = if role.ingests() {
        Some(ingest::Settings::from_config(&config)?)
    } else {
        None
    };
    let jobs = if role.ingests() {
        Some(jobs::Settings::from_config(&config)?)
    } else {
        None
    };

    // The broker counts towards readiness only for a process that does nothing
    // else. One that also serves the web can still serve while the broker is
    // away, and pulling it out of rotation for that would only make things worse.
    let readiness = Arc::new(match role {
        Role::Ingest => Readiness::new([CHECK_DATABASE, CHECK_MQTT]),
        _ => Readiness::new([CHECK_DATABASE]),
    });
    let state = AppState::new(readiness.clone(), config.clone());

    // Listening comes first, so that the probes answer while the database is
    // migrated or waited for: a process busy migrating is alive, only not ready.
    let addr = config.listen_addr()?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    info!("listening on http://{addr}");

    let app = web::router(role, state.clone());
    let state_reader = state.reader.clone();
    let mut server_stopping = stopping.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = server_stopping.wait_for(|stop| *stop).await;
            })
            .await
    });

    let prepared = tokio::select! {
        prepared = prepare(role, database.clone(), state) => prepared,
        _ = wait_until(stopping.clone()) => Ok(None),
    };

    // Once the web can answer, the first requests to the busiest pages should
    // not be the ones that read their part of the database from the disk.
    let warming = (role.serves() && prepared.is_ok() && !*stopping.borrow())
        .then(|| tokio::spawn(warm_cache(database, stopping.clone())));

    // Kept here as well as in the store, so that it can be closed properly
    // however the ingester ends.
    let writer = prepared.as_ref().ok().cloned().flatten();

    // The daily jobs run beside the ingester, on the same writer, which is what
    // keeps them from writing at the same time as it.
    let jobs = match (&writer, jobs) {
        (Some(writer), Some(settings)) => Some(tokio::spawn(jobs::run(
            settings,
            writer.clone(),
            stopping.clone(),
        ))),
        _ => None,
    };

    let result = match (prepared, mqtt) {
        (Ok(Some(writer)), Some(settings)) => match Store::new(writer).await {
            Ok(store) => ingest::run(settings, store, readiness, stopping.clone()).await,
            Err(e) => Err(e),
        },
        (Ok(_), _) => {
            wait_until(stopping.clone()).await;
            Ok(())
        }
        (Err(e), _) => Err(e),
    };

    if let Err(e) = &result {
        error!("{e:#}");
    }
    info!("stopping");
    let _ = stop.send(true);

    // In order: no more requests, then no more readers, then the writer last,
    // so that the connection that tidies the log away is the final one.
    match tokio::time::timeout(DRAIN_GRACE, server).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => warn!("the web server stopped with an error: {e}"),
        Ok(Err(e)) => warn!("the web server task failed: {e}"),
        Err(_) => warn!("still serving requests after {DRAIN_GRACE:?}; stopping anyway"),
    }
    if let Some(warming) = warming {
        let _ = warming.await;
    }
    if let Some(reader) = state_reader.get().cloned() {
        db::close_reader(reader.pool().clone()).await;
    }
    if let Some(jobs) = jobs {
        // A job in the middle of something is abandoned, which rolls it back.
        let _ = jobs.await;
    }
    if let Some(writer) = writer {
        db::close_writer(writer).await;
    }
    info!("stopped");

    result
}

/// Copies the database to `output`, for a scheduler outside Meshview to keep.
pub async fn backup(config: &Config, output: &std::path::Path) -> Result<()> {
    let database = config.database()?;
    let size = db::backup(&database, output).await?;
    info!("{database} copied to {} ({size} bytes)", output.display());
    Ok(())
}

/// How far back the top nodes look by default: a day.
const TOP_WINDOW_US: i64 = 86_400 * 1_000_000;

/// Reads, once and in the background, what the busiest pages read first: the
/// totals of the home page and the statistics, and the window of the top
/// nodes. Right after a start none of it is in memory, and on a slow disk the
/// first visitor waits seconds for each, after which it is instant. Done on a
/// connection of its own, which does not warn about slow statements as the
/// web's do, and given up as soon as the process stops.
async fn warm_cache(database: DatabaseUrl, stopping: watch::Receiver<bool>) {
    let started = std::time::Instant::now();
    let pool = match db::open_quietly_for_reading(&database).await {
        Ok(pool) => pool,
        Err(e) => {
            debug!("not warming the cache: {e:#}");
            return;
        }
    };
    let reader = db::Reader::new(pool.clone());
    let work = async {
        reader.count_packets(None).await?;
        reader.count_sightings(db::SightingsOf::All).await?;
        reader
            .top_nodes(&db::TopQuery {
                window_us: TOP_WINDOW_US,
                channel: None,
                active: true,
                limit: 50,
                offset: 0,
            })
            .await?;
        anyhow::Ok(())
    };
    tokio::select! {
        done = work => match done {
            Ok(()) => info!("database cache warmed in {:.1?}", started.elapsed()),
            Err(e) => debug!("warming the cache: {e:#}"),
        },
        _ = wait_until(stopping) => {}
    }
    db::close_reader(pool).await;
}

/// Gets the database where this role needs it, and says so through readiness.
/// Answers the writer, for a role that ingests.
async fn prepare(role: Role, database: DatabaseUrl, state: AppState) -> Result<Option<Pool>> {
    // The writer migrates. Doing it before the reader looks is what lets `run`
    // be ready at once instead of waiting on itself.
    let writer = if role.ingests() {
        Some(db::open_for_writing(&database).await?)
    } else {
        None
    };

    if role.serves() {
        let reader = wait_for_schema(&database).await?;
        let _ = state.reader.set(db::Reader::new(reader));
    }

    state.readiness.set(CHECK_DATABASE, true);
    info!("database ready");

    Ok(writer)
}

/// Waits until the database is at the schema this build knows, and answers a
/// read pool onto it.
///
/// A web server never migrates, so a database that is not there yet, or is still
/// behind, means the ingester has not got to it and the thing to do is wait. One
/// that is ahead was migrated by a newer build and would be misread, and one that
/// cannot be migrated never will be: both stop the process.
async fn wait_for_schema(database: &DatabaseUrl) -> Result<Pool> {
    let known = db::known_versions(database);
    let mut said = false;

    loop {
        if let Some(pool) = db::open_for_reading(database).await? {
            let schema = db::inspect(&pool).await?;
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
                    "{database} has been migrated to {found} by a newer Meshview, and this one \
                     only knows up to {latest_known}: run the newer version"
                ),
                Compatibility::Incompatible(why) => bail!("{database}: {why}"),
            }
            db::close_reader(pool).await;
        } else if !said {
            info!("{database} does not exist yet; waiting for the ingester to create it");
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
