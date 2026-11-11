// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What the HTTP handlers share.

use crate::db::Reader;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

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
}

/// The readiness check for the database being at the expected schema.
pub const CHECK_DATABASE: &str = "database";

/// The readiness check for being connected to the MQTT broker.
pub const CHECK_MQTT: &str = "mqtt";
