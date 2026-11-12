// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The API answers the same on SQLite and on PostgreSQL, byte for byte, from
//! the same rows. Needs a server: see `common::postgres`.
//!
//! The rows are what the ingester leaves from real messages of every kind, and
//! written directly, what makes the two databases differ if nothing is done
//! about it: NULLs everywhere, names that sort differently by bytes and by
//! language, ties in time, and payloads with zero bytes in them.

#![cfg(feature = "postgres")]
// The rows are written as tables of tuples, one line each, to be read as rows.
#![allow(clippy::type_complexity)]

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use common::postgres::database;
use http_body_util::BodyExt;
use meshview::app::Role;
use meshview::config::{Config, DatabaseUrl};
use meshview::db::write::{self, Arg};
use meshview::db::{self, Pool, Reader};
use meshview::ingest::keys::Keyring;
use meshview::ingest::opened::Opened;
use meshview::ingest::store::Store;
use meshview::state::{AppState, CHECK_DATABASE, Readiness};
use meshview::web;
use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
use meshview_proto::meshtastic::telemetry::Variant;
use meshview_proto::meshtastic::{
    Data, DeviceMetrics, MapReport, MeshPacket, Neighbor, NeighborInfo, PortNum, Position,
    RouteDiscovery, ServiceEnvelope, Telemetry, User,
};
use prost::Message;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

const BROADCAST: u32 = 0xffff_ffff;
const HOUR_US: i64 = 3_600_000_000;

fn now_us() -> i64 {
    jiff::Timestamp::now().as_microsecond()
}

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
            rx_time: Some(1_794_355_200 + id),
            rx_snr: 6.25,
            rx_rssi: Some(-90),
            hop_limit: 2,
            hop_start: 3,
            payload_variant: Some(PayloadVariant::Decoded(Data {
                portnum: portnum as i32,
                payload,
                ..Default::default()
            })),
            ..Default::default()
        }),
        channel_id: channel.into(),
        gateway_id: gateway.into(),
    };
    Opened::open(&envelope.encode_to_vec(), &Keyring::new::<&str>(&[])).unwrap()
}

fn user(id: &str, long: &str, short: &str, key: &[u8]) -> Vec<u8> {
    User {
        id: id.into(),
        long_name: long.into(),
        short_name: short.into(),
        hw_model: 43,
        role: 2,
        public_key: key.to_vec(),
        ..Default::default()
    }
    .encode_to_vec()
}

