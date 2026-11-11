// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Configuration: the Python meshview's `config.ini`, read the way Python's
//! `configparser` reads it, with environment variables on top.
//!
//! The file format is kept on purpose. Somebody moving from the Python version
//! brings their `config.ini` and it means the same thing here, so the parser
//! follows `configparser` rather than any INI crate's idea of the format: keys
//! are case insensitive, a `#` or `;` starts a comment only at the beginning of a
//! line (which is what lets `LongFast = #4363d8` be a colour), quotes are part of
//! the value, and an indented line continues the value above it.
//!
//! Any value can be overridden from the environment as
//! `MESHVIEW_<SECTION>__<KEY>`, which is how a container or a Kubernetes pod is
//! configured without a file at all: `MESHVIEW_DATABASE__CONNECTION_STRING`,
//! `MESHVIEW_MQTT__PASSWORD` and so on. The double underscore separates the
//! section from the key because both may contain single ones.

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

/// The prefix every configuration variable in the environment carries.
const ENV_PREFIX: &str = "MESHVIEW_";

/// Between the section and the key in an environment variable's name.
const ENV_SEPARATOR: &str = "__";

/// The Python version's default port, kept so that nothing in front of it has
/// to change.
const DEFAULT_PORT: u16 = 8081;

/// Also the Python version's default: a file called packets.db next to where it
/// was started.
const DEFAULT_CONNECTION_STRING: &str = "sqlite:///packets.db";

/// Sections, each a map of lowercase keys to values.
type Sections = BTreeMap<String, BTreeMap<String, String>>;

/// The configuration as read, with the environment already applied.
#[derive(Debug, Clone, Default)]
pub struct Config {
    sections: Sections,
    source: Option<PathBuf>,
}

impl Config {
    /// Reads the file at `path`, if there is one, and applies the environment.
    ///
    /// A missing file is an error only when `required` is set, which is the case
    /// when somebody named it on the command line. Otherwise the defaults and the
    /// environment are a complete configuration, which is what a container
    /// started with nothing but variables wants.
    pub fn load(path: &Path, required: bool) -> Result<Self> {
        let (text, source) = match std::fs::read_to_string(path) {
            Ok(text) => (text, Some(path.to_path_buf())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => {
                (String::new(), None)
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };

        let mut config =
            Self::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.source = source;
        config.apply_env(std::env::vars());
        Ok(config)
    }

    /// Parses INI text the way `configparser.ConfigParser` would.
    pub fn parse(text: &str) -> Result<Self> {
        let mut sections = Sections::new();
        let mut section: Option<String> = None;
        // The key a following indented line would continue.
        let mut last_key: Option<String> = None;

        for (number, raw) in text.lines().enumerate() {
            let number = number + 1;
            let trimmed = raw.trim();

            if trimmed.is_empty() {
                last_key = None;
                continue;
            }
            if trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }

            let indented = raw.starts_with([' ', '\t']);
            if indented && let (Some(section), Some(key)) = (&section, &last_key) {
                let value = sections
                    .get_mut(section)
                    .and_then(|keys| keys.get_mut(key))
                    .expect("the key being continued was inserted above");
                value.push('\n');
                value.push_str(trimmed);
                continue;
            }

            if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                if sections.contains_key(name) {
                    bail!("line {number}: section [{name}] appears twice");
                }
                sections.insert(name.to_string(), BTreeMap::new());
                section = Some(name.to_string());
                last_key = None;
                continue;
            }

            let Some(current) = &section else {
                bail!("line {number}: a value before any [section]");
            };

            // `=` or `:`, whichever comes first, as configparser has it.
            let Some(split) = trimmed.find(['=', ':']) else {
                bail!("line {number}: expected `key = value`");
            };
            let key = trimmed[..split].trim().to_lowercase();
            let value = trimmed[split + 1..].trim().to_string();
            if key.is_empty() {
                bail!("line {number}: a value with no key");
            }

            let keys = sections.get_mut(current).expect("inserted with its header");
            if keys.insert(key.clone(), value).is_some() {
                bail!("line {number}: [{current}] {key} appears twice");
            }
            last_key = Some(key);
        }

        Ok(Self {
            sections,
            source: None,
        })
    }

