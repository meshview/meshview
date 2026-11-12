# Meshview documentation

| Document | What it covers |
|---|---|
| [Configuration](configuration.md) | Where the configuration is read from, environment variables, and every section and key. |
| [Deployment](deployment.md) | The four roles, probes, stopping, the binary, the container image and Kubernetes. |
| [Coming from the Python Meshview](migrating-from-python.md) | Taking a database over, converting `config.ini`, and what is different. |
| [Ingestion](ingestion.md) | How messages are read from MQTT, decrypted, and what each one stores. |
| [API](api.md) | Where the reference is, and what changed from the Python version: versions, redirects and differences. |
| [Development](development.md) | Layout, tests, migrations, protobufs, the API reference, the web pages, CI and releases. |

## What is done

Meshview was rewritten from the Python Meshview 4.0 one part at a time, each
checked against the Python version's behaviour before it was called done.

| Part | State |
|---|---|
| Configuration, database takeover and migrations | Done |
| Giving a database back to the Python version | Done, while the schema is unchanged |
| MQTT ingestion (packets, sightings, nodes, traceroutes) | Done, identical databases side by side with Python |
| MQTT over TLS | Done (new in Meshview) |
| Probes, API reference, redirects from the unversioned API | Done |
| Clean shutdown | Done |
| Daily cleanup and snapshot | Done, identical databases side by side with Python |
| Consistent backups (`meshview backup`) | Done (new in Meshview); the Python version's scheduled backups are deprecated |
| API | Done, answering what the Python version answers |
| Web pages | Done, the Python version's converted to Tera 2 |
| Coverage prediction | Done, with NTIA's model, in the [`longley-rice`](../longley-rice) crate |
| PostgreSQL | Done: the Python version's schema, and the same answers as SQLite |