fn messages() -> Vec<Opened> {
    let (a, b) = ("!12345678", "!0000abcd");
    let text = |s: &str| s.as_bytes().to_vec();
    let position = |lat, lon, bits| {
        Position {
            latitude_i: Some(lat),
            longitude_i: Some(lon),
            altitude: Some(120),
            precision_bits: bits,
            ..Default::default()
        }
        .encode_to_vec()
    };
    let telemetry = |channel, air| {
        Telemetry {
            variant: Some(Variant::DeviceMetrics(DeviceMetrics {
                channel_utilization: Some(channel),
                air_util_tx: Some(air),
                battery_level: Some(80),
                ..Default::default()
            })),
            ..Default::default()
        }
        .encode_to_vec()
    };
    vec![
        message(
            1,
            0xa1,
            BROADCAST,
            PortNum::NodeinfoApp,
            user("!000000a1", "Alfa Ñandú", "alfa", &[7; 32]),
            a,
            "LongFast",
        ),
        message(
            2,
            0xb2,
            BROADCAST,
            PortNum::NodeinfoApp,
            user("!000000b2", "bravo", "Bravo", &[]),
            a,
            "MediumFast",
        ),
        message(
            3,
            0x1234_5678,
            BROADCAST,
            PortNum::NodeinfoApp,
            user("!12345678", "Gate", "GW", &[]),
            a,
            "LongFast",
        ),
        message(
            4,
            0xa1,
            BROADCAST,
            PortNum::TextMessageApp,
            text("hola a todos"),
            a,
            "LongFast",
        ),
        message(
            4,
            0xa1,
            BROADCAST,
            PortNum::TextMessageApp,
            text("hola a todos"),
            b,
            "LongFast",
        ),
        message(
            5,
            0xb2,
            0xa1,
            PortNum::TextMessageApp,
            text("secreto"),
            a,
            "MediumFast",
        ),
        message(
            6,
            0xa1,
            BROADCAST,
            PortNum::PositionApp,
            position(423_000_000, -80_000_000, 13),
            a,
            "LongFast",
        ),
        message(
            7,
            0xb2,
            BROADCAST,
            PortNum::PositionApp,
            position(424_000_000, -81_000_000, 32),
            b,
            "MediumFast",
        ),
        message(
            8,
            0xa1,
            BROADCAST,
            PortNum::TelemetryApp,
            telemetry(18.3, 2.25),
            a,
            "LongFast",
        ),
        message(
            9,
            0xb2,
            BROADCAST,
            PortNum::TelemetryApp,
            telemetry(41.0, 9.5),
            a,
            "MediumFast",
        ),
        message(
            10,
            0xa1,
            0xb2,
            PortNum::TracerouteApp,
            RouteDiscovery {
                route: vec![0x1234_5678],
                ..Default::default()
            }
            .encode_to_vec(),
            a,
            "LongFast",
        ),
        message(
            11,
            0xb2,
            0xa1,
            PortNum::TracerouteApp,
            RouteDiscovery {
                route: vec![0x1234_5678],
                route_back: vec![0x1234_5678],
                ..Default::default()
            }
            .encode_to_vec(),
            a,
            "LongFast",
        ),
        message(
            12,
            0xa1,
            BROADCAST,
            PortNum::NeighborinfoApp,
            NeighborInfo {
                node_id: 0xa1,
                neighbors: vec![Neighbor {
                    node_id: 0xb2,
                    snr: 5.0,
                    ..Default::default()
                }],
                ..Default::default()
            }
            .encode_to_vec(),
            a,
            "LongFast",
        ),
        message(
            13,
            0xc3,
            BROADCAST,
            PortNum::MapReportApp,
            MapReport {
                long_name: "Charlie".into(),
                short_name: "ch".into(),
                firmware_version: "2.7.15".into(),
                latitude_i: 425_000_000,
                longitude_i: -82_000_000,
                altitude: 300,
                position_precision: 15,
                ..Default::default()
            }
            .encode_to_vec(),
            a,
            "Galicia",
        ),
        message(
            14,
            0xa1,
            BROADCAST,
            PortNum::TextMessageApp,
            text("Buenos días, red"),
            b,
            "LongFast",
        ),
    ]
}

