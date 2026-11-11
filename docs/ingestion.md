# Ingestion

The ingester (`run` or `ingest`) subscribes to the configured MQTT topics, opens
each Meshtastic message it receives, and stores it. What it stores is what the
Python meshview 4.0 stores from the same traffic, row for row and byte for byte:
that was checked by running both side by side on a public broker and comparing
the databases.

## The connection

| | |
|---|---|
| Protocol | MQTT 3.1.1, plain or over TLS (`[mqtt] tls`) |
| Client id | `meshview-rs-<random>`, new on every start, so two ingesters never throw each other off |
| Session | clean: every connection subscribes again |
| QoS | 0, at most once |
| Keep-alive | 60 seconds |
| Largest message | 1 MiB. Larger ones break the connection rather than being skipped, so the limit is generous; what is stored is limited far below it. |
| Reconnecting | 1 second after losing the broker, for ever |

Two tasks share the work. One does nothing but keep the connection and take
messages off it; the other opens and stores them, one at a time, from a queue of
up to 10,000. Kept apart, a slow write never stops the connection from answering
the broker's keep-alives. If the queue fills, because the database cannot keep up,
new messages are dropped and the count of those dropped is logged.

Every 10,000 messages the ingester logs how many it has processed and at what
rate.

## Opening a message

1. It has to be a `ServiceEnvelope`. Anything else is skipped silently.
2. Its packet is decrypted with the first key that works: the default key
   first, then each of `[mqtt] secondary_keys` in order. A packet sent in the
   clear needs none.
3. A packet from a node in `[mqtt] skip_node_ids` is dropped here.

Meshtastic encrypts a packet's payload with AES in CTR mode, under the channel's
key, with a nonce made of the packet id and the sender. Nothing authenticates
it, so the only sign that a key was the right one is that the result reads as a
`Data` message with a port number: a wrong key yields noise, and noise now and
then parses. A port of 0 is taken as noise, and the next key is tried.

A packet no key opens is skipped, logged at debug level. The first time a
secondary key opens something, that is logged once.

Keys are base64, 16 or 32 bytes (AES-128 or AES-256). A single byte is
Meshtastic's shorthand for a variant of the default key, as the apps write it:
`"AQ=="` is PSK 1, the default key itself, `"Ag=="` PSK 2, and so on. `"AA=="`,
PSK 0, means no encryption, which is not a key, and is skipped with a warning,
as is anything that is not base64 or not a valid length.

## What is stored

A message larger than **1024 bytes** once decrypted, or with a topic longer than
**256 characters**, is not stored at all: the largest packet the firmware can
produce is 381 bytes, and anything stored is later sent to every browser.

For every other message:

| Table | Written |
|---|---|
| `packet` | One row per packet id, the first time it is seen: port, sender, recipient, channel, time of import, and the packet itself, decrypted, as `payload`. |
| `packet_seen` | One row per packet, gateway and receive time: who relayed it to the broker, on which topic, with the signal (SNR, RSSI) and hops it arrived with. Not written when the gateway id is missing or malformed, which is logged. |
| `node` | Updated, or created, from what a packet says about its sender: see below. |
| `traceroute` | One row per traceroute packet per gateway that relays it. |

Times are microseconds since the epoch, UTC.

### Nodes

| Port | What it does to the node |
|---|---|
| `NODEINFO_APP` | Names, hardware model, role and channel, from the node's own description. Names are trimmed, and an empty one or one with control characters becomes `unknown name`. The node is found by its id as spelt (`!00abcd`), or failing that by number, since the same node may be stored under another spelling. |
| `POSITION_APP` | Latitude, longitude, and altitude when given. A node not seen before is created with its number as its long name and the last four hex digits as its short name. Positions within one degree of (0, 0), where devices without a fix say they are, are ignored. |
| `MAP_REPORT_APP` | Names, hardware model, role, firmware, and position, as the node reports itself to the broker. Applied before, and whatever becomes of, the packet. |

