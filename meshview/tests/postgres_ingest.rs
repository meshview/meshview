// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The same messages stored in SQLite and in PostgreSQL leave the same rows.
//! Needs a server: see `common::postgres`.

#![cfg(feature = "postgres")]

mod common;

use common::postgres::database;
use meshview::config::DatabaseUrl;
use meshview::db::{self, Pool};
use meshview::ingest::keys::Keyring;
use meshview::ingest::opened::Opened;
use meshview::ingest::store::{Outcome, Store};
use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
use meshview_proto::meshtastic::{
    Data, MapReport, MeshPacket, PortNum, Position, RouteDiscovery, ServiceEnvelope, User,
};
use prost::Message;
use tempfile::TempDir;

const BROADCAST: u32 = 0xffff_ffff;

/// An envelope with a packet in the clear, from a gateway, on a channel.
fn message(
    id: u32,
    from: u32,
    to: u32,
    portnum: PortNum,
    payload: Vec<u8>,
    gateway: &str,
    channel: &str,
) -> Opened {
    let envelope = ServiceEnvelope {
        packet: Some(MeshPacket {
            id,
            from,
            to,
            rx_time: Some(1_794_355_200),
            rx_snr: 6.25,
            rx_rssi: Some(-90),
            hop_limit: 2,
            hop_start: 3,
            payload_variant: Some(PayloadVariant::Decoded(Data {
                portnum: portnum as i32,
                payload,
                want_response: portnum == PortNum::TracerouteApp && id.is_multiple_of(2),
                ..Default::default()
            })),
            ..Default::default()
        }),
        channel_id: channel.into(),
        gateway_id: gateway.into(),
    };
    Opened::open(&envelope.encode_to_vec(), &Keyring::new::<&str>(&[])).unwrap()
}

fn user(id: &str, long: &str, short: &str) -> Vec<u8> {
    User {
        id: id.into(),
        long_name: long.into(),
        short_name: short.into(),
        hw_model: 43,
        role: 2,
        ..Default::default()
    }
    .encode_to_vec()
}

