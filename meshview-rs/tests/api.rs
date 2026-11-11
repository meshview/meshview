// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The API endpoints, against a small database. That they answer what the
//! Python version answers is checked against it with scripts/compare-api.py;
//! these keep them answering it.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use meshview_rs::app::Role;
use meshview_rs::config::Config;
use meshview_rs::db::{Reader, sqlite};
use meshview_rs::state::{AppState, CHECK_DATABASE, Readiness};
use meshview_rs::web;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

struct Api {
    app: Router,
    pool: SqlitePool,
    _dir: TempDir,
}

async fn api() -> Api {
    let dir = TempDir::new().unwrap();
    let pool = sqlite::open_for_writing(&dir.path().join("packets.db"))
        .await
        .unwrap();
    let reader = Reader::new(pool.clone());
    let state = AppState::new(
        Arc::new(Readiness::new([CHECK_DATABASE])),
        Config::default(),
    );
    state.reader.set(reader).unwrap();
    Api {
        app: web::router(Role::Serve, state),
        pool,
        _dir: dir,
    }
}

impl Api {
    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let response = self
            .app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn exec(&self, sql: &'static str) {
        sqlx::query(sql).execute(&self.pool).await.unwrap();
    }
}

fn now_us() -> i64 {
    jiff::Timestamp::now().as_microsecond()
}

#[tokio::test]
async fn version_says_which_build_this_is() {
    let api = api().await;
    let (status, body) = api.get("/version").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    for key in ["release_date", "git_revision", "git_revision_short"] {
        assert!(body[key].is_string(), "{key}");
    }
}

#[tokio::test]
async fn channels_are_those_heard_in_the_window() {
    let api = api().await;
    let now = now_us();
    for (id, channel, ago_hours) in [(1, "MediumFast", 1), (2, "LongFast", 2), (3, "Old", 30)] {
        sqlx::query("INSERT INTO packet (id, channel, import_time_us) VALUES (?, ?, ?)")
            .bind(id)
            .bind(channel)
            .bind(now - ago_hours * 3_600_000_000_i64)
            .execute(&api.pool)
            .await
            .unwrap();
    }

    assert_eq!(
        api.get("/api/v1/channels").await,
        (
            StatusCode::OK,
            json!({"channels": ["LongFast", "MediumFast"]})
        )
    );
    assert_eq!(
        api.get("/api/v1/channels?period_type=day&length=2").await.1,
        json!({"channels": ["LongFast", "MediumFast", "Old"]})
    );
    assert_eq!(
        api.get("/api/v1/channels?period_type=week").await,
        (
            StatusCode::OK,
            json!({"channels": [], "error": "period_type must be 'hour' or 'day'"})
        )
    );
    assert_eq!(
        api.get("/api/v1/channels?length=x").await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "length must be an integer"})
        )
    );
}

#[tokio::test]
async fn daily_snapshots_come_oldest_first() {
    let api = api().await;
    api.exec(
        "INSERT INTO daily_snapshot VALUES ('2026-11-09', 1, 10, 2, 100), \
         ('2026-11-10', 3, 30, 4, 200), ('2026-11-08', 5, 50, 6, 50)",
    )
    .await;

    let (_, all) = api.get("/api/v1/snapshots/daily").await;
    let dates: Vec<&str> = all["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|day| day["date"].as_str().unwrap())
        .collect();
    assert_eq!(dates, ["2026-11-08", "2026-11-09", "2026-11-10"]);

    let (_, latest) = api.get("/api/v1/snapshots/daily?length=2").await;
    assert_eq!(
        latest["data"],
        json!([
            {"date": "2026-11-09", "node_count": 1, "packet_count": 10, "gateway_count": 2, "captured_at_us": 100},
            {"date": "2026-11-10", "node_count": 3, "packet_count": 30, "gateway_count": 4, "captured_at_us": 200},
        ])
    );

    // Present and empty is not absent, for this one parameter.
    assert_eq!(
        api.get("/api/v1/snapshots/daily?length=").await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn sightings_come_latest_first() {
    let api = api().await;
    api.exec(
        "INSERT INTO packet_seen (packet_id, node_id, rx_time, hop_limit, hop_start, channel, \
         rx_snr, rx_rssi, topic, import_time_us) VALUES \
         (7, 20, 100, 3, 3, 'LongFast', 6.25, -90, 'msh/a', 1000), \
         (7, 21, 101, 2, 3, 'LongFast', NULL, -95, 'msh/b', 2000)",
    )
    .await;

    let (status, body) = api.get("/api/v1/packets_seen/7").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"seen": [
            {"packet_id": 7, "node_id": 21, "rx_time": 101, "hop_limit": 2, "hop_start": 3,
             "channel": "LongFast", "rx_snr": null, "rx_rssi": -95, "topic": "msh/b",
             "import_time_us": 2000},
            {"packet_id": 7, "node_id": 20, "rx_time": 100, "hop_limit": 3, "hop_start": 3,
             "channel": "LongFast", "rx_snr": 6.25, "rx_rssi": -90, "topic": "msh/a",
             "import_time_us": 1000},
        ]})
    );

    assert_eq!(
        api.get("/api/v1/packets_seen/8").await.1,
        json!({"seen": []})
    );
    assert_eq!(
        api.get("/api/v1/packets_seen/x").await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "Invalid or missing packet_id"})
        )
    );
}

