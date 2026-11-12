// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The database, and the line between what is true of any of them and what is
//! true of each.
//!
//! SQLite always, and PostgreSQL with the `postgres` feature. Everything that
//! knows which one it is talking to — connection options, the catalogue, the
//! migrations, the SQL — lives in [`sqlite`] or [`postgres`]. What sits here is
//! what the rest of the program reasons about whichever database it is, and a
//! [`Pool`] that is one or the other.

#[cfg(feature = "postgres")]
pub mod postgres;
pub mod sqlite;

use crate::config::DatabaseUrl;
use anyhow::{Result, bail};
use sqlx::SqlitePool;
use std::fmt;
use std::path::Path;

/// Connections to the database, whichever it is.
#[derive(Debug, Clone)]
pub enum Pool {
    Sqlite(SqlitePool),
    #[cfg(feature = "postgres")]
    Postgres(sqlx::PgPool),
}

impl From<SqlitePool> for Pool {
    fn from(pool: SqlitePool) -> Self {
        Self::Sqlite(pool)
    }
}

/// Refuses PostgreSQL in a build without it.
#[cfg(not(feature = "postgres"))]
fn no_postgres() -> anyhow::Error {
    anyhow::anyhow!(
        "this build of Meshview has no PostgreSQL support: use a SQLite connection string, or \
         a build with the `postgres` feature"
    )
}

/// The versions of the migrations this build carries, in order. Both databases
/// carry the same steps under the same numbers.
pub fn known_versions(database: &DatabaseUrl) -> Vec<i64> {
    match database {
        DatabaseUrl::Sqlite(_) => sqlite::known_versions(),
        #[cfg(feature = "postgres")]
        DatabaseUrl::Postgres(_) => postgres::known_versions(),
        #[cfg(not(feature = "postgres"))]
        DatabaseUrl::Postgres(_) => Vec::new(),
    }
}

/// Opens the database for writing, creating what it can, and brings its schema
/// up to date.
pub async fn open_for_writing(database: &DatabaseUrl) -> Result<Pool> {
    match database {
        DatabaseUrl::Sqlite(path) => Ok(Pool::Sqlite(sqlite::open_for_writing(path).await?)),
        #[cfg(feature = "postgres")]
        DatabaseUrl::Postgres(url) => Ok(Pool::Postgres(
            postgres::open_for_writing(url, &database.to_string()).await?,
        )),
        #[cfg(not(feature = "postgres"))]
        DatabaseUrl::Postgres(_) => Err(no_postgres()),
    }
}

/// Opens the database for reading, without ever changing it. `None` while a
/// SQLite file is not there yet.
pub async fn open_for_reading(database: &DatabaseUrl) -> Result<Option<Pool>> {
    match database {
        DatabaseUrl::Sqlite(path) => Ok(sqlite::open_for_reading(path).await?.map(Pool::Sqlite)),
        #[cfg(feature = "postgres")]
        DatabaseUrl::Postgres(url) => Ok(Some(Pool::Postgres(
            postgres::open_for_reading(url, &database.to_string()).await?,
        ))),
        #[cfg(not(feature = "postgres"))]
        DatabaseUrl::Postgres(_) => Err(no_postgres()),
    }
}

/// Opens one read connection that does not warn about slow statements.
pub async fn open_quietly_for_reading(database: &DatabaseUrl) -> Result<Pool> {
    match database {
        DatabaseUrl::Sqlite(path) => {
            Ok(Pool::Sqlite(sqlite::open_quietly_for_reading(path).await?))
        }
        #[cfg(feature = "postgres")]
        DatabaseUrl::Postgres(url) => Ok(Pool::Postgres(
            postgres::open_quietly_for_reading(url, &database.to_string()).await?,
        )),
        #[cfg(not(feature = "postgres"))]
        DatabaseUrl::Postgres(_) => Err(no_postgres()),
    }
}

