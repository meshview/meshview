Real-time data from a Meshtastic mesh network: its nodes, the packets they send and the paths those packets take, as MQTT gateways hear them.

## Conventions

- Everything is read-only and answered as JSON, errors too: `{"error": "..."}`, with 400 for a parameter that is not valid, 404 for something that is not there, 500 for a failure on the server's side and 503 while the database is not ready.
- No token or key is needed: whoever reaches the server can ask. A reverse proxy in front of it may ask for more.
- Times ending in `_us`, and the `since` and `before` parameters, are microseconds since the Unix epoch. The hours and days of statistics and snapshots are UTC.
- A node's position is in whole numbers, degrees times 10 000 000: divide `last_lat` and `last_long` by 10 000 000. Coverage answers in degrees already.
- A node is known by its number, `node_id`, the Meshtastic node number; its `id` is the same number as Meshtastic writes it, `!` and eight hex digits.
- A packet is known by its stored `id`, also answered as `storage_id`, which is what every `packet_id` parameter takes. A packet's `packet_id` field is something else: the id it was sent with over the air, as `reply_id` is. One cannot be used for the other.
- Numbers are written in decimal. The packets' sender and recipient filters, the node contact and the coverage also take `0x` hex.
- Text messages sent to everyone are shown as they are; those sent to one node come as `<redacted>`, as do waypoints and messages replayed from history sent to one node. Text messages are the packets of port 1: `/api/v1/packets?portnum=1`. What a packet says is text from the mesh, never HTML to trust.

## Versions

This is version 1, under `/api/v1`. The addresses without a version, such as `/api/nodes`, are the Python Meshview's: they answer 308 with the same address under `/api/v1`, and `Deprecation` and `Link` headers, and will go away. `/health`, `/version`, `/livez` and `/readyz` are outside any version.

## Examples

```sh
curl 'http://localhost:8081/api/v1/nodes?days_active=3'
curl 'http://localhost:8081/api/v1/packets?portnum=1&limit=100'
curl 'http://localhost:8081/api/v1/stats/count'
curl 'http://localhost:8081/api/v1/coverage/1234?radius_km=20'
```