#[tokio::test]
async fn unknown_api_addresses_are_json_404s() {
    let api = api().await;
    assert_eq!(
        api.get("/api/v1/nothing").await,
        (StatusCode::NOT_FOUND, json!({"error": "Not found"}))
    );
}

#[tokio::test]
async fn large_answers_are_compressed_for_clients_that_take_it() {
    let api = api().await;
    for day in 1..=28 {
        sqlx::query("INSERT INTO daily_snapshot VALUES (?, 1, 1, 1, 1)")
            .bind(format!("2026-10-{day:02}"))
            .execute(&api.pool)
            .await
            .unwrap();
    }

    let get = |encoding: &'static str| {
        api.app.clone().oneshot(
            Request::get("/api/v1/snapshots/daily")
                .header(header::ACCEPT_ENCODING, encoding)
                .body(Body::empty())
                .unwrap(),
        )
    };

    let gzipped = get("gzip").await.unwrap();
    assert_eq!(gzipped.headers()[header::CONTENT_ENCODING], "gzip");
    let refused = get("gzip;q=0").await.unwrap();
    assert!(refused.headers().get(header::CONTENT_ENCODING).is_none());
}

async fn api_with(config: Config) -> Api {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("packets.db");
    let pool = sqlite::open_for_writing(&database).await.unwrap();
    let reader = Reader::new(pool.clone());
    let mut config = config;
    config.database.connection_string = format!("sqlite:///{}", database.display());
    let state = AppState::new(Arc::new(Readiness::new([CHECK_DATABASE])), config);
    state.reader.set(reader).unwrap();
    Api {
        app: web::router(Role::Serve, state),
        pool,
        _dir: dir,
    }
}

#[tokio::test]
async fn the_site_configuration_keeps_the_python_shapes() {
    let config = Config::parse(include_str!("../../sample.config.toml")).unwrap();
    let api = api_with(config).await;
    let (status, body) = api.get("/api/v1/config").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["site"]["title"], "Bay Area Mesh");
    // Switches are strings, as the pages expect them.
    assert_eq!(body["site"]["nodes"], "true");
    assert_eq!(body["site"]["map_tile_name"], "OpenStreetMap");
    assert_eq!(body["site"]["map_channel_colors"]["longfast"], "#4363d8");
    assert_eq!(body["mqtt"]["server"], "mqtt.bayme.sh");
    assert!(body["mqtt"].get("password").is_none(), "no credentials");
    assert_eq!(
        body["cleanup"],
        json!({"enabled": "false", "days_to_keep": "14", "hour": "2", "minute": "0", "vacuum": "false"})
    );
}

#[tokio::test]
async fn translations_fall_back_to_english() {
    let api = api().await;

    let (status, spanish) = api.get("/api/v1/lang?lang=es&section=top").await;
    assert_eq!(status, StatusCode::OK);
    assert!(spanish.is_object());

    let (_, unknown) = api.get("/api/v1/lang?lang=fr&section=top").await;
    let (_, english) = api.get("/api/v1/lang?lang=en&section=top").await;
    assert_eq!(unknown, english);

    assert_eq!(
        api.get("/api/v1/lang?lang=fr&section=nothing").await,
        (
            StatusCode::NOT_FOUND,
            json!({"error": "Section 'nothing' not found in fr"})
        )
    );
}

