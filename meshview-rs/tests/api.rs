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
