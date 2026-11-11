# meshview-rs documentation

| Document | What it covers |
|---|---|
| [Configuration](configuration.md) | Where the configuration is read from, environment variables, and every section and key. |
| [Deployment](deployment.md) | The four roles, probes, stopping, the binary, the container image and Kubernetes. |
| [Coming from the Python meshview](migrating-from-python.md) | Taking a database over, converting `config.ini`, and what is different. |
| [Ingestion](ingestion.md) | How messages are read from MQTT, decrypted, and what each one stores. |
| [API](api.md) | Where the reference is, and what changed from the Python version: versions, redirects and differences. |
| [Development](development.md) | Layout, tests, protobufs, comparing against the Python version, CI and releases. |

## What is ported so far

meshview-rs is being ported from the Python meshview 4.0 one part at a time,
each checked against the Python version's behaviour before it is called done.

| Part | State |
|---|---|
| Configuration, database takeover and migrations | Done |
| Giving a database back to the Python version | Done, while the schema is unchanged |
| MQTT ingestion (packets, sightings, nodes, traceroutes) | Done, identical databases side by side with Python |
| MQTT over TLS | Done (new in meshview-rs) |
| Probes, API reference, redirects from the unversioned API | Done |
| Clean shutdown | Done |
| Daily cleanup and snapshot | Done, identical databases side by side with Python |
| Consistent backups (`meshview-rs backup`) | Done (new in meshview-rs); the Python version's scheduled backups are deprecated |
| API | Done, every endpoint but `coverage`, answering what the Python version answers |
| Web pages | Not yet |
| Coverage prediction | Later |
| PostgreSQL | Later |
