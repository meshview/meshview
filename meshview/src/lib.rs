// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Meshview: Meshtastic mesh networks, from MQTT to a database to the web.
//!
//! A library as well as a binary only so that the integration tests under
//! `tests/` can reach the pieces; nothing else is meant to depend on it.

pub mod app;
pub mod config;
pub mod coverage;
pub mod db;
pub mod decode;
pub mod ingest;
pub mod jobs;
pub mod state;
pub mod traceroute;
pub mod web;
