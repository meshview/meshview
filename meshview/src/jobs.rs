// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The daily jobs the writer runs: pruning old data, and recording each day's
//! figures before that pruning takes them away.
//!
//! Both are the Python version's, done the same way, at the same local time of
//! day. What each run did is a row in `job_run`, where any process reading the
//! database can find it.
//!
//! They share the writer's single connection with the ingester, so they never
//! write at the same time: while a cleanup deletes, the messages arriving wait in
//! the ingester's queue, as the Python version paused its ingestion.

use crate::config::Config;
use anyhow::{Context, Result, bail};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, info};

/// The jobs, by the name their runs are recorded under.
pub const CLEANUP: &str = "cleanup";
pub const SNAPSHOT: &str = "snapshot";

/// How many runs of each job are kept: a quarter of a year of daily ones.
const HISTORY: i64 = 90;

const MICROS_PER_DAY: i64 = 86_400 * 1_000_000;

/// A time of day, in the local time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOfDay {
    pub hour: i8,
    pub minute: i8,
}

impl TimeOfDay {
    fn new(section: &str, hour: u8, minute: u8) -> Result<Self> {
        if hour > 23 || minute > 59 {
            bail!("[{section}] hour and minute have to be a time of day: {hour}:{minute:02}");
        }
        Ok(Self {
            hour: hour as i8,
            minute: minute as i8,
        })
    }

    /// The next time this time of day comes round, after `now`.
    ///
    /// In `now`'s time zone, with its daylight saving time: on the day a clock
    /// skips the hour, a time inside the gap is taken as the one after it, and on
    /// the day it repeats one, as the first of the two.
    pub fn next_after(&self, now: &Zoned) -> Result<Zoned> {
        let at = |date: Date| -> Result<Zoned> {
            Ok(date
                .at(self.hour, self.minute, 0, 0)
                .to_zoned(now.time_zone().clone())?)
        };
        let today = at(now.date())?;
        if today > *now {
            Ok(today)
        } else {
            at(now.date().tomorrow()?)
        }
    }
}

/// What the jobs are told to do.
#[derive(Debug, Clone)]
pub struct Settings {
    /// When cleanup is on, how many days of data to keep, and at what time.
    pub cleanup: Option<Cleanup>,
    pub snapshot_at: TimeOfDay,
}

#[derive(Debug, Clone)]
pub struct Cleanup {
    pub days_to_keep: u32,
    pub at: TimeOfDay,
    pub vacuum: bool,
}

impl Settings {
    pub fn from_config(config: &Config) -> Result<Self> {
        let cleanup = &config.cleanup;
        let at = TimeOfDay::new("cleanup", cleanup.hour, cleanup.minute)?;
        let snapshot_at = TimeOfDay::new("snapshot", config.snapshot.hour, config.snapshot.minute)?;
        Ok(Self {
            cleanup: cleanup.enabled.then_some(Cleanup {
                days_to_keep: cleanup.days_to_keep,
                at,
                vacuum: cleanup.vacuum,
            }),
            snapshot_at,
        })
    }
}

/// Runs the jobs at their times until told to stop.
///
/// A job interrupted by the stop is abandoned where it was, which rolls its
/// transaction back: its run stays recorded as running, and the next start marks
/// it interrupted.
pub async fn run(
    settings: Settings,
    pool: impl Into<crate::db::Pool>,
    mut stopping: watch::Receiver<bool>,
) {
    // Only until PostgreSQL has its own; without it there is only SQLite.
    #[allow(clippy::infallible_destructuring_match)]
    let pool = match pool.into() {
        crate::db::Pool::Sqlite(pool) => pool,
        #[cfg(feature = "postgres")]
        crate::db::Pool::Postgres(_) => {
            tracing::warn!("the daily jobs on PostgreSQL are not written yet");
            return;
        }
    };
    if let Err(e) = mark_interrupted(&pool).await {
        error!("could not tidy up the job runs of a previous start: {e:#}");
    }

    match &settings.cleanup {
        Some(cleanup) => info!(
            "Daily cleanup enabled: keeping {} days of data at {:02}:{:02}",
            cleanup.days_to_keep, cleanup.at.hour, cleanup.at.minute
        ),
        None => info!("Daily cleanup is disabled by configuration."),
    }
    info!(
        "Daily snapshots enabled: capturing at {:02}:{:02}",
        settings.snapshot_at.hour, settings.snapshot_at.minute
    );

    let cleanups = async {
        let Some(cleanup) = &settings.cleanup else {
            return std::future::pending().await;
        };
        loop {
            wait_until(CLEANUP, cleanup.at).await;
            match self::cleanup(&pool, cleanup.days_to_keep, cleanup.vacuum).await {
                Ok(details) => info!("Cleanup completed successfully: {details}"),
                Err(e) => error!("Error during cleanup: {e:#}"),
            }
        }
    };

    let snapshots = async {
        loop {
            wait_until(SNAPSHOT, settings.snapshot_at).await;
            match snapshot(&pool).await {
                Ok(details) => info!("Daily snapshot captured successfully: {details}"),
                Err(e) => error!("Error capturing daily snapshot: {e:#}"),
            }
        }
    };

    tokio::select! {
        () = cleanups => {}
        () = snapshots => {}
        _ = stopping.wait_for(|stop| *stop) => {}
    }
}

