// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What becomes of a packet once it has been opened: the rows it adds and the
//! nodes it updates.
//!
//! This follows the Python meshview's `mqtt_store.process_envelope` closely,
//! quirks included, because the point of this stage of the port is that both
//! write the same database from the same traffic. Where the Python version does
//! something odd it is kept and said so, to be changed deliberately later rather
//! than by accident now.

use super::opened::Opened;
use anyhow::{Context, Result};
use meshview_proto::meshtastic::config::device_config::Role;
use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
use meshview_proto::meshtastic::{
    Data, HardwareModel, MapReport, MeshPacket, PortNum, Position, User,
};
use meshview_proto::wire;
use sqlx::{Sqlite, SqlitePool, Transaction};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

/// The largest envelope stored. The largest MeshPacket the firmware can encode is
/// 381 bytes, and whatever is accepted here is stored and later sent to every
/// browser, so a message no node could have produced is not stored at all.
const MAX_ENVELOPE_BYTES: usize = 1024;

/// The longest topic stored, in characters, for the same reason.
const MAX_TOPIC_CHARS: usize = 256;

/// What map reports are recorded as being sent to: they go to the broker, not to
/// any node.
const MQTT_DIRECT_NODE_ID: i64 = 1;

/// The name a node gets when the one it sent is empty or has control characters
/// in it.
const UNKNOWN_NODE_NAME: &str = "unknown name";

/// What happened to a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Stored, with everything it implies.
    Stored,
    /// Too large to be a real packet; nothing stored.
    Oversized,
    /// A packet with no id that is not a map report: there is nothing to store
    /// it under. A map report in it may still have updated its node.
    NoPacketId,
    /// The packet was stored, but the gateway that relayed it did not say who it
    /// was, so not that it was seen.
    NoGateway,
    /// Node info naming a node by an id that is not one. Packet and sighting were
    /// stored; the node was not touched.
    BadNodeInfo,
    /// Not decoded, which the reader should have made sure of.
    NotDecoded,
}

/// The writer, and what it remembers between messages.
pub struct Store {
    pool: SqlitePool,
    /// Gateways already marked as such, so the mark is written once per process
    /// rather than once per packet.
    gateways: HashSet<i64>,
}

impl Store {
    /// Takes the writer pool, and loads the gateways already marked.
    pub async fn new(pool: SqlitePool) -> Result<Self> {
        let gateways: Vec<i64> =
            sqlx::query_scalar("SELECT node_id FROM node WHERE is_mqtt_gateway = 1")
                .fetch_all(&pool)
                .await
                .context("loading the known gateways")?;

        Ok(Self {
            pool,
            gateways: gateways.into_iter().collect(),
        })
    }

