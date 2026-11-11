// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! An envelope made by the Python protobufs, opened and stored, has to leave
//! the rows the Python meshview leaves.

use meshview_rs::db::sqlite;
use meshview_rs::ingest::keys::Keyring;
use meshview_rs::ingest::opened::Opened;
use meshview_rs::ingest::store::{Outcome, Store};
use sqlx::Row;
use tempfile::TempDir;

/// A text message from 0xdeadbeef, encrypted with the default key and wrapped
/// in an envelope from gateway !12345678 on LongFast, as Python serialises it.
const ENVELOPE: &str = "0a420defbeadde15ffffffff18082a160aaef8924d5fb11ccc5f8f2a6f0ee510a4dca9\
a71efb351eaba15c3d00b0f36a450000c840480360a6ffffffffffffffff01780312084c6f6e67466173741a092131\
32333435363738";

/// What the Python meshview stores as that packet's payload: the MeshPacket
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
