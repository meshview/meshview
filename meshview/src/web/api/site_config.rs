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

/// The map's colours for the channels the official presets name, by the name
/// the firmware gives each, lowercased as the map compares them: a hue a family
/// of presets, darker for the slow ones and lighter for the turbo ones, with
/// LongFast in the site's own blue. `custom` is the channel of a radio set up
/// without a preset. `[map_channel_colors]` overrides any of them.
pub const PRESET_COLOURS: [(&str, &str); 17] = [
    ("longslow", "#133a6c"),
    ("longfast", "#245ea8"),
    ("longmod", "#617dd1"),
    ("longturbo", "#1f96d6"),
    ("mediumslow", "#0b5a5f"),
    ("mediumfast", "#12858b"),
    ("mediumturbo", "#1aa39c"),
    ("shortslow", "#9c4506"),
    ("shortfast", "#e07012"),
    ("shortturbo", "#f29a3f"),
    ("narrowslow", "#4b2a8a"),
    ("narrowfast", "#7a4cc8"),
    ("liteslow", "#8a1d51"),
    ("litefast", "#cf3d7e"),
    ("tinyslow", "#765800"),
    ("tinyfast", "#b38600"),
    ("custom", "#6e6e6e"),
];

/// Whether a channel is one the official presets name, compared as the map
/// compares them: without case, spaces, hyphens or underscores. `Custom`, the
/// channel of a radio set up without a preset, is not.
pub fn is_preset_channel(channel: &str) -> bool {
    let name: String = channel
        .chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect();
    name != "custom" && PRESET_COLOURS.iter().any(|(preset, _)| *preset == name)
}

const DEFAULT_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
const DEFAULT_TILE_ATTRIBUTION: &str =
    "&copy; <a href=\"https://www.openstreetmap.org/copyright\">OpenStreetMap</a> contributors";

#[derive(Debug, Serialize, ToSchema)]
pub struct SiteConfig {
    /// `[site]`, with the map's tiles resolved.
    pub site: Site,
    /// The broker the mesh is read from.
    pub mqtt: Mqtt,
    /// The daily cleanup.
    pub cleanup: Cleanup,
}

