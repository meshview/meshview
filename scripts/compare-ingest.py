#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>
"""Compare what the Python meshview and meshview-rs stored from the same traffic.

    scripts/compare-ingest.py python.db rust.db

Both databases are opened read only. Times of import differ by construction and
are left out; everything else has to match, and the script exits with 1 when it
does not. See docs/development.md for how to produce the two databases.

Only the standard library is used.
"""

import sqlite3
import sys
from collections import Counter

QUERIES = {
    "packet (real ids)": (
        "SELECT id, portnum, from_node_id, to_node_id, payload, channel FROM packet WHERE id > 0"
    ),
    # Map reports without an id are stored under one made of the time of import,
    # which differs, so they are compared by what they hold.
    "packet (map reports)": (
        "SELECT portnum, from_node_id, to_node_id, payload, channel FROM packet WHERE id < 0"
    ),
    "packet_seen": (
        "SELECT packet_id, node_id, rx_time, hop_limit, hop_start, channel, rx_snr, rx_rssi, "
        "topic FROM packet_seen WHERE packet_id > 0"
    ),
    "packet_seen (map reports)": (
        "SELECT s.node_id, s.rx_time, s.channel, s.topic, p.payload FROM packet_seen s "
        "JOIN packet p ON p.id = s.packet_id WHERE s.packet_id < 0"
    ),
    "node": (
        "SELECT id, node_id, long_name, short_name, hw_model, firmware, role, last_lat, "
        "last_long, last_altitude, channel, is_mqtt_gateway, first_seen_us IS NULL FROM node"
    ),
    "traceroute": "SELECT packet_id, gateway_node_id, done, route, route_return FROM traceroute",
}


def rows(database, sql):
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    try:
        return connection.execute(sql).fetchall()
    finally:
        connection.close()


def shown(row):
    return tuple(value.hex()[:80] if isinstance(value, bytes) else value for value in row)


def compare(name, python, rust, show=5):
    python, rust = Counter(python), Counter(rust)
    only_python, only_rust = python - rust, rust - python
    same = not only_python and not only_rust
    print(
        f"{'OK' if same else 'DIFF':4} {name}: python={python.total()} rust={rust.total()} "
        f"only_python={only_python.total()} only_rust={only_rust.total()}"
    )
    for label, difference in (("  python only", only_python), ("  rust only  ", only_rust)):
        for row in list(difference)[:show]:
            print(label, shown(row))
    return same


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    python_db, rust_db = sys.argv[1:]
    results = [compare(name, rows(python_db, sql), rows(rust_db, sql)) for name, sql in QUERIES.items()]
    print("ALL MATCH" if all(results) else "DIFFERENCES FOUND")
    sys.exit(0 if all(results) else 1)


if __name__ == "__main__":
    main()
