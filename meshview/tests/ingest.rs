// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! An envelope made by the Python protobufs, opened and stored, has to leave
//! the rows the Python Meshview leaves.

use meshview::db::sqlite;
use meshview::ingest::keys::Keyring;
use meshview::ingest::opened::Opened;
use meshview::ingest::store::{Outcome, Store};
use sqlx::Row;
use tempfile::TempDir;

/// A text message from 0xdeadbeef, encrypted with the default key and wrapped
/// in an envelope from gateway !12345678 on LongFast, as Python serialises it.
const ENVELOPE: &str = "0a420defbeadde15ffffffff18082a160aaef8924d5fb11ccc5f8f2a6f0ee510a4dca9\
a71efb351eaba15c3d00b0f36a450000c840480360a6ffffffffffffffff01780312084c6f6e67466173741a092131\
32333435363738";

/// What the Python Meshview stores as that packet's payload: the MeshPacket
/// with the decrypted Data in place of the encrypted bytes.
const STORED: &str = "0defbeadde15ffffffff1808221608011212486f6c612064657364652047616c69636961\
351eaba15c3d00b0f36a450000c840480360a6ffffffffffffffff017803";

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[tokio::test]
async fn a_python_envelope_is_stored_as_python_stores_it() {
    let dir = TempDir::new().unwrap();
    let pool = sqlite::open_for_writing(&dir.path().join("packets.db"))
        .await
        .unwrap();
    let mut store = Store::new(pool.clone()).await.unwrap();

    let opened = Opened::open(&hex(ENVELOPE), &Keyring::new::<&str>(&[])).unwrap();
    assert_eq!(opened.key, 0);
    assert!(opened.was_encrypted);
    assert_eq!(opened.packet_bytes, hex(STORED));

    let topic = "msh/EU_868/2/e/LongFast/!12345678";
    assert_eq!(
        store.process(topic, &opened).await.unwrap(),
        Outcome::Stored
    );
    // The same message from the same gateway again changes nothing.
    assert_eq!(
        store.process(topic, &opened).await.unwrap(),
        Outcome::Stored
    );

    let packet = sqlx::query("SELECT * FROM packet")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(packet.get::<i64, _>("id"), 0x5ca1_ab1e);
    assert_eq!(packet.get::<i64, _>("portnum"), 1);
    assert_eq!(packet.get::<i64, _>("from_node_id"), 0xdead_beef);
    assert_eq!(packet.get::<i64, _>("to_node_id"), 0xffff_ffff);
    assert_eq!(packet.get::<String, _>("channel"), "LongFast");
    assert_eq!(packet.get::<Vec<u8>, _>("payload"), hex(STORED));

    let seen = sqlx::query("SELECT * FROM packet_seen")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(seen.len(), 1);
    let seen = &seen[0];
    assert_eq!(seen.get::<i64, _>("node_id"), 0x1234_5678);
    assert_eq!(seen.get::<i64, _>("rx_time"), 1_794_355_200);
    assert_eq!(seen.get::<f64, _>("rx_snr"), 6.25);
    assert_eq!(seen.get::<i64, _>("rx_rssi"), -90);
    assert_eq!(seen.get::<i64, _>("hop_limit"), 3);
    assert_eq!(seen.get::<i64, _>("hop_start"), 3);
    assert_eq!(seen.get::<String, _>("topic"), topic);
    // The firmware did not say who it heard it from.
    assert_eq!(seen.get::<Option<i64>, _>("relay_node"), None);
}

#[tokio::test]
async fn who_a_gateway_heard_a_packet_from_is_kept() {
    let dir = TempDir::new().unwrap();
    let pool = sqlite::open_for_writing(&dir.path().join("packets.db"))
        .await
        .unwrap();
    let mut store = Store::new(pool.clone()).await.unwrap();

    // The same envelope, its packet with relay_node (field 19) set to 0xa2,
    // as the firmware sets it: outside the encryption, at the packet's end.
    let mut envelope = hex(ENVELOPE);
    let packet_len = usize::from(envelope[1]);
    envelope.splice(2 + packet_len..2 + packet_len, [0x98, 0x01, 0xa2, 0x01]);
    envelope[1] += 4;
    let opened = Opened::open(&envelope, &Keyring::new::<&str>(&[])).unwrap();
    assert_eq!(
        store
            .process("msh/EU_868/2/e/LongFast/!12345678", &opened)
            .await
            .unwrap(),
        Outcome::Stored
    );

    let relay: Option<i64> = sqlx::query_scalar("SELECT relay_node FROM packet_seen")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(relay, Some(0xa2));
}