Every one of them also moves the node's last-seen time on. Gateways are marked as
such (`is_mqtt_gateway`) the first time this process sees them relay something.

Enum values the bundled protobufs do not know, from a newer firmware, are stored
as `unknown(N)` rather than refused.

### Fields of the wrong wire type

A field can arrive encoded as a type other than its own: a `fixed32` as a
varint, say. That is what garbage looks like, and some real packets carry it.
Python's protobuf takes such a field for one it does not know and reads the rest
of the message; prost alone would refuse the whole message. meshview-rs reads
messages as Python's protobuf does, so it keeps the packets the Python version
kept, with the odd field stored where it was, and it accepts what the Python
version accepted when a wrong key's decryption happens to parse.

### Fields these protobufs do not know

A firmware newer than the bundled protobufs sends fields they do not declare.
They are kept, not dropped: the stored packet has them where Python's protobuf
puts them, after the fields it knows, in the order they came, both in the packet
and inside its decrypted payload. Updating the protobufs is then enough to read
them in what is already stored.

meshview-rs supports the **official Meshtastic firmware** and its protobufs only.
Some gateways run modified firmware that adds fields of its own (fields 63 and
64 of `MeshPacket` have been seen, for one); those are kept like any other
unknown field, but will not be declared or interpreted.

## Inherited from the Python version

This stage of the port reproduces the Python version exactly, including a few
things it does that are odd. They are listed here so that changing them later is
a decision rather than an accident:

- **Gateways are marked once per process.** The mark is remembered even when the
  gateway's node does not exist yet, in which case nothing is marked until the
  next start reloads the list from the database.
- **Map report names are stored as sent**, unlike node info names, which are
  cleaned.
- **Map reports without a packet id** are stored under a negative id made of the
  time and the sender, so that every report is kept rather than only the first.
  Other packets without an id are not stored.
- **Traceroutes get one row per gateway** that relays them, under the packet's
  id, rather than one per traceroute.
- **A packet from node 0** creates node 0 when it carries a position.
- **Map reports go to node 1**: their recipient is stored as 1, standing for the
  broker.

## Daily jobs

The writer, `run` or `ingest`, also runs two jobs once a day, at a
[local time](configuration.md#local-time) of day each. They are the Python
version's, and do the same: run side by side with it on the same data, both
leave identical databases.

| Job | When | What |
|---|---|---|
| Cleanup | `[cleanup] hour`/`minute`, if `[cleanup] enabled` | Deletes the packets, sightings and traceroutes imported more than `days_to_keep` days ago, and the nodes not seen since, all in one transaction; then, with `vacuum`, runs `VACUUM`. Each table goes by its own time: a sighting today of an old packet stays. Nodes never seen at all are kept. |
| Snapshot | `[snapshot] hour`/`minute`, always | Records the previous complete UTC day in `daily_snapshot`: the nodes active that day (they sent a packet, or an older packet of theirs was heard again), the packets imported, and the gateways that relayed anything. Running it again for the same day replaces what was recorded. |

The snapshot is what keeps the history the cleanup would otherwise take away:
with 14 days kept, a month ago's packets are gone, but how many there were each
day is still there.

Both share the writer's one connection with the ingester, so they never write
at the same time. While a cleanup deletes, and vacuums, messages keep arriving
and wait in the ingester's queue, as the Python version paused its ingestion.

Every run is recorded in the `job_run` table, with when it started and finished,
how it ended (`ok`, `error`, or `interrupted` when the process stopped in the
middle of it), and what it did: for a cleanup the cutoff and the rows deleted
from each table, for a snapshot the figures recorded. The last 90 runs of each
job are kept. The Python version wrote the last cleanup to a JSON file for its
web server to read; a table is readable by every process that can open the
database, which is what the web server will report it from.

A job the process is stopped in the middle of is abandoned, which rolls back
what it had not committed: a cleanup either deletes everything it was going to
or nothing.
