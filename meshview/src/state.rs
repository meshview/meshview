// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What the HTTP handlers share.

use crate::config::Config;
use crate::db::Edge;
use crate::db::Reader;
use crate::db::SightingsOf;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::warn;

/// The things that have to be true before this process is worth sending
/// traffic to, each of which turns true on its own schedule.
///
/// Which ones there are depends on what the process does: a web server waits
/// for the database to be at the schema it knows, an ingester will also wait for
/// its broker. Each is set once it holds, and `/readyz` reports them all.
#[derive(Debug, Default)]
pub struct Readiness {
    checks: BTreeMap<&'static str, AtomicBool>,
}

impl Readiness {
    /// A set of checks, none of them passing yet.
    pub fn new(names: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            checks: names
                .into_iter()
                .map(|name| (name, AtomicBool::new(false)))
                .collect(),
        }
    }

    /// Records whether one check passes.
    ///
    /// A check this process does not count is ignored rather than refused: the
    /// ingester reports on its broker whichever role it runs in, and only in the
    /// one where the broker is all there is does that decide readiness.
    pub fn set(&self, name: &str, passing: bool) {
        if let Some(check) = self.checks.get(name) {
            check.store(passing, Ordering::Release);
        }
    }

    /// Every check and whether it passes.
    pub fn report(&self) -> BTreeMap<String, bool> {
        self.checks
            .iter()
            .map(|(name, passing)| (name.to_string(), passing.load(Ordering::Acquire)))
            .collect()
    }

    /// Whether all of them pass.
    pub fn is_ready(&self) -> bool {
        self.checks
            .values()
            .all(|passing| passing.load(Ordering::Acquire))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_not_counted_is_ignored() {
        let readiness = Readiness::new(["database"]);
        readiness.set("mqtt", true);
        assert!(!readiness.is_ready());
        readiness.set("database", true);
        assert!(readiness.is_ready());
        assert_eq!(readiness.report().len(), 1);
    }
}

/// Handed to every handler.
#[derive(Clone)]
pub struct AppState {
    pub readiness: Arc<Readiness>,
    /// The read pool, there once the database is at a schema this build knows.
    /// Handlers that need the database answer 503 until then.
    pub reader: Arc<OnceLock<Reader>>,
    /// The configuration the process started with, for the answers that report
    /// on it (`/api/v1/config`, `/health`).
    pub config: Arc<Config>,
    /// Answers kept for a while because they are costly to build.
    pub caches: Arc<Caches>,
}

/// A cached answer, and until when it holds.
pub type Cached<T> = (Instant, Arc<T>);

/// What is cached, and until when.
#[derive(Debug, Default)]
pub struct Caches {
    /// `/api/v1/edges`, by the type asked for.
    pub edges: Mutex<HashMap<Option<&'static str>, Cached<Vec<Edge>>>>,
    /// `/api/v1/stats/count` without filters.
    pub totals: Arc<Totals>,
}

/// How many packets and sightings there are in all, which takes reading the
/// whole of both tables: seconds, or on a slow disk with a cold cache, tens of
/// them. Counted once and kept; once old, the count kept is still answered
/// while a new one is made in the background, so that no visitor ever waits
/// for one but the very first.
#[derive(Debug, Default)]
pub struct Totals {
    /// The latest count, packets and sightings, and when it was made.
    pub counted: Mutex<Option<(Instant, (i64, i64))>>,
    /// Held while counting, so that only one count runs at a time.
    pub counting: tokio::sync::Mutex<()>,
    /// Whether a count is on its way in the background.
    pub refreshing: AtomicBool,
}

/// How long a count of everything is answered before a new one is made.
pub const TOTALS_FRESH_FOR: Duration = Duration::from_secs(5 * 60);

impl Totals {
    /// The packets and sightings there are, as last counted. The first time it
    /// counts, and whoever asks meanwhile waits for that count; once the count
    /// kept is older than [`TOTALS_FRESH_FOR`], it is still answered, and a new
    /// one is made in the background for whoever asks next.
    pub async fn get(self: &Arc<Self>, db: &Reader) -> anyhow::Result<(i64, i64)> {
        if let Some((counts, stale)) = self.latest() {
            if stale && !self.refreshing.swap(true, Ordering::AcqRel) {
                let totals = Arc::clone(self);
                let db = db.clone();
                tokio::spawn(async move {
                    if let Err(e) = totals.count(&db).await {
                        warn!("counting the packets and sightings: {e:#}");
                    }
                    totals.refreshing.store(false, Ordering::Release);
                });
            }
            return Ok(counts);
        }
        let _counting = self.counting.lock().await;
        // Whoever counted while this waited has the answer.
        if let Some((counts, _)) = self.latest() {
            return Ok(counts);
        }
        self.count_now(db).await
    }

    /// Counts now, once any count already running is done, and keeps it.
    pub async fn count(&self, db: &Reader) -> anyhow::Result<(i64, i64)> {
        let _counting = self.counting.lock().await;
        self.count_now(db).await
    }

    async fn count_now(&self, db: &Reader) -> anyhow::Result<(i64, i64)> {
        let counts = (
            db.count_packets(None).await?,
            db.count_sightings(SightingsOf::All).await?,
        );
        *self.counted.lock().expect("never poisoned") = Some((Instant::now(), counts));
        Ok(counts)
    }

    /// The count kept, and whether it is old.
    fn latest(&self) -> Option<((i64, i64), bool)> {
        self.counted
            .lock()
            .expect("never poisoned")
            .map(|(at, counts)| (counts, at.elapsed() >= TOTALS_FRESH_FOR))
    }
}

impl AppState {
    /// A process's state, with nothing read yet and nothing cached.
    pub fn new(readiness: Arc<Readiness>, config: Config) -> Self {
        Self {
            readiness,
            reader: Arc::new(OnceLock::new()),
            config: Arc::new(config),
            caches: Arc::default(),
        }
    }
}

/// The readiness check for the database being at the expected schema.
pub const CHECK_DATABASE: &str = "database";

/// The readiness check for being connected to the MQTT broker.
pub const CHECK_MQTT: &str = "mqtt";