/// What the database holds, read from its catalogue.
pub async fn inspect(pool: &Pool) -> Result<Schema> {
    match pool {
        Pool::Sqlite(pool) => sqlite::inspect(pool).await,
        #[cfg(feature = "postgres")]
        Pool::Postgres(pool) => postgres::inspect(pool).await,
    }
}

/// Closes the writer: for SQLite, folding its log back in first.
pub async fn close_writer(pool: Pool) {
    match pool {
        Pool::Sqlite(pool) => sqlite::close_writer(pool).await,
        #[cfg(feature = "postgres")]
        Pool::Postgres(pool) => pool.close().await,
    }
}

/// Closes a pool that only read, waiting for its connections.
pub async fn close_reader(pool: Pool) {
    match pool {
        Pool::Sqlite(pool) => sqlite::close_reader(pool).await,
        #[cfg(feature = "postgres")]
        Pool::Postgres(pool) => pool.close().await,
    }
}

/// Copies the database to `output`. Only for SQLite: PostgreSQL has its own
/// tools for that, which do it better than anything here could.
pub async fn backup(database: &DatabaseUrl, output: &Path) -> Result<u64> {
    match database {
        DatabaseUrl::Sqlite(path) => sqlite::backup(path, output).await,
        DatabaseUrl::Postgres(_) => bail!(
            "meshview backup copies SQLite databases; back PostgreSQL up with its own tools, \
             such as pg_dump"
        ),
    }
}

/// Hands a read to the module of the database the pool is on.
macro_rules! on {
    ($self:ident, $name:ident ( $($arg:expr),* )) => {
        match &$self.pool {
            Pool::Sqlite(pool) => sqlite::queries::$name(pool $(, $arg)*).await,
            #[cfg(feature = "postgres")]
            Pool::Postgres(pool) => postgres::queries::$name(pool $(, $arg)*).await,
        }
    };
}

/// Reading the database, for the web server and the API.
///
/// Every query the web side makes is a method here, and nothing outside `db`
/// writes SQL. Each hands over to the queries of the database the pool is on,
/// and nothing that calls it needs to know which.
#[derive(Debug, Clone)]
pub struct Reader {
    pool: Pool,
}

impl Reader {
    pub fn new(pool: impl Into<Pool>) -> Self {
        Self { pool: pool.into() }
    }

    /// The pool, for closing it.
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// What the database takes on disk, where the database itself says: in
    /// PostgreSQL. A SQLite database is a file, and its size the file's.
    pub async fn size(&self) -> Result<Option<u64>> {
        match &self.pool {
            Pool::Sqlite(_) => Ok(None),
            #[cfg(feature = "postgres")]
            Pool::Postgres(pool) => postgres::queries::size(pool).await,
        }
    }

    /// The channels of the packets imported since `since_us`, without repeats,
    /// in byte order.
    pub async fn channels_since(&self, since_us: i64) -> Result<Vec<String>> {
        on!(self, channels_since(since_us))
    }

    /// The daily snapshots, oldest first: all of them, or only the latest
    /// `latest` days.
    pub async fn daily_snapshots(&self, latest: Option<i64>) -> Result<Vec<DailySnapshot>> {
        on!(self, daily_snapshots(latest))
    }

    /// When the latest packet was imported, if there is any.
    pub async fn last_import_us(&self) -> Result<Option<i64>> {
        on!(self, last_import_us())
    }

    /// The latest run of a daily job, if it has run.
    pub async fn latest_job_run(&self, job: &str) -> Result<Option<JobRun>> {
        on!(self, latest_job_run(job))
    }

    /// How many packets were imported in each hour or day since `filter.since_us`,
    /// oldest first, leaving out those with none.
    pub async fn packets_per_period(
        &self,
        period: Period,
        filter: &PacketFilter,
    ) -> Result<Vec<PeriodCount>> {
        on!(self, packets_per_period(period, filter))
    }

    /// How many packets match `filter`, all of them when it is `None`.
    pub async fn count_packets(&self, filter: Option<&PacketFilter>) -> Result<i64> {
        on!(self, count_packets(filter))
    }