#[tokio::test]
async fn health_reports_the_database_and_the_last_cleanup() {
    let mut config = Config::default();
    config.cleanup.enabled = true;
    let api = api_with(config).await;
    // One packet old enough to go, one recent enough to stay.
    api.exec("INSERT INTO packet (id, import_time_us) VALUES (1, 1)")
        .await;
    sqlx::query("INSERT INTO packet (id, import_time_us) VALUES (2, ?)")
        .bind(now_us() - 5_000_000)
        .execute(&api.pool)
        .await
        .unwrap();
    meshview_rs::jobs::cleanup(&api.pool, 14, false)
        .await
        .unwrap();

    let (status, body) = api.get("/health").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "healthy");
    assert_eq!(body["database"], "connected");
    assert!(body["seconds_since_last_message"].as_f64().unwrap() > 0.0);
    assert!(body["database_size_bytes"].as_u64().unwrap() > 0);
    assert_eq!(body["cleanup"]["status"], "ok");
    assert_eq!(body["cleanup"]["last_run"]["rows_deleted"]["packet"], 1);
    assert!(
        body["cleanup"]["last_run"]["cutoff_at"]
            .as_str()
            .unwrap()
            .ends_with("+00:00")
    );
    assert_eq!(
        body["backup"],
        json!({"enabled": false, "status": "disabled"})
    );
}

#[tokio::test]
async fn health_is_503_while_the_database_is_not_ready() {
    let state = AppState::new(
        Arc::new(Readiness::new([CHECK_DATABASE])),
        Config::default(),
    );
    let app = web::router(Role::Serve, state);
    let response = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// Packets from node 10 to 20 and back, an hour apart, the latest just now.
async fn traffic(api: &Api) {
    let now = now_us();
    for (id, from, to, hours_ago, channel) in [
        (1, 10, 20, 0, "LongFast"),
        (2, 10, 20, 1, "LongFast"),
        (3, 20, 10, 1, "MediumFast"),
        (4, 10, 30, 50, "LongFast"),
    ] {
        sqlx::query(
            "INSERT INTO packet (id, portnum, from_node_id, to_node_id, import_time_us, channel) \
             VALUES (?, 1, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(from)
        .bind(to)
        .bind(now - hours_ago * 3_600_000_000_i64)
        .bind(channel)
        .execute(&api.pool)
        .await
        .unwrap();
    }
    for (packet, gateway) in [(1, 20), (1, 21), (2, 20), (3, 21)] {
        sqlx::query(
            "INSERT INTO packet_seen (packet_id, node_id, rx_time, import_time_us) \
             VALUES (?, ?, 0, ?)",
        )
        .bind(packet)
        .bind(gateway)
        .bind(now)
        .execute(&api.pool)
        .await
        .unwrap();
    }
    api.exec(
        "INSERT INTO node (id, node_id, long_name, short_name, channel) VALUES \
         ('!0000000a', 10, 'Ten', 'TEN', 'LongFast'), \
         ('!00000014', 20, 'Twenty', 'TWNT', 'MediumFast'), \
         ('!0000001e', 30, 'Thirty', 'THRT', 'LongFast')",
    )
    .await;
}

#[tokio::test]
async fn stats_count_packets_per_period() {
    let api = api().await;
    traffic(&api).await;

    let (status, body) = api.get("/api/v1/stats?period_type=day&length=1").await;
    assert_eq!(status, StatusCode::OK);
    let total: i64 = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|period| period["count"].as_i64().unwrap())
        .sum();
    assert_eq!(total, 3, "the packet 50 hours ago is outside the day");

    let (_, on_channel) = api
        .get("/api/v1/stats?period_type=day&length=1&channel=longfast")
        .await;
    assert_eq!(on_channel["data"][0]["count"], 2);
    assert_eq!(on_channel["channel"], "longfast");

    let (_, node) = api
        .get("/api/v1/stats?node=10&period_type=day&length=3")
        .await;
    assert_eq!(
        node,
        json!({"node_id": 10, "period_type": "day", "length": 3, "sent": 3, "seen": 1})
    );

    assert_eq!(
        api.get("/api/v1/stats?period_type=week").await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "Invalid period_type. Must be one of {'hour', 'day'}"})
        )
    );
    assert_eq!(
        api.get("/api/v1/stats?portnum=").await.1,
        json!({"error": "portnum must be an integer"})
    );
}

