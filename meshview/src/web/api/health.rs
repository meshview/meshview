// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! `/health`: the Python version's report on the installation, for people and
//! monitoring, beside the probes `/livez` and `/readyz` meant for orchestrators.
//!
//! Its shape is the Python version's. What it reports about the daily jobs
//! comes from `job_run` instead of the JSON files the Python version wrote, the
//! backups Meshview does not make are reported as off, and the snapshot is
//! reported too.

use super::now_us;
use super::version::REVISION;
use crate::config::{Config, DatabaseUrl};
use crate::db::JobRun;
use crate::jobs;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct Health {
    /// `healthy`, or `unhealthy` when the database cannot be read.
    #[schema(example = "healthy")]
    pub status: &'static str,
    /// Now, in UTC.
    #[schema(example = "2026-11-11T10:20:30.123456+00:00")]
    pub timestamp: String,
    /// Meshview's version.
    #[schema(example = "0.1.0")]
    pub version: &'static str,
    /// The commit this was built from, shortened.
    #[schema(example = "c43c989")]
    pub git_revision: &'static str,
    /// The daily cleanup: `enabled`, `days_to_keep`, `vacuum`, its
    /// `scheduled_time` (local, `HH:MM`) and `status`: `disabled`, `unknown`
    /// before its first run, or how the last run went (`ok`, `error`,
    /// `running`, `interrupted`), which is then in `last_run`, with when it
    /// started and finished, the cutoff and the rows deleted from each table.
    #[schema(value_type = Object)]
    pub cleanup: Value,
    /// Backups, which Meshview does not make: always `disabled`. Use
    /// `meshview backup`, from outside.
    #[schema(value_type = Object)]
    pub backup: Value,
    /// The daily snapshot, which the Python version did not report: its
    /// `scheduled_time`, `status` and `last_run`, as the cleanup's, with the
    /// day and the figures recorded.
    #[schema(value_type = Object)]
    pub snapshot: Value,
    /// `connected` or `disconnected`.
    #[schema(example = "connected")]
    pub database: &'static str,
    /// How long ago the latest packet was stored, to a tenth of a second;
    /// not there before the first one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 4.2)]
    pub seconds_since_last_message: Option<f64>,
    /// The database file's size, readable, as `412.37 MB`, when it can be
    /// known.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "245.66 MB")]
    pub database_size: Option<String>,
    /// The same size, in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 257589248)]
    pub database_size_bytes: Option<u64>,
}

/// A time as Python's `datetime.isoformat()` writes one in UTC: microseconds
/// only when there are any, and `+00:00` for the zone.
fn iso_utc(us: i64) -> String {
    let Ok(at) = Timestamp::from_microsecond(us) else {
        return us.to_string();
    };
    let zoned = at.to_zoned(TimeZone::UTC);
    let micros = us.rem_euclid(1_000_000);
    let fraction = if micros == 0 {
        String::new()
    } else {
        format!(".{micros:06}")
    };
    format!("{}{fraction}+00:00", zoned.strftime("%Y-%m-%dT%H:%M:%S"))
}

/// A run as the Python version reported its last cleanup: what the job
/// recorded, with when it started and ended.
fn last_run(run: &JobRun) -> Value {
    let mut out = json!({
        "status": run.status,
        "started_at": iso_utc(run.started_us),
        "completed_at": run.finished_us.map(iso_utc),
    });
    if let Some(Value::Object(details)) = run
        .details
        .as_deref()
        .and_then(|details| serde_json::from_str(details).ok())
    {
        for (key, value) in details {
            // Kept in microseconds, reported as the Python version wrote it.
            if key == "cutoff_us" {
                if let Some(us) = value.as_i64() {
                    out["cutoff_at"] = json!(iso_utc(us));
                }
            } else if key != "cutoff_at" {
                out[key] = value;
            }
        }
    }
    out["error"] = json!(run.error);
    out
}

fn job(scheduled: (u8, u8), enabled: bool, run: Option<&JobRun>) -> Value {
    let mut out = json!({
        "scheduled_time": format!("{:02}:{:02}", scheduled.0, scheduled.1),
        "status": if enabled { "unknown" } else { "disabled" },
    });
    if let Some(run) = run {
        out["status"] = json!(run.status);
        out["last_run"] = last_run(run);
    }
    out
}

fn readable_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let size = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if size < KB * KB {
        format!("{:.2} KB", size / KB)
    } else if size < KB * KB * KB {
        format!("{:.2} MB", size / (KB * KB))
    } else {
        format!("{:.2} GB", size / (KB * KB * KB))
    }
}

fn database_size(config: &Config) -> Option<u64> {
    match config.database().ok()? {
        DatabaseUrl::Sqlite(path) => std::fs::metadata(path).ok().map(|metadata| metadata.len()),
        DatabaseUrl::Postgres(_) => None,
    }
}

