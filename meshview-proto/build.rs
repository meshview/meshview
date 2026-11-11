// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use prost::Message;
use std::path::{Path, PathBuf};

/// Where the vendored Meshtastic protobufs live, relative to this crate.
const PROTO_ROOT: &str = "protobufs";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(PROTO_ROOT);
    let meshtastic = root.join("meshtastic");

    // Every file in the directory rather than a list kept by hand: updating the
    // protobufs is then copying the new ones in, with nothing here to remember.
    let mut files: Vec<_> = std::fs::read_dir(&meshtastic)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "proto"))
        .collect();
    files.sort();

    let descriptors = protox::compile(&files, [root])?;

    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    // The descriptors themselves, for reading and printing messages by their
    // description rather than their generated type: see `descriptors()`.
    std::fs::write(out.join("descriptors.bin"), descriptors.encode_to_vec())?;

    prost_build::Config::new()
        // Meshtastic names its packages `meshtastic`, so everything lands in one
        // module. Enums stay as i32 on the wire types, which is what lets a value
        // a newer firmware sends, and these files do not know yet, arrive intact.
        .compile_fds(descriptors)?;

    println!("cargo:rerun-if-changed={PROTO_ROOT}");
    Ok(())
}
