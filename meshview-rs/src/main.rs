// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use meshview_rs::app::{self, Role};
use meshview_rs::config::Config;
use std::path::{Path, PathBuf};
use tracing::info;
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

#[derive(Debug, Clone, Copy, Subcommand)]
enum Command {
    /// Ingest from MQTT and serve the web, in one process (the default).
    Run,
    /// Only ingest from MQTT into the database. The one process that writes.
    Ingest,
    /// Only serve the web, reading a database an ingester writes.
    Serve,
    /// Bring the database's schema up to date, and exit.
    Migrate,
}

impl From<Command> for Role {
    fn from(command: Command) -> Self {
        match command {
            Command::Run => Self::Run,
            Command::Ingest => Self::Ingest,
            Command::Serve => Self::Serve,
            Command::Migrate => Self::Migrate,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    let role = Role::from(cli.command.unwrap_or(Command::Run));

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

    app::run(role, config).await
}