    /// Lays `MESHVIEW_<SECTION>__<KEY>` variables over what the file said.
    pub fn apply_env(&mut self, vars: impl IntoIterator<Item = (String, String)>) {
        for (name, value) in vars {
            let Some(rest) = name.strip_prefix(ENV_PREFIX) else {
                continue;
            };
            let Some((section, key)) = rest.split_once(ENV_SEPARATOR) else {
                continue;
            };
            if section.is_empty() || key.is_empty() {
                continue;
            }
            self.sections
                .entry(section.to_lowercase())
                .or_default()
                .insert(key.to_lowercase(), value);
        }
    }

    /// The file this came from, or none when it is defaults and environment only.
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// One raw value, as a string.
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .get(section)
            .and_then(|keys| keys.get(key))
            .map(String::as_str)
    }

    /// A whole section, for the ones that are maps rather than settings, such as
    /// `[map_channel_colors]`.
    pub fn section(&self, section: &str) -> Option<&BTreeMap<String, String>> {
        self.sections.get(section)
    }

    /// A value that has to be a boolean, in any of the spellings configparser's
    /// `getboolean` accepts.
    pub fn get_bool(&self, section: &str, key: &str, default: bool) -> Result<bool> {
        match self.get(section, key).map(str::trim) {
            None | Some("") => Ok(default),
            Some(value) => match value.to_lowercase().as_str() {
                "1" | "yes" | "true" | "on" => Ok(true),
                "0" | "no" | "false" | "off" => Ok(false),
                _ => bail!("[{section}] {key} is not a boolean: {value}"),
            },
        }
    }

    /// Where the web server listens.
    pub fn listen_addr(&self) -> Result<SocketAddr> {
        let port = match self.get("server", "port").map(str::trim) {
            None | Some("") => DEFAULT_PORT,
            Some(value) => value
                .parse()
                .with_context(|| format!("[server] port is not a port number: {value}"))?,
        };

        // `*` is the Python version's way of saying every interface.
        let ip = match self.get("server", "bind").map(str::trim) {
            None | Some("") | Some("*") => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            Some(value) => value
                .parse()
                .with_context(|| format!("[server] bind is not an IP address: {value}"))?,
        };

        Ok(SocketAddr::new(ip, port))
    }

    /// The database to use.
    pub fn database(&self) -> Result<DatabaseUrl> {
        let value = self
            .get("database", "connection_string")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_CONNECTION_STRING);
        DatabaseUrl::parse(value)
    }
}

/// Which database, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseUrl {
    /// A SQLite file.
    Sqlite(PathBuf),
}

