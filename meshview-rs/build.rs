// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

// The migrations are compiled in by `sqlx::migrate!`, which cannot tell Cargo
// that a new file in the directory is a reason to build again. This does: without
// it a new migration is silently left out until something else changes.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