    /// How many sightings there are: of one packet, of the packets matching a
    /// filter, or all of them.
    pub async fn count_sightings(&self, of: SightingsOf<'_>) -> Result<i64> {
        on!(self, count_sightings(of))
    }

    /// The nodes by traffic, the most heard first, and how many there are.
    pub async fn top_nodes(&self, query: &TopQuery) -> Result<(i64, Vec<TopNode>)> {
        on!(self, top_nodes(query))
    }

    /// The nodes matching a filter, in order, a page of them if asked.
    pub async fn nodes(&self, query: &NodeQuery) -> Result<Vec<Node>> {
        on!(self, nodes(query))
    }

    /// One node, by its number.
    pub async fn node(&self, node_id: i64) -> Result<Option<Node>> {
        on!(self, node(node_id))
    }

    /// The stored node info packets a node sent, the latest first.
    pub async fn nodeinfo_payloads(&self, node_id: i64) -> Result<Vec<Option<Vec<u8>>>> {
        on!(self, nodeinfo_payloads(node_id))
    }

    /// The latest position packet and map report of each of these nodes: the
    /// node, the port, the time and the stored packet, two rows at most for each.
    pub async fn latest_positions(
        &self,
        node_ids: &[i64],
    ) -> Result<Vec<(i64, i64, Option<i64>, Option<Vec<u8>>)>> {
        on!(self, latest_positions(node_ids))
    }

    /// The telemetry packets heard since a time, the latest first: the sender
    /// and the stored packet.
    pub async fn telemetry_since(
        &self,
        since_us: i64,
    ) -> Result<Vec<(Option<i64>, Option<Vec<u8>>)>> {
        on!(self, telemetry_since(since_us))
    }

    /// The packets imported since a time, map reports aside, counted by
    /// channel and by whether they went to everyone.
    pub async fn packets_by_channel(
        &self,
        since_us: i64,
    ) -> Result<Vec<(Option<String>, bool, i64)>> {
        on!(self, packets_by_channel(since_us))
    }

    /// How many nodes match a filter.
    pub async fn count_nodes(&self, filter: &NodeFilter) -> Result<i64> {
        on!(self, count_nodes(filter))
    }

    /// The values a node field has among the nodes matching a filter, the most
    /// common first, with how many nodes have each.
    pub async fn node_facet(&self, field: NodeFacet, filter: &NodeFilter) -> Result<Vec<Facet>> {
        on!(self, node_facet(field, filter))
    }

    /// The traceroute rows of a packet, one per gateway that heard it, the
    /// first heard first, at most `limit`.
    pub async fn traceroutes(&self, packet_id: i64, limit: i64) -> Result<Vec<TracerouteRow>> {
        on!(self, traceroutes(packet_id, limit))
    }

    /// The traceroute rows imported since a time, the first imported first,
    /// with the endpoints of their packet.
    pub async fn traceroutes_since(&self, since_us: i64) -> Result<Vec<TracerouteHop>> {
        on!(self, traceroutes_since(since_us))
    }

    /// One packet, by its row id.
    pub async fn packet(&self, id: i64) -> Result<Option<PacketRow>> {
        on!(self, packet(id))
    }

    /// The latest packets matching a filter, latest first.
    pub async fn packets(&self, query: &PacketQuery) -> Result<Vec<PacketRow>> {
        on!(self, packets(query))
    }

    /// Every sighting of one packet, latest first.
    pub async fn packet_sightings(&self, packet_id: i64) -> Result<Vec<PacketSeen>> {
        on!(self, packet_sightings(packet_id))
    }
}

/// A row of `daily_snapshot`.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct DailySnapshot {
    pub snapshot_date: String,
    pub node_count: i64,
    pub packet_count: i64,
    pub gateway_count: i64,
    pub captured_at_us: i64,
}

/// The length of the periods packets are counted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Hour,
    Day,
}

impl Period {
    pub fn micros(self) -> i64 {
        match self {
            Self::Hour => 3_600_000_000,
            Self::Day => 86_400_000_000,
        }
    }
}