impl DatabaseUrl {
    /// Reads a connection string in SQLAlchemy's form, which is what every
    /// existing `config.ini` has.
    ///
    /// That form is `sqlite:///relative/path` and `sqlite:////absolute/path`,
    /// three slashes and then the path. The driver name after a `+`, such as the
    /// `aiosqlite` in `sqlite+aiosqlite:///packets.db`, means nothing here and is
    /// accepted and ignored, and so is a query string.
    pub fn parse(value: &str) -> Result<Self> {
        let Some((scheme, rest)) = value.split_once("://") else {
            bail!("[database] connection_string is not a URL: {value}");
        };
        let dialect = scheme.split('+').next().unwrap_or(scheme);

        match dialect {
            "sqlite" => {
                let Some(path) = rest.strip_prefix('/') else {
                    bail!(
                        "[database] connection_string needs three slashes before a SQLite \
                         path, as in sqlite:///packets.db: {value}"
                    );
                };
                let path = path.split('?').next().unwrap_or(path);
                if path.is_empty() || path == ":memory:" {
                    bail!("[database] connection_string has to name a SQLite file: {value}");
                }
                Ok(Self::Sqlite(PathBuf::from(path)))
            }
            "postgresql" | "postgres" => {
                bail!("PostgreSQL is not supported yet; use a SQLite connection string")
            }
            _ => bail!("[database] connection_string names an unknown database: {value}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_values_the_way_configparser_does() {
        let config = Config::parse(
            "# a comment\n\
             [site]\n\
             Title = Bay Area Mesh\n\
             message: with a colon\n\
             ; another comment\n\
             [map_channel_colors]\n\
             LongFast = #4363d8\n\
             [mqtt]\n\
             topics = [\"msh/US/bayarea/#\"]\n\
             empty =\n\
             long = first\n\
             \x20 second\n",
        )
        .unwrap();

        assert_eq!(config.get("site", "title"), Some("Bay Area Mesh"));
        assert_eq!(config.get("site", "message"), Some("with a colon"));
        assert_eq!(
            config.get("map_channel_colors", "longfast"),
            Some("#4363d8")
        );
        assert_eq!(config.get("mqtt", "topics"), Some("[\"msh/US/bayarea/#\"]"));
        assert_eq!(config.get("mqtt", "empty"), Some(""));
        assert_eq!(config.get("mqtt", "long"), Some("first\nsecond"));
    }

    #[test]
    fn refuses_what_configparser_refuses() {
        assert!(Config::parse("key = value\n").is_err());
        assert!(Config::parse("[a]\nkey = 1\nKEY = 2\n").is_err());
        assert!(Config::parse("[a]\n[a]\n").is_err());
        assert!(Config::parse("[a]\njust words\n").is_err());
    }

    #[test]
    fn the_environment_wins() {
        let mut config = Config::parse("[database]\nconnection_string = sqlite:///a.db\n").unwrap();
        config.apply_env([
            (
                "MESHVIEW_DATABASE__CONNECTION_STRING".to_string(),
                "sqlite:////data/b.db".to_string(),
            ),
            ("MESHVIEW_MQTT__PASSWORD".to_string(), "secret".to_string()),
            ("MESHVIEW_CONFIG".to_string(), "ignored".to_string()),
            ("PATH".to_string(), "ignored".to_string()),
        ]);

        assert_eq!(
            config.database().unwrap(),
            DatabaseUrl::Sqlite("/data/b.db".into())
        );
        assert_eq!(config.get("mqtt", "password"), Some("secret"));
        assert!(config.section("config").is_none());
    }

    #[test]
    fn connection_strings_mean_what_they_meant_in_python() {
        let sqlite = |value| DatabaseUrl::parse(value).unwrap();
        assert_eq!(
            sqlite("sqlite+aiosqlite:///packets.db"),
            DatabaseUrl::Sqlite("packets.db".into())
        );
        assert_eq!(
            sqlite("sqlite:////var/lib/meshview/packets.db?mode=ro"),
            DatabaseUrl::Sqlite("/var/lib/meshview/packets.db".into())
        );
        assert!(DatabaseUrl::parse("sqlite://packets.db").is_err());
        assert!(DatabaseUrl::parse("sqlite:///:memory:").is_err());
        assert!(DatabaseUrl::parse("postgresql+asyncpg://u:p@h/db").is_err());
        assert!(DatabaseUrl::parse("packets.db").is_err());
    }

    #[test]
    fn defaults_are_the_python_ones() {
        let config = Config::default();
        assert_eq!(
            config.listen_addr().unwrap(),
            "0.0.0.0:8081".parse().unwrap()
        );
        assert_eq!(
            config.database().unwrap(),
            DatabaseUrl::Sqlite("packets.db".into())
        );
    }

    #[test]
    fn booleans_in_any_spelling() {
        let config = Config::parse("[a]\nx = True\ny = off\nz = maybe\n").unwrap();
        assert!(config.get_bool("a", "x", false).unwrap());
        assert!(!config.get_bool("a", "y", true).unwrap());
        assert!(config.get_bool("a", "missing", true).unwrap());
        assert!(config.get_bool("a", "z", false).is_err());
    }
}