#[tokio::test]
async fn totals_count_what_is_asked() {
    let api = api().await;
    traffic(&api).await;

    assert_eq!(
        api.get("/api/v1/stats/count").await.1,
        json!({"total_packets": 4, "total_seen": 4})
    );
    // Any filter means the latest day by default.
    assert_eq!(
        api.get("/api/v1/stats/count?from_node=10").await.1,
        json!({"total_packets": 2, "total_seen": 3})
    );
    assert_eq!(
        api.get("/api/v1/stats/count?packet_id=1").await.1,
        json!({"total_packets": 4, "total_seen": 2})
    );
    // An empty filter filters nothing.
    assert_eq!(
        api.get("/api/v1/stats/count?channel=").await.1,
        json!({"total_packets": 4, "total_seen": 4})
    );
    assert_eq!(
        api.get("/api/v1/stats/count?period_type=week").await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "period_type must be 'hour' or 'day'"})
        )
    );
}

#[tokio::test]
async fn the_top_nodes_are_the_most_heard() {
    let api = api().await;
    traffic(&api).await;

    let (status, body) = api.get("/api/v1/stats/top?active=true").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 2);
    assert_eq!(
        body["nodes"],
        json!([
            {"node_id": 10, "long_name": "Ten", "short_name": "TEN", "channel": "LongFast",
             "sent": 2, "seen": 3, "avg": 1.5},
            {"node_id": 20, "long_name": "Twenty", "short_name": "TWNT", "channel": "MediumFast",
             "sent": 1, "seen": 1, "avg": 1.0},
        ])
    );

    // Without `active`, every node, the quiet ones last.
    let (_, all) = api.get("/api/v1/stats/top").await;
    assert_eq!(all["total"], 3);
    assert_eq!(all["nodes"][2]["avg"], 0.0);

    assert_eq!(
        api.get("/api/v1/stats/top?active=maybe").await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "active must be true or false"})
        )
    );
}

