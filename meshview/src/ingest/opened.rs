// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! A message off the broker, decoded and decrypted, with the packet as it is to
//! be stored.

use super::keys::Keyring;
use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
use meshview_proto::meshtastic::{MeshPacket, ServiceEnvelope};
use meshview_proto::wire;
use prost::Message;

/// The field numbers this needs to find its way around the raw bytes.
const ENVELOPE_PACKET: u32 = 1;
const PACKET_DECODED: u32 = 4;

/// Why a message could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotOpened {
    /// Not a ServiceEnvelope at all.
    NotAnEnvelope,
    /// None of the keys opened it.
    NoKey { id: u32, channel: u32 },
}

/// An opened message.
#[derive(Debug, Clone)]
pub struct Opened {
    pub envelope: ServiceEnvelope,
    /// Which key opened it: 0 for the default, or for a packet that came in the
    /// clear.
    pub key: usize,
    /// Whether it arrived encrypted.
    pub was_encrypted: bool,
    /// The packet, decrypted, encoded the way Python's protobuf encodes it: the
    /// fields these protobufs know first, then the ones they do not, as they
    /// came, inside the decrypted Data and after it.
    pub packet_bytes: Vec<u8>,
    /// The size of the decrypted envelope, counted the same way, which is what
    /// the limit on what is stored is measured against.
    pub size: usize,
}

impl Opened {
    /// Decodes a raw message and opens it with the first key that works.
    pub fn open(raw: &[u8], keyring: &Keyring) -> Result<Self, NotOpened> {
        // Read as Python's protobuf reads it: a field of the wrong wire type
        // is one it does not know, not a reason to drop the message.
        let mut envelope: ServiceEnvelope =
            wire::decode("meshtastic.ServiceEnvelope", raw).ok_or(NotOpened::NotAnEnvelope)?;

        let has_packet = envelope.packet.is_some();
        let packet = envelope.packet.get_or_insert_default();
        let was_encrypted = matches!(packet.payload_variant, Some(PayloadVariant::Encrypted(_)));
        let Some((key, plain)) = keyring.open(packet) else {
            return Err(NotOpened::NoKey {
                id: packet.id,
                channel: packet.channel,
            });
        };

        let packet_bytes = faithful_packet(raw, packet, plain.as_deref());

        let mut size = ServiceEnvelope {
            packet: None,
            ..envelope.clone()
        }
        .encoded_len()
            + wire::unknown_fields("meshtastic.ServiceEnvelope", raw).len();
        if has_packet {
            size += 1 + wire::varint_len(packet_bytes.len() as u64) + packet_bytes.len();
        }

        Ok(Self {
            envelope,
            key,
            was_encrypted,
            packet_bytes,
            size,
        })
    }

    /// The packet, which an opened message always has.
    pub fn packet(&self) -> &MeshPacket {
        self.envelope
            .packet
            .as_ref()
            .expect("opening puts a packet in")
    }
}

/// The decoded packet, encoded with the fields it came with that these
/// protobufs do not know put back where Python's protobuf would put them.
fn faithful_packet(raw_envelope: &[u8], packet: &MeshPacket, plain: Option<&[u8]>) -> Vec<u8> {
    let original = wire::last_len_field(raw_envelope, ENVELOPE_PACKET).unwrap_or_default();
    let data = match plain {
        Some(plain) => plain,
        None => wire::last_len_field(original, PACKET_DECODED).unwrap_or_default(),
    };

    let data_unknown = wire::unknown_fields("meshtastic.Data", data);
    let packet_unknown = wire::unknown_fields("meshtastic.MeshPacket", original);

    let mut bytes = wire::extend_len_field(&packet.encode_to_vec(), PACKET_DECODED, &data_unknown);
    bytes.extend_from_slice(&packet_unknown);
    bytes
}
