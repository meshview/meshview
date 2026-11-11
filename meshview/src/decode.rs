// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What a stored packet says, as the web pages show it.
//!
//! The Python Meshview's `Packet.from_model` and `decode_payload`, rule for
//! rule: the payload of the ports it knows is decoded and shown in the protobuf
//! text format; a text message is shown as its text when it went to everyone and
//! as `<redacted>` when it was direct; a routing answer is `ACK` or `NAK: why`;
//! and what cannot be read is `Did not decode`.

use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
use meshview_proto::meshtastic::{
    MapReport, MeshPacket, NeighborInfo, PortNum, Position, RouteDiscovery, Routing, StatusMessage,
    StoreAndForward, Telemetry, User, Waypoint, routing, store_and_forward,
};
use meshview_proto::{text_format, wire};

/// What is shown for a payload that cannot be read.
pub const DID_NOT_DECODE: &str = "Did not decode";

/// What is shown for a direct text message.
pub const REDACTED: &str = "<redacted>";

/// Where a broadcast goes.
pub const BROADCAST: i64 = 0xffff_ffff;

/// A traceroute longer than this either way cannot be real: the firmware keeps
/// at most 8 hops, and one with thousands makes every page that draws it slow.
pub const MAX_ROUTE_HOPS: usize = 8;

/// A stored packet, read.
#[derive(Debug, Clone, Default)]
pub struct Shown {
    /// The packet, when its bytes were one.
    pub mesh: Option<MeshPacket>,
    /// What it says, as the pages show it; not trimmed.
    pub text: String,
}

impl Shown {
    /// The packet's own id, as sent, when it has one.
    pub fn packet_id(&self) -> Option<i64> {
        self.mesh
            .as_ref()
            .map(|mesh| i64::from(mesh.id))
            .filter(|&id| id != 0)
    }

    /// The packet this one answers, when it says.
    pub fn reply_id(&self) -> Option<u32> {
        match self.mesh.as_ref()?.payload_variant.as_ref()? {
            PayloadVariant::Decoded(data) if data.reply_id != 0 => Some(data.reply_id),
            _ => None,
        }
    }
}