/// Which packets to count: those imported since a time, and optionally only
/// those on a channel, of a port, or to or from a node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PacketFilter {
    pub since_us: i64,
    /// Compared with the channel lowercased.
    pub channel: Option<String>,
    pub portnum: Option<i64>,
    pub to_node: Option<i64>,
    pub from_node: Option<i64>,
}

/// One period and how many packets it had.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PeriodCount {
    /// `YYYY-MM-DD HH:00` for hours, `YYYY-MM-DD` for days, in UTC.
    pub period: String,
    pub count: i64,
}

/// Whose sightings to count.
#[derive(Debug, Clone, Copy)]
pub enum SightingsOf<'a> {
    All,
    Packet(i64),
    /// The packets matching a filter, by when the packet was imported.
    Packets(&'a PacketFilter),
}

/// What the top nodes are asked for with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopQuery {
    /// How far back from the latest packet, and the latest sighting, to count.
    pub window_us: i64,
    /// Only nodes on this channel.
    pub channel: Option<String>,
    /// Only nodes that sent or were heard in the window.
    pub active: bool,
    pub limit: i64,
    pub offset: i64,
}

/// A node, and how much it sent and was heard in the window.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TopNode {
    pub node_id: Option<i64>,
    pub long_name: Option<String>,
    pub short_name: Option<String>,
    pub channel: Option<String>,
    pub sent: i64,
    pub seen: i64,
}

/// A link between two nodes, and how it is known.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub struct Edge {
    /// One end: for a traceroute, the end nearer the node that started it;
    /// for a neighbour, the neighbour.
    #[schema(example = 12345678)]
    pub from: i64,
    /// The other end: for a neighbour, the node that reported it.
    #[schema(example = 87654321)]
    pub to: i64,
    /// `traceroute` when a traceroute went through it, `neighbor` when a node
    /// reported the other as its neighbour.
    #[serde(rename = "type")]
    #[schema(example = "traceroute")]
    pub kind: &'static str,
}

/// A traceroute row with its packet's endpoints, for drawing its links.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TracerouteHop {
    pub gateway_node_id: Option<i64>,
    pub done: Option<bool>,
    pub route: Option<Vec<u8>>,
    /// Whether the packet the row belongs to is there.
    pub has_packet: bool,
    pub from_node_id: Option<i64>,
    pub to_node_id: Option<i64>,
}

/// A row of `traceroute`.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TracerouteRow {
    pub gateway_node_id: Option<i64>,
    pub done: Option<bool>,
    /// The RouteDiscovery, as sent.
    pub route: Option<Vec<u8>>,
    pub import_time_us: Option<i64>,
}

/// A row of `packet`, with the names of its sender and recipient when they
/// are known nodes.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PacketRow {
    pub id: i64,
    pub portnum: Option<i64>,
    pub from_node_id: Option<i64>,
    pub to_node_id: Option<i64>,
    pub payload: Option<Vec<u8>>,
    pub import_time_us: Option<i64>,
    pub channel: Option<String>,
    /// Whether the sender is a node there is a row for, whose long name may
    /// still be null.
    pub from_known: bool,
    pub from_long_name: Option<String>,
    pub to_known: bool,
    pub to_long_name: Option<String>,
}

/// Which packets to list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PacketQuery {
    pub from_node_id: Option<i64>,
    pub to_node_id: Option<i64>,
    /// From or to this node.
    pub node_id: Option<i64>,
    pub channel: Option<String>,
    pub portnum: Option<i64>,
    /// Imported after this time.
    pub after_us: Option<i64>,
    /// Imported before this time: the page cursor.
    pub before_us: Option<i64>,
    /// In the stored packet's bytes read as text, ignoring case, literally.
    /// Text messages only to everyone, as the Python version filtered them.
    pub contains: Option<String>,
    pub limit: i64,
}