/// Every kind of message the store treats in its own way, in an order that
/// exercises updates as well as inserts.
fn messages() -> Vec<(&'static str, Opened)> {
    let gw = "!12345678";
    let text = |s: &str| s.as_bytes().to_vec();
    vec![
        (
            "text to all",
            message(
                1,
                0xa1,
                BROADCAST,
                PortNum::TextMessageApp,
                text("hola"),
                gw,
                "LongFast",
            ),
        ),
        (
            "text direct",
            message(
                2,
                0xa1,
                0xb2,
                PortNum::TextMessageApp,
                text("psst"),
                gw,
                "LongFast",
            ),
        ),
        (
            "same text, again",
            message(
                1,
                0xa1,
                BROADCAST,
                PortNum::TextMessageApp,
                text("hola"),
                gw,
                "LongFast",
            ),
        ),
        (
            "same text, other gateway",
            message(
                1,
                0xa1,
                BROADCAST,
                PortNum::TextMessageApp,
                text("hola"),
                "!0000abcd",
                "LongFast",
            ),
        ),
        (
            "no gateway",
            message(
                3,
                0xa1,
                BROADCAST,
                PortNum::TextMessageApp,
                text("x"),
                "",
                "LongFast",
            ),
        ),
        (
            "node info to all",
            message(
                4,
                0xa1,
                BROADCAST,
                PortNum::NodeinfoApp,
                user("!000000a1", " Alfa ", "A1"),
                gw,
                "LongFast",
            ),
        ),
        (
            "node info, directed",
            message(
                5,
                0xa1,
                0xb2,
                PortNum::NodeinfoApp,
                user("!000000a1", "Alfa", "A1"),
                gw,
                "Secondary",
            ),
        ),
        (
            "node info, respelled",
            message(
                6,
                0xa1,
                BROADCAST,
                PortNum::NodeinfoApp,
                user("!a1", "Alfa 2", "A2"),
                gw,
                "LongFast",
            ),
        ),
        (
            "node info, bad id",
            message(
                7,
                0xa1,
                BROADCAST,
                PortNum::NodeinfoApp,
                user("nope", "X", "X"),
                gw,
                "LongFast",
            ),
        ),
        (
            "node info, empty name",
            message(
                8,
                0xc3,
                BROADCAST,
                PortNum::NodeinfoApp,
                user("!000000c3", "\u{7}", ""),
                gw,
                "MediumFast",
            ),
        ),
        (
            "gateway's own node info",
            message(
                9,
                0x1234_5678,
                BROADCAST,
                PortNum::NodeinfoApp,
                user("!12345678", "Gate", "GW"),
                gw,
                "LongFast",
            ),
        ),
        (
            "gateway marked on its next packet",
            message(
                10,
                0xa1,
                BROADCAST,
                PortNum::TextMessageApp,
                text("y"),
                "!12345678",
                "LongFast",
            ),
        ),
        (
            "position of a new node",
            message(
                11,
                0xd4,
                BROADCAST,
                PortNum::PositionApp,
                Position {
                    latitude_i: Some(423_000_000),
                    longitude_i: Some(-80_000_000),
                    altitude: Some(125),
                    ..Default::default()
                }
                .encode_to_vec(),
                gw,
                "LongFast",
            ),
        ),
        (
            "position without altitude",
            message(
                12,
                0xd4,
                BROADCAST,
                PortNum::PositionApp,
                Position {
                    latitude_i: Some(424_000_000),
                    longitude_i: Some(-81_000_000),
                    ..Default::default()
                }
                .encode_to_vec(),
                gw,
                "LongFast",
            ),
        ),
        (
            "position with no fix",
            message(
                13,
                0xa1,
                BROADCAST,
                PortNum::PositionApp,
                Position {
                    latitude_i: Some(0),
                    longitude_i: Some(0),
                    ..Default::default()
                }
                .encode_to_vec(),
                gw,
                "LongFast",
            ),
        ),
        (
            "traceroute asked",
            message(
                14,
                0xa1,
                0xd4,
                PortNum::TracerouteApp,
                RouteDiscovery {
                    route: vec![0xc3],
                    ..Default::default()
                }
                .encode_to_vec(),
                gw,
                "LongFast",
            ),
        ),
        (
            "traceroute answered",
            message(
                15,
                0xd4,
                0xa1,
                PortNum::TracerouteApp,
                RouteDiscovery {
                    route: vec![0xc3],
                    route_back: vec![0xc3],
                    ..Default::default()
                }
                .encode_to_vec(),
                gw,
                "LongFast",
            ),
        ),
        (
            "map report with an id",
            message(
                16,
                0xe5,
                BROADCAST,
                PortNum::MapReportApp,
                MapReport {
                    long_name: "Eco".into(),
                    short_name: "E".into(),
                    firmware_version: "2.7.15".into(),
                    latitude_i: 425_000_000,
                    longitude_i: -82_000_000,
                    altitude: 300,
                    position_precision: 13,
                    ..Default::default()
                }
                .encode_to_vec(),
                gw,
                "LongFast",
            ),
        ),
        (
            "map report of a known node",
            message(
                17,
                0xa1,
                BROADCAST,
                PortNum::MapReportApp,
                MapReport {
                    long_name: "Alfa map".into(),
                    short_name: "AM".into(),
                    firmware_version: "2.7.16".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
                gw,
                "LongFast",
            ),
        ),
        (
            "map report without an id",
            message(
                0,
                0xe5,
                BROADCAST,
                PortNum::MapReportApp,
                MapReport {
                    long_name: "Eco".into(),
                    short_name: "E".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
                gw,
                "LongFast",
            ),
        ),
    ]
}

/// What the store leaves, as text the two databases can be compared by, the
/// import times left out: they are whenever the test ran.
async fn rows(pool: &Pool) -> Vec<String> {
    let hex = |column: &str| {
        let bytes = match pool {
            Pool::Sqlite(_) => format!("lower(hex({column}))"),
            Pool::Postgres(_) => format!("encode({column}, 'hex')"),
        };
        format!("CASE WHEN {column} IS NULL THEN 'null' ELSE {bytes} END")
    };
    let flag = |column: &str| {
        format!("CASE WHEN {column} IS NULL THEN 'null' WHEN {column} THEN 'true' ELSE 'false' END")
    };
    let text = |column: &str| format!("coalesce(CAST({column} AS TEXT), 'null')");
    let tables = [
        (
            "packet",
            // Map reports without an id get one made of the time.
            vec![
                "CASE WHEN id < 0 THEN 'made' ELSE CAST(id AS TEXT) END".to_string(),
                text("portnum"),
                text("from_node_id"),
                text("to_node_id"),
                hex("payload"),
                text("channel"),
            ],
        ),
        (
            "packet_seen",
            vec![
                "CASE WHEN packet_id < 0 THEN 'made' ELSE CAST(packet_id AS TEXT) END".to_string(),
                text("node_id"),
                text("rx_time"),
                text("hop_limit"),
                text("hop_start"),
                text("channel"),
                text("rx_snr"),
                text("rx_rssi"),
                text("topic"),
            ],
        ),
        (
            "traceroute",
            vec![
                text("packet_id"),
                text("gateway_node_id"),
                flag("done"),
                hex("route"),
                hex("route_return"),
            ],
        ),
        (
            "node",
            vec![
                text("id"),
                text("node_id"),
                text("long_name"),
                text("short_name"),
                text("hw_model"),
                text("firmware"),
                text("role"),
                text("last_lat"),
                text("last_long"),
                text("last_altitude"),
                text("channel"),
                flag("is_mqtt_gateway"),
                "CASE WHEN first_seen_us IS NULL THEN 'never' ELSE 'seen' END".to_string(),
                "CASE WHEN last_seen_us IS NULL THEN 'never' ELSE 'seen' END".to_string(),
            ],
        ),
    ];
    let mut out = Vec::new();
    for (table, columns) in tables {
        let sql = format!(
            "SELECT '{table}', {} AS line FROM {table}",
            columns.join(" || '|' || "),
        );
        let lines: Vec<(String, String)> = match pool {
            Pool::Sqlite(pool) => sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .fetch_all(pool)
                .await
                .unwrap(),
            Pool::Postgres(pool) => sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .fetch_all(pool)
                .await
                .unwrap(),
        };
        // Sorted here, by bytes: in SQL each database sorts text its own way.
        let mut lines: Vec<String> = lines
            .into_iter()
            .map(|(table, line)| format!("{table}: {line}"))
            .collect();
        lines.sort();
        out.extend(lines);
    }
    out
}

async fn store_all(pool: Pool) -> (Vec<Outcome>, Vec<String>) {
    let mut store = Store::new(pool.clone()).await.unwrap();
    let mut outcomes = Vec::new();
    for (what, opened) in messages() {
        let topic = "msh/EU_868/2/e/LongFast/!12345678";
        outcomes.push(
            store
                .process(topic, &opened)
                .await
                .unwrap_or_else(|e| panic!("{what}: {e:#}")),
        );
    }
    (outcomes, rows(&pool).await)
}

#[tokio::test]
async fn the_same_messages_leave_the_same_rows_in_either_database() {
    let Some(postgres) = database(None).await else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let sqlite = db::open_for_writing(&DatabaseUrl::Sqlite(dir.path().join("packets.db")))
        .await
        .unwrap();
    let pg = db::open_for_writing(&DatabaseUrl::Postgres(postgres.url.clone()))
        .await
        .unwrap();

    let (sqlite_outcomes, sqlite_rows) = store_all(sqlite).await;
    let (pg_outcomes, pg_rows) = store_all(pg.clone()).await;

    assert_eq!(sqlite_outcomes, pg_outcomes);
    assert!(sqlite_outcomes.contains(&Outcome::NoGateway));
    assert!(sqlite_outcomes.contains(&Outcome::BadNodeInfo));
    assert_eq!(sqlite_rows, pg_rows);
    // Every table has something in it, or the comparison proves little.
    for table in ["packet:", "packet_seen:", "traceroute:", "node:"] {
        assert!(
            sqlite_rows.iter().any(|row| row.starts_with(table)),
            "no {table}"
        );
    }

    db::close_writer(pg).await;
    postgres.drop().await;
}
