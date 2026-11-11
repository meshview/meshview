// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Configuration: a TOML file, with environment variables on top.
//!
//! The sections and keys are the Python Meshview's, so moving a `config.ini`
//! over is a matter of syntax rather than of meaning: strings get quotes, and
//! booleans, numbers and lists become real ones (`topics` is a list, as is
//! `secondary_keys`, and `skip_node_ids` is a list of numbers, hex allowed).
//! What it does not have is the INI file's tolerance for keys nobody reads: a key
//! or a section this program does not know is an error, so a typo is found
//! rather than quietly ignored.
//!
//! Any value can be overridden from the environment as
//! `MESHVIEW_<SECTION>__<KEY>`, which is how a container or a Kubernetes pod is
//! configured without a file at all. The value is read as the type the key has:
//! `MESHVIEW_MQTT__PASSWORD=1234` is a string, `MESHVIEW_MQTT__TLS=true` a
//! boolean and `MESHVIEW_MQTT__TOPICS='["msh/#"]'` a list.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use toml::{Table, Value};

/// The prefix every configuration variable in the environment carries.
const ENV_PREFIX: &str = "MESHVIEW_";

/// Between the section and the key in an environment variable's name. Double,
/// because both may contain single ones.
const ENV_SEPARATOR: &str = "__";

/// The whole configuration. Every section and every key has a default, so an
/// empty file, or none, is a valid configuration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: Server,
    pub site: Site,
    /// Map colours by channel name, as `#rrggbb`.
    pub map_channel_colors: BTreeMap<String, String>,
    pub mqtt: Mqtt,
    pub database: Database,
    pub cleanup: Cleanup,
    pub snapshot: Snapshot,
    pub logging: Logging,
    /// The file this came from, if any.
    #[serde(skip)]
    source: Option<PathBuf>,
    /// The `MESHVIEW_` variables laid over it, by name.
    #[serde(skip)]
    environment: Vec<EnvSetting>,
    /// Whether the file named the database.
    #[serde(skip)]
    database_in_file: bool,
}

/// A variable of the environment that set a key.
#[derive(Debug, Clone, PartialEq)]
struct EnvSetting {
    name: String,
    section: String,
    key: String,
}

/// `[server]`: where the web server listens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Server {
    /// The address to listen on. `*` means every interface.
    pub bind: String,
    pub port: u16,
    /// Deprecated, and ignored: HTTPS is a reverse proxy's to serve. Kept for
    /// one version so that a converted Python configuration still starts, with
    /// a warning; see [`Config::deprecations`].
    pub tls_cert: Option<String>,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            bind: "*".into(),
            // The Python version's, so that nothing in front of it has to change.
            port: 8081,
            tls_cert: None,
        }
    }
}

/// `[site]`: what the web pages show and how they behave.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Site {
    pub domain: String,
    pub language: String,
    pub title: String,
    pub message: String,
    pub announcement: String,
    pub nodes: bool,
    pub chat: bool,
    pub everything: bool,
    pub graphs: bool,
    pub stats: bool,
    pub net: bool,
    pub map: bool,
    pub top: bool,
    pub api: bool,
    pub meshviewworld: bool,
    pub map_top_left_lat: f64,
    pub map_top_left_lon: f64,
    pub map_bottom_right_lat: f64,
    pub map_bottom_right_lon: f64,
    pub map_interval: i64,
    pub map_tile_url: String,
    pub map_tile_attribution: String,
    pub map_tile_max_zoom: u8,
    pub map_tile_name: String,
    pub firehose_interval: i64,
    pub weekly_net_message: String,
    pub net_tag: String,
}

impl Default for Site {
    /// The Python version's defaults, as its `/api/config` applies them.
    fn default() -> Self {
        Self {
            domain: String::new(),
            language: "en".into(),
            title: String::new(),
            message: String::new(),
            announcement: String::new(),
            nodes: true,
            chat: true,
            everything: true,
            graphs: true,
            stats: true,
            net: true,
            map: true,
            top: true,
            api: true,
            meshviewworld: true,
            map_top_left_lat: 39.0,
            map_top_left_lon: -123.0,
            map_bottom_right_lat: 36.0,
            map_bottom_right_lon: -121.0,
            map_interval: 3,
            map_tile_url: String::new(),
            map_tile_attribution: String::new(),
            map_tile_max_zoom: 19,
            map_tile_name: String::new(),
            firehose_interval: 3,
            weekly_net_message: "Weekly Mesh check-in message.".into(),
            net_tag: "#BayMeshNet".into(),
        }
    }
}

