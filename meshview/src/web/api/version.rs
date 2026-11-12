// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

/// Which build is answering.
#[derive(Debug, Serialize, ToSchema)]
pub struct Version {
    /// The version, as in Cargo.toml.
    #[schema(example = "5.0.0")]
    pub version: &'static str,
    /// The date of the commit it was built from, `unknown` when that was not
    /// known at build time. The Python version gave its release date here.
    #[schema(example = "2026-11-11")]
    pub release_date: &'static str,
    /// The commit it was built from, or `unknown`.
    #[schema(example = "75279f5e2a0c4b0b8f1e3c4d5a6b7c8d9e0f1a2b")]
    pub git_revision: &'static str,
    /// Its first seven characters, or `unknown`.
    #[schema(example = "75279f5")]
    pub git_revision_short: &'static str,
}

pub const REVISION: &str = env!("MESHVIEW_GIT_REVISION");

/// The build: version, and the commit it was built from.
pub fn current() -> Version {
    Version {
        version: env!("CARGO_PKG_VERSION"),
        release_date: env!("MESHVIEW_GIT_DATE"),
        git_revision: REVISION,
        git_revision_short: if REVISION == "unknown" {
            REVISION
        } else {
            &REVISION[..REVISION.len().min(7)]
        },
    }
}

/// Version
///
/// The version of Meshview answering, and the commit it was built from.
/// The commit is `unknown` when it was built from outside a Git checkout.
#[utoipa::path(
    get,
    path = "/version",
    tag = "meta",
    responses((status = 200, description = "The version", body = Version))
)]
pub async fn version() -> Json<Version> {
    Json(current())
}
