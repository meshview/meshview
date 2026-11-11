// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! scripts/revert-to-python.py gives a taken over database back to the Python
//! meshview exactly as it was, and refuses what it cannot.
//!
//! These run the script, so they need `python3`. Without it they say so and
//! pass, rather than fail on a machine that only builds.

mod common;

use common::{PYTHON_HEAD, connect, database_from, shape};
use meshview_rs::db::sqlite;
use sqlx::{Connection, Executor};
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scripts/revert-to-python.py"
);

fn python() -> bool {
    let found = Command::new("python3").arg("--version").output().is_ok();
    if !found {
        eprintln!("python3 not found: skipping");
    }
    found
}

fn revert(database: &Path, yes: bool) -> Output {
    let mut command = Command::new("python3");
    command.arg("-I").arg(SCRIPT).arg(database);
    if yes {
        command.arg("--yes");
    }
    command.output().unwrap()
}

/// A Python database, as the Python version leaves it, taken over by
/// meshview-rs and closed.
async fn taken_over(dir: &TempDir) -> (std::path::PathBuf, Vec<String>) {
    let database = database_from(dir, "packets.db", PYTHON_HEAD).await;
    let as_python_left_it = shape(&database).await;
    sqlite::close_writer(sqlite::open_for_writing(&database).await.unwrap()).await;
    (database, as_python_left_it)
}

#[tokio::test]
async fn a_taken_over_database_goes_back_exactly_as_it_was() {
    if !python() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let (database, as_python_left_it) = taken_over(&dir).await;
    let as_rust_has_it = shape(&database).await;
    assert_ne!(as_rust_has_it, as_python_left_it);

    // Without --yes it only checks.
    let checked = revert(&database, false);
    assert!(checked.status.success(), "{checked:?}");
    assert_eq!(shape(&database).await, as_rust_has_it);

    let reverted = revert(&database, true);
    assert!(reverted.status.success(), "{reverted:?}");
    assert_eq!(shape(&database).await, as_python_left_it);

    let mut conn = connect(&database, false).await;
    let revision: String = sqlx::query_scalar("SELECT version_num FROM alembic_version")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(revision, "44612e74a798");
    for count in [
        "SELECT count(*) FROM node",
        "SELECT count(*) FROM packet",
        "SELECT count(*) FROM packet_seen",
        "SELECT count(*) FROM traceroute",
        "SELECT count(*) FROM daily_snapshot",
    ] {
        let rows: i64 = sqlx::query_scalar(count)
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(rows, 1, "{count}");
    }
    conn.close().await.unwrap();

    // And meshview-rs can take it over again.
    sqlite::close_writer(sqlite::open_for_writing(&database).await.unwrap()).await;
}

#[tokio::test]
async fn a_schema_changed_since_python_is_refused_untouched() {
    if !python() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let (database, _) = taken_over(&dir).await;

    let mut conn = connect(&database, false).await;
    conn.execute(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (9999, 'from the future', 1, x'00', 0)",
    )
    .await
    .unwrap();
    conn.close().await.unwrap();
    let before = shape(&database).await;

    let refused = revert(&database, true);

    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("refusing"));
    assert_eq!(shape(&database).await, before);
}

#[tokio::test]
async fn a_python_database_is_refused() {
    if !python() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let database = database_from(&dir, "packets.db", PYTHON_HEAD).await;

    assert!(!revert(&database, true).status.success());
}

/// The script knows which migrations it can undo. A new one makes this fail
/// until somebody decides: teach the script to undo it, or retire the script.
#[test]
fn the_script_knows_every_migration_there_is() {
    let script = std::fs::read_to_string(SCRIPT).unwrap();
    let line = script
        .lines()
        .find(|line| line.starts_with("REVERTIBLE = "))
        .expect("REVERTIBLE in the script");

    let known: Vec<String> = sqlite::MIGRATOR
        .iter()
        .map(|migration| format!("({}, \"{}\")", migration.version, migration.description))
        .collect();

    assert_eq!(
        line,
        format!("REVERTIBLE = [{}]", known.join(", ")),
        "a migration was added or changed: teach scripts/revert-to-python.py to undo it, or \
         retire the script and say so in docs/migrating-from-python.md"
    );
}
