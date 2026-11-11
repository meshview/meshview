// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use meshview_rs::app::{self, Role};
use meshview_rs::config::Config;
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// The configuration file read when none is named. Optional: without it the
/// defaults and the environment are the whole configuration.
const DEFAULT_CONFIG: &str = "config.toml";

/// The Python version's configuration file, which this does not read.
const PYTHON_CONFIG: &str = "config.ini";

/// Real-time monitoring and diagnostics for Meshtastic mesh networks.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// The configuration file. Named here or in MESHVIEW_CONFIG it has to exist;
    /// otherwise ./config.toml is read if it is there.
    #[arg(long, short, env = "MESHVIEW_CONFIG", global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Ingest from MQTT and serve the web, in one process (the default).
    Run,
    /// Only ingest from MQTT into the database. The one process that writes.
    Ingest,
    /// Only serve the web, reading a database an ingester writes.
    Serve,
    /// Bring the database's schema up to date, and exit.
    Migrate,
    /// Copy the database to a file, consistently, even while it is in use. For a
    /// scheduler to run; the file must not exist yet.
    Backup {
        /// Where the copy goes.
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Run);

    info!(
        "{} {} — free software under the AGPL, version 3 or later",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    let config = match &cli.config {
        Some(path) => Config::load(path, true)?,
        None => {
            // A config.ini left from the Python version and no config.toml is
            // somebody who has not converted it yet. Starting on the defaults
            // instead would quietly create a new, empty database somewhere else.
            if Path::new(PYTHON_CONFIG).exists() && !Path::new(DEFAULT_CONFIG).exists() {
                bail!(
                    "found {PYTHON_CONFIG} but no {DEFAULT_CONFIG}: meshview-rs reads TOML, with \
                     the same sections and keys. Convert it (see sample.config.toml) or name a \
                     file with --config."
                );
            }
            Config::load(DEFAULT_CONFIG.as_ref(), false)?
        }
    };
    match config.source() {
        Some(path) => info!("configuration read from {}", path.display()),
        None => info!("no configuration file; using the defaults and the environment"),
    }
    for warning in config.deprecations() {
        warn!("{warning}");
    }

    match command {
        Command::Run => app::run(Role::Run, config).await,
        Command::Ingest => app::run(Role::Ingest, config).await,
        Command::Serve => app::run(Role::Serve, config).await,
        Command::Migrate => app::run(Role::Migrate, config).await,
        Command::Backup { output } => app::backup(&config, &output).await,
    }
}
