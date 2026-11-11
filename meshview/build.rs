// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use std::process::Command;

fn main() {
    // The migrations are compiled in by `sqlx::migrate!`, which cannot tell Cargo
    // that a new file in the directory is a reason to build again. This does:
    // without it a new migration is silently left out until something else
    // changes.
    println!("cargo:rerun-if-changed=migrations");

    // Which commit this was built from, and that commit's date, for /version.
    // From git when the source is a checkout; from the environment when it is
    // not, as in a container build that copies the source without .git; and
    // "unknown" otherwise, as the Python version answers.
    let revision = env_or_git("MESHVIEW_GIT_REVISION", &["rev-parse", "HEAD"]);
    let date = env_or_git("MESHVIEW_GIT_DATE", &["show", "-s", "--format=%cs", "HEAD"]);
    println!("cargo:rustc-env=MESHVIEW_GIT_REVISION={revision}");
    println!("cargo:rustc-env=MESHVIEW_GIT_DATE={date}");
    println!("cargo:rerun-if-env-changed=MESHVIEW_GIT_REVISION");
    println!("cargo:rerun-if-env-changed=MESHVIEW_GIT_DATE");
    // A new commit moves HEAD, or the branch HEAD points to.
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs");
}

fn env_or_git(variable: &str, args: &[&str]) -> String {
    if let Ok(value) = std::env::var(variable)
        && !value.trim().is_empty()
    {
        return value.trim().to_string();
    }
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}
