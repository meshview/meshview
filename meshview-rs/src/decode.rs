// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What a stored packet says, as the web pages show it.
//!
//! The Python meshview's `Packet.from_model` and `decode_payload`, rule for
//! rule: the payload of the ports it knows is decoded and shown in the protobuf
//! text format; a text message is shown as its text when it went to everyone and
//! as `<redacted>` when it was direct; a routing answer is `ACK` or `NAK: why`;
//! and what cannot be read is `Did not decode`.

use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
use meshview_proto::meshtastic::{
    MapReport, MeshPacket, NeighborInfo, PortNum, Position, RouteDiscovery, Routing, Telemetry,
    User, routing,
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
        PortNum::TracerouteApp => {
            let route: RouteDiscovery = wire::decode("meshtastic.RouteDiscovery", bytes)?;
            let real = route.route.len().max(route.route_back.len()) <= MAX_ROUTE_HOPS;
            message("meshtastic.RouteDiscovery", real)
        }
        PortNum::RoutingApp => wire::decode::<Routing>("meshtastic.Routing", bytes)
            .map(|routing| Payload::Routing(routing, bytes.to_vec())),
        PortNum::TextMessageApp => String::from_utf8(bytes.to_vec()).ok().map(Payload::Text),
        _ => None,
    }
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