async fn some_nodes(api: &Api) {
    let now = now_us();
    for (node_id, short, long, role, channel, lat, gateway, seen_hours_ago) in [
        (
            1_i64,
            "alfa",
            "Alfa 100%",
            "CLIENT",
            "LongFast",
            423_000_000_i64,
            true,
            1_i64,
        ),
        (2, "Bravo", "Bravo_two", "ROUTER", "LongFast", 0, false, 2),
        (
            3,
            "charlie",
            "Charlie",
            "CLIENT",
            "MediumFast",
            400_000_000,
            false,
            24 * 40,
        ),
    ] {
        sqlx::query(
            "INSERT INTO node (id, node_id, short_name, long_name, role, channel, last_lat, \
             last_long, is_mqtt_gateway, last_seen_us) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("!{node_id:08x}"))
        .bind(node_id)
        .bind(short)
        .bind(long)
        .bind(role)
        .bind(channel)
        .bind(lat)
        .bind(-80_000_000_i64)
        .bind(gateway)
        .bind(now - seen_hours_ago * 3_600_000_000)
        .execute(&api.pool)
        .await
        .unwrap();
    }
    // Never seen: never listed.
    api.exec("INSERT INTO node (id, node_id) VALUES ('!00000004', 4)")
        .await;
}

fn ids(body: &Value) -> Vec<i64> {
    body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["node_id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn nodes_are_filtered_sorted_and_paged() {
    let api = api().await;
    some_nodes(&api).await;

    // By short name, exactly, as it always was: uppercase first.
    assert_eq!(ids(&api.get("/api/v1/nodes").await.1), [2, 1, 3]);
    assert_eq!(
        ids(&api.get("/api/v1/nodes?sort=short_name").await.1),
        [1, 2, 3]
    );
    assert_eq!(
        ids(&api.get("/api/v1/nodes?sort=-last_seen_us").await.1),
        [1, 2, 3]
    );
    assert_eq!(
        ids(&api.get("/api/v1/nodes?days_active=30").await.1),
        [2, 1]
    );
    assert_eq!(ids(&api.get("/api/v1/nodes?role=client").await.1), [1, 3]);
    assert_eq!(
        ids(&api.get("/api/v1/nodes?has_position=false").await.1),
        [2]
    );
    assert_eq!(
        ids(&api.get("/api/v1/nodes?is_mqtt_gateway=true").await.1),
        [1]
    );

    // Searched for literally: % and _ are not wildcards.
    assert_eq!(ids(&api.get("/api/v1/nodes?search=100%25").await.1), [1]);
    // As a wildcard, a_f would match "alfa"; literally, nothing has it.
    assert_eq!(
        ids(&api.get("/api/v1/nodes?search=a_f").await.1),
        Vec::<i64>::new()
    );
    // And as text it matches, in any case.
    assert_eq!(ids(&api.get("/api/v1/nodes?search=O_T").await.1), [2]);
    assert_eq!(ids(&api.get("/api/v1/nodes?search=bravo_").await.1), [2]);

    // An empty filter filters nothing, but no favourites are no nodes.
    assert_eq!(ids(&api.get("/api/v1/nodes?role=").await.1), [2, 1, 3]);
    assert_eq!(
        ids(&api.get("/api/v1/nodes?ids=").await.1),
        Vec::<i64>::new()
    );
    assert_eq!(ids(&api.get("/api/v1/nodes?ids=3,1").await.1), [1, 3]);
    assert_eq!(
        ids(&api.get("/api/v1/nodes?node_id=x").await.1),
        Vec::<i64>::new()
    );

    let (_, page) = api
        .get("/api/v1/nodes?sort=node_id&limit=1&offset=1&fields=short_name")
        .await;
    assert_eq!(
        page,
        json!({"nodes": [{"node_id": 2, "short_name": "Bravo"}], "total": 3, "limit": 1, "offset": 1})
    );
    assert_eq!(
        api.get("/api/v1/nodes?offset=0").await.1["limit"],
        Value::Null
    );

    assert_eq!(
        api.get("/api/v1/nodes?sort=nope").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        api.get("/api/v1/nodes?fields=nope").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        api.get("/api/v1/nodes?ids=1,x").await.1,
        json!({"error": "ids must be a comma-separated list of integers"})
    );
}

#[tokio::test]
async fn facets_count_the_nodes_by_field() {
    let api = api().await;
    some_nodes(&api).await;

    let (status, body) = api.get("/api/v1/nodes/facets").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 3);
    assert_eq!(
        body["facets"]["role"],
        json!([{"value": "CLIENT", "count": 2}, {"value": "ROUTER", "count": 1}])
    );
    assert_eq!(
        body["facets"]["firmware"],
        json!([{"value": null, "count": 3}])
    );
    assert_eq!(
        api.get("/api/v1/nodes/facets?channel=MediumFast").await.1["total"],
        1
    );
}

#[tokio::test]
async fn a_traceroute_is_assembled_from_what_its_gateways_reported() {
    use meshview_proto::meshtastic::RouteDiscovery;
    use prost::Message;

    let api = api().await;
    // The answer: from the traced node, 222, back to 111, which started it.
    api.exec(
        "INSERT INTO packet (id, portnum, from_node_id, to_node_id, channel) \
         VALUES (9, 70, 222, 111, 'LongFast')",
    )
    .await;
    let finished = RouteDiscovery {
        route: vec![444, 555],
        route_back: vec![555, 444],
        snr_back: vec![1, 2, 3],
        ..Default::default()
    };
    let partial = RouteDiscovery {
        route: vec![444],
        ..Default::default()
    };
    for (gateway, done, route, at) in [
        (333, true, finished.encode_to_vec(), 1),
        (334, false, partial.encode_to_vec(), 2),
        // Not a route at all: left out, but counted in the indexes.
        (335, false, vec![0xff], 3),
    ] {
        sqlx::query(
            "INSERT INTO traceroute (packet_id, gateway_node_id, done, route, import_time_us) \
             VALUES (9, ?, ?, ?, ?)",
        )
        .bind(gateway)
        .bind(done)
        .bind(route)
        .bind(at)
        .execute(&api.pool)
        .await
        .unwrap();
    }

    let (status, body) = api.get("/api/v1/traceroute/9").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["packet"],
        json!({"id": 9, "from": 222, "to": 111, "initiator": 111, "target": 222, "channel": "LongFast"})
    );
    assert_eq!(body["traceroute_packets"].as_array().unwrap().len(), 2);
    assert_eq!(body["traceroute_packets"][1]["index"], 1);
    assert_eq!(body["traceroute_packets"][0]["reverse_complete"], true);
    assert_eq!(
        body["unique_forward_paths"],
        json!([
            {"path": [444], "count": 1, "complete": false},
            {"path": [444, 555], "count": 1, "complete": true},
        ])
    );
    assert_eq!(body["unique_reverse_paths"], json!([[555, 444]]));
    assert_eq!(
        body["winning_paths"],
        json!({"forward": [[111, 444, 555, 222]], "reverse": [[222, 555, 444, 111]]})
    );

    assert_eq!(
        api.get("/api/v1/traceroute/10").await,
        (StatusCode::NOT_FOUND, json!({"error": "Packet not found"}))
    );
    assert_eq!(
        api.get("/api/v1/traceroute/x").await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "packet_id must be an integer"})
        )
    );
}