/// Sleeps until the next time `at` comes round, in the local time zone.
async fn wait_until(job: &str, at: TimeOfDay) {
    let now = Zoned::now();
    let next = match at.next_after(&now) {
        Ok(next) => next,
        Err(e) => {
            // Only a date beyond what jiff represents could do this.
            error!("could not schedule the {job}: {e}");
            return std::future::pending().await;
        }
    };
    info!("Next {job} scheduled at {next}");

    let delay = now
        .duration_until(&next)
        .try_into()
        .unwrap_or(Duration::ZERO);
    tokio::time::sleep(delay).await;
}

/// Deletes what is older than `days_to_keep`, as the Python version does: the
/// packets, sightings and traceroutes imported before the cutoff, and the nodes
/// not seen since. All four in one transaction, then optionally a VACUUM.
pub async fn cleanup(pool: &SqlitePool, days_to_keep: u32, vacuum: bool) -> Result<Value> {
    let started = now_us();
    let cutoff_us = started - i64::from(days_to_keep) * MICROS_PER_DAY;
    let mut details = json!({
        "cutoff_us": cutoff_us,
        "cutoff_at": format_us(cutoff_us),
        "days_to_keep": days_to_keep,
        "vacuum_requested": vacuum,
        "vacuum_completed": false,
        "rows_deleted": {"packet": null, "packet_seen": null, "traceroute": null, "node": null},
    });
    let run = start(pool, CLEANUP, started, &details).await?;
    info!(
        "Running cleanup for records older than {}...",
        format_us(cutoff_us)
    );

    let result = async {
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        for (table, sql) in [
            ("packet", "DELETE FROM packet WHERE import_time_us < ?"),
            (
                "packet_seen",
                "DELETE FROM packet_seen WHERE import_time_us < ?",
            ),
            (
                "traceroute",
                "DELETE FROM traceroute WHERE import_time_us < ?",
            ),
            ("node", "DELETE FROM node WHERE last_seen_us < ?"),
        ] {
            let deleted = sqlx::query(sql)
                .bind(cutoff_us)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("deleting from {table}"))?
                .rows_affected();
            details["rows_deleted"][table] = json!(deleted);
            info!("Deleted {deleted} rows from {table}");
        }
        tx.commit().await?;

        if vacuum {
            info!("Running VACUUM...");
            sqlx::query("VACUUM")
                .execute(pool)
                .await
                .context("VACUUM")?;
            details["vacuum_completed"] = json!(true);
            info!("VACUUM completed.");
        }
        anyhow::Ok(())
    }
    .await;

    finish(pool, run, &details, result.as_ref().err()).await?;
    result.map(|()| details)
}

/// Records the previous complete UTC day's figures in `daily_snapshot`, as the
/// Python version does, replacing what an earlier run recorded for that day.
///
/// Active nodes are those that sent a packet that day, or whose older packet was
/// heard again that day; gateways are those that relayed anything that day.
pub async fn snapshot(pool: &SqlitePool) -> Result<Value> {
    let started = now_us();
    let today = Timestamp::now().to_zoned(TimeZone::UTC).date();
    let day = today.yesterday()?;
    let start_us = midnight_utc_us(day)?;
    let end_us = midnight_utc_us(today)?;
    let mut details = json!({"snapshot_date": day.to_string()});
    let run = start(pool, SNAPSHOT, started, &details).await?;

    let result = async {
        let node_count: i64 = sqlx::query_scalar(
            "SELECT count(from_node_id) FROM ( \
               SELECT from_node_id FROM packet \
                WHERE import_time_us >= ?1 AND import_time_us < ?2 \
               UNION \
               SELECT packet.from_node_id FROM packet \
                 JOIN packet_seen ON packet_seen.packet_id = packet.id \
                WHERE packet_seen.import_time_us >= ?1 AND packet_seen.import_time_us < ?2 \
             )",
        )
        .bind(start_us)
        .bind(end_us)
        .fetch_one(pool)
        .await
        .context("counting the active nodes")?;

        let packet_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM packet WHERE import_time_us >= ? AND import_time_us < ?",
        )
        .bind(start_us)
        .bind(end_us)
        .fetch_one(pool)
        .await
        .context("counting the packets")?;

        let gateway_count: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT node_id) FROM packet_seen \
             WHERE import_time_us >= ? AND import_time_us < ?",
        )
        .bind(start_us)
        .bind(end_us)
        .fetch_one(pool)
        .await
        .context("counting the gateways")?;

        sqlx::query(
            "INSERT INTO daily_snapshot \
             (snapshot_date, node_count, packet_count, gateway_count, captured_at_us) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (snapshot_date) DO UPDATE SET \
               node_count = ?2, packet_count = ?3, gateway_count = ?4, captured_at_us = ?5",
        )
        .bind(day.to_string())
        .bind(node_count)
        .bind(packet_count)
        .bind(gateway_count)
        .bind(now_us())
        .execute(pool)
        .await
        .context("storing the snapshot")?;

        details["node_count"] = json!(node_count);
        details["packet_count"] = json!(packet_count);
        details["gateway_count"] = json!(gateway_count);
        anyhow::Ok(())
    }
    .await;

    finish(pool, run, &details, result.as_ref().err()).await?;
    result.map(|()| details)
}