/// A row of `node`.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Node {
    pub id: String,
    pub node_id: Option<i64>,
    pub long_name: Option<String>,
    pub short_name: Option<String>,
    pub hw_model: Option<String>,
    pub firmware: Option<String>,
    pub role: Option<String>,
    pub last_lat: Option<i64>,
    pub last_long: Option<i64>,
    pub last_altitude: Option<i64>,
    pub channel: Option<String>,
    pub is_mqtt_gateway: Option<bool>,
    pub first_seen_us: Option<i64>,
    pub last_seen_us: Option<i64>,
}

/// Which nodes. Every condition set must hold; only nodes ever seen count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeFilter {
    /// This node, or none at all when the id asked for is not a number.
    pub node_id: Option<Option<i64>>,
    /// Compared uppercased.
    pub role: Option<String>,
    pub channel: Option<String>,
    pub hw_model: Option<String>,
    pub firmware: Option<String>,
    /// Seen after this time.
    pub seen_since_us: Option<i64>,
    /// Somewhere in the long or short name, the id or the number, ignoring
    /// case; taken literally, wildcards and all.
    pub search: Option<String>,
    /// One of these nodes: an empty list matches none.
    pub ids: Option<Vec<i64>>,
    pub is_mqtt_gateway: Option<bool>,
    /// Whether it has a position other than (0, 0).
    pub has_position: Option<bool>,
}

/// The columns nodes can be sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSort {
    ShortName,
    LongName,
    HwModel,
    Firmware,
    Role,
    Channel,
    LastLat,
    LastLong,
    LastAltitude,
    IsMqttGateway,
    FirstSeenUs,
    LastSeenUs,
    NodeId,
}

impl NodeSort {
    pub const ALL: [(&'static str, Self); 13] = [
        ("short_name", Self::ShortName),
        ("long_name", Self::LongName),
        ("hw_model", Self::HwModel),
        ("firmware", Self::Firmware),
        ("role", Self::Role),
        ("channel", Self::Channel),
        ("last_lat", Self::LastLat),
        ("last_long", Self::LastLong),
        ("last_altitude", Self::LastAltitude),
        ("is_mqtt_gateway", Self::IsMqttGateway),
        ("first_seen_us", Self::FirstSeenUs),
        ("last_seen_us", Self::LastSeenUs),
        ("node_id", Self::NodeId),
    ];

    pub fn column(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, sort)| *sort == self)
            .map(|(name, _)| *name)
            .expect("every sort is listed")
    }

    /// Whether it is text, and so compared ignoring case.
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Self::ShortName
                | Self::LongName
                | Self::HwModel
                | Self::Firmware
                | Self::Role
                | Self::Channel
        )
    }
}

/// What nodes to list, and how.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeQuery {
    pub filter: NodeFilter,
    /// The column and whether descending; by short name when none.
    pub sort: Option<(NodeSort, bool)>,
    pub limit: Option<i64>,
    pub offset: i64,
}

/// The node fields facets are counted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeFacet {
    Role,
    Channel,
    HwModel,
    Firmware,
}

impl NodeFacet {
    pub const ALL: [Self; 4] = [Self::Role, Self::Channel, Self::HwModel, Self::Firmware];

    pub fn column(self) -> &'static str {
        match self {
            Self::Role => "role",
            Self::Channel => "channel",
            Self::HwModel => "hw_model",
            Self::Firmware => "firmware",
        }
    }
}

/// A value and how many have it.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Facet {
    pub value: Option<String>,
    pub count: i64,
}

/// A row of `job_run`: one run of a daily job.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct JobRun {
    pub job: String,
    pub status: String,
    pub started_us: i64,
    pub finished_us: Option<i64>,
    /// JSON, the job's own.
    pub details: Option<String>,
    pub error: Option<String>,
}

/// A row of `packet_seen`: one gateway hearing one packet.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PacketSeen {
    pub packet_id: i64,
    pub node_id: i64,
    pub rx_time: i64,
    pub hop_limit: Option<i64>,
    pub hop_start: Option<i64>,
    pub channel: Option<String>,
    pub rx_snr: Option<f64>,
    pub rx_rssi: Option<i64>,
    pub topic: Option<String>,
    pub import_time_us: Option<i64>,
}

