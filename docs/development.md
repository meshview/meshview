# Development

## Layout

A Cargo workspace of two crates. The line between them is compile time: the
protobufs are a lot of generated code that changes only when the `.proto` files
do, and in a crate of their own they are built once and left alone.

| Path | What it is |
|---|---|
| `meshview-proto/` | The Meshtastic protobufs, vendored under `protobufs/` and compiled by prost, and their descriptors. Also `wire`, for what prost reads differently from Python's protobuf (unknown fields, fields of the wrong wire type), and `text_format`, the protobuf text format as Python writes it. |
| `meshview-rs/src/config.rs` | The configuration: TOML, environment variables, defaults. |
| `meshview-rs/src/db/` | The database. `mod.rs` is what holds for any backend; `sqlite.rs` everything that knows it is SQLite, so that PostgreSQL can be a sibling module. |
| `meshview-rs/migrations/sqlite/` | The SQLite migrations, compiled into the binary. |
| `meshview-rs/assets/` | What the web side serves and is built into the binary: so far the translation catalogs under `lang/`, the Python version's. |
| `meshview-rs/src/ingest/` | MQTT ingestion: the connection (`mod.rs`), keys and decryption (`keys.rs`), opening a message (`opened.rs`) and storing it (`store.rs`). |
| `meshview-rs/src/traceroute.rs` | Paths out of traceroutes: from the node that started one to the one it traced, and back. |
| `meshview-rs/src/decode.rs` | What a stored packet says, as the pages show it: decoded, in the protobuf text format, redacted or `Did not decode`. |
| `meshview-rs/src/web/` | HTTP: the API router, its reference, the redirects and the probes. `web/api/` has the API's endpoints, its errors, and its parameter parsing. |
| `meshview-rs/src/jobs.rs` | The daily cleanup and snapshot, and their schedule. |
| `meshview-rs/src/app.rs` | The roles, starting and stopping a process, and `backup`. |
| `meshview-rs/tests/` | Integration tests. |
| `scripts/` | Updating the protobufs, comparing databases and API answers with the Python version, and giving a database back to it. |

## Building and testing

```sh
git config core.hooksPath .githooks   # once per clone
cargo build
cargo test --workspace
```

The pre-commit hook refuses a commit that is not formatted (`cargo fmt --all`)
or that clippy warns about (`cargo clippy --workspace --all-targets
--all-features -- -D warnings`), the same two checks CI runs. Tests are left to
CI so that committing stays quick.