/// Records that a run of `job` started, and answers its id.
async fn start(pool: &SqlitePool, job: &str, started_us: i64, details: &Value) -> Result<i64> {
    let id = sqlx::query_scalar(
        "INSERT INTO job_run (job, status, started_us, details) VALUES (?, 'running', ?, ?) \
         RETURNING id",
    )
    .bind(job)
    .bind(started_us)
    .bind(details.to_string())
    .fetch_one(pool)
    .await
    .context("recording the job's start")?;
    Ok(id)
}

/// Records how a run ended, and forgets the oldest runs of its job.
async fn finish(
    pool: &SqlitePool,
    run: i64,
    details: &Value,
    failure: Option<&anyhow::Error>,
) -> Result<()> {
    sqlx::query(
        "UPDATE job_run SET status = ?, finished_us = ?, details = ?, error = ? WHERE id = ?",
    )
    .bind(if failure.is_some() { "error" } else { "ok" })
    .bind(now_us())
    .bind(details.to_string())
    .bind(failure.map(|e| format!("{e:#}")))
    .bind(run)
    .execute(pool)
    .await
    .context("recording the job's end")?;

    sqlx::query(
        "DELETE FROM job_run WHERE job = (SELECT job FROM job_run WHERE id = ?1) \
         AND id NOT IN (SELECT id FROM job_run WHERE job = (SELECT job FROM job_run WHERE id = ?1) \
                        ORDER BY started_us DESC LIMIT ?2)",
    )
    .bind(run)
    .bind(HISTORY)
    .execute(pool)
    .await
    .context("forgetting old job runs")?;
    Ok(())
}

/// Runs left as running by a process that was stopped or died in the middle of
/// them. Only the writer runs jobs, and there is one, so any such run is over.
async fn mark_interrupted(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "UPDATE job_run SET status = 'interrupted', finished_us = ? WHERE status = 'running'",
    )
    .bind(now_us())
    .execute(pool)
    .await?;
    Ok(())
}

fn midnight_utc_us(date: Date) -> Result<i64> {
    let at = date.at(0, 0, 0, 0).to_zoned(TimeZone::UTC)?;
    Ok(at.timestamp().as_microsecond())
}

fn format_us(us: i64) -> String {
    Timestamp::from_microsecond(us)
        .map(|t| t.to_string())
        .unwrap_or_else(|_| us.to_string())
}

fn now_us() -> i64 {
    Timestamp::now().as_microsecond()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn madrid(text: &str) -> Zoned {
        format!("{text}[Europe/Madrid]").parse().unwrap()
    }

    #[test]
    fn the_next_run_is_today_until_its_time_has_passed() {
        let at = TimeOfDay { hour: 2, minute: 0 };

        let before = madrid("2026-11-11T01:30:00");
        assert_eq!(
            at.next_after(&before).unwrap(),
            madrid("2026-11-11T02:00:00")
        );

        let after = madrid("2026-11-11T02:00:00");
        assert_eq!(
            at.next_after(&after).unwrap(),
            madrid("2026-11-12T02:00:00")
        );
    }

    #[test]
    fn daylight_saving_time_moves_with_the_clock() {
        // Spain skips from 02:00 to 03:00 on the last Sunday of March: a job at
        // 02:30 runs at 03:30 that day, and at 02:30 again the next.
        let at = TimeOfDay {
            hour: 2,
            minute: 30,
        };
        let night_before = madrid("2026-03-29T00:00:00");
        let next = at.next_after(&night_before).unwrap();
        assert_eq!(next.date(), night_before.date());
        assert_eq!(next.hour(), 3);

        let after = at.next_after(&next).unwrap();
        assert_eq!(after, madrid("2026-03-30T02:30:00"));
    }

    #[test]
    fn times_of_day_are_checked() {
        assert!(TimeOfDay::new("cleanup", 24, 0).is_err());
        assert!(TimeOfDay::new("cleanup", 23, 60).is_err());
        assert!(TimeOfDay::new("cleanup", 23, 59).is_ok());
    }
}
