// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Channel keys, and opening what was sent with them.
//!
//! Meshtastic encrypts a packet's `Data` with AES in CTR mode, under the
//! channel's key, with a nonce made of the packet's id and its sender. Nothing
//! authenticates it, so the only way to know a key was the right one is that what
//! comes out parses as `Data` with a port: a wrong key yields noise, and noise
//! occasionally parses.

use aes::{Aes128, Aes256};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ctr::Ctr128BE;
use ctr::cipher::{KeyIvInit, StreamCipher};
use meshview_proto::meshtastic::{Data, MeshPacket, mesh_packet::PayloadVariant};
use tracing::{error, warn};

/// Meshtastic's well known default key: what the single byte PSK 1 stands for,
/// and what the public channels use.
pub const DEFAULT_PSK: [u8; 16] = [
    0xd4, 0xf1, 0xbb, 0x3a, 0x20, 0x29, 0x07, 0x59, 0xf0, 0xbc, 0xff, 0xab, 0xcf, 0x4e, 0x69, 0x01,
];

/// The keys to try, in order: the default first, then the configured ones.
#[derive(Debug, Clone)]
pub struct Keyring {
    keys: Vec<Vec<u8>>,
}

impl Keyring {
    /// The default key and whatever `[mqtt] secondary_keys` adds to it.
    ///
    /// The setting is a list of base64 keys, each 16 or 32 bytes. Quotes around
    /// one are tolerated, as the Python version tolerated them. A single byte is
    /// Meshtastic's shorthand for a variant of the default key, as the apps write
    /// it, and 0 means no encryption at all, which is not a key. Anything else
    /// that will not do is said and skipped rather than stopping the ingester, as
    /// the Python version does.
    pub fn new<S: AsRef<str>>(secondary: &[S]) -> Self {
        let mut keys = vec![DEFAULT_PSK.to_vec()];

        for value in secondary
            .iter()
            .map(|v| v.as_ref().trim())
            .filter(|v| !v.is_empty())
        {
            let cleaned = strip_quotes(value).trim();
            if cleaned.is_empty() {
                continue;
            }

            let Ok(key) = STANDARD.decode(cleaned) else {
                warn!("Invalid base64 key in mqtt.secondary_keys: {value}");
                continue;
            };

            let key = match key.as_slice() {
                [0] => {
                    warn!(
                        "Skipping PSK {value} in mqtt.secondary_keys: index 0 means no encryption"
                    );
                    continue;
                }
                [index] => expand_short_psk(*index).to_vec(),
                _ => key,
            };

            if key.len() != 16 && key.len() != 32 {
                error!(
                    "Skipping key {value} in mqtt.secondary_keys: decoded to {} bytes, expected \
                     16 or 32",
                    key.len()
                );
                continue;
            }

            keys.push(key);
        }

        Self { keys }
    }

    /// How many keys there are, the default included.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Never: the default key is always there.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Opens the packet with the first key that works, leaving it decoded, and
    /// answers which key that was and what it decrypted to. A packet that arrived
    /// in the clear answers key 0, as though the default key had opened it, and
    /// nothing decrypted.
    pub fn open(&self, packet: &mut MeshPacket) -> Option<(usize, Option<Vec<u8>>)> {
        self.keys
            .iter()
            .enumerate()
            .find_map(|(index, key)| decrypt(packet, key).map(|plain| (index, plain)))
    }
}

/// Opens one packet with one key, replacing its encrypted payload with the
/// decoded one if that worked, and answering the bytes it decrypted to. A packet
/// that was never encrypted is open already, with nothing decrypted.
pub fn decrypt(packet: &mut MeshPacket, key: &[u8]) -> Option<Option<Vec<u8>>> {
    let encrypted: &[u8] = match &packet.payload_variant {
        Some(PayloadVariant::Decoded(_)) => return Some(None),
        Some(PayloadVariant::Encrypted(bytes)) => bytes,
        None => &[],
    };

    let mut nonce = [0u8; 16];
    nonce[..8].copy_from_slice(&u64::from(packet.id).to_le_bytes());
    nonce[8..].copy_from_slice(&u64::from(packet.from).to_le_bytes());

    let mut plain = encrypted.to_vec();
    let applied = match key.len() {
        16 => Ctr128BE::<Aes128>::new_from_slices(key, &nonce)
            .map(|mut cipher| cipher.apply_keystream(&mut plain)),
        32 => Ctr128BE::<Aes256>::new_from_slices(key, &nonce)
            .map(|mut cipher| cipher.apply_keystream(&mut plain)),
        _ => return None,
    };
    applied.ok()?;

    // Leniently, as Python's protobuf parses it: a field of the wrong wire type
    // is unknown, not fatal. Noise from a wrong key passes this more often than
    // it would pass prost alone, and what the Python version took for a packet
    // is what this has to take for one.
    let data: Data = meshview_proto::wire::decode("meshtastic.Data", &plain)?;
    // A wrong key can still come out as something that parses, made of nothing
    // but unknown fields. Port 0 is what that looks like, and it must not stop
    // the next key from being tried.
    if data.portnum == 0 {
        return None;
    }

    packet.payload_variant = Some(PayloadVariant::Decoded(data));
    Some(Some(plain))
}

