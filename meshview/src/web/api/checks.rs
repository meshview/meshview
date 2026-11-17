// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What may be wrong with how a node is set up, as its own packets show it.
//!
//! A setting is how the node is now, so each check reads only the latest of
//! the node's packets, or those of the latest day: what it costs does not
//! grow with how busy the node is, nor with how long it has been heard.

use super::error::{ApiError, ErrorBody};
use super::keys;
use super::now_us;
use super::params::parse_db_int;
use crate::config;
use crate::db::{Node, PortCount};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
use meshview_proto::meshtastic::{HardwareModel, MeshPacket, PortNum, Position, User};
use meshview_proto::wire;
use serde::Serialize;
use std::collections::BTreeSet;
use utoipa::ToSchema;

/// How many of the node's latest packets tell the hops it sets.
const HOP_PACKETS: i64 = 50;

/// How many of its latest node info packets tell its key and hardware, as for
/// its contact.
const NODEINFO_PACKETS: i64 = 50;

/// The most packets of a port a node may send to everyone in a day before it
/// sends them too often: position and telemetry every five minutes, node info
/// every hour, neighbour info every two hours. Well above the firmware's own
/// defaults, so that only a setting far off them is told.
const DAILY_LIMITS: [(PortNum, i64); 4] = [
    (PortNum::PositionApp, 288),
    (PortNum::TelemetryApp, 288),
    (PortNum::NodeinfoApp, 24),
    (PortNum::NeighborinfoApp, 12),
];

const DAY_US: i64 = 86_400_000_000;

/// How recently another node must have been seen to tell that it shares the
/// key: one long gone, as the same radio under an id it no longer uses, is no
/// longer a risk.
const SHARED_KEY_DAYS: i64 = 7;

#[derive(Debug, Serialize, ToSchema)]
pub struct Checks {
    /// The node asked about.
    #[schema(example = 2925607268_i64)]
    pub node_id: i64,
    /// What may be wrong, warnings first; none when nothing is.
    pub checks: Vec<Check>,
}

/// One thing that may be wrong with how a node is set up.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct Check {
    /// What it is: `public_key_shared`, `public_key_conflict`,
    /// `hardware_changed`, `hop_limit`, `position_without_coordinates`,
    /// `frequent_broadcasts` or `default_name`.
    #[schema(example = "hop_limit")]
    pub check: &'static str,
    /// `warning` or `info`.
    #[schema(example = "warning")]
    pub severity: &'static str,
    /// What was found: the hops its packets start with, the packets of the
    /// port sent in a day, the keys it gave, or the nodes it shares its key
    /// with; null when there is no figure.
    #[schema(example = 7)]
    pub value: Option<i64>,
    /// What it should be at most: the hops this mesh advises, the packets a
    /// day.
    #[schema(example = 3)]
    pub limit: Option<i64>,
    /// The port, for `frequent_broadcasts`.
    #[schema(example = 3)]
    pub portnum: Option<i64>,
    /// The hardware models it gave, for `hardware_changed`.
    pub models: Vec<String>,
    /// The other nodes with its key, latest seen first, for
    /// `public_key_shared`.
    pub nodes: Vec<SharedWith>,
}

/// A node that has the same public key as the one checked.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct SharedWith {
    #[schema(example = 1975835426_i64)]
    pub node_id: i64,
    #[schema(example = "Sierra de Gata")]
    pub long_name: Option<String>,
    #[schema(example = "db22")]
    pub short_name: Option<String>,
}

impl Check {
    fn new(check: &'static str, severity: &'static str) -> Self {
        Self {
            check,
            severity,
            value: None,
            limit: None,
            portnum: None,
            models: Vec::new(),
            nodes: Vec::new(),
        }
    }
}

/// The user a stored node info packet carries.
fn user(payload: &[u8]) -> Option<User> {
    let packet: MeshPacket = wire::decode("meshtastic.MeshPacket", payload)?;
    let Some(PayloadVariant::Decoded(data)) = packet.payload_variant else {
        return None;
    };
    wire::decode("meshtastic.User", &data.payload)
}