/// From a firmware newer than these protobufs: Data carries a field 99 inside
/// the encryption, the packet fields 63 and 64, and the envelope a field 4.
const WITH_UNKNOWNS: &str = "0a300d0d0c0b0a15ffffffff2a0f1a1b914458f1fd5dc3f16c2e1afc0735040302013d00\
b0f36a4803fd03ffbfe04280040112084c6f6e67466173741a092131323334353637382005";

/// What Python stores for it, every unknown field where Python puts it.
const WITH_UNKNOWNS_STORED: &str = "0d0d0c0b0a15ffffffff220f08011208756e6b6e6f776e7398060735040302013d\
00b0f36a4803fd03ffbfe042800401";

#[test]
fn fields_these_protobufs_do_not_know_are_kept_where_python_keeps_them() {
    let opened = Opened::open(&hex(WITH_UNKNOWNS), &Keyring::new::<&str>(&[])).unwrap();

    assert_eq!(opened.packet_bytes, hex(WITH_UNKNOWNS_STORED));
    // Python's ByteSize() of the decrypted envelope.
    assert_eq!(opened.size, 73);
}

/// A text message whose Data has its field 4, `dest`, a fixed32, sent as a
/// varint: Python's protobuf takes it for unknown, keeps it, and stores the
/// packet; prost alone would have refused the envelope.
const WRONG_WIRE_TYPE: &str = "0a1c0d0d0c0b0a15ffffffff220b08011204686f6c6120f70a350403020112084c6f6e\
67466173741a09213132333435363738";

/// What Python stores for it, the odd field kept where it was.
const WRONG_WIRE_TYPE_STORED: &str = "0d0d0c0b0a15ffffffff220b08011204686f6c6120f70a3504030201";

#[test]
fn a_field_of_the_wrong_wire_type_is_kept_as_python_keeps_it() {
    let opened = Opened::open(&hex(WRONG_WIRE_TYPE), &Keyring::new::<&str>(&[])).unwrap();

    assert!(!opened.was_encrypted);
    assert_eq!(opened.packet_bytes, hex(WRONG_WIRE_TYPE_STORED));
    // Python's ByteSize() of the envelope.
    assert_eq!(opened.size, 51);
}

/// A node info from `from` to `to`, sent in the clear on `channel`.
fn node_info(id: u32, from: u32, to: u32, channel: &str) -> Opened {
    node_info_with_key(id, from, to, channel, Vec::new())
}

/// The same, with this public key.
fn node_info_with_key(id: u32, from: u32, to: u32, channel: &str, key: Vec<u8>) -> Opened {
    use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
    use meshview_proto::meshtastic::{Data, MeshPacket, PortNum, ServiceEnvelope, User};
    use prost::Message;

    let envelope = ServiceEnvelope {
        packet: Some(MeshPacket {
            id,
            from,
            to,
            payload_variant: Some(PayloadVariant::Decoded(Data {
                portnum: PortNum::NodeinfoApp as i32,
                payload: User {
                    id: format!("!{from:08x}"),
                    long_name: "Gateway".into(),
                    short_name: "GW".into(),
                    public_key: key,
                    ..Default::default()
                }
                .encode_to_vec(),
                ..Default::default()
            })),
            ..Default::default()
        }),
        channel_id: channel.into(),
        gateway_id: "!12345678".into(),
    };
    Opened::open(&envelope.encode_to_vec(), &Keyring::new::<&str>(&[])).unwrap()
}

