<div align="center">

<img src="meshview/assets/static/meshview-icon.svg" alt="Meshview" width="128" height="128">

# Meshview

**Real-time monitoring and diagnostics for [Meshtastic](https://meshtastic.org/) mesh networks**

*A live map of the mesh, the packets it carries and the paths they take.*

[![CI](https://img.shields.io/github/actions/workflow/status/meshview/meshview/ci.yml?branch=master&style=for-the-badge&label=CI&logo=githubactions&logoColor=white)](https://github.com/meshview/meshview/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/meshview/meshview?style=for-the-badge&label=version&color=245ea8)](https://github.com/meshview/meshview/releases/latest)
[![License: AGPL v3](https://img.shields.io/badge/license-AGPL--3.0-245ea8?style=for-the-badge)](LICENSE)
<br>
[![Rust](https://img.shields.io/badge/Rust-single%20static%20binary-e66100?style=for-the-badge&logo=rust&logoColor=white)](#running-it)
[![SQLite · PostgreSQL](https://img.shields.io/badge/SQLite%20%C2%B7%20PostgreSQL-4169e1?style=for-the-badge&logo=postgresql&logoColor=white)](docs/configuration.md#database-connection-strings)
[![Meshtastic over MQTT](https://img.shields.io/badge/Meshtastic-MQTT-67ea94?style=for-the-badge)](https://meshtastic.org/docs/software/integrations/mqtt/)
[![Container image](https://img.shields.io/badge/container-ghcr.io-2496ed?style=for-the-badge&logo=docker&logoColor=white)](https://github.com/meshview/meshview/pkgs/container/meshview)

[Website](https://meshview.github.io/) · [Documentation](docs/README.md) · [Running it](#running-it) · [Coming from Python](docs/migrating-from-python.md) · [API](docs/api.md) · [MeshviewWorld](https://meshview.world)

<img src="https://meshview.github.io/shots/en-dashboard.webp" alt="Meshview's dashboard: the live map beside the latest packets" width="860">

</div>

---

Meshview reads packets from an MQTT broker into a database and serves a web
interface and an API over them: a live map of the nodes, the packets they send,
the paths those packets take, and statistics about all of it. This is its
rewrite in Rust, which takes over from the
[Python version](https://github.com/pablorevilla-meshtastic/meshview): its
databases, its configuration and its API carry over.

What ships is a single binary, statically linked and bringing its own libc, with
everything it serves inside it.

## Status

Everything the Python Meshview 4.0 does, it does: it reads its configuration,
takes a Python Meshview database over or creates one, in SQLite or in
PostgreSQL, ingests from MQTT (plain or over TLS), runs the daily cleanup and
snapshot, makes consistent backups, and serves the API and the web pages.

Ingestion stores what the Python version stores, byte for byte: run side by side
on the same broker, both write identical databases. The API answers what the
Python version answers, apart from the bugs it fixes: every packet, traceroute
and node contact of a real database has been compared.

[What is done](docs/README.md#what-is-done).

## Running it

```sh
cp sample.config.toml config.toml   # and set [mqtt] to your broker
meshview                          # ingests and serves on http://localhost:8081
```

Or with the container image, configured from the environment:

```sh
podman run -d --name meshview -p 8081:8081 -v meshview-data:/data \
  -e MESHVIEW_MQTT__SERVER=mqtt.example.org \
  -e MESHVIEW_MQTT__TOPICS='["msh/EU_868/#"]' \
  ghcr.io/meshview/meshview
```

One binary, four roles: `run` (the default) ingests and serves; `ingest` and
`serve` split that, so one ingester can feed several web servers; `migrate`
brings the database up to date and exits. And `meshview backup <file>`
copies the database, consistently, for a scheduler to run.

## Documentation

- [Configuration](docs/configuration.md): the file, environment variables, and
  every key.
- [Deployment](docs/deployment.md): roles, probes, stopping, the container image
  and Kubernetes.
- [Coming from the Python Meshview](docs/migrating-from-python.md): taking a
  database over and converting `config.ini`.
- [Ingestion](docs/ingestion.md): what is read from MQTT and what is stored.
- [API](docs/api.md): where the reference is, and what changed from the Python
  version.
- [Development](docs/development.md): building, testing, protobufs, CI and
  releases.

## License

AGPL-3.0-or-later, as the Python Meshview it comes from. See [LICENSE](LICENSE).

The vendored Meshtastic protobufs are GPL-3.0; their license is beside them.