/// The payload of a port, decoded as the Python version decodes it.
enum Payload {
    Text(String),
    Message(&'static str, Vec<u8>),
    Routing(Routing, Vec<u8>),
}

fn decode_payload(portnum: i32, bytes: &[u8]) -> Option<Payload> {
    let message = |name: &'static str, ok: bool| ok.then(|| Payload::Message(name, bytes.to_vec()));
    match PortNum::try_from(portnum).ok()? {
        PortNum::PositionApp => message(
            "meshtastic.Position",
            wire::decode::<Position>("meshtastic.Position", bytes).is_some(),
        ),
        PortNum::NeighborinfoApp => message(
            "meshtastic.NeighborInfo",
            wire::decode::<NeighborInfo>("meshtastic.NeighborInfo", bytes).is_some(),
        ),
        PortNum::NodeinfoApp => message(
            "meshtastic.User",
            wire::decode::<User>("meshtastic.User", bytes).is_some(),
        ),
        PortNum::TelemetryApp => message(
            "meshtastic.Telemetry",
            wire::decode::<Telemetry>("meshtastic.Telemetry", bytes).is_some(),
        ),
        PortNum::MapReportApp => message(
            "meshtastic.MapReport",
            wire::decode::<MapReport>("meshtastic.MapReport", bytes).is_some(),
        ),
        PortNum::TracerouteApp => message("meshtastic.RouteDiscovery", route(bytes).is_some()),
        PortNum::RoutingApp => wire::decode::<Routing>("meshtastic.Routing", bytes)
            .map(|routing| Payload::Routing(routing, bytes.to_vec())),
        PortNum::TextMessageApp => String::from_utf8(bytes.to_vec()).ok().map(Payload::Text),
        // Not decoded by the Python version.
        PortNum::WaypointApp => message(
            "meshtastic.Waypoint",
            wire::decode::<Waypoint>("meshtastic.Waypoint", bytes).is_some(),
        ),
        PortNum::NodeStatusApp => message(
            "meshtastic.StatusMessage",
            wire::decode::<StatusMessage>("meshtastic.StatusMessage", bytes).is_some(),
        ),
        PortNum::StoreForwardApp => message(
            "meshtastic.StoreAndForward",
            wire::decode::<StoreAndForward>("meshtastic.StoreAndForward", bytes).is_some(),
        ),
        _ => None,
    }
}

/// Whether a message sent to one node says what only that node should read: a
/// waypoint, or the store and forward of a message from history, which can be
/// a direct one. Those are redacted as a direct text message is.
fn is_private(name: &str, bytes: &[u8]) -> bool {
    match name {
        "meshtastic.Waypoint" => true,
        "meshtastic.StoreAndForward" => {
            wire::decode::<StoreAndForward>("meshtastic.StoreAndForward", bytes)
                .is_some_and(|sf| matches!(sf.variant, Some(store_and_forward::Variant::Text(_))))
        }
        _ => false,
    }
}

/// A traceroute's route, or `None` when it cannot be read or cannot be real.
pub fn route(bytes: &[u8]) -> Option<RouteDiscovery> {
    let route: RouteDiscovery = wire::decode("meshtastic.RouteDiscovery", bytes)?;
    (route.route.len().max(route.route_back.len()) <= MAX_ROUTE_HOPS).then_some(route)
}

/// An enum value's name, or `unknown(N)`.
fn routing_error_name(value: i32) -> String {
    routing::Error::try_from(value)
        .map(|error| error.as_str_name().to_string())
        .unwrap_or_else(|_| format!("unknown({value})"))
}

/// What a stored packet says. `portnum` and `to_node_id` are the database's
/// columns, which decide how it is shown, as they did in the Python version;
/// what it is decoded as is the port the packet itself gives.
pub fn show(payload: Option<&[u8]>, portnum: Option<i64>, to_node_id: Option<i64>) -> Shown {
    let Some(mesh) =
        payload.and_then(|bytes| wire::decode::<MeshPacket>("meshtastic.MeshPacket", bytes))
    else {
        return Shown {
            mesh: None,
            text: DID_NOT_DECODE.to_string(),
        };
    };

    let decoded = match &mesh.payload_variant {
        Some(PayloadVariant::Decoded(data)) => decode_payload(data.portnum, &data.payload),
        _ => None,
    };
    let to_everyone = to_node_id == Some(BROADCAST);

    let text = match decoded {
        None => DID_NOT_DECODE.to_string(),
        Some(Payload::Routing(routing, bytes)) => match (portnum, &routing.variant) {
            (Some(5), Some(routing::Variant::ErrorReason(0))) => "ACK".to_string(),
            (Some(5), Some(routing::Variant::ErrorReason(reason))) => {
                format!("NAK: {}", routing_error_name(*reason))
            }
            _ => text_format::decode_to_text("meshtastic.Routing", &bytes)
                .unwrap_or_else(|| DID_NOT_DECODE.to_string()),
        },
        Some(Payload::Message(name, bytes)) if !to_everyone && is_private(name, &bytes) => {
            REDACTED.to_string()
        }
        Some(Payload::Message(name, bytes)) => {
            text_format::decode_to_text(name, &bytes).unwrap_or_else(|| DID_NOT_DECODE.to_string())
        }
        Some(Payload::Text(_)) if portnum == Some(1) && to_node_id != Some(BROADCAST) => {
            REDACTED.to_string()
        }
        Some(Payload::Text(text)) => text,
    };

    Shown {
        mesh: Some(mesh),
        text,
    }
}

/// Trimmed as Python's `str.strip()` trims: Rust's idea of white space, and the
/// four ASCII separators (0x1c to 0x1f), which Python counts and Rust does not.
pub fn python_strip(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
}

/// `seq 123`, the text of a message sent only to test a link: the pages leave
/// those out.
pub fn is_sequence_only(text: &str) -> bool {
    text.strip_prefix("seq ")
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshview_proto::meshtastic::Data;
    use prost::Message;

    fn packet(portnum: PortNum, payload: Vec<u8>) -> Vec<u8> {
        MeshPacket {
            payload_variant: Some(PayloadVariant::Decoded(Data {
                portnum: portnum as i32,
                payload,
                ..Default::default()
            })),
            ..Default::default()
        }
        .encode_to_vec()
    }

    fn shown(portnum: PortNum, payload: Vec<u8>, to: i64) -> String {
        show(
            Some(&packet(portnum, payload)),
            Some(portnum as i64),
            Some(to),
        )
        .text
    }

    #[test]
    fn a_node_status_is_shown() {
        let status = StatusMessage {
            status: "On the hill".into(),
        };
        assert_eq!(
            shown(PortNum::NodeStatusApp, status.encode_to_vec(), BROADCAST),
            "status: \"On the hill\"\n"
        );
    }

    #[test]
    fn a_waypoint_is_shown_to_everyone_and_redacted_to_one() {
        let waypoint = Waypoint {
            name: "Summit".into(),
            ..Default::default()
        }
        .encode_to_vec();
        assert!(shown(PortNum::WaypointApp, waypoint.clone(), BROADCAST).contains("Summit"));
        assert_eq!(shown(PortNum::WaypointApp, waypoint, 1234), REDACTED);
    }

    #[test]
    fn store_and_forward_hides_the_history_it_replays_to_one_node() {
        let text = StoreAndForward {
            variant: Some(store_and_forward::Variant::Text(
                b"a direct message".to_vec(),
            )),
            ..Default::default()
        }
        .encode_to_vec();
        assert_eq!(shown(PortNum::StoreForwardApp, text, 1234), REDACTED);

        let heartbeat = StoreAndForward {
            variant: Some(store_and_forward::Variant::Heartbeat(
                store_and_forward::Heartbeat {
                    period: 900,
                    secondary: 0,
                },
            )),
            ..Default::default()
        }
        .encode_to_vec();
        assert!(shown(PortNum::StoreForwardApp, heartbeat, 1234).contains("period: 900"));
    }
}
