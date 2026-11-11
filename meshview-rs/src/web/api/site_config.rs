// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What the web pages need to know about this installation.
//!
//! The shapes and types are the Python version's, odd ones included: the
//! switches are the strings `"true"` and `"false"` rather than booleans, and the
//! cleanup's figures are strings too, because that is what its INI file gave it
//! and what the pages have been written against.

use crate::config::Config;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;

const DEFAULT_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
const DEFAULT_TILE_ATTRIBUTION: &str =
    "&copy; <a href=\"https://www.openstreetmap.org/copyright\">OpenStreetMap</a> contributors";

#[derive(Debug, Serialize, ToSchema)]
pub struct SiteConfig {
    pub site: Site,
    pub mqtt: Mqtt,
    pub cleanup: Cleanup,
}

/// `[site]`, as the pages read it.
#[derive(Debug, Serialize, ToSchema)]
pub struct Site {
    pub domain: String,
    pub language: String,
    pub title: String,
    pub message: String,
    pub announcement: String,
    /// `"true"` or `"false"`, as are the other switches.
    #[schema(example = "true")]
    pub nodes: &'static str,
    pub chat: &'static str,
    pub everything: &'static str,
    pub graphs: &'static str,
    pub stats: &'static str,
    pub net: &'static str,
    pub map: &'static str,
    pub top: &'static str,
    pub api: &'static str,
    pub meshviewworld: &'static str,
    pub map_top_left_lat: f64,
    pub map_top_left_lon: f64,
    pub map_bottom_right_lat: f64,
    pub map_bottom_right_lon: f64,
    pub map_interval: i64,
    /// The tile server: the configured one, or OpenStreetMap.
    pub map_tile_url: String,
    pub map_tile_attribution: String,
    pub map_tile_max_zoom: u8,
    pub map_tile_name: String,
    /// Whether the tiles come from a server of the installation's own.
    pub map_tile_custom: bool,
    /// Channel colours, by lowercased channel name, only those that are
    /// `#rrggbb`.
    pub map_channel_colors: BTreeMap<String, String>,
    pub firehose_interval: i64,
    pub weekly_net_message: String,
    pub net_tag: String,
    /// meshview-rs's version.
    pub version: &'static str,
}

/// The broker, without its credentials.
#[derive(Debug, Serialize, ToSchema)]
pub struct Mqtt {
    pub server: String,
    pub topics: Vec<String>,
}

/// `[cleanup]`, every value a string, as in the Python version.
#[derive(Debug, Serialize, ToSchema)]
pub struct Cleanup {
    #[schema(example = "false")]
    pub enabled: &'static str,
    #[schema(example = "14")]
    pub days_to_keep: String,
    #[schema(example = "2")]
    pub hour: String,
    #[schema(example = "0")]
    pub minute: String,
    #[schema(example = "false")]
    pub vacuum: &'static str,
}

fn switch(on: bool) -> &'static str {
    if on { "true" } else { "false" }
}

/// The host part of a URL template, lowercased and without a port, which is
/// what the tile layer is named after when nothing else names it. Templates such
/// as `{s}.tile.example.org` are not URLs a parser accepts, so this is by hand.
fn hostname(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(bracketed) = host.strip_prefix('[') {
        bracketed.split(']').next()?
    } else {
        host.split(':').next()?
    };
    Some(host.to_lowercase()).filter(|host| !host.is_empty())
}

pub fn build(config: &Config) -> SiteConfig {
    let site = &config.site;
    let url = site.map_tile_url.trim();
    let custom = !url.is_empty();
    let attribution = site.map_tile_attribution.trim();
    let name = site.map_tile_name.trim();

    SiteConfig {
        site: Site {
            domain: site.domain.clone(),
            language: site.language.clone(),
            title: site.title.clone(),
            message: site.message.clone(),
            announcement: site.announcement.clone(),
            nodes: switch(site.nodes),
            chat: switch(site.chat),
            everything: switch(site.everything),
            graphs: switch(site.graphs),
            stats: switch(site.stats),
            net: switch(site.net),
            map: switch(site.map),
            top: switch(site.top),
            api: switch(site.api),
            meshviewworld: switch(site.meshviewworld),
            map_top_left_lat: site.map_top_left_lat,
            map_top_left_lon: site.map_top_left_lon,
            map_bottom_right_lat: site.map_bottom_right_lat,
            map_bottom_right_lon: site.map_bottom_right_lon,
            map_interval: site.map_interval,
            map_tile_url: if custom { url } else { DEFAULT_TILE_URL }.to_string(),
            map_tile_attribution: if custom || !attribution.is_empty() {
                attribution
            } else {
                DEFAULT_TILE_ATTRIBUTION
            }
            .to_string(),
            map_tile_max_zoom: site.map_tile_max_zoom.clamp(1, 22),
            map_tile_name: if !name.is_empty() {
                name.to_string()
            } else if custom {
                hostname(url).unwrap_or_else(|| "Map".to_string())
            } else {
                "OpenStreetMap".to_string()
            },
            map_tile_custom: custom,
            map_channel_colors: config
                .map_channel_colors
                .iter()
                .map(|(channel, colour)| (channel.to_lowercase(), colour.trim().to_string()))
                .filter(|(_, colour)| {
                    colour.len() == 7
                        && colour.starts_with('#')
                        && colour[1..].bytes().all(|b| b.is_ascii_hexdigit())
                })
                .collect(),
            firehose_interval: site.firehose_interval,
            weekly_net_message: site.weekly_net_message.clone(),
            net_tag: site.net_tag.clone(),
            version: env!("CARGO_PKG_VERSION"),
        },
        mqtt: Mqtt {
            server: config.mqtt.server.clone(),
            topics: config.mqtt.topics.clone(),
        },
        cleanup: Cleanup {
            enabled: switch(config.cleanup.enabled),
            days_to_keep: config.cleanup.days_to_keep.to_string(),
            hour: config.cleanup.hour.to_string(),
            minute: config.cleanup.minute.to_string(),
            vacuum: switch(config.cleanup.vacuum),
        },
    }
}

/// Site configuration
///
/// What the web pages need to know about this installation: its title and
/// texts, which links to show, the map's bounds and tiles, the broker it reads
/// (not its credentials) and when it cleans up.
#[utoipa::path(
    get,
    path = "/config",
    tag = "meta",
    responses((status = 200, description = "The configuration", body = SiteConfig))
)]
pub async fn config(State(state): State<AppState>) -> Json<SiteConfig> {
    Json(build(&state.config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_layers_are_named_after_their_host() {
        assert_eq!(
            hostname("https://{s}.Tiles.Example.org:8443/{z}/{x}/{y}.png").as_deref(),
            Some("{s}.tiles.example.org")
        );
        assert_eq!(hostname("http://user@[::1]:80/t").as_deref(), Some("::1"));
        assert_eq!(hostname("not a url"), None);
    }

    #[test]
    fn the_defaults_are_openstreetmap() {
        let built = build(&Config::default());
        assert_eq!(built.site.map_tile_url, DEFAULT_TILE_URL);
        assert_eq!(built.site.map_tile_name, "OpenStreetMap");
        assert!(!built.site.map_tile_custom);
        assert_eq!(built.site.nodes, "true");
        assert_eq!(built.cleanup.enabled, "false");
    }
}
