// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The protobuf wire format, for the one thing prost will not do: keep the
//! fields it does not know.
//!
//! A newer firmware sends fields these protobufs do not declare yet. prost reads
//! past them and forgets them; Python's protobuf keeps them and writes them back
//! out, after the fields it knows, in the order they came. The Python meshview
//! stored packets that way, and so does this: the database keeps what was sent,
//! and updating the protobufs later is enough to read what is in it.

use crate::known_fields;

/// The wire types this format has, as they appear in a tag.
const VARINT: u8 = 0;
const FIXED64: u8 = 1;
const LEN: u8 = 2;
const START_GROUP: u8 = 3;
const END_GROUP: u8 = 4;
const FIXED32: u8 = 5;

/// One field as it appears in a message: its number, its wire type, the whole
/// of it as encoded (tag included), and for a length delimited one, its content.
#[derive(Debug, Clone, Copy)]
pub struct Field<'a> {
    pub number: u32,
    pub wire_type: u8,
    pub raw: &'a [u8],
    pub content: &'a [u8],
}

/// The fields of an encoded message, in order, or none at all if any of it is
/// not valid protobuf.
pub fn fields(mut bytes: &[u8]) -> Option<Vec<Field<'_>>> {
    let mut fields = Vec::new();
    while !bytes.is_empty() {
        let start = bytes;
        let tag = varint(&mut bytes)?;
        let number = u32::try_from(tag >> 3).ok()?;
        let wire_type = (tag & 7) as u8;
        if number == 0 {
            return None;
        }

        let content = match wire_type {
            VARINT => {
                varint(&mut bytes)?;
                &[][..]
            }
            FIXED64 => take(&mut bytes, 8)?,
            FIXED32 => take(&mut bytes, 4)?,
            LEN => {
                let len = usize::try_from(varint(&mut bytes)?).ok()?;
                take(&mut bytes, len)?
            }
            // Groups have been deprecated since proto2 and Meshtastic has none.
            // Anything carrying one is not something worth keeping as it is.
            START_GROUP | END_GROUP => return None,
            _ => return None,
        };

        let used = start.len() - bytes.len();
        fields.push(Field {
            number,
            wire_type,
            raw: &start[..used],
            content,
        });
    }
    Some(fields)
}

/// The fields of an encoded `message` that its declaration does not have, as
/// encoded and in the order they came.
pub fn unknown_fields(message: &str, bytes: &[u8]) -> Vec<u8> {
    let Some(known) = known_fields(message) else {
        return Vec::new();
    };
    let Some(fields) = fields(bytes) else {
        return Vec::new();
    };
    fields
        .iter()
        .filter(|field| known.binary_search(&field.number).is_err())
        .flat_map(|field| field.raw.iter().copied())
        .collect()
}

/// The content of the last occurrence of a length delimited field, which is the
/// one a parser keeps.
pub fn last_len_field(bytes: &[u8], number: u32) -> Option<&[u8]> {
    fields(bytes)?
        .into_iter()
        .rev()
        .find(|field| field.number == number && field.wire_type == LEN)
        .map(|field| field.content)
}

/// `encoded`, with `extra` appended to the content of its length delimited
/// field `number`, as though the message in it had had those fields all along.
pub fn extend_len_field(encoded: &[u8], number: u32, extra: &[u8]) -> Vec<u8> {
    if extra.is_empty() {
        return encoded.to_vec();
    }
    let Some(fields) = fields(encoded) else {
        return encoded.to_vec();
    };

    let mut out = Vec::with_capacity(encoded.len() + extra.len() + 2);
    for field in fields {
        if field.number == number && field.wire_type == LEN {
            put_varint(&mut out, (u64::from(number) << 3) | u64::from(LEN));
            put_varint(&mut out, (field.content.len() + extra.len()) as u64);
            out.extend_from_slice(field.content);
            out.extend_from_slice(extra);
        } else {
            out.extend_from_slice(field.raw);
        }
    }
    out
}

/// How many bytes a varint takes.
pub fn varint_len(value: u64) -> usize {
    let bits = 64 - (value | 1).leading_zeros() as usize;
    bits.div_ceil(7)
}

fn varint(bytes: &mut &[u8]) -> Option<u64> {
    let mut value = 0u64;
    for (i, byte) in bytes.iter().enumerate().take(10) {
        value |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            *bytes = &bytes[i + 1..];
            return Some(value);
        }
    }
    None
}

fn take<'a>(bytes: &mut &'a [u8], len: usize) -> Option<&'a [u8]> {
    if bytes.len() < len {
        return None;
    }
    let (taken, rest) = bytes.split_at(len);
    *bytes = rest;
    Some(taken)
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_are_measured_and_written_alike() {
        for value in [
            0,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            u64::from(u32::MAX),
            u64::MAX,
        ] {
            let mut out = Vec::new();
            put_varint(&mut out, value);
            assert_eq!(out.len(), varint_len(value), "{value}");
            assert_eq!(varint(&mut out.as_slice()), Some(value));
        }
    }

    #[test]
    fn unknown_fields_are_kept_in_order() {
        // from = 1 (fixed32), then field 63 as fixed32 and field 64 as a varint,
        // neither of which MeshPacket declares at this revision.
        let bytes = [
            0x0d, 1, 0, 0, 0, 0xfd, 0x03, 0xff, 0xbf, 0xe0, 0x42, 0x80, 0x04, 0x01,
        ];
        assert_eq!(
            unknown_fields("meshtastic.MeshPacket", &bytes),
            [0xfd, 0x03, 0xff, 0xbf, 0xe0, 0x42, 0x80, 0x04, 0x01]
        );
    }

    #[test]
    fn a_len_field_can_be_extended() {
        // field 4 = "ab", then field 6 as fixed32.
        let bytes = [0x22, 2, b'a', b'b', 0x35, 1, 2, 3, 4];
        assert_eq!(
            extend_len_field(&bytes, 4, b"cd"),
            [0x22, 4, b'a', b'b', b'c', b'd', 0x35, 1, 2, 3, 4]
        );
        assert_eq!(last_len_field(&bytes, 4), Some(&b"ab"[..]));
    }

    #[test]
    fn broken_input_is_none() {
        assert!(fields(&[0x22, 5, b'a']).is_none());
        assert!(fields(&[0x80]).is_none());
        assert!(fields(&[0x0b]).is_none());
    }
}
