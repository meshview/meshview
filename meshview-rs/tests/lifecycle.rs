// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! A whole process, from start to stop: it comes up with no broker to talk to,
//! stops when asked, and leaves only the database behind.

use meshview_rs::app::{self, Role};
use meshview_rs::config::Config;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::watch;

fn side_files(database: &Path) -> Vec<String> {
    ["-wal", "-shm", "-journal"]
        .iter()
        .map(|suffix| format!("{}{suffix}", database.display()))
        .filter(|file| Path::new(file).exists())
        .collect()
}

/// A configuration on a free port, a database in `dir`, and a broker that
/// refuses every connection.
fn config(dir: &TempDir) -> (Config, PathBuf) {
    let database = dir.path().join("packets.db");
    let config = Config::parse(&format!(
        "[server]\nbind = \"127.0.0.1\"\nport = 0\n\
         [mqtt]\nserver = \"127.0.0.1\"\nport = 9\ntopics = [\"msh/#\"]\n\
         [database]\nconnection_string = \"sqlite:///{}\"\n",
        database.display()
    ))
    .unwrap();
    (config, database)
}

async fn start_and_stop(role: Role) {
    let dir = TempDir::new().unwrap();
    let (config, database) = config(&dir);
    let (stop, stopping) = watch::channel(false);

    let process = tokio::spawn(app::run_until(role, config, stop.clone(), stopping));

    // Up far enough to have created the database and tried the broker.
    for _ in 0..50 {
        if database.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(database.exists(), "{role:?} never created the database");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!process.is_finished(), "{role:?} stopped on its own");

    stop.send(true).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), process)
        .await
        .expect("did not stop in time")
        .unwrap();

    assert!(result.is_ok(), "{role:?} ended with {result:?}");
    assert_eq!(side_files(&database), Vec::<String>::new());
}

#[tokio::test]
async fn run_survives_a_missing_broker_and_stops_cleanly() {
    start_and_stop(Role::Run).await;
}

#[tokio::test]
async fn ingest_survives_a_missing_broker_and_stops_cleanly() {
    start_and_stop(Role::Ingest).await;
}