/// `[mqtt]`: the broker to read the mesh from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Mqtt {
    pub server: String,
    /// 1883, or 8883 with TLS, when not given.
    pub port: Option<u16>,
    pub topics: Vec<String>,
    pub username: String,
    pub password: String,
    /// Nodes whose packets are not stored.
    pub skip_node_ids: Vec<u32>,
    /// Base64 keys tried after the default one.
    pub secondary_keys: Vec<String>,
    pub tls: bool,
    pub tls_ca_file: String,
    pub tls_client_cert: String,
    pub tls_client_key: String,
}

/// `[database]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Database {
    /// SQLAlchemy's form, as the Python version: `sqlite:///relative.db` or
    /// `sqlite:////absolute.db`.
    pub connection_string: String,
}

impl Default for Database {
    fn default() -> Self {
        Self {
            connection_string: "sqlite:///packets.db".into(),
        }
    }
}

/// `[cleanup]`: pruning old data and backing the database up, daily.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cleanup {
    pub enabled: bool,
    pub days_to_keep: u32,
    pub hour: u8,
    pub minute: u8,
    pub vacuum: bool,
    /// Deprecated, and ignored: Meshview does not back up by itself. Kept
    /// for one version so that a converted Python configuration still starts,
    /// with a warning; see [`Config::deprecations`].
    pub backup_enabled: Option<bool>,
    pub backup_dir: Option<String>,
    pub backup_hour: Option<u8>,
    pub backup_minute: Option<u8>,
}

impl Default for Cleanup {
    fn default() -> Self {
        Self {
            enabled: false,
            days_to_keep: 14,
            hour: 2,
            minute: 0,
            vacuum: false,
            backup_enabled: None,
            backup_dir: None,
            backup_hour: None,
            backup_minute: None,
        }
    }
}

/// `[snapshot]`: when the previous day's figures are recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Snapshot {
    pub hour: u8,
    pub minute: u8,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self { hour: 1, minute: 0 }
    }
}

/// `[logging]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Logging {
    pub access_log: bool,
    /// Deprecated, and ignored: every log goes to standard output, where the
    /// process's supervisor collects it. Kept for one version, with a warning.
    pub db_cleanup_logfile: Option<String>,
}

/// Where the configuration is looked for when none is named, in order: the
/// working directory first, then the system's place for it. Both optional:
/// without either, the defaults and the environment are the configuration.
pub const SEARCHED: [&str; 2] = ["config.toml", "/etc/meshview/config.toml"];

/// The first of `candidates` that exists.
pub fn first_existing<'a>(candidates: &[&'a Path]) -> Option<&'a Path> {
    candidates.iter().copied().find(|path| path.exists())
}