/// The last Alembic revision of the Python Meshview, and the only one this
/// program agrees to take a database over from. A database at an earlier one
/// has to be brought up to date by the Python version first.
pub const PYTHON_HEAD_REVISION: &str = "44612e74a798";

/// What a database holds, as far as its schema goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schema {
    /// Nothing at all: a new file, about to be built from scratch.
    Empty,
    /// Created by the Python Meshview and not yet adopted, at this Alembic
    /// revision.
    Python { revision: String },
    /// Already ours, with these migrations applied, in order.
    Migrated { applied: Vec<i64> },
    /// Tables, but neither ours nor Python's bookkeeping. Not something to write
    /// over.
    Unknown,
}

/// How a database's schema compares with what this build expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility {
    /// Exactly what this build was written against.
    Current,
    /// Not there yet. A writer would migrate it; a reader waits for that.
    Behind,
    /// Migrated by a newer build than this one, which this one cannot be trusted
    /// to understand.
    Ahead { latest_known: i64, found: i64 },
    /// Something that cannot become current by migrating, and why.
    Incompatible(String),
}

impl Schema {
    /// Compares this schema with the migrations a build carries.
    pub fn compatibility(&self, known: &[i64]) -> Compatibility {
        let latest_known = known.last().copied().unwrap_or(0);
        match self {
            Self::Empty => Compatibility::Behind,
            Self::Python { revision } if revision == PYTHON_HEAD_REVISION => Compatibility::Behind,
            Self::Python { revision } => Compatibility::Incompatible(format!(
                "this database is at the Python Meshview's Alembic revision {revision}, and \
                 Meshview only takes over databases at {PYTHON_HEAD_REVISION}. Run the \
                 Python meshview 4.0 against it once so that it migrates, then try again."
            )),
            Self::Migrated { applied } => {
                let found = applied.last().copied().unwrap_or(0);
                if found > latest_known {
                    Compatibility::Ahead {
                        latest_known,
                        found,
                    }
                } else if applied.as_slice() == known {
                    Compatibility::Current
                } else {
                    Compatibility::Behind
                }
            }
            Self::Unknown => Compatibility::Incompatible(
                "this database has tables but was not created by meshview: refusing to touch it"
                    .to_string(),
            ),
        }
    }
}

impl fmt::Display for Schema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty"),
            Self::Python { revision } => write!(f, "Python meshview at Alembic {revision}"),
            Self::Migrated { applied } => match applied.last() {
                Some(last) => write!(f, "Meshview at migration {last}"),
                None => write!(f, "Meshview with no migrations applied"),
            },
            Self::Unknown => write!(f, "not a meshview database"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: &[i64] = &[1, 2];

    #[test]
    fn only_the_last_python_revision_is_taken_over() {
        let head = Schema::Python {
            revision: PYTHON_HEAD_REVISION.into(),
        };
        let older = Schema::Python {
            revision: "4d2a7c9e1b30".into(),
        };

        assert_eq!(head.compatibility(KNOWN), Compatibility::Behind);
        assert!(matches!(
            older.compatibility(KNOWN),
            Compatibility::Incompatible(_)
        ));
    }

    #[test]
    fn migrated_databases_are_compared_with_what_this_build_knows() {
        let at = |applied: &[i64]| Schema::Migrated {
            applied: applied.to_vec(),
        };

        assert_eq!(at(&[1, 2]).compatibility(KNOWN), Compatibility::Current);
        assert_eq!(at(&[1]).compatibility(KNOWN), Compatibility::Behind);
        assert_eq!(
            at(&[1, 2, 3]).compatibility(KNOWN),
            Compatibility::Ahead {
                latest_known: 2,
                found: 3
            }
        );
        assert_eq!(Schema::Empty.compatibility(KNOWN), Compatibility::Behind);
        assert!(matches!(
            Schema::Unknown.compatibility(KNOWN),
            Compatibility::Incompatible(_)
        ));
    }
}
