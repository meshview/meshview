# Development

## Layout

A Cargo workspace of two crates. The line between them is compile time: the
protobufs are a lot of generated code that changes only when the `.proto` files
do, and in a crate of their own they are built once and left alone.

| Path | What it is |
|---|---|
| `longley-rice/` | The ITS Irregular Terrain Model in area mode, from NTIA's C++, with its own licence and README: a crate meant to be published on its own. |
| `meshview-proto/` | The Meshtastic protobufs, vendored under `protobufs/` and compiled by prost, and their descriptors. Also `wire`, for what prost reads differently from Python's protobuf (unknown fields, fields of the wrong wire type), and `text_format`, the protobuf text format as Python writes it. |
| `meshview/src/config.rs` | The configuration: TOML, environment variables, defaults. |
| `meshview/src/db/` | The database. `mod.rs` is what holds for either, and the pool that is one or the other; `sqlite/` and `postgres/` everything that knows which, each with its queries; `write.rs` the SQL the ingester and the jobs write, once for both. |
| `meshview/migrations/sqlite/`, `meshview/migrations/postgres/` | The migrations of each database, compiled into the binary. |
| `meshview/templates/` | The web pages' templates, Tera 2. See [The web pages](#the-web-pages). |
| `meshview/assets/` | What the web side serves and is built into the binary: the pages' files under `static/`, the translation catalogs under `lang/`, and the API reference's introduction (`api/intro.md`) and translations (`api-lang/`). |
| `meshview/src/ingest/` | MQTT ingestion: the connection (`mod.rs`), keys and decryption (`keys.rs`), opening a message (`opened.rs`) and storing it (`store.rs`). |
| `meshview/src/traceroute.rs` | Paths out of traceroutes: from the node that started one to the one it traced, and back. |
| `meshview/src/coverage.rs` | Predicted coverage: where a node is heard, sampled around it with the ITM. |
| `meshview/src/decode.rs` | What a stored packet says, as the pages show it: decoded, in the protobuf text format, redacted or `Did not decode`. |
| `meshview/src/web/` | HTTP: the pages (`pages.rs`), the API router, its reference, the redirects and the probes. `web/api/` has the API's endpoints, its errors, and its parameter parsing. |
| `meshview/src/jobs.rs` | The daily cleanup and snapshot, and their schedule. |
| `meshview/src/app.rs` | The roles, starting and stopping a process, and `backup`. |
| `meshview/tests/` | Integration tests. |
| `scripts/` | Updating the protobufs, and for those coming from the Python version, converting its configuration and giving a database back to it. |

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

PostgreSQL is the `postgres` feature, on by default; `cargo build
--no-default-features` leaves it out, and the checks should pass that way too.
Its tests need a server, named in `MESHVIEW_TEST_POSTGRES`, where the role can
create databases: each test makes one of its own, sorting text by language
through ICU where the server has it, and drops it. Without the variable they
pass doing nothing.

```sh
MESHVIEW_TEST_POSTGRES=postgresql://postgres@127.0.0.1:5432/postgres cargo test --workspace
```

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
| `postgres_schema.rs` | As `schema.rs`, on PostgreSQL, against the schema the Python version leaves there. |
| `postgres_ingest.rs` | The same messages stored in SQLite and in PostgreSQL leave the same rows. |
| `postgres_jobs.rs` | The daily jobs do the same on either. |
| `postgres_api.rs` | Every endpoint answers the same on either, from the same rows, NULLs, names sorted by bytes and by language, and zero bytes in payloads included. |

The fixtures were made with the Python Meshview; each test says how.

## Database migrations

