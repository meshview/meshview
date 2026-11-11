# meshview-rs

Real-time monitoring and diagnostics for [Meshtastic](https://meshtastic.org/)
mesh networks, written in Rust.

meshview-rs is a port of [meshview](https://github.com/pablorevilla-meshtastic/meshview)
from Python. It reads packets from an MQTT broker into a database and serves a
web interface and an API over them: a live map of the nodes, the packets they
send, the paths those packets take, and statistics about all of it.

What ships is a single binary, statically linked and bringing its own libc, with
everything it serves inside it.

## Status

Early days. It reads the configuration, takes over or creates the database,
ingests from MQTT, and answers the probes and the API reference. The API itself
and the web pages are being ported next, against the behaviour of the Python
meshview 4.0.

Ingestion stores what the Python version stores, byte for byte: run side by side
on the same broker, both write the same packets, sightings, nodes and
traceroutes. That includes the fields a firmware newer than the bundled
protobufs sends, which are kept rather than dropped, so updating the protobufs
later is enough to read them.

## Roles

One binary, four ways to run it:

| Command | What it does |
|---|---|
| `meshview-rs run` | Ingests from MQTT and serves the web, in one process. The default. |
| `meshview-rs ingest` | Only ingests. The one process that writes to the database, and the only one that migrates it. |
| `meshview-rs serve` | Only serves, reading the database read only. Never creates or migrates it: until the ingester has, it waits and reports itself not ready. |
| `meshview-rs migrate` | Brings the database's schema up to date and exits. |

Splitting them lets one ingester feed several web servers. With SQLite they
have to be on the same machine, because SQLite's WAL mode shares memory between
the processes using the file: never put the database on a network filesystem
(NFS, CephFS, any `ReadWriteMany` volume). Spreading web servers across machines
will need PostgreSQL, which is planned but not supported yet.

Every role listens on the configured port and answers:

- `GET /livez`: 200 while the process is running.
- `GET /readyz`: 200 once it can serve, 503 with the list of what it is waiting
  for until then. For `ingest` that includes being connected to the broker; for
  `run` it does not, since the web can still be served while the broker is away.

## Configuration

The configuration is a TOML file. meshview-rs looks for `./config.toml`, or the
file named with `--config` or `MESHVIEW_CONFIG`, and runs on its defaults when
there is none. [sample.config.toml](sample.config.toml) has every key, with its
default or an example.

It only ever reads the file: the configuration can live somewhere meshview-rs
cannot write to, as it usually should. A key or section it does not know is an
error rather than something quietly ignored.

Any value can be set from the environment as `MESHVIEW_<SECTION>__<KEY>`, which
wins over the file. Each is read as the type its key has, so a numeric password
stays a string and a list is written as one:

```sh
MESHVIEW_DATABASE__CONNECTION_STRING=sqlite:////var/lib/meshview/packets.db \
MESHVIEW_MQTT__TOPICS='["msh/EU_868/#"]' \
MESHVIEW_MQTT__TLS=true \
meshview-rs ingest
```

MQTT can also run over TLS, which the Python version could not: `tls = true` in
`[mqtt]`, optionally with `tls_ca_file` for a broker with its own CA and
`tls_client_cert`/`tls_client_key` for one that wants a client certificate. The
public roots are built into the binary, so the container image needs no CA
bundle.

Connection strings keep the SQLAlchemy form the Python version used: three
slashes before a relative path (`sqlite:///packets.db`), four before an absolute
one (`sqlite:////data/packets.db`).

## Coming from the Python meshview

meshview-rs takes over a database from the Python version only at its last
schema, Alembic revision `44612e74a798`, which is what meshview 4.0 leaves. A
database at an earlier revision has to be opened once by the Python meshview 4.0
first, so that it migrates it; meshview-rs will refuse it and say so until then.

1. Stop the Python meshview, both the database writer and the web server.
2. Back the database up.
3. Convert `config.ini` to `config.toml`. The sections and keys are the same;
   what changes is the syntax:

   | `config.ini` | `config.toml` |
   |---|---|
   | `title = Bay Area Mesh` | `title = "Bay Area Mesh"` |
   | `nodes = True` | `nodes = true` |
   | `port = 1883` | `port = 1883` |
   | `topics = ["msh/US/bayarea/#"]` | `topics = ["msh/US/bayarea/#"]` |
   | `skip_node_ids = 0xdeadbeef, 305419896` | `skip_node_ids = [0xdeadbeef, 305419896]` |
   | `secondary_keys = AQ==, 1PG7OiApB1nwvP+rz05pAQ==` | `secondary_keys = ["AQ==", "1PG7OiApB1nwvP+rz05pAQ=="]` |
   | `LongFast = #4363d8` | `LongFast = "#4363d8"` |
   | `minute = 00` | `minute = 0` |

   meshview-rs will not start from a directory with a `config.ini` and no
   `config.toml`, rather than start on its defaults and miss your database.
4. Run `meshview-rs migrate` against the database (or just start `meshview-rs`).

The takeover is one way. Once meshview-rs has migrated a database, it removes
the Python version's bookkeeping, and going back means restoring the backup.

## API

The API is versioned under `/api/v1`, and its reference is served at
[`/api/docs`](http://localhost:8081/api/docs), built from the same code that
registers the routes. The OpenAPI document itself is at `/api/openapi.json`.

The Python version's unversioned addresses (`/api/nodes`, `/api/stats/top`, ...)
are redirected with a 308 to their `/api/v1` counterparts, with a `Deprecation`
header and a `Link` to the successor. They will be removed once nothing uses
them any more.

## Container image

```sh
podman run -d --name meshview -p 8081:8081 \
  -v meshview-data:/data \
  ghcr.io/ogarcia/meshview-rs
```

The database lives in `/data`, which is also the working directory, so a
`config.toml` mounted there is read without saying where it is. The image runs as
UID 1000.

Building it yourself compiles the binary inside the build:

```sh
podman build -t meshview-rs .
```

## Development

```sh
git config core.hooksPath .githooks   # once per clone: fmt and clippy before each commit
cargo test --workspace
```

The Meshtastic protobufs are vendored under `meshview-proto/protobufs`, at the
revision in `UPSTREAM_REV`, and compiled with [protox](https://crates.io/crates/protox),
so `protoc` is not needed. `scripts/update-protobufs.sh <revision>` replaces
them.

## License

AGPL-3.0-or-later, as the Python meshview it comes from. See [LICENSE](LICENSE).

The vendored Meshtastic protobufs are GPL-3.0; their license is beside them.