impl Config {
    /// Reads the file at `path`, if there is one, and applies the environment.
    ///
    /// A missing file is an error only when `required` is set, which is the case
    /// when somebody named it. Otherwise the defaults and the environment are a
    /// complete configuration, which is what a container started with nothing but
    /// variables wants.
    pub fn load(path: &Path, required: bool) -> Result<Self> {
        if path.extension().is_some_and(|ext| ext == "ini") {
            bail!(
                "{} is an INI file, and Meshview reads TOML: the sections and keys are the \
                 Python meshview's, with strings quoted and lists as lists. See \
                 sample.config.toml.",
                path.display()
            );
        }

        let (text, source) = match std::fs::read_to_string(path) {
            Ok(text) => (text, Some(path.to_path_buf())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => {
                (String::new(), None)
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };

        let mut table: Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let database_in_file = names_the_database(&table);
        let environment = apply_env(&mut table, std::env::vars())?;

        let mut config: Self = Value::Table(table)
            .try_into()
            .with_context(|| match &source {
                Some(path) => format!("in {} or the environment", path.display()),
                None => "in the environment".to_string(),
            })?;
        config.source = source;
        config.environment = environment;
        config.database_in_file = database_in_file;
        Ok(config)
    }

    /// Parses TOML text, without the environment.
    pub fn parse(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    /// Parses TOML text and lays these variables over it, as [`Config::load`]
    /// does with the real environment.
    pub fn parse_with_env(
        text: &str,
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self> {
        let mut table: Table = toml::from_str(text)?;
        let database_in_file = names_the_database(&table);
        let environment = apply_env(&mut table, vars)?;
        let mut config: Self = Value::Table(table).try_into()?;
        config.environment = environment;
        config.database_in_file = database_in_file;
        Ok(config)
    }

    /// The `MESHVIEW_` variables that set keys, by name: what to log at start,
    /// without their values, which can be secrets.
    pub fn environment_variables(&self) -> impl Iterator<Item = &str> {
        self.environment.iter().map(|setting| setting.name.as_str())
    }

    /// Where the database's connection string came from, to log beside it: a
    /// variable left set in the environment wins over the file without a word
    /// otherwise.
    pub fn database_origin(&self) -> String {
        let from_env =
            self.environment.iter().rev().find(|setting| {
                setting.section == "database" && setting.key == "connection_string"
            });
        match from_env {
            Some(setting) => format!("set by {}", setting.name),
            None if self.database_in_file => "set in the configuration file".to_string(),
            None => "the default".to_string(),
        }
    }

    /// What in this configuration is deprecated, as warnings to log at start-up.
    ///
    /// Deprecated keys are still accepted so that a configuration converted from
    /// the Python version starts, but they do nothing, and they go in the next
    /// version: a key that is accepted for ever is one nobody ever removes.
    pub fn deprecations(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        if self.server.tls_cert.is_some() {
            warnings.push(
                "[server] tls_cert is deprecated and ignored, and will be removed in the next \
                 version: serve HTTPS from a reverse proxy in front, such as nginx, Caddy or a \
                 Kubernetes ingress."
                    .to_string(),
            );
        }

        let backup = [
            ("backup_enabled", self.cleanup.backup_enabled.is_some()),
            ("backup_dir", self.cleanup.backup_dir.is_some()),
            ("backup_hour", self.cleanup.backup_hour.is_some()),
            ("backup_minute", self.cleanup.backup_minute.is_some()),
        ];
        for (key, given) in backup {
            if given {
                warnings.push(format!(
                    "[cleanup] {key} is deprecated and ignored, and will be removed in the next \
                     version: Meshview does not back up by itself. Run `meshview backup \
                     <file>` from your scheduler instead."
                ));
            }
        }

        if self.logging.db_cleanup_logfile.is_some() {
            warnings.push(
                "[logging] db_cleanup_logfile is deprecated and ignored, and will be removed in \
                 the next version: the cleanup logs where everything else does. Collect the \
                 process's output to keep it in a file."
                    .to_string(),
            );
        }

        warnings
    }

    /// The file this came from, or none when it is defaults and environment only.
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// Where the web server listens.
    pub fn listen_addr(&self) -> Result<SocketAddr> {
        let ip = match self.server.bind.trim() {
            "" | "*" => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            bind => bind
                .parse()
                .with_context(|| format!("[server] bind is not an IP address: {bind}"))?,
        };
        Ok(SocketAddr::new(ip, self.server.port))
    }

    /// The database to use.
    pub fn database(&self) -> Result<DatabaseUrl> {
        DatabaseUrl::parse(self.database.connection_string.trim())
    }
}

/// Whether a parsed file has `[database] connection_string`.
fn names_the_database(table: &Table) -> bool {
    table
        .get("database")
        .and_then(Value::as_table)
        .is_some_and(|database| database.contains_key("connection_string"))
}

/// Lays `MESHVIEW_<SECTION>__<KEY>` variables over a parsed file, and answers
/// those it laid.
///
/// Each value is read as the type its key has, which the defaults say. A key
/// that has no default to go by (an optional number) is read as a TOML value if
/// it is one and as a string otherwise. Unknown sections and keys are put in
/// anyway, so that reading the result refuses them as it would in the file.
fn apply_env(
    table: &mut Table,
    vars: impl IntoIterator<Item = (String, String)>,
) -> Result<Vec<EnvSetting>> {
    let mut applied = Vec::new();
    let defaults = Table::try_from(Config::default()).expect("the defaults serialise");
    let a_string = Value::String(String::new());

    for (name, raw) in vars {
        let Some(rest) = name.strip_prefix(ENV_PREFIX) else {
            continue;
        };
        let Some((section, key)) = rest.split_once(ENV_SEPARATOR) else {
            continue;
        };
        if section.is_empty() || key.is_empty() {
            continue;
        }
        let (section, key) = (section.to_lowercase(), key.to_lowercase());

        let expected = defaults.get(&section).and_then(|s| match s {
            // A section that is a map of strings, such as the channel colours.
            Value::Table(keys) if keys.is_empty() => Some(&a_string),
            Value::Table(keys) => keys.get(&key),
            _ => None,
        });
        let value = coerce(&raw, expected).with_context(|| format!("{name}={raw:?}"))?;

        let entry = table
            .entry(section.clone())
            .or_insert_with(|| Value::Table(Table::new()));
        let Value::Table(keys) = entry else {
            bail!("{name}: [{section}] is not a section");
        };
        keys.insert(key.clone(), value);
        applied.push(EnvSetting { name, section, key });
    }
    Ok(applied)
}

/// Reads a value from the environment as the type of `expected`.
fn coerce(raw: &str, expected: Option<&Value>) -> Result<Value> {
    let literal = || {
        toml::from_str::<Table>(&format!("v = {raw}"))
            .ok()
            .and_then(|mut table| table.remove("v"))
    };

    Ok(match expected {
        Some(Value::String(_)) => Value::String(raw.to_string()),
        Some(Value::Boolean(_)) => Value::Boolean(match raw.trim().to_lowercase().as_str() {
            "1" | "yes" | "true" | "on" => true,
            "0" | "no" | "false" | "off" => false,
            _ => bail!("not a boolean"),
        }),
        Some(Value::Integer(_)) => match literal() {
            Some(value @ Value::Integer(_)) => value,
            _ => bail!("not a whole number"),
        },
        Some(Value::Float(_)) => match literal() {
            Some(Value::Float(f)) => Value::Float(f),
            Some(Value::Integer(i)) => Value::Float(i as f64),
            _ => bail!("not a number"),
        },
        Some(Value::Array(_)) => match literal() {
            Some(value @ Value::Array(_)) => value,
            _ => bail!("not a list, such as [\"a\", \"b\"]"),
        },
        _ => literal().unwrap_or_else(|| Value::String(raw.to_string())),
    })
}

/// Which database, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseUrl {
    /// A SQLite file.
    Sqlite(PathBuf),
}

impl DatabaseUrl {
    /// Reads a connection string in SQLAlchemy's form, which is what every
    /// existing configuration has.
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

    const SAMPLE: &str = include_str!("../../sample.config.toml");

    fn env(vars: &[(&str, &str)]) -> Vec<(String, String)> {
        vars.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_sample_is_a_valid_configuration() {
        let config = Config::parse(SAMPLE).unwrap();

        assert_eq!(config.site.title, "Bay Area Mesh");
        assert!(config.site.nodes);
        assert_eq!(config.site.map_top_left_lon, -123.0);
        // The presets' colours are built in: the sample sets none.
        assert!(config.map_channel_colors.is_empty());
        assert_eq!(
            config.mqtt.topics,
            [
                "msh/US/bayarea/#",
                "msh/US/CA/mrymesh/#",
                "msh/US/CA/sacvalley/#"
            ]
        );
        assert_eq!(config.mqtt.port, Some(1883));
        assert_eq!(config.cleanup.days_to_keep, 14);
        assert_eq!(
            config.database().unwrap(),
            DatabaseUrl::Sqlite("packets.db".into())
        );
    }

    #[test]
    fn nothing_at_all_is_the_python_defaults() {
        let config = Config::parse("").unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(
            config.listen_addr().unwrap(),
            "0.0.0.0:8081".parse().unwrap()
        );
        assert_eq!(
            config.site.weekly_net_message,
            "Weekly Mesh check-in message."
        );
        assert_eq!(config.snapshot.hour, 1);
    }

    #[test]
    fn typos_are_refused() {
        assert!(Config::parse("[mqtt]\npasword = \"x\"\n").is_err());
        assert!(Config::parse("[sitte]\ntitle = \"x\"\n").is_err());
        assert!(Config::parse("[server]\nport = \"8081\"\n").is_err());
        assert!(Config::parse("[server]\nport = 70000\n").is_err());
    }

    #[test]
    fn hex_node_ids_are_numbers() {
        let config = Config::parse("[mqtt]\nskip_node_ids = [0xdeadbeef, 305419896]\n").unwrap();
        assert_eq!(config.mqtt.skip_node_ids, [0xdead_beef, 305_419_896]);
    }

    #[test]
    fn the_environment_wins_and_keeps_each_key_its_type() {
        let config = Config::parse_with_env(
            "[mqtt]\nserver = \"from-file\"\npassword = \"file\"\n",
            env(&[
                ("MESHVIEW_MQTT__SERVER", "from-env"),
                ("MESHVIEW_MQTT__PASSWORD", "1234"),
                ("MESHVIEW_MQTT__TLS", "True"),
                ("MESHVIEW_MQTT__PORT", "8884"),
                (
                    "MESHVIEW_MQTT__TOPICS",
                    r#"["msh/EU_868/#", "msh/EU_N_868/#"]"#,
                ),
                ("MESHVIEW_SITE__MAP_TOP_LEFT_LAT", "43"),
                ("MESHVIEW_MAP_CHANNEL_COLORS__LONGFAST", "#4363d8"),
                ("MESHVIEW_CLEANUP__BACKUP_HOUR", "3"),
                ("MESHVIEW_CONFIG", "ignored"),
                ("PATH", "ignored"),
            ]),
        )
        .unwrap();

        assert_eq!(config.mqtt.server, "from-env");
        assert_eq!(config.mqtt.password, "1234");
        assert!(config.mqtt.tls);
        assert_eq!(config.mqtt.port, Some(8884));
        assert_eq!(config.mqtt.topics, ["msh/EU_868/#", "msh/EU_N_868/#"]);
        assert_eq!(config.site.map_top_left_lat, 43.0);
        assert_eq!(config.map_channel_colors["longfast"], "#4363d8");
        assert_eq!(config.cleanup.backup_hour, Some(3));
    }

    #[test]
    fn the_environment_is_checked_as_the_file_is() {
        let refused =
            |name: &str, value: &str| Config::parse_with_env("", env(&[(name, value)])).is_err();
        assert!(refused("MESHVIEW_MQTT__PASWORD", "x"));
        assert!(refused("MESHVIEW_MQTT__TLS", "maybe"));
        assert!(refused("MESHVIEW_SERVER__PORT", "http"));
        assert!(refused("MESHVIEW_MQTT__TOPICS", "msh/#"));
    }

    #[test]
    fn deprecated_keys_are_accepted_with_a_warning_only_when_given() {
        assert!(Config::default().deprecations().is_empty());

        let config = Config::parse(
            "[server]\ntls_cert = \"\"\n\
             [cleanup]\nbackup_enabled = true\nbackup_dir = \"/b\"\n\
             [logging]\ndb_cleanup_logfile = \"x.log\"\n",
        )
        .unwrap();
        let warnings = config.deprecations();

        assert_eq!(warnings.len(), 4);
        assert!(
            warnings
                .iter()
                .all(|w| w.contains("removed in the next version"))
        );
        assert!(warnings[0].contains("reverse proxy"));
        assert!(warnings[1].contains("meshview backup"));
    }

    #[test]
    fn where_the_database_was_named_is_known() {
        let default = Config::parse_with_env("", Vec::new()).unwrap();
        assert_eq!(default.database_origin(), "the default");

        let file = "[database]\nconnection_string = \"sqlite:////var/lib/m/packets.db\"\n";
        let in_file = Config::parse_with_env(file, Vec::new()).unwrap();
        assert_eq!(in_file.database_origin(), "set in the configuration file");

        let overridden = Config::parse_with_env(
            file,
            env(&[
                (
                    "MESHVIEW_DATABASE__CONNECTION_STRING",
                    "sqlite:////data/packets.db",
                ),
                ("MESHVIEW_MQTT__PASSWORD", "secret"),
            ]),
        )
        .unwrap();
        assert_eq!(
            overridden.database_origin(),
            "set by MESHVIEW_DATABASE__CONNECTION_STRING"
        );
        // Names only: values can be secrets.
        let names: Vec<_> = overridden.environment_variables().collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"MESHVIEW_MQTT__PASSWORD"));
    }

    #[test]
    fn the_working_directory_comes_before_the_system() {
        let dir = tempfile::TempDir::new().unwrap();
        let local = dir.path().join("config.toml");
        let system = dir.path().join("etc-config.toml");
        let candidates = [local.as_path(), system.as_path()];

        assert_eq!(first_existing(&candidates), None);
        std::fs::write(&system, "").unwrap();
        assert_eq!(first_existing(&candidates), Some(system.as_path()));
        std::fs::write(&local, "").unwrap();
        assert_eq!(first_existing(&candidates), Some(local.as_path()));
        assert_eq!(SEARCHED[1], "/etc/meshview/config.toml");
    }

    #[test]
    fn an_ini_file_is_pointed_at_the_new_format() {
        let error = Config::load(Path::new("config.ini"), false).unwrap_err();
        assert!(format!("{error:#}").contains("sample.config.toml"));
    }

    /// Every key there is, as sections of key names: the defaults with every
    /// optional key given a value, so that serialising them leaves none out.
    /// A new optional key has to be set here too, or the check below misses it.
    fn every_key() -> BTreeMap<String, Vec<String>> {
        let mut config = Config::default();
        config.mqtt.port = Some(1883);
        config.server.tls_cert = Some(String::new());
        config.cleanup.backup_enabled = Some(false);
        config.cleanup.backup_dir = Some("./backups".into());
        config.cleanup.backup_hour = Some(2);
        config.cleanup.backup_minute = Some(0);
        config.logging.db_cleanup_logfile = Some("dbcleanup.log".into());

        Table::try_from(config)
            .unwrap()
            .into_iter()
            .map(|(section, keys)| {
                let keys = match keys {
                    Value::Table(keys) => keys.keys().cloned().collect(),
                    _ => Vec::new(),
                };
                (section, keys)
            })
            .collect()
    }

    /// The documented keys, by section: the rows of each section's table in
    /// docs/configuration.md.
    fn documented_keys() -> BTreeMap<String, Vec<String>> {
        const DOCS: &str = include_str!("../../docs/configuration.md");
        let mut documented = BTreeMap::new();

        for part in DOCS.split("\n### `[").skip(1) {
            let (section, body) = part.split_once("]`").unwrap();
            let body = body.split("\n## ").next().unwrap();
            let keys = body
                .lines()
                .filter_map(|line| line.strip_prefix("| `"))
                .filter_map(|line| line.split_once("` |"))
                .map(|(key, _)| key.to_string())
                .collect();
            documented.insert(section.to_string(), keys);
        }
        documented
    }

    #[test]
    fn the_documentation_has_every_key_and_no_other() {
        let every = every_key();
        let documented = documented_keys();

        assert_eq!(
            every.keys().collect::<Vec<_>>(),
            documented.keys().collect::<Vec<_>>(),
            "docs/configuration.md and the code disagree about the sections"
        );
        for (section, keys) in &every {
            let mut keys = keys.clone();
            let mut written = documented[section].clone();
            keys.sort();
            written.sort();
            assert_eq!(
                keys, written,
                "docs/configuration.md and the code disagree about [{section}]"
            );
        }
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
}