// A node broadcasts its node info only on its primary channel, but answers a
// node it has just found with one sent to that node alone, on whichever
// channel reaches it: only the
// broadcasts tell the node's channel, unless nothing else has yet.
#[tokio::test]
async fn a_node_info_to_one_node_does_not_move_the_node_to_its_channel() {
    let dir = TempDir::new().unwrap();
    let pool = sqlite::open_for_writing(&dir.path().join("packets.db"))
        .await
        .unwrap();
    let mut store = Store::new(pool.clone()).await.unwrap();
    let channel = async || {
        sqlx::query_scalar::<_, String>("SELECT channel FROM node WHERE node_id = 3735928559")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    let gateway = 0xdead_beef;

    // Heard first through one sent to a single node: its channel is all there is.
    let to_one = node_info(1, gateway, 0x0102_0304, "Secondary");
    store
        .process("msh/EU_868/2/e/Secondary/!12345678", &to_one)
        .await
        .unwrap();
    assert_eq!(channel().await, "Secondary");

    let broadcast = node_info(2, gateway, 0xffff_ffff, "LongFast");
    store
        .process("msh/EU_868/2/e/LongFast/!12345678", &broadcast)
        .await
        .unwrap();
    assert_eq!(channel().await, "LongFast");

    let to_one = node_info(3, gateway, 0x0102_0304, "Secondary");
    store
        .process("msh/EU_868/2/e/Secondary/!12345678", &to_one)
        .await
        .unwrap();
    assert_eq!(channel().await, "LongFast");
}

// The key is kept from the latest node info that has a whole one, to find the
// nodes that share it.
#[tokio::test]
async fn a_node_keeps_its_latest_whole_key() {
    let dir = TempDir::new().unwrap();
    let pool = sqlite::open_for_writing(&dir.path().join("packets.db"))
        .await
        .unwrap();
    let mut store = Store::new(pool.clone()).await.unwrap();
    let key = async || {
        sqlx::query_scalar::<_, Option<Vec<u8>>>(
            "SELECT public_key FROM node WHERE node_id = 3735928559",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let topic = "msh/EU_868/2/e/LongFast/!12345678";
    let node = 0xdead_beef;

    let first = node_info_with_key(1, node, 0xffff_ffff, "LongFast", vec![1; 32]);
    store.process(topic, &first).await.unwrap();
    assert_eq!(key().await, Some(vec![1; 32]));

    // None, or one cut short, leaves it.
    for (id, short) in [(2, Vec::new()), (3, vec![2; 16])] {
        let without = node_info_with_key(id, node, 0xffff_ffff, "LongFast", short);
        store.process(topic, &without).await.unwrap();
        assert_eq!(key().await, Some(vec![1; 32]));
    }

    let made_anew = node_info_with_key(4, node, 0xffff_ffff, "LongFast", vec![3; 32]);
    store.process(topic, &made_anew).await.unwrap();
    assert_eq!(key().await, Some(vec![3; 32]));
}

/// A message from `from` of `portnum` carrying `payload`, sent in the clear.
fn message(id: u32, from: u32, portnum: i32, payload: Vec<u8>) -> Opened {
    use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
    use meshview_proto::meshtastic::{Data, MeshPacket, ServiceEnvelope};
    use prost::Message;

    let envelope = ServiceEnvelope {
        packet: Some(MeshPacket {
            id,
            from,
            to: 0xffff_ffff,
            payload_variant: Some(PayloadVariant::Decoded(Data {
                portnum,
                payload,
                ..Default::default()
            })),
            ..Default::default()
        }),
        channel_id: "LongFast".into(),
        gateway_id: "!12345678".into(),
    };
    Opened::open(&envelope.encode_to_vec(), &Keyring::new::<&str>(&[])).unwrap()
}

// The precision of the latest position or map report is kept with the node,
// for the map: 0 when it says none, and a position with no coordinates counts.
#[tokio::test]
async fn a_node_keeps_the_precision_of_its_latest_position() {
    use meshview_proto::meshtastic::{MapReport, PortNum, Position};
    use prost::Message;

    let dir = TempDir::new().unwrap();
    let pool = sqlite::open_for_writing(&dir.path().join("packets.db"))
        .await
        .unwrap();
    let mut store = Store::new(pool.clone()).await.unwrap();
    let precision = async || {
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT position_precision FROM node WHERE node_id = 3735928559",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let topic = "msh/EU_868/2/e/LongFast/!12345678";
    let node = 0xdead_beef;
    let position = |bits: u32, located: bool| {
        Position {
            latitude_i: located.then_some(433_000_000),
            longitude_i: located.then_some(-83_000_000),
            precision_bits: bits,
            ..Default::default()
        }
        .encode_to_vec()
    };

    let located = message(1, node, PortNum::PositionApp as i32, position(13, true));
    store.process(topic, &located).await.unwrap();
    assert_eq!(precision().await, Some(13));

    let nowhere = message(2, node, PortNum::PositionApp as i32, position(0, false));
    store.process(topic, &nowhere).await.unwrap();
    assert_eq!(precision().await, Some(0));

    let report = MapReport {
        long_name: "Gateway".into(),
        latitude_i: 433_000_000,
        longitude_i: -83_000_000,
        position_precision: 40,
        ..Default::default()
    };
    let report = message(
        3,
        node,
        PortNum::MapReportApp as i32,
        report.encode_to_vec(),
    );
    store.process(topic, &report).await.unwrap();
    assert_eq!(precision().await, Some(32));
}
