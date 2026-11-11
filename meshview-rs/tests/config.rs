// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The Python meshview's own sample configuration, read here and by Python's
//! configparser, has to come out the same.
//!
//! `python-sample.config.json` is what configparser made of
//! `python-sample.config.ini`:
//!
//! ```sh
//! python -I -c 'import configparser, json, sys
//! p = configparser.ConfigParser(); p.read(sys.argv[1])
//! print(json.dumps({s: dict(p.items(s)) for s in p.sections()}, indent=2,
//!                  ensure_ascii=False))' python-sample.config.ini
//! ```

use meshview_rs::config::Config;
use std::collections::BTreeMap;

const INI: &str = include_str!("fixtures/python-sample.config.ini");
const EXPECTED: &str = include_str!("fixtures/python-sample.config.json");

#[test]
fn the_python_sample_reads_as_configparser_reads_it() {
    let config = Config::parse(INI).unwrap();
    let expected: BTreeMap<String, BTreeMap<String, String>> =
        serde_json::from_str(EXPECTED).unwrap();

    for (section, keys) in &expected {
        assert_eq!(
            config.section(section),
            Some(keys),
            "section [{section}] differs"
        );
    }

    // And nothing configparser did not see either.
    for section in ["server", "site", "map_channel_colors", "mqtt", "database"] {
        assert!(expected.contains_key(section));
    }
}