#[tokio::test]
async fn edges_are_traceroute_hops_and_reported_neighbours() {
    use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
    use meshview_proto::meshtastic::{
        Data, MeshPacket, Neighbor, NeighborInfo, PortNum, RouteDiscovery,
    };
    use prost::Message;

    let api = api().await;
    let now = now_us();

    // A finished traceroute 111 -> 444 -> 222, its answer stored from 222 to 111.
    api.exec("INSERT INTO packet (id, portnum, from_node_id, to_node_id) VALUES (9, 70, 222, 111)")
        .await;
    let route = RouteDiscovery {
        route: vec![444],
        ..Default::default()
    };
    sqlx::query(
        "INSERT INTO traceroute (packet_id, gateway_node_id, done, route, import_time_us) \
         VALUES (9, 333, 1, ?, ?)",
    )
    .bind(route.encode_to_vec())
    .bind(now)
    .execute(&api.pool)
    .await
    .unwrap();

    // Node 500 reports 600 as a neighbour.
    let report = MeshPacket {
        from: 500,
        payload_variant: Some(PayloadVariant::Decoded(Data {
            portnum: PortNum::NeighborinfoApp as i32,
            payload: NeighborInfo {
                node_id: 500,
                neighbors: vec![Neighbor {
                    node_id: 600,
                    ..Default::default()
                }],
                ..Default::default()
            }
            .encode_to_vec(),
            ..Default::default()
        })),
        ..Default::default()
    };
    sqlx::query(
        "INSERT INTO packet (id, portnum, from_node_id, payload, import_time_us) \
         VALUES (10, 71, 500, ?, ?)",
    )
    .bind(report.encode_to_vec())
    .bind(now)
    .execute(&api.pool)
    .await
    .unwrap();

    let (status, body) = api.get("/api/v1/edges").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["edges"],
        json!([
            {"from": 111, "to": 444, "type": "traceroute"},
            {"from": 444, "to": 222, "type": "traceroute"},
            {"from": 600, "to": 500, "type": "neighbor"},
        ])
    );
    assert_eq!(
        api.get("/api/v1/edges?type=neighbor&node_id=500").await.1["edges"],
        json!([{"from": 600, "to": 500, "type": "neighbor"}])
    );
    assert_eq!(
        api.get("/api/v1/edges?type=bogus").await.1,
        json!({"edges": []})
    );
    assert_eq!(
        api.get("/api/v1/edges?node_id=x").await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "node_id must be integer"})
        )
    );

    // Built once a minute: what arrives in between shows up after.
    api.exec("DELETE FROM traceroute").await;
    assert_eq!(
        api.get("/api/v1/edges").await.1["edges"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

async fn nodeinfo(api: &Api, id: i64, node_id: i64, key: Vec<u8>, at: i64) {
    use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
    use meshview_proto::meshtastic::{Data, MeshPacket, PortNum, User};
    use prost::Message;

    let packet = MeshPacket {
        from: node_id as u32,
        payload_variant: Some(PayloadVariant::Decoded(Data {
            portnum: PortNum::NodeinfoApp as i32,
            payload: User {
                id: format!("!{node_id:08x}"),
                public_key: key,
                is_unmessagable: Some(false),
                ..Default::default()
            }
            .encode_to_vec(),
            ..Default::default()
        })),
        ..Default::default()
    };
    sqlx::query(
        "INSERT INTO packet (id, portnum, from_node_id, payload, import_time_us) \
         VALUES (?, 4, ?, ?, ?)",
    )
    .bind(id)
    .bind(node_id)
    .bind(packet.encode_to_vec())
    .bind(at)
    .execute(&api.pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn a_node_is_offered_as_the_contact_python_encoded() {
    let api = api().await;
    api.exec(
        "INSERT INTO node (id, node_id, long_name, short_name, hw_model, role, last_seen_us) \
         VALUES ('!0000abcd', 43981, 'Ñandú', 'ÑD', 'heltec_v3', 'ROUTER', 1)",
    )
    .await;
    nodeinfo(&api, 1, 0xabcd, (0..32).collect(), 10).await;

    let (status, body) = api.get("/api/v1/node/0xabcd/qr").await;
    assert_eq!(status, StatusCode::OK);
    // Made with Python's protobuf from the same node and key.
    assert_eq!(
        body["meshtastic_url"],
        "https://meshtastic.org/v/#CM3XAhJBCgkhMDAwMGFiY2QSB8ORYW5kw7oaA8ORRCgrOAJCIAABAgMEBQYHCAkKCwwN\
         Dg8QERITFBUWFxgZGhscHR4fSAA"
    );
    assert_eq!(body["public_key_conflict"], false);
    assert_eq!(body["role"], "ROUTER");

    // Two keys: no contact, but the latest key is still reported.
    nodeinfo(&api, 2, 0xabcd, vec![7; 32], 20).await;
    let (_, conflict) = api.get("/api/v1/node/43981/qr").await;
    assert_eq!(conflict["public_key_conflict"], true);
    assert_eq!(conflict["meshtastic_url"], Value::Null);
    assert!(conflict["public_key"].is_string());

    assert_eq!(
        api.get("/api/v1/node/1/qr").await,
        (StatusCode::NOT_FOUND, json!({"error": "Node not found"}))
    );
    assert_eq!(
        api.get("/api/v1/node/x/qr").await,
        (StatusCode::BAD_REQUEST, json!({"error": "Invalid node_id"}))
    );
}

#[tokio::test]
async fn coverage_is_predicted_around_a_node_with_a_position() {
    let api = api().await;
    api.exec(
        "INSERT INTO node (id, node_id, last_lat, last_long, last_seen_us) VALUES \
         ('!00000001', 1, 426000000, -84000000, 1), ('!00000002', 2, NULL, NULL, 1)",
    )
    .await;

    let (status, body) = api.get("/api/v1/coverage/1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "perimeter");
    assert_eq!(body["threshold_dbm"], -120.0);
    let perimeter = body["perimeter"].as_array().unwrap();
    assert_eq!(perimeter.len(), 72, "a point every 5°");
    // Around the node, a few kilometres out.
    let lat = perimeter[0][0].as_f64().unwrap();
    assert!(lat > 42.6 && lat < 43.0, "{lat}");

    let (_, heatmap) = api
        .get("/api/v1/coverage/0x1?mode=heatmap&radius_km=3&step_km=1")
        .await;
    assert_eq!(heatmap["mode"], "heatmap");
    assert_eq!(heatmap["points"].as_array().unwrap().len(), 3 * 72);

    assert_eq!(
        api.get("/api/v1/coverage/2").await,
        (
            StatusCode::NOT_FOUND,
            json!({"error": "Node not found or missing location"})
        )
    );
    assert_eq!(
        api.get("/api/v1/coverage/1?freq_mhz=0.5").await.1,
        json!({"error": "freq_mhz must be between 1.0 and 10000.0"})
    );
}