Migrations live in `meshview/migrations/sqlite/` and
`meshview/migrations/postgres/`, numbered, and are applied by the writer on
start. Each step is in both, under the same number, written for each, and a
test fails when the two lists disagree. A migration that has been released is never edited: a
change is a new migration. They should only add to the schema, so that web
servers of the previous version keep working while an update rolls out (see
[Deployment](deployment.md#kubernetes)).

A new migration also ends the way back to the Python version, unless
`scripts/revert-to-python.py` is taught to undo it: the script keeps the list of
migrations it can undo in `REVERTIBLE`, and a test fails as soon as the
migrations and that list disagree. Either extend the script, and its test in
`tests/revert.rs`, or retire it and say from which version on in
[Coming from the Python Meshview](migrating-from-python.md#going-back) and in the
release notes.

`0001_baseline.sql` is the Python Meshview's schema at Alembic revision
`44612e74a798`, copied from what `alembic upgrade head` produces rather than from
its models, which disagree about some indexes; for PostgreSQL, read back with
`pg_dump` from a database the Python version migrated there. Every statement in
it is `IF NOT EXISTS`, which is what lets it adopt a Python database by changing
nothing.

`scripts/revert-to-python.py` is for SQLite only: there is no way back from
PostgreSQL, and the migration guide says to try Meshview on a copy instead.

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

## The API reference

The reference is written where the API is: each endpoint's doc comment is its
title (the first line) and description, its `#[utoipa::path]` has the
parameters and answers, and the doc comments of the answers' fields describe
them. What holds for the whole API, the reference's front page, is
`meshview/assets/api/intro.md`. All of it is Markdown, for Scalar to render.

The reference is translated into the languages of `assets/api-lang/`, one JSON
catalog each from the English text to its translation, piece by piece: a
paragraph, a heading or an item of a list, its lines joined by spaces. A test
fails while a catalog lacks a piece, or has one there no longer is, so any
change to the reference's text has to be followed by its catalogs:

```sh
cargo test -p meshview --lib reference
```

prints, for each catalog, what is missing as JSON with empty translations, to
fill in and add, and what is stale, to remove. A piece is its whole paragraph,
so changing a word of it makes the paragraph missing and its old translation
stale: translate it anew rather than only renaming the key.

A new language is a new `assets/api-lang/<code>.json`, with every piece; the
reference is then served in it when it is `[site] language`. Scalar has its
own words in a few languages, `es` and `ru` among them; in one it does not
have, they are English.

## The web pages

The pages are Tera 2 templates under `meshview/templates/`, and the files
they use, scripts, styles, icons and the bundled libraries, under
`meshview/assets/static/`; all of it is built into the binary, and every page
is rendered once, at start. They began as the Python version's Jinja templates,
converted, and are developed here now. The node page is the one exception, and
only in its link preview: the apps that show one do not run scripts, so the
node's name and details go into its `og:` tags as the page is answered, in the
places a second rendering at start finds for them.

A few templates are not pages but parts the pages include: the node page's
coverage help (`coverage-help.html`), the map's dot borders
(`marker-stroke.js`) and the footer's corner for the project, with its link
to GitHub and the about dialog (`footer-project.html`).

Templates translate their strings with the `tr` filter, `{{ "Text" | tr }}`,
from the `ui` section of the catalogs under `meshview/assets/lang/`, keyed by
the English text; the scripts the pages run use the other sections, by key, through
`/api/v1/lang`. A test fails while a `tr` string is missing from a catalog's
`ui`, English included, and another renders every page in every language, so a
template Tera cannot render fails the build.

The bundled libraries live in folders named after their version
(`vendor/leaflet-1.9.4/`), which is what lets them be cached for good: a new
version is a new folder, never new contents in an old one.

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
which is what `podman build -t meshview .` does. CI already has the binaries,
so it passes `--build-arg BINARY_SOURCE=prebuilt` and puts them under `dist/` as
`meshview-amd64` and `meshview-arm64`, and the build only copies them in.

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

`longley-rice` is released on its own, to crates.io, with a version of its own
in `longley-rice/Cargo.toml`. Its tags are its name and that version, again with
no `v` (`longley-rice-0.1.0`), which the release above does not answer to. They
are plain tags, not annotated: the tag says all there is to say.

```sh
cargo publish -p longley-rice
git tag longley-rice-0.1.0
git push origin longley-rice-0.1.0
```
