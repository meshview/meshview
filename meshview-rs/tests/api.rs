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
use std::sync::{Arc, OnceLock};
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
    let reader = OnceLock::new();
    reader.set(Reader::new(pool.clone())).unwrap();
    let state = AppState {
        readiness: Arc::new(Readiness::new([CHECK_DATABASE])),
        reader: Arc::new(reader),
        config: Arc::new(Config::default()),
    };
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
    let reader = OnceLock::new();
    reader.set(Reader::new(pool.clone())).unwrap();
    let mut config = config;
    config.database.connection_string = format!("sqlite:///{}", database.display());
    let state = AppState {
        readiness: Arc::new(Readiness::new([CHECK_DATABASE])),
        reader: Arc::new(reader),
        config: Arc::new(config),
    };
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
    let state = AppState {
        readiness: Arc::new(Readiness::new([CHECK_DATABASE])),
        reader: Arc::new(OnceLock::new()),
        config: Arc::new(Config::default()),
    };
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