Nothing needs installing beyond Rust: SQLite is compiled in, and the protobufs
are compiled by [protox](https://crates.io/crates/protox), so there is no
`protoc` to have.

What the integration tests cover:

| Test | |
|---|---|
| `schema.rs` | Taking a Python database over, at its last revision and refusing others; a new database and an adopted one end up with the same schema. |
| `ingest.rs` | Envelopes encrypted and serialised by the Python protobufs and `cryptography` are stored exactly as the Python version stores them, unknown fields included. |
| `http.rs` | Routes per role, probes, the reference, and the redirects. |
| `api.rs` | The API endpoints, against a small database: what they answer, their errors, and compression. |
| `shutdown.rs` | Closing the database leaves no `-wal` or `-shm` behind. |
| `lifecycle.rs` | Whole processes, `run` and `ingest`, started against an unreachable broker and stopped. |
| `jobs.rs` | The daily cleanup and snapshot, the record of their runs, and `backup` copying a database that is being written. |
| `decode.rs` | Real packets of every port, shown exactly as the Python version showed them. |
| `revert.rs` | `scripts/revert-to-python.py` gives a taken over database back exactly as Python left it, and refuses what it cannot. Needs `python3`; skipped without it. |

The fixtures were made with the Python meshview; each test says how.

## Database migrations

Migrations live in `meshview-rs/migrations/sqlite/`, numbered, and are applied
by the writer on start. A migration that has been released is never edited: a
change is a new migration. They should only add to the schema, so that web
servers of the previous version keep working while an update rolls out (see
[Deployment](deployment.md#kubernetes)).

A new migration also ends the way back to the Python version, unless
`scripts/revert-to-python.py` is taught to undo it: the script keeps the list of
migrations it can undo in `REVERTIBLE`, and a test fails as soon as the
migrations and that list disagree. Either extend the script, and its test in
`tests/revert.rs`, or retire it and say from which version on in
[Coming from the Python meshview](migrating-from-python.md#going-back) and in the
release notes.

`0001_baseline.sql` is the Python meshview's schema at Alembic revision
`44612e74a798`, copied from what `alembic upgrade head` produces rather than from
its models, which disagree about some indexes. Every statement in it is
`IF NOT EXISTS`, which is what lets it adopt a Python database by changing
nothing.

## The protobufs

The `.proto` files under `meshview-proto/protobufs/` are vendored, at the
upstream revision in `UPSTREAM_REV`, rather than a submodule, so that a plain
clone, the CI checkout and the container build all have them. To move to
another revision of [meshtastic/protobufs](https://github.com/meshtastic/protobufs):

```sh
scripts/update-protobufs.sh <commit, tag or branch>
cargo test --workspace
```

Only the official protobufs are vendored; fields added by modified firmware are
not declared (see [Ingestion](ingestion.md#fields-these-protobufs-do-not-know)).

## Comparing with the Python version

Ingestion is meant to store what the Python meshview stores. The way to check
it is to run both against the same broker at the same time, each into its own
database, and compare.

1. Get the Python meshview 4.0 running from a checkout, with its virtual
   environment, and a `config.ini` whose database is `python.db`, cleanup
   disabled.
2. Write the same broker, credentials and topics into a `config.toml` for
   meshview-rs, with `rust.db` as its database and a port that does not clash.
3. Start both at once, and leave them for a while: half an hour on a quiet mesh
   is enough to see every kind of packet.

   ```sh
   python startdb.py --config config.ini &
   meshview-rs --config config.toml ingest &
   ```

4. Stop both, and compare:

   ```sh
   scripts/compare-ingest.py python.db rust.db
   ```

```text
OK   packet (real ids): python=129 rust=129 only_python=0 only_rust=0
OK   packet (map reports): python=2 rust=2 only_python=0 only_rust=0
OK   packet_seen: python=155 rust=155 only_python=0 only_rust=0
OK   packet_seen (map reports): python=2 rust=2 only_python=0 only_rust=0
OK   node: python=76 rust=76 only_python=0 only_rust=0
OK   traceroute: python=10 rust=10 only_python=0 only_rust=0
ALL MATCH
```

Times of import differ by construction and are not compared. A packet that
arrives while only one of the two is connected, right at the start or the end,
shows up as a difference, and is not one.

## Comparing the API

The API is meant to answer what the Python meshview answers. The way to check
it is to run both web servers on copies of the same database, with nothing
ingesting into either, and send both the same requests.

1. Make two copies of a Python meshview database: `python.db` stays the
   Python version's, `rust.db` is taken over with `meshview-rs migrate`.
2. Start the Python web server on `python.db` (`python main.py --config
   config.ini`), and `meshview-rs serve` on `rust.db` on another port, both with
   the same `[site]` settings: the Python `sample.config.ini` and this project's
   `sample.config.toml` are the same configuration.
3. Compare:

   ```sh
   scripts/compare-api.py http://localhost:8081 http://localhost:8082 scripts/api-requests.txt
   ```

`scripts/api-requests.txt` has the requests, ordinary ones and the edge cases:
every endpoint ported adds its own. Each is sent to the Python server at its
unversioned address and to meshview-rs under `/api/v1`, and the status, content
type and JSON values compared; keys that differ by nature, such as the version,
are listed after the request as `ignore=`. A request about a particular packet
or node needs one that is in the database: the file has those of the database
it was last run against, and they may need changing for another.

The differences that are meant, listed in [API](api.md#differences-from-the-python-version),
are of values the comparison does not see (the JSON's spacing) or of answers
the requests do not ask for.

## CI

Every push to `master` and every pull request runs, in `.github/workflows/ci.yml`:

| Job | |
|---|---|
| Format, lint and test | `cargo fmt --check`, clippy with warnings as errors, `cargo test --workspace --locked`. |
| Static binary | For `amd64` and `arm64`, musl, checked to have no dynamic loader. |
| Container image | Built from those binaries, started, asked `/readyz`, `/api/openapi.json` and a redirect, stopped, and checked to have left nothing but `packets.db` in `/data`. Published to GHCR on a push. |
| Release page | Only for a tag: see below. |

The x86_64 binary is built for the base x86-64 instruction set
(`.cargo/config.toml`), C included, so that it runs on CPUs without AVX.

## The container image

`Containerfile` has two ways to get the binary. By default it compiles it,
which is what `podman build -t meshview-rs .` does. CI already has the binaries,
so it passes `--build-arg BINARY_SOURCE=prebuilt` and puts them under `dist/` as
`meshview-rs-amd64` and `meshview-rs-arm64`, and the build only copies them in.

## Releases

A release is a tag named exactly the version in `Cargo.toml`, with no `v`
(`0.1.0`), and annotated: its message becomes the release notes.

```sh
git tag -a 0.1.0 -m "What changed"
git push origin 0.1.0
```

CI refuses a tag that does not match `Cargo.toml`, and then publishes the image
tagged with the version and as `latest`, and a release page with the binaries as
tarballs and their checksums.
