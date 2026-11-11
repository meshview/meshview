# Development

## Layout

A Cargo workspace of two crates. The line between them is compile time: the
protobufs are a lot of generated code that changes only when the `.proto` files
do, and in a crate of their own they are built once and left alone.

| Path | What it is |
|---|---|
| `meshview-proto/` | The Meshtastic protobufs, vendored under `protobufs/` and compiled by prost. Also `wire`, for the protobuf wire format prost does not cover (keeping unknown fields), and `known_fields`, generated from the descriptors. |
| `meshview-rs/src/config.rs` | The configuration: TOML, environment variables, defaults. |
| `meshview-rs/src/db/` | The database. `mod.rs` is what holds for any backend; `sqlite.rs` everything that knows it is SQLite, so that PostgreSQL can be a sibling module. |
| `meshview-rs/migrations/sqlite/` | The SQLite migrations, compiled into the binary. |
| `meshview-rs/src/ingest/` | MQTT ingestion: the connection (`mod.rs`), keys and decryption (`keys.rs`), opening a message (`opened.rs`) and storing it (`store.rs`). |
| `meshview-rs/src/web/` | HTTP: the API router, its reference, the redirects and the probes. |
| `meshview-rs/src/app.rs` | The roles, and starting and stopping a process. |
| `meshview-rs/tests/` | Integration tests. |
| `scripts/` | Updating the protobufs, comparing databases with the Python version, and giving a database back to it. |

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
| `shutdown.rs` | Closing the database leaves no `-wal` or `-shm` behind. |
| `lifecycle.rs` | Whole processes, `run` and `ingest`, started against an unreachable broker and stopped. |
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