/// Health
///
/// The installation at a glance: whether the database can be read and how long
/// ago the latest packet arrived, its size, and how the daily jobs last went.
/// 503 when the database cannot be read.
///
/// For people and monitoring, as in the Python version. A daily job gone
/// wrong shows in its `status` but does not make the answer a failure. For
/// orchestrators, `/livez` and `/readyz` are simpler.
#[utoipa::path(
    get,
    path = "/health",
    tag = "meta",
    responses(
        (status = 200, description = "Healthy", body = Health, example = json!({
            "status": "healthy",
            "timestamp": "2026-11-11T16:01:22.553357+00:00",
            "version": "0.1.0",
            "git_revision": "c43c989",
            "cleanup": {
                "days_to_keep": 14,
                "enabled": true,
                "scheduled_time": "02:00",
                "status": "ok",
                "vacuum": false,
                "last_run": {
                    "status": "ok",
                    "started_at": "2026-11-11T01:00:00.012345+00:00",
                    "completed_at": "2026-11-11T01:00:03.456789+00:00",
                    "cutoff_at": "2026-10-28T01:00:00.012345+00:00",
                    "days_to_keep": 14,
                    "vacuum_requested": false,
                    "vacuum_completed": false,
                    "rows_deleted": {"packet": 1200, "packet_seen": 3400, "traceroute": 42, "node": 3},
                    "error": null,
                },
            },
            "backup": {"enabled": false, "status": "disabled"},
            "snapshot": {"scheduled_time": "01:00", "status": "unknown"},
            "database": "connected",
            "seconds_since_last_message": 4.2,
            "database_size": "245.66 MB",
            "database_size_bytes": 257589248,
        })),
        (status = 503, description = "The database cannot be read", body = Health),
    )
)]
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<Health>) {
    let config = &state.config;
    let db = state.reader.get();
    let latest = |name: &'static str| async move {
        match db {
            Some(db) => db.latest_job_run(name).await.ok().flatten(),
            None => None,
        }
    };
    let cleanup_run = latest(jobs::CLEANUP).await;
    let snapshot_run = latest(jobs::SNAPSHOT).await;

    let mut cleanup = json!({
        "enabled": config.cleanup.enabled,
        "days_to_keep": config.cleanup.days_to_keep,
    });
    let schedule = job(
        (config.cleanup.hour, config.cleanup.minute),
        config.cleanup.enabled,
        cleanup_run.as_ref(),
    );
    cleanup["scheduled_time"] = schedule["scheduled_time"].clone();
    cleanup["vacuum"] = json!(config.cleanup.vacuum);
    for key in ["status", "last_run"] {
        if let Some(value) = schedule.get(key) {
            cleanup[key] = value.clone();
        }
    }

    let mut health = Health {
        status: "healthy",
        timestamp: iso_utc(now_us()),
        version: env!("CARGO_PKG_VERSION"),
        git_revision: if REVISION == "unknown" {
            REVISION
        } else {
            &REVISION[..REVISION.len().min(7)]
        },
        cleanup,
        // Meshview does not back up by itself: see `meshview backup`.
        backup: json!({"enabled": false, "status": "disabled"}),
        snapshot: job(
            (config.snapshot.hour, config.snapshot.minute),
            true,
            snapshot_run.as_ref(),
        ),
        database: "connected",
        seconds_since_last_message: None,
        database_size: None,
        database_size_bytes: None,
    };

    let last = match db {
        Some(db) => db.last_import_us().await,
        None => Err(anyhow::anyhow!("the database is not ready")),
    };
    match last {
        Ok(last) => {
            health.seconds_since_last_message = last.map(|last| {
                let seconds = (now_us() - last) as f64 / 1_000_000.0;
                (seconds * 10.0).round_ties_even() / 10.0
            });
        }
        Err(_) => {
            health.database = "disconnected";
            health.status = "unhealthy";
            return (StatusCode::SERVICE_UNAVAILABLE, Json(health));
        }
    }

    if let Some(bytes) = database_size(config) {
        health.database_size = Some(readable_size(bytes));
        health.database_size_bytes = Some(bytes);
    }

    (StatusCode::OK, Json(health))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_written_as_python_writes_them() {
        assert_eq!(iso_utc(1_794_355_200_000_000), "2026-11-11T00:00:00+00:00");
        assert_eq!(
            iso_utc(1_794_355_200_123_456),
            "2026-11-11T00:00:00.123456+00:00"
        );
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(readable_size(512), "512 B");
        assert_eq!(readable_size(2048), "2.00 KB");
        assert_eq!(readable_size(432_400_384), "412.37 MB");
        assert_eq!(readable_size(5 * 1024 * 1024 * 1024), "5.00 GB");
    }
}
