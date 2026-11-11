// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The API, version 1: the Python meshview 4.0's, address for address and
//! answer for answer, apart from the differences listed in docs/api.md.

pub mod error;
pub mod params;

mod channels;
mod edges;
mod health;
mod lang;
mod nodes;
mod packets;
mod packets_seen;
mod site_config;
mod snapshots;
mod stats;
mod traceroute;
pub mod version;

use crate::db::Reader;
use crate::state::AppState;
use error::ApiError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// The routes under `/api/v1`. Paths are relative to it.
pub fn v1() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(channels::channels))
        .routes(routes!(snapshots::daily))
        .routes(routes!(packets_seen::packets_seen))
        .routes(routes!(site_config::config))
        .routes(routes!(lang::lang))
        .routes(routes!(stats::stats))
        .routes(routes!(stats::count))
        .routes(routes!(stats::top))
        .routes(routes!(nodes::nodes))
        .routes(routes!(nodes::facets))
        .routes(routes!(packets::packets))
        .routes(routes!(traceroute::traceroute))
        .routes(routes!(edges::edges))
}

/// The routes outside any version, at their own full paths.
pub fn unversioned() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(version::version))
        .routes(routes!(health::health))
}

impl AppState {
    /// The database to read from, or 503 while it is not ready.
    pub fn db(&self) -> Result<&Reader, ApiError> {
        self.reader.get().ok_or_else(ApiError::not_ready)
    }
}

/// Microseconds since the epoch, as every time in the database is.
pub(crate) fn now_us() -> i64 {
    jiff::Timestamp::now().as_microsecond()
}
