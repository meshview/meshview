// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

use prost_types::DescriptorProto;
use std::fmt::Write as _;
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

    // Which field numbers each message has, for telling the fields these files
    // know from the ones a newer firmware added. See `fields` in lib.rs.
    let mut known = String::from(
        "/// The field numbers each message declares, by its full protobuf name.\n\
         pub fn known_fields(message: &str) -> Option<&'static [u32]> {\n    match message {\n",
    );
    for file in &descriptors.file {
        let package = file.package();
        for message in &file.message_type {
            list_fields(&mut known, package, message);
        }
    }
    known.push_str("        _ => None,\n    }\n}\n");
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    std::fs::write(out.join("known_fields.rs"), known)?;

    prost_build::Config::new()
        // Meshtastic names its packages `meshtastic`, so everything lands in one
        // module. Enums stay as i32 on the wire types, which is what lets a value
        // a newer firmware sends, and these files do not know yet, arrive intact.
        .compile_fds(descriptors)?;

    println!("cargo:rerun-if-changed={PROTO_ROOT}");
    Ok(())
}

/// Adds a message, and the messages declared inside it, to the match arms.
fn list_fields(out: &mut String, scope: &str, message: &DescriptorProto) {
    let name = format!("{scope}.{}", message.name());
    let mut numbers: Vec<i32> = message.field.iter().map(|field| field.number()).collect();
    numbers.sort_unstable();
    let numbers: Vec<String> = numbers.iter().map(i32::to_string).collect();
    let _ = writeln!(out, "        {name:?} => Some(&[{}]),", numbers.join(", "));

    for nested in &message.nested_type {
        list_fields(out, &name, nested);
    }
}