/// The latitude and longitude a stored position packet carries, as sent.
fn coordinates(payload: &[u8]) -> Option<(i32, i32)> {
    let packet: MeshPacket = wire::decode("meshtastic.MeshPacket", payload)?;
    let Some(PayloadVariant::Decoded(data)) = packet.payload_variant else {
        return None;
    };
    let position: Position = wire::decode("meshtastic.Position", &data.payload)?;
    Some((
        position.latitude_i.unwrap_or(0),
        position.longitude_i.unwrap_or(0),
    ))
}

/// Within a degree of 0,0: no fix, or position sharing off. The ingester does
/// not take it as a position either.
fn no_coordinates((lat, lon): (i32, i32)) -> bool {
    lat.unsigned_abs() <= 10_000_000 && lon.unsigned_abs() <= 10_000_000
}

/// Several keys, or several hardware models, in the node's node info. A copy
/// of a key damaged on the air is not another key.
fn identity_checks(users: &[User]) -> Vec<Check> {
    let mut checks = Vec::new();
    let keys = keys::tell_apart(users.iter().map(|user| user.public_key.as_slice()));
    if keys.len() > 1 {
        let mut check = Check::new("public_key_conflict", "warning");
        check.value = Some(keys.len() as i64);
        checks.push(check);
    }
    let models: BTreeSet<String> = users
        .iter()
        .filter(|user| user.hw_model != HardwareModel::Unset as i32)
        .map(|user| {
            HardwareModel::try_from(user.hw_model)
                .map(|model| model.as_str_name().to_string())
                .unwrap_or_else(|_| format!("unknown({})", user.hw_model))
        })
        .collect();
    if models.len() > 1 {
        let mut check = Check::new("hardware_changed", "warning");
        check.models = models.into_iter().collect();
        checks.push(check);
    }
    checks
}

/// More hops than this mesh advises, as most of the latest packets started
/// with: the setting, which a packet now and then sent with others, as a
/// traceroute asked for with more, does not change. Of two as common, the
/// latest. A note from `hop_limit_notice` on, a warning from
/// `hop_limit_warning`.
fn hop_check(hop_starts: &[Option<i64>], limits: &config::Checks) -> Option<Check> {
    let told: Vec<i64> = hop_starts.iter().flatten().copied().collect();
    let count = |hops: i64| told.iter().filter(|&&other| other == hops).count();
    // max_by_key keeps the last of the greatest, so the list goes oldest first.
    let hops = told.iter().rev().copied().max_by_key(|&hops| count(hops))?;
    let severity = if hops >= i64::from(limits.hop_limit_warning) {
        "warning"
    } else if hops >= i64::from(limits.hop_limit_notice) {
        "info"
    } else {
        return None;
    };
    let mut check = Check::new("hop_limit", severity);
    check.value = Some(hops);
    check.limit = Some(i64::from(limits.hop_limit_notice) - 1);
    Some(check)
}

/// The ports the node sent to everyone more often than a day allows.
fn broadcast_checks(counts: &[PortCount]) -> Vec<Check> {
    DAILY_LIMITS
        .iter()
        .filter_map(|(port, limit)| {
            let count = counts.iter().find(|count| count.portnum == *port as i64)?;
            (count.packets > *limit).then(|| {
                let mut check = Check::new("frequent_broadcasts", "warning");
                check.value = Some(count.packets);
                check.limit = Some(*limit);
                check.portnum = Some(*port as i64);
                check
            })
        })
        .collect()
}

/// The name the firmware gives a node until it is named: `Meshtastic` and
/// the last four hex digits of its id.
fn has_default_name(node: &Node) -> bool {
    let Some(node_id) = node.node_id else {
        return false;
    };
    node.long_name.as_deref().map(str::trim)
        == Some(format!("Meshtastic {:04x}", node_id & 0xffff).as_str())
}