    /// Stores one opened message that arrived on `topic`.
    pub async fn process(&mut self, topic: &str, opened: &Opened) -> Result<Outcome> {
        let envelope = &opened.envelope;
        let packet = opened.packet();
        let Some(PayloadVariant::Decoded(data)) = &packet.payload_variant else {
            return Ok(Outcome::NotDecoded);
        };

        let size = opened.size;
        let topic_chars = topic.chars().count();
        if size > MAX_ENVELOPE_BYTES || topic_chars > MAX_TOPIC_CHARS {
            // Sizes only: the content is whatever the sender chose.
            warn!(
                "Skipping oversized MQTT message: {size} bytes, topic of {topic_chars} characters"
            );
            return Ok(Outcome::Oversized);
        }

        // A map report updates its node in a transaction of its own, before and
        // whatever becomes of the packet, as in the Python version.
        if data.portnum == PortNum::MapReportApp as i32 {
            self.map_report(packet, data, &envelope.channel_id).await?;
        }

        let Some(packet_id) = storage_packet_id(packet, data) else {
            return Ok(Outcome::NoPacketId);
        };

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        sqlx::query(
            "INSERT INTO packet \
             (id, portnum, from_node_id, to_node_id, payload, import_time_us, channel) \
             VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT (id) DO NOTHING",
        )
        .bind(packet_id)
        .bind(data.portnum)
        .bind(i64::from(packet.from))
        .bind(storage_to_node_id(packet, data))
        .bind(&opened.packet_bytes)
        .bind(now_us())
        .bind(&envelope.channel_id)
        .execute(&mut *tx)
        .await
        .context("storing the packet")?;

        let Some(gateway) = parse_node_id(&envelope.gateway_id) else {
            // Most likely a misconfigured publisher.
            warn!(
                "Skipping PacketSeen entry: missing or malformed gateway_id {:?}",
                envelope.gateway_id
            );
            tx.commit().await?;
            return Ok(Outcome::NoGateway);
        };

        // Marked once per process, and remembered even if the node does not
        // exist yet, in which case nothing is marked until the next start loads
        // the cache from the database again. The Python version's behaviour.
        if self.gateways.insert(gateway) {
            sqlx::query("UPDATE node SET is_mqtt_gateway = 1 WHERE node_id = ?")
                .bind(gateway)
                .execute(&mut *tx)
                .await
                .context("marking the gateway")?;
        }

        sqlx::query(
            "INSERT INTO packet_seen \
             (packet_id, node_id, channel, rx_time, rx_snr, rx_rssi, hop_limit, hop_start, \
              topic, import_time_us) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (packet_id, node_id, rx_time) DO NOTHING",
        )
        .bind(packet_id)
        .bind(gateway)
        .bind(&envelope.channel_id)
        .bind(i64::from(packet.rx_time.unwrap_or(0)))
        // Not finite is not a reading, and JSON could not carry it anyway.
        .bind(
            packet
                .rx_snr
                .is_finite()
                .then_some(f64::from(packet.rx_snr)),
        )
        .bind(packet.rx_rssi.unwrap_or(0))
        .bind(i64::from(packet.hop_limit))
        .bind(i64::from(packet.hop_start))
        .bind(topic)
        .bind(now_us())
        .execute(&mut *tx)
        .await
        .context("storing the sighting")?;

        let mut outcome = Outcome::Stored;

        if data.portnum == PortNum::NodeinfoApp as i32
            && let Some(user) = wire::decode::<User>("meshtastic.User", &data.payload)
            && !user.id.is_empty()
        {
            match parse_node_id(&user.id) {
                Some(node_id) => node_info(&mut tx, &user, node_id, &envelope.channel_id).await?,
                None => {
                    warn!(
                        "Skipping NODEINFO_APP packet {}: unable to determine node_id",
                        packet.id
                    );
                    outcome = Outcome::BadNodeInfo;
                }
            }
        }

        if data.portnum == PortNum::PositionApp as i32
            && let Some(position) = wire::decode::<Position>("meshtastic.Position", &data.payload)
        {
            let (lat, lon) = (
                position.latitude_i.unwrap_or(0),
                position.longitude_i.unwrap_or(0),
            );
            if has_valid_location(lat, lon) {
                let node_id = i64::from(packet.from);
                ensure_node_exists(&mut tx, node_id, &envelope.channel_id).await?;
                sqlx::query(
                    "UPDATE node SET last_lat = ?, last_long = ?, \
                     last_altitude = COALESCE(?, last_altitude), last_seen_us = ?, \
                     first_seen_us = COALESCE(first_seen_us, ?) WHERE node_id = ?",
                )
                .bind(i64::from(lat))
                .bind(i64::from(lon))
                .bind(position.altitude.map(i64::from))
                .bind(now_us())
                .bind(now_us())
                .bind(node_id)
                .execute(&mut *tx)
                .await
                .context("storing the position")?;
            }
        }

        // One row per gateway that heard it, under the packet's own id.
        if data.portnum == PortNum::TracerouteApp as i32 {
            sqlx::query(
                "INSERT INTO traceroute \
                 (packet_id, route, done, gateway_node_id, import_time_us) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(i64::from(packet.id))
            .bind(&data.payload)
            .bind(!data.want_response)
            .bind(gateway)
            .bind(now_us())
            .execute(&mut *tx)
            .await
            .context("storing the traceroute")?;
        }

        tx.commit().await?;
        Ok(outcome)
    }

    /// A map report: the node as it describes itself to the broker.
    async fn map_report(&self, packet: &MeshPacket, data: &Data, channel: &str) -> Result<()> {
        let Some(report) = wire::decode::<MapReport>("meshtastic.MapReport", &data.payload) else {
            warn!("Error processing MAP_REPORT_APP: not a map report");
            return Ok(());
        };

        let node_id = i64::from(packet.from);
        let now = now_us();
        let located = has_valid_location(report.latitude_i, report.longitude_i);
        let (lat, lon) = if located {
            (
                Some(i64::from(report.latitude_i)),
                Some(i64::from(report.longitude_i)),
            )
        } else {
            (None, None)
        };
        // A map report has no way to say it has no altitude, so 0 is taken to
        // mean exactly that.
        let altitude = (located && report.altitude != 0).then_some(i64::from(report.altitude));
        let hw_model = hw_model_name(report.hw_model);
        let role = role_name(report.role);

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        // Names as they come, unlike node info: the Python version does not
        // clean these.
        let updated = sqlx::query(
            "UPDATE node SET long_name = ?, short_name = ?, hw_model = ?, role = ?, \
             channel = CASE WHEN channel IS NULL OR channel = '' THEN ? ELSE channel END, \
             last_lat = COALESCE(?, last_lat), last_long = COALESCE(?, last_long), \
             last_altitude = COALESCE(?, last_altitude), firmware = ?, last_seen_us = ?, \
             first_seen_us = COALESCE(first_seen_us, ?) WHERE node_id = ?",
        )
        .bind(&report.long_name)
        .bind(&report.short_name)
        .bind(&hw_model)
        .bind(&role)
        .bind(channel)
        .bind(lat)
        .bind(lon)
        .bind(altitude)
        .bind(&report.firmware_version)
        .bind(now)
        .bind(now)
        .bind(node_id)
        .execute(&mut *tx)
        .await
        .context("updating a node from its map report")?;

        if updated.rows_affected() == 0 {
            sqlx::query(
                "INSERT INTO node \
                 (id, node_id, long_name, short_name, hw_model, role, channel, firmware, \
                  first_seen_us, last_seen_us, last_lat, last_long, last_altitude) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(node_user_id(node_id))
            .bind(node_id)
            .bind(&report.long_name)
            .bind(&report.short_name)
            .bind(&hw_model)
            .bind(&role)
            .bind(channel)
            .bind(&report.firmware_version)
            .bind(now)
            .bind(now)
            .bind(lat)
            .bind(lon)
            .bind(altitude)
            .execute(&mut *tx)
            .await
            .context("adding a node from its map report")?;
        }

        tx.commit().await?;
        Ok(())
    }
}

/// Node info: the node as it describes itself on the mesh.
async fn node_info(
    tx: &mut Transaction<'_, Sqlite>,
    user: &User,
    node_id: i64,
    channel: &str,
) -> Result<()> {
    let now = now_us();
    let long_name = normalize_node_name(&user.long_name);
    let short_name = normalize_node_name(&user.short_name);
    let hw_model = hw_model_name(user.hw_model);
    let role = role_name(user.role);

    // By the id as the node spells it first, and failing that by number: the
    // same node may already be stored under a differently spelled id, such as
    // "!00abcd" and "!abcd", and node_id is unique.
    for column in ["id", "node_id"] {
        let sql = match column {
            "id" => {
                "UPDATE node SET node_id = ?, long_name = ?, short_name = ?, hw_model = ?, \
                 role = ?, channel = ?, last_seen_us = ?, \
                 first_seen_us = COALESCE(first_seen_us, ?) WHERE id = ?"
            }
            _ => {
                "UPDATE node SET node_id = ?, long_name = ?, short_name = ?, hw_model = ?, \
                 role = ?, channel = ?, last_seen_us = ?, \
                 first_seen_us = COALESCE(first_seen_us, ?) WHERE node_id = ?"
            }
        };
        let query = sqlx::query(sql)
            .bind(node_id)
            .bind(&long_name)
            .bind(&short_name)
            .bind(&hw_model)
            .bind(&role)
            .bind(channel)
            .bind(now)
            .bind(now);
        let query = if column == "id" {
            query.bind(&user.id)
        } else {
            query.bind(node_id)
        };
        let updated = query
            .execute(&mut **tx)
            .await
            .context("updating a node from its node info")?;
        if updated.rows_affected() > 0 {
            return Ok(());
        }
    }

    sqlx::query(
        "INSERT INTO node \
         (id, node_id, long_name, short_name, hw_model, role, channel, first_seen_us, \
          last_seen_us) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&user.id)
    .bind(node_id)
    .bind(&long_name)
    .bind(&short_name)
    .bind(&hw_model)
    .bind(&role)
    .bind(channel)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await
    .context("adding a node from its node info")?;

    Ok(())
}

/// Makes sure there is a row for a node about to be given a position, with a
/// placeholder name if this is the first that is heard of it.
async fn ensure_node_exists(
    tx: &mut Transaction<'_, Sqlite>,
    node_id: i64,
    channel: &str,
) -> Result<()> {
    let now = now_us();
    let updated = sqlx::query(
        "UPDATE node SET last_seen_us = ?, first_seen_us = COALESCE(first_seen_us, ?), \
         channel = CASE WHEN ? <> '' AND (channel IS NULL OR channel = '') THEN ? \
                        ELSE channel END \
         WHERE node_id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(channel)
    .bind(channel)
    .bind(node_id)
    .execute(&mut **tx)
    .await
    .context("touching a node")?;

    if updated.rows_affected() == 0 {
        let id = node_user_id(node_id);
        let short_name = id[id.len() - 4..].to_string();
        sqlx::query(
            "INSERT INTO node \
             (id, node_id, long_name, short_name, channel, first_seen_us, last_seen_us) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(node_id)
        .bind(node_id.to_string())
        .bind(short_name)
        .bind(channel)
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await
        .context("adding a node first heard of through its position")?;
    }

    Ok(())
}

/// The id a packet is stored under.
///
/// Its own, normally. A map report may have none, and gets a negative one made
/// of the time and the sender, which cannot collide with a real id and keeps
/// every report rather than only the first.
fn storage_packet_id(packet: &MeshPacket, data: &Data) -> Option<i64> {
    if packet.id != 0 {
        return Some(i64::from(packet.id));
    }
    if data.portnum != PortNum::MapReportApp as i32 {
        return None;
    }
    let from = i64::from(packet.from) & 0x7ff;
    Some(-(now_us() * 2048 + from))
}

fn storage_to_node_id(packet: &MeshPacket, data: &Data) -> i64 {
    if data.portnum == PortNum::MapReportApp as i32 {
        MQTT_DIRECT_NODE_ID
    } else {
        i64::from(packet.to)
    }
}

/// A `!deadbeef` style id as a number, or none when it is not one.
///
/// Gateway and user ids arrive over MQTT and are not to be trusted. Up to 16 hex
/// digits are accepted, which is more lenient than Meshtastic needs, as long as
/// the result fits the database's signed 64 bit columns. Like the Python regular
/// expression it replaces, a single trailing newline is tolerated.
pub fn parse_node_id(value: &str) -> Option<i64> {
    let value = value.strip_suffix('\n').unwrap_or(value);
    let digits = value.strip_prefix('!')?;
    if digits.is_empty() || digits.len() > 16 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let id = u64::from_str_radix(digits, 16).ok()?;
    i64::try_from(id).ok()
}

/// Whether a position is anywhere: the box within one degree of (0, 0) is where
/// devices without a fix say they are. Coordinates are in 1e-7 degrees.
fn has_valid_location(lat: i32, lon: i32) -> bool {
    lat.unsigned_abs() > 10_000_000 || lon.unsigned_abs() > 10_000_000
}

fn node_user_id(node_id: i64) -> String {
    format!("!{node_id:08x}")
}

/// A node's name with the surrounding space trimmed, or the placeholder when
/// nothing is left or what is left has control characters in it.
///
/// Trimmed as Python's `str.strip` trims, which counts the four ASCII
/// separators (0x1c to 0x1f) as space where Rust does not.
fn normalize_node_name(name: &str) -> String {
    let name = name.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c));
    if name.is_empty() || name.chars().any(|c| c <= '\x1f' || c == '\x7f') {
        UNKNOWN_NODE_NAME.to_string()
    } else {
        name.to_string()
    }
}

/// An enum value's name, or `unknown(N)` for one these protobufs do not know,
/// which a newer firmware or a forged packet can send.
fn hw_model_name(value: i32) -> String {
    HardwareModel::try_from(value)
        .map(|model| model.as_str_name().to_string())
        .unwrap_or_else(|_| format!("unknown({value})"))
}

fn role_name(value: i32) -> String {
    Role::try_from(value)
        .map(|role| role.as_str_name().to_string())
        .unwrap_or_else(|_| format!("unknown({value})"))
}

/// Now, in microseconds since the epoch, which is how every time is stored.
fn now_us() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since.as_micros()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_parse_as_python_parses_them() {
        assert_eq!(parse_node_id("!deadbeef"), Some(0xdead_beef));
        assert_eq!(parse_node_id("!ABCD"), Some(0xabcd));
        assert_eq!(parse_node_id("!abcd\n"), Some(0xabcd));
        assert_eq!(parse_node_id("!7fffffffffffffff"), Some(i64::MAX));
        assert_eq!(parse_node_id("!8000000000000000"), None);
        assert_eq!(parse_node_id("!12345678901234567"), None);
        assert_eq!(parse_node_id("deadbeef"), None);
        assert_eq!(parse_node_id("!"), None);
        assert_eq!(parse_node_id("!+abc"), None);
        assert_eq!(parse_node_id(""), None);
    }

    #[test]
    fn names_are_cleaned_as_python_cleans_them() {
        assert_eq!(normalize_node_name("  Ana  "), "Ana");
        assert_eq!(normalize_node_name("\x1fAna\x1c"), "Ana");
        assert_eq!(normalize_node_name("A\x07na"), UNKNOWN_NODE_NAME);
        assert_eq!(normalize_node_name("   "), UNKNOWN_NODE_NAME);
        assert_eq!(normalize_node_name("Ñandú 📡"), "Ñandú 📡");
    }

    #[test]
    fn enum_names_survive_values_from_the_future() {
        assert_eq!(hw_model_name(HardwareModel::HeltecV3 as i32), "HELTEC_V3");
        assert_eq!(role_name(Role::Router as i32), "ROUTER");
        assert_eq!(hw_model_name(99_999), "unknown(99999)");
    }

    #[test]
    fn the_null_island_box_is_nowhere() {
        assert!(!has_valid_location(0, 0));
        assert!(!has_valid_location(9_999_999, -10_000_000));
        assert!(has_valid_location(423_000_000, -86_000_000));
        assert!(has_valid_location(i32::MIN, 0));
    }
}