/// The default key with its last byte moved on by `index - 1`, which is what a
/// one byte PSK means.
fn expand_short_psk(index: u8) -> [u8; 16] {
    let mut key = DEFAULT_PSK;
    key[15] = key[15].wrapping_add(index).wrapping_sub(1);
    key
}

fn strip_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && bytes[0] == bytes[bytes.len() - 1]
        && (bytes[0] == b'"' || bytes[0] == b'\'')
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshview_proto::meshtastic::PortNum;

    // Made with Python's `cryptography`, as the Python Meshview decrypts: a Data
    // with port 1 and "Hola desde Galicia", from 0xdeadbeef, packet 0x5ca1ab1e.
    const CIPHER_128: &str = "0aaef8924d5fb11ccc5f8f2a6f0ee510a4dca9a71efb";
    const CIPHER_256: &str = "fb6d7ab49a0929d18b0a7b38bbd2f41ea0be944d0630";

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn packet(encrypted: &str) -> MeshPacket {
        MeshPacket {
            from: 0xdead_beef,
            id: 0x5ca1_ab1e,
            payload_variant: Some(PayloadVariant::Encrypted(hex(encrypted))),
            ..Default::default()
        }
    }

    fn text(packet: &MeshPacket) -> String {
        match &packet.payload_variant {
            Some(PayloadVariant::Decoded(data)) => {
                assert_eq!(data.portnum, PortNum::TextMessageApp as i32);
                String::from_utf8(data.payload.clone()).unwrap()
            }
            other => panic!("not decoded: {other:?}"),
        }
    }

    #[test]
    fn opens_what_python_encrypted_with_the_default_key() {
        let mut packet = packet(CIPHER_128);
        assert_eq!(
            Keyring::new::<&str>(&[])
                .open(&mut packet)
                .map(|(key, _)| key),
            Some(0)
        );
        assert_eq!(text(&packet), "Hola desde Galicia");
    }

    #[test]
    fn opens_with_a_secondary_256_bit_key() {
        let key: Vec<u8> = (0..32).collect();
        let keyring = Keyring::new(&[format!("'{}'", STANDARD.encode(&key))]);
        let mut packet = packet(CIPHER_256);

        assert_eq!(keyring.open(&mut packet).map(|(key, _)| key), Some(1));
        assert_eq!(text(&packet), "Hola desde Galicia");
    }

    #[test]
    fn a_wrong_key_leaves_the_packet_as_it_was() {
        let mut packet = packet(CIPHER_256);
        let before = packet.clone();

        assert_eq!(Keyring::new::<&str>(&[]).open(&mut packet), None);
        assert_eq!(packet, before);
    }

    #[test]
    fn short_psks_expand_as_the_apps_expand_them() {
        // "1PG7OiApB1nwvP+rz05pAg==" is what Python makes of PSK 2.
        assert_eq!(
            STANDARD.encode(expand_short_psk(2)),
            "1PG7OiApB1nwvP+rz05pAg=="
        );
        assert_eq!(expand_short_psk(1), DEFAULT_PSK);
    }

    #[test]
    fn bad_secondary_keys_are_skipped() {
        let keyring = Keyring::new(&["AA==", "not base64!", "AQI=", "", "\"Ag==\""]);
        // Only "Ag==" (PSK 2) survives: 0 is no encryption, the next is not
        // base64, and two bytes is no key length.
        assert_eq!(keyring.len(), 2);
    }
}