/// Node checks
///
/// What may be wrong with how a node is set up, as its own packets show it:
/// a public key other nodes seen in the latest week have too, so that each can
/// read the direct messages sent to the others, as radios given the same key
/// at the factory do; node info that disagrees on its key, which two radios with the same id
/// give, and so does one whose key was made anew, or on its hardware;
/// packets that start with more hops than the mesh advises, a note from
/// `[checks] hop_limit_notice` hops on (4 unless set) and a warning from
/// `hop_limit_warning` (7); a latest position with no coordinates (no fix, or
/// position sharing off); more broadcasts of a port in the latest day than any
/// usual setting sends; and the name the firmware gives a node until it is
/// named.
///
/// A setting is how the node is now, so only its latest packets are read: the
/// latest 50 for the hops and for the node info, the latest position, and the
/// broadcasts of the latest 24 hours.
#[utoipa::path(
    get,
    path = "/node/{node_id}/checks",
    tag = "nodes",
    params(
        ("node_id" = i64, Path, description = "The node's number, in decimal."),
    ),
    responses(
        (status = 200, description = "What may be wrong, maybe nothing", body = Checks),
        (status = 400, description = "`node_id` is not an integer", body = ErrorBody),
        (status = 500, description = "It could not be read", body = ErrorBody),
    )
)]
pub async fn checks(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
) -> Result<Json<Checks>, ApiError> {
    let node_id = parse_db_int(&node_id)
        .ok_or_else(|| ApiError::bad_request("Invalid or missing node_id"))?;
    let db = state.db()?;
    let internal = |e| ApiError::internal("Internal server error", &e);

    let users: Vec<User> = db
        .latest_payloads(node_id, PortNum::NodeinfoApp as i64, NODEINFO_PACKETS)
        .await
        .map_err(internal)?
        .into_iter()
        .flatten()
        .filter_map(|payload| user(&payload))
        .collect();
    let mut checks = Vec::new();
    let sharing = db
        .nodes_sharing_key(node_id, now_us() - SHARED_KEY_DAYS * DAY_US)
        .await
        .map_err(internal)?;
    if !sharing.is_empty() {
        let mut check = Check::new("public_key_shared", "warning");
        check.value = Some(sharing.len() as i64);
        check.nodes = sharing
            .into_iter()
            .filter_map(|node| {
                Some(SharedWith {
                    node_id: node.node_id?,
                    long_name: node.long_name,
                    short_name: node.short_name,
                })
            })
            .collect();
        checks.push(check);
    }
    checks.extend(identity_checks(&users));

    let hop_starts = db
        .latest_hop_starts(node_id, HOP_PACKETS)
        .await
        .map_err(internal)?;
    checks.extend(hop_check(&hop_starts, &state.config.checks));

    let position = db
        .latest_payloads(node_id, PortNum::PositionApp as i64, 1)
        .await
        .map_err(internal)?;
    if position
        .into_iter()
        .flatten()
        .filter_map(|payload| coordinates(&payload))
        .next()
        .is_some_and(no_coordinates)
    {
        checks.push(Check::new("position_without_coordinates", "warning"));
    }

    let counts = db
        .broadcasts_by_port(node_id, now_us() - DAY_US)
        .await
        .map_err(internal)?;
    checks.extend(broadcast_checks(&counts));

    if db
        .node(node_id)
        .await
        .map_err(internal)?
        .is_some_and(|node| has_default_name(&node))
    {
        checks.push(Check::new("default_name", "info"));
    }

    // Stable, so each keeps the order it was checked in.
    checks.sort_by_key(|check| check.severity != "warning");
    Ok(Json(Checks { node_id, checks }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_with(key: u8, model: HardwareModel) -> User {
        User {
            public_key: vec![key; 32],
            hw_model: model as i32,
            ..Default::default()
        }
    }

    #[test]
    fn node_info_that_disagrees_tells_of_two_radios() {
        let same = [
            user_with(1, HardwareModel::HeltecV3),
            user_with(1, HardwareModel::HeltecV3),
        ];
        assert!(identity_checks(&same).is_empty());

        let users = [
            user_with(1, HardwareModel::HeltecV3),
            user_with(2, HardwareModel::Tbeam),
            // No model said is not a third one.
            user_with(2, HardwareModel::Unset),
        ];
        let checks = identity_checks(&users);
        assert_eq!(checks.len(), 2);
        assert_eq!(
            (checks[0].check, checks[0].value),
            ("public_key_conflict", Some(2))
        );
        assert_eq!(checks[1].check, "hardware_changed");
        assert_eq!(checks[1].models, ["HELTEC_V3", "TBEAM"]);
    }

    #[test]
    fn the_hops_most_packets_start_with_are_the_ones_set() {
        let limits = config::Checks::default();
        let hops = |starts: &[Option<i64>]| hop_check(starts, &limits);
        // Latest first. A traceroute now and then with more is not the setting.
        assert!(hops(&[Some(4), Some(3), Some(3), None, Some(3)]).is_none());
        let check = hops(&[Some(7), None, Some(7), Some(3)]).unwrap();
        assert_eq!(
            (check.severity, check.value, check.limit),
            ("warning", Some(7), Some(3))
        );
        // Of two as common, the latest: the setting was just changed.
        let check = hops(&[Some(5), Some(3)]).unwrap();
        assert_eq!((check.severity, check.value), ("info", Some(5)));
        assert!(hops(&[Some(3), Some(5)]).is_none());
        assert!(hops(&[None]).is_none());
    }

    #[test]
    fn each_mesh_tells_the_hops_it_needs() {
        let limits = config::Checks {
            hop_limit_notice: 6,
            hop_limit_warning: 7,
        };
        assert!(hop_check(&[Some(5)], &limits).is_none());
        let check = hop_check(&[Some(6)], &limits).unwrap();
        assert_eq!((check.severity, check.limit), ("info", Some(5)));
        assert_eq!(hop_check(&[Some(7)], &limits).unwrap().severity, "warning");
    }

    #[test]
    fn only_broadcasts_far_off_the_defaults_are_told() {
        let counts = [
            PortCount {
                portnum: PortNum::PositionApp as i64,
                packets: 288,
            },
            PortCount {
                portnum: PortNum::NodeinfoApp as i64,
                packets: 30,
            },
            PortCount {
                portnum: PortNum::TextMessageApp as i64,
                packets: 5000,
            },
        ];
        let checks = broadcast_checks(&counts);
        assert_eq!(checks.len(), 1);
        assert_eq!(
            (checks[0].portnum, checks[0].value, checks[0].limit),
            (Some(PortNum::NodeinfoApp as i64), Some(30), Some(24))
        );
    }

    #[test]
    fn a_position_near_0_0_has_no_coordinates() {
        assert!(no_coordinates((0, 0)));
        assert!(no_coordinates((9_000_000, -10_000_000)));
        assert!(!no_coordinates((433_091_189, -83_834_500)));
    }

    #[test]
    fn the_default_name_is_meshtastic_and_the_last_four_digits() {
        let node = |long: &str| Node {
            id: "!ae613964".into(),
            node_id: Some(0xae61_3964),
            long_name: Some(long.into()),
            short_name: None,
            hw_model: None,
            firmware: None,
            role: None,
            last_lat: None,
            last_long: None,
            last_altitude: None,
            channel: None,
            is_mqtt_gateway: None,
            first_seen_us: None,
            last_seen_us: None,
        };
        assert!(has_default_name(&node("Meshtastic 3964")));
        assert!(!has_default_name(&node("Meshtastic 1a2b")));
        assert!(!has_default_name(&node("KMesh2")));
    }
}