/// `[site]`, as the pages read it.
#[derive(Debug, Serialize, ToSchema)]
pub struct Site {
    /// The installation's domain, as configured; it is only shown.
    #[schema(example = "example.com")]
    pub domain: String,
    /// The language of the pages: `en`, `es` or `ru`.
    #[schema(example = "en")]
    pub language: String,
    /// The site's title, for the title bar and the headers.
    #[schema(example = "Meshview")]
    pub title: String,
    /// The tagline, for the page footer and description; empty when unset.
    pub message: String,
    /// The text of the banner every page shows until a visitor dismisses
    /// it; empty when unset.
    pub announcement: String,
    /// Whether to link the nodes page: `"true"` or `"false"`, a string, as
    /// are the other switches. They only hide links; the pages and the API
    /// answer either way.
    #[schema(example = "true")]
    pub nodes: &'static str,
    /// Whether to link the chat.
    #[schema(example = "true")]
    pub chat: &'static str,
    /// Whether to link the firehose, every packet.
    #[schema(example = "true")]
    pub everything: &'static str,
    /// Whether to link the graphs.
    #[schema(example = "true")]
    pub graphs: &'static str,
    /// Whether to link the statistics.
    #[schema(example = "true")]
    pub stats: &'static str,
    /// Whether to link the weekly net.
    #[schema(example = "true")]
    pub net: &'static str,
    /// Whether to link the map.
    #[schema(example = "true")]
    pub map: &'static str,
    /// Whether to link the top nodes.
    #[schema(example = "true")]
    pub top: &'static str,
    /// Whether to link this reference.
    #[schema(example = "true")]
    pub api: &'static str,
    /// Whether to link MeshviewWorld.
    #[schema(example = "true")]
    pub meshviewworld: &'static str,
    /// The map's bounds: latitude of the top left corner, in degrees.
    #[schema(example = 39.0)]
    pub map_top_left_lat: f64,
    /// Longitude of the top left corner.
    #[schema(example = -123.0)]
    pub map_top_left_lon: f64,
    /// Latitude of the bottom right corner.
    #[schema(example = 36.0)]
    pub map_bottom_right_lat: f64,
    /// Longitude of the bottom right corner.
    #[schema(example = -121.0)]
    pub map_bottom_right_lon: f64,
    /// Seconds between the map's refreshes; 0 or less, none.
    #[schema(example = 3)]
    pub map_interval: i64,
    /// The template of the tiles' addresses: the configured one, or
    /// OpenStreetMap's.
    #[schema(example = "https://tile.openstreetmap.org/{z}/{x}/{y}.png")]
    pub map_tile_url: String,
    /// The tiles' attribution, in HTML.
    #[schema(
        example = "&copy; <a href=\"https://www.openstreetmap.org/copyright\">OpenStreetMap</a> contributors"
    )]
    pub map_tile_attribution: String,
    /// The highest zoom the tiles have.
    #[schema(example = 19)]
    pub map_tile_max_zoom: u8,
    /// The name of the tiles in the map's layer picker.
    #[schema(example = "OpenStreetMap")]
    pub map_tile_name: String,
    /// Whether the tiles come from a server of the installation's own; the
    /// maps then leave out the OpenTopoMap layer too.
    pub map_tile_custom: bool,
    /// Colours for channels on the map, by lowercased channel name: those of
    /// the official presets' channels, and those configured as `#rrggbb`,
    /// which take their place. Other channels get a colour of their own. The
    /// map matches names ignoring case, spaces, hyphens and underscores.
    #[schema(example = json!({"longfast": "#245ea8", "narrowfast": "#7a4cc8"}))]
    pub map_channel_colors: BTreeMap<String, String>,
    /// Seconds between the firehose's refreshes; 0 or less, none.
    #[schema(example = 3)]
    pub firehose_interval: i64,
    /// The text of the weekly net page.
    #[schema(example = "Weekly Mesh check-in message.")]
    pub weekly_net_message: String,
    /// The tag the weekly net page looks for in messages.
    #[schema(example = "#BayMeshNet")]
    pub net_tag: String,
    /// Meshview's version.
    #[schema(example = "0.1.0")]
    pub version: &'static str,
}

/// The broker, without its credentials.
#[derive(Debug, Serialize, ToSchema)]
pub struct Mqtt {
    /// The broker's host.
    #[schema(example = "mqtt.example.com")]
    pub server: String,
    /// The topics read.
    #[schema(example = json!(["msh/EU_868/#"]))]
    pub topics: Vec<String>,
}

/// `[cleanup]`, every value a string, as in the Python version.
#[derive(Debug, Serialize, ToSchema)]
pub struct Cleanup {
    /// Whether old data is deleted every day.
    #[schema(example = "false")]
    pub enabled: &'static str,
    /// How many days of data are kept.
    #[schema(example = "14")]
    pub days_to_keep: String,
    /// The hour of the day it is done at, local time.
    #[schema(example = "2")]
    pub hour: String,
    /// The minute of that hour.
    #[schema(example = "0")]
    pub minute: String,
    /// Whether the database is vacuumed afterwards, to give the space back.
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
            map_channel_colors: PRESET_COLOURS
                .iter()
                .map(|(channel, colour)| (channel.to_string(), colour.to_string()))
                .chain(
                    config
                        .map_channel_colors
                        .iter()
                        .map(|(channel, colour)| {
                            (channel.to_lowercase(), colour.trim().to_string())
                        })
                        .filter(|(_, colour)| {
                            colour.len() == 7
                                && colour.starts_with('#')
                                && colour[1..].bytes().all(|b| b.is_ascii_hexdigit())
                        }),
                )
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
///
/// The types are the Python version's, which the pages were written against:
/// switches are the strings `"true"` and `"false"`, and so are the cleanup's
/// figures; the map's bounds are numbers, the refresh intervals integers and
/// `map_tile_custom` a boolean. The configuration is read at start: a change
/// shows once the server is restarted and the page reloaded.
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
