// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! scripts/convert-config.py turns a Python meshview config.ini into a
//! config.toml meshview-rs reads as the Python version read the INI.
//!
//! These run the script, so they need `python3`. Without it they say so and
//! pass, as tests/revert.rs does.

use meshview_rs::config::Config;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../scripts/convert-config.py");

/// The Python meshview's sample.config.ini, as it was when this was written.
const PYTHON_SAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/python-sample.config.ini"
);

fn python() -> bool {
    let found = Command::new("python3").arg("--version").output().is_ok();
    if !found {
        eprintln!("python3 not found: skipping");
    }
    found
}

fn convert(ini: &Path, extra: &[&str]) -> Output {
    Command::new("python3")
        .arg("-I")
        .arg(SCRIPT)
        .arg(ini)
        .args(extra)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_python_sample_becomes_this_projects_sample() {
    if !python() {
        return;
    }
    let output = convert(Path::new(PYTHON_SAMPLE), &[]);
    assert!(output.status.success(), "{}", stderr(&output));

    let converted = Config::parse(std::str::from_utf8(&output.stdout).unwrap())
        .unwrap_or_else(|e| panic!("{e:#}\n{}", String::from_utf8_lossy(&output.stdout)));
    let sample = Config::parse(include_str!("../../sample.config.toml")).unwrap();
    assert_eq!(converted, sample);

    // What it leaves out, it says.
    let notes = stderr(&output);
    for left_out in [
        "tls_cert",
        "backup_enabled",
        "db_cleanup_logfile",
        "longfast",
    ] {
        assert!(notes.contains(left_out), "{left_out} in {notes}");
    }
}

#[test]
fn python_leftovers_are_handled_as_the_python_version_handled_them() {
    if !python() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let ini = dir.path().join("config.ini");
    std::fs::write(
        &ini,
        "[site]\nconversations = False\nfirehose_interal = 10\nstarting = /map\n\
         map_top_left_lat = 43\nannouncement =\n\
         [mqtt]\nskip_node_ids = 0xdeadbeef, 305419896\nsecondary_keys = \"AQ==\", Ag==\n\
         [cleanup]\nminute = 05\n\
         [map_channel_colors]\nLongFast = #4363d8\nMyChannel = #18aa18\n",
    )
    .unwrap();

    let output = convert(&ini, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let config = Config::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap();

    assert!(!config.site.chat, "conversations is chat");
    // Python ignored the misspelt key, so the default stays.
    assert_eq!(
        config.site.firehose_interval,
        Config::default().site.firehose_interval
    );
    assert!(stderr(&output).contains("did you mean firehose_interval?"));
    assert_eq!(config.site.map_top_left_lat, 43.0);
    assert_eq!(config.mqtt.skip_node_ids, [0xdead_beef, 305_419_896]);
    assert_eq!(config.mqtt.secondary_keys, ["AQ==", "Ag=="]);
    assert_eq!(config.cleanup.minute, 5);
    // The Python sample's colour gives way to meshview-rs's; a chosen one stays.
    assert!(!config.map_channel_colors.contains_key("longfast"));
    assert_eq!(config.map_channel_colors["mychannel"], "#18aa18");
}

#[test]
fn a_value_of_the_wrong_type_stops_it() {
    if !python() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let ini = dir.path().join("config.ini");
    std::fs::write(&ini, "[server]\nport = eighty\n").unwrap();

    let output = convert(&ini, &[]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("[server] port"));
    assert!(output.stdout.is_empty());
}

#[test]
fn an_existing_output_is_kept_unless_forced() {
    if !python() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let ini = dir.path().join("config.ini");
    let toml = dir.path().join("config.toml");
    std::fs::write(&ini, "[server]\nport = 8082\n").unwrap();
    std::fs::write(&toml, "kept").unwrap();

    let output = convert(&ini, &["-o", toml.to_str().unwrap()]);
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(&toml).unwrap(), "kept");

    let output = convert(&ini, &["-o", toml.to_str().unwrap(), "--force"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let config = Config::parse(&std::fs::read_to_string(&toml).unwrap()).unwrap();
    assert_eq!(config.server.port, 8082);
}