/// The rows the ingester would not leave, written as they are.
async fn rows(pool: &Pool, now: i64) {
    let mut tx = write::begin(pool).await.unwrap();
    let nodes: [(
        &str,
        i64,
        Option<&str>,
        Option<&str>,
        Option<&str>,
        Option<&str>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    ); 9] = [
        (
            "!00000100",
            256,
            Some("Éco"),
            Some("éco"),
            Some("ROUTER"),
            Some("LongFast"),
            Some(now - HOUR_US),
            Some(1),
            Some(1),
        ),
        (
            "!00000101",
            257,
            Some("eco"),
            Some("eco"),
            Some("CLIENT"),
            None,
            Some(now - 2 * HOUR_US),
            None,
            Some(0),
        ),
        (
            "!00000102",
            258,
            None,
            None,
            None,
            Some("LongFast"),
            Some(now - 3 * HOUR_US),
            None,
            None,
        ),
        (
            "!00000103",
            259,
            Some("zulu"),
            Some("Zulu"),
            Some("CLIENT_MUTE"),
            Some("ShortFast"),
            Some(now - 40 * 24 * HOUR_US),
            Some(0),
            Some(0),
        ),
        (
            "!00000104",
            260,
            Some("_under"),
            Some("_u"),
            Some("ROUTER_LATE"),
            Some("Galicia"),
            Some(now - HOUR_US),
            Some(1),
            None,
        ),
        (
            "!00000105",
            261,
            Some("100% cobertura"),
            Some("%"),
            Some("CLIENT"),
            Some("LongFast"),
            Some(now - HOUR_US),
            Some(0),
            Some(1),
        ),
        (
            "!00000106",
            262,
            Some("Ñu"),
            Some("ñu"),
            Some("CLIENT"),
            Some("MediumFast"),
            Some(now - HOUR_US),
            None,
            None,
        ),
        (
            "!00000107",
            263,
            Some("aardvark"),
            None,
            Some("TRACKER"),
            Some("LongFast"),
            Some(now - HOUR_US),
            Some(0),
            None,
        ),
        (
            "!00000108",
            264,
            Some("never"),
            Some("nv"),
            None,
            None,
            None,
            None,
            None,
        ),
    ];
    for (id, node_id, long, short, role, channel, seen, gateway, located) in nodes {
        tx.execute(
            "INSERT INTO node (id, node_id, long_name, short_name, role, channel, last_seen_us, \
             first_seen_us, is_mqtt_gateway, last_lat, last_long, hw_model, firmware) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                Arg::Text(id),
                Arg::Int(node_id),
                Arg::OptText(long),
                Arg::OptText(short),
                Arg::OptText(role),
                Arg::OptText(channel),
                Arg::OptInt(seen),
                Arg::OptInt(seen.map(|seen| seen - HOUR_US)),
                match gateway {
                    Some(flag) => Arg::Bool(flag == 1),
                    None => Arg::OptInt(None),
                },
                Arg::OptInt(located.map(|on| on * 421_000_000)),
                Arg::OptInt(located.map(|on| on * -79_000_000)),
                Arg::OptText(if node_id % 2 == 0 {
                    Some("RAK4631")
                } else {
                    None
                }),
                Arg::OptText(if node_id % 3 == 0 {
                    Some("2.6.11")
                } else {
                    Some("2.7.15")
                }),
            ],
        )
        .await
        .unwrap();
    }
    // Ties in time, no time, no channel, no recipient, and zero bytes before
    // the text a search looks for.
    let at = now - 5 * HOUR_US;
    let packets: [(
        i64,
        Option<i64>,
        i64,
        Option<i64>,
        &[u8],
        Option<i64>,
        Option<&str>,
    ); 6] = [
        (
            900,
            Some(67),
            256,
            Some(BROADCAST.into()),
            b"\x0d\x00\x01\x00\x00MESHTASTIC",
            Some(at),
            Some("LongFast"),
        ),
        (
            901,
            Some(67),
            257,
            Some(BROADCAST.into()),
            b"\x00\x00mesh",
            Some(at),
            Some("longfast"),
        ),
        (902, Some(4), 258, None, b"\x00", Some(at), None),
        (903, Some(3), 256, Some(1), b"pos", None, Some("LongFast")),
        (
            904,
            None,
            259,
            Some(256),
            b"",
            Some(at - HOUR_US),
            Some("ShortFast"),
        ),
        (
            905,
            Some(73),
            260,
            Some(1),
            b"Galicia map",
            Some(at),
            Some("Galicia"),
        ),
    ];
    for (id, portnum, from, to, payload, time, channel) in packets {
        tx.execute(
            "INSERT INTO packet (id, portnum, from_node_id, to_node_id, payload, import_time_us, \
             channel) VALUES (?, ?, ?, ?, ?, ?, ?)",
            &[
                Arg::Int(id),
                Arg::OptInt(portnum),
                Arg::Int(from),
                Arg::OptInt(to),
                Arg::Bytes(payload),
                Arg::OptInt(time),
                Arg::OptText(channel),
            ],
        )
        .await
        .unwrap();
    }
    for (packet, node, time) in [
        (900, 0x1234_5678, Some(at)),
        (900, 256, Some(at)),
        (901, 256, None),
        (904, 257, Some(at)),
    ] {
        tx.execute(
            "INSERT INTO packet_seen (packet_id, node_id, rx_time, import_time_us, channel) \
             VALUES (?, ?, 0, ?, 'LongFast')",
            &[Arg::Int(packet), Arg::Int(node), Arg::OptInt(time)],
        )
        .await
        .unwrap();
    }
    for (day, nodes) in [("2026-11-01", 10), ("2026-11-02", 12), ("2026-11-03", 9)] {
        tx.execute_either(
            "INSERT INTO daily_snapshot VALUES (?, ?, 100, 3, 1)",
            "INSERT INTO daily_snapshot VALUES (CAST(? AS DATE), ?, 100, 3, 1)",
            &[Arg::Text(day), Arg::Int(nodes)],
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}

async fn fill(pool: &Pool, now: i64) {
    let mut store = Store::new(pool.clone()).await.unwrap();
    for opened in messages() {
        store
            .process("msh/EU_868/2/e/LongFast/!12345678", &opened)
            .await
            .unwrap();
    }
    rows(pool, now).await;
}

fn router(pool: &Pool) -> Router {
    let state = AppState::new(
        Arc::new(Readiness::new([CHECK_DATABASE])),
        Config::default(),
    );
    state.reader.set(Reader::new(pool.clone())).unwrap();
    state.readiness.set(CHECK_DATABASE, true);
    web::router(Role::Serve, state)
}

/// An answer with the times the ingester took left out, as they are whenever
/// each database was filled; anything else as it came.
fn comparable(body: &[u8]) -> String {
    fn strip(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map.iter_mut() {
                    if key.ends_with("_us") || key == "latest_import_time" || key == "next_before" {
                        *value = serde_json::Value::Null;
                    } else {
                        strip(value);
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(mut value) => {
            strip(&mut value);
            value.to_string()
        }
        Err(_) => String::from_utf8_lossy(body).into_owned(),
    }
}

async fn get(app: &Router, uri: &str) -> (u16, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    (
        status,
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}

const URIS: &[&str] = &[
    "/api/v1/channels",
    "/api/v1/channels?period_type=day&length=30",
    "/api/v1/snapshots/daily",
    "/api/v1/snapshots/daily?days=2",
    "/api/v1/packets_seen/4",
    "/api/v1/packets_seen/900",
    "/api/v1/stats",
    "/api/v1/stats?period_type=day&length=14",
    "/api/v1/stats?period_type=hour&length=24&portnum=1",
    "/api/v1/stats?period_type=hour&length=48&channel=LONGFAST",
    "/api/v1/stats?period_type=day&length=7&from_node=161",
    "/api/v1/stats?period_type=day&length=7&to_node=256",
    "/api/v1/stats/count",
    "/api/v1/stats/count?period_type=day&length=3",
    "/api/v1/stats/count?packet_id=900",
    "/api/v1/stats/count?channel=longfast",
    "/api/v1/stats/channels",
    "/api/v1/stats/channels?period_type=day&length=30",
    "/api/v1/stats/top",
    "/api/v1/stats/top?limit=3&offset=1",
    "/api/v1/stats/top?channel=LongFast&active=false",
    "/api/v1/nodes",
    "/api/v1/nodes?days_active=3",
    "/api/v1/nodes?sort=short_name",
    "/api/v1/nodes?sort=-short_name&limit=4&offset=2",
    "/api/v1/nodes?sort=long_name",
    "/api/v1/nodes?sort=-long_name",
    "/api/v1/nodes?sort=hw_model",
    "/api/v1/nodes?sort=-firmware",
    "/api/v1/nodes?sort=role",
    "/api/v1/nodes?sort=channel",
    "/api/v1/nodes?sort=-channel",
    "/api/v1/nodes?sort=last_lat",
    "/api/v1/nodes?sort=-last_altitude",
    "/api/v1/nodes?sort=is_mqtt_gateway",
    "/api/v1/nodes?sort=-is_mqtt_gateway",
    "/api/v1/nodes?sort=-last_seen_us&limit=3",
    "/api/v1/nodes?sort=first_seen_us",
    "/api/v1/nodes?sort=node_id&limit=2&offset=3",
    "/api/v1/nodes?search=eco",
    "/api/v1/nodes?search=%C3%89CO",
    "/api/v1/nodes?search=_",
    "/api/v1/nodes?search=100%25",
    "/api/v1/nodes?search=00000a",
    "/api/v1/nodes?role=router",
    "/api/v1/nodes?channel=LongFast",
    "/api/v1/nodes?hw_model=RAK4631",
    "/api/v1/nodes?firmware=2.6.11",
    "/api/v1/nodes?is_mqtt_gateway=true",
    "/api/v1/nodes?is_mqtt_gateway=false",
    "/api/v1/nodes?has_position=true",
    "/api/v1/nodes?has_position=false",
    "/api/v1/nodes?ids=161,178,256,999",
    "/api/v1/nodes?ids=",
    "/api/v1/nodes?node_id=x",
    "/api/v1/nodes?days_active=3&has_position=true&fields=position_precision",
    "/api/v1/nodes?fields=channel_utilization,air_util_tx,short_name",
    "/api/v1/nodes/facets",
    "/api/v1/nodes/facets?is_mqtt_gateway=true",
    "/api/v1/nodes/facets?days_active=7",
    "/api/v1/packets",
    "/api/v1/packets?limit=3",
    "/api/v1/packets?portnum=1",
    "/api/v1/packets?portnum=67",
    "/api/v1/packets?node_id=256",
    "/api/v1/packets?from_node_id=161&portnum=3",
    "/api/v1/packets?to_node_id=161",
    "/api/v1/packets?channel=LongFast",
    "/api/v1/packets?contains=mesh",
    "/api/v1/packets?contains=MESHTASTIC",
    "/api/v1/packets?contains=hola",
    "/api/v1/packets?contains=%25",
    "/api/v1/packets?portnum=1&contains=buenos",
    "/api/v1/packets?packet_id=4",
    "/api/v1/traceroute/10",
    "/api/v1/traceroute/11",
    "/api/v1/edges",
    "/api/v1/edges?type=neighbor",
    "/api/v1/edges?type=traceroute",
    "/api/v1/node/161/qr",
    "/api/v1/node/178/qr",
    "/api/v1/coverage/256",
    "/api/v1/version",
    "/node/161",
    "/node/256",
];

#[tokio::test]
async fn the_api_answers_the_same_on_either_database() {
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
    let now = now_us();
    fill(&sqlite, now).await;
    fill(&pg, now).await;

    let (on_sqlite, on_pg) = (router(&sqlite), router(&pg));
    let mut different = Vec::new();
    let mut answered = 0;
    for uri in URIS {
        let (a, b) = (get(&on_sqlite, uri).await, get(&on_pg, uri).await);
        answered += usize::from(a.0 == 200);
        let (a, b) = ((a.0, comparable(&a.1)), (b.0, comparable(&b.1)));
        if a != b {
            different.push(format!(
                "{uri}\n  sqlite   {} {}\n  postgres {} {}",
                a.0,
                a.1.chars().take(600).collect::<String>(),
                b.0,
                b.1.chars().take(600).collect::<String>(),
            ));
        }
    }
    assert!(different.is_empty(), "{}", different.join("\n"));
    // Most have something to answer, or the comparison proves little.
    // The contact of a node with a key, and one without.
    assert_eq!(get(&on_pg, "/api/v1/node/161/qr").await.0, 200);
    assert!(
        answered > URIS.len() * 9 / 10,
        "only {answered} of {} answered 200",
        URIS.len()
    );

    db::close_writer(pg).await;
    db::close_writer(sqlite).await;
    postgres.drop().await;
}
