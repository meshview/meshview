// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Packets as the web pages show them, which has to be what the Python
//! meshview showed.
//!
//! `python-packet-texts.json` is real packets from a public mesh, three of each
//! port and a few more, with what the Python meshview's `/api/packets?packet_id=`
//! answered for each: their stored bytes, their own id, and the text shown. One
//! of them has a field of the wrong wire type, which Python's protobuf takes for
//! unknown and prost alone would refuse the whole packet for.
//!
//! Waypoints, node statuses and store and forward the Python version did not
//! decode; meshview-rs does, and those are checked to be decoded, or redacted,
//! instead of to match.

use meshview_rs::decode::{self, DID_NOT_DECODE, REDACTED, python_strip};

/// The ports meshview-rs decodes and the Python version did not: waypoints,
/// node statuses and store and forward.
const DECODED_SINCE_PYTHON: [i64; 3] = [8, 36, 65];
use serde::Deserialize;

#[derive(Deserialize)]
struct Sample {
    id: i64,
    portnum: Option<i64>,
    to_node_id: Option<i64>,
    payload: String,
    packet_id: Option<i64>,
    text: String,
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn packets_are_shown_as_python_showed_them() {
    let samples: Vec<Sample> =
        serde_json::from_str(include_str!("fixtures/python-packet-texts.json")).unwrap();
    assert!(samples.len() > 50);

    for sample in samples {
        let shown = decode::show(
            Some(&hex(&sample.payload)),
            sample.portnum,
            sample.to_node_id,
        );
        if sample
            .portnum
            .is_some_and(|port| DECODED_SINCE_PYTHON.contains(&port))
        {
            assert_eq!(sample.text, DID_NOT_DECODE, "packet {}", sample.id);
            assert_ne!(shown.text, DID_NOT_DECODE, "packet {}", sample.id);
            if sample.to_node_id != Some(decode::BROADCAST) && sample.portnum == Some(8) {
                assert_eq!(shown.text, REDACTED, "packet {}", sample.id);
            }
        } else {
            assert_eq!(
                python_strip(&shown.text),
                sample.text,
                "packet {} (port {:?})",
                sample.id,
                sample.portnum
            );
        }
        assert_eq!(shown.packet_id(), sample.packet_id, "packet {}", sample.id);
    }
}

#[test]
fn text_messages_are_shown_to_everyone_and_redacted_when_direct() {
    use meshview_proto::meshtastic::{Data, MeshPacket, PortNum, mesh_packet::PayloadVariant};
    use prost::Message;

    let packet = MeshPacket {
        id: 7,
        payload_variant: Some(PayloadVariant::Decoded(Data {
            portnum: PortNum::TextMessageApp as i32,
            payload: b"  hola  ".to_vec(),
            reply_id: 5,
            ..Default::default()
        })),
        ..Default::default()
    }
    .encode_to_vec();

    let broadcast = decode::show(Some(&packet), Some(1), Some(decode::BROADCAST));
    assert_eq!(python_strip(&broadcast.text), "hola");
    assert_eq!(broadcast.reply_id(), Some(5));
    assert_eq!(
        decode::show(Some(&packet), Some(1), Some(42)).text,
        decode::REDACTED
    );
    // No recipient known is no broadcast either.
    assert_eq!(
        decode::show(Some(&packet), Some(1), None).text,
        decode::REDACTED
    );

    assert_eq!(
        decode::show(None, Some(1), None).text,
        decode::DID_NOT_DECODE
    );
    assert_eq!(
        decode::show(Some(b"\xff"), Some(1), None).text,
        decode::DID_NOT_DECODE
    );
}

#[test]
fn link_tests_are_recognised() {
    assert!(decode::is_sequence_only("seq 12"));
    assert!(!decode::is_sequence_only("seq "));
    assert!(!decode::is_sequence_only("seq 12 hola"));
    assert!(!decode::is_sequence_only("sequence 12"));
}
