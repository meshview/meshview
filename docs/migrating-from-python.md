# Coming from the Python Meshview

Meshview takes over from the Python Meshview 4.0: its database, as it is,
and its configuration, rewritten in TOML with the same sections and keys.

## The database

Meshview takes over a database only at the Python version's last schema,
**Alembic revision `44612e74a798`**, which is what meshview 4.0 leaves. It does
not carry the Python version's migrations, so a database at an earlier revision
has to be brought up to date by the Python Meshview 4.0 first: start it once
against the database and let it migrate. Until then Meshview refuses the
database, names the revision it found, and changes nothing.

To see which revision a database is at:

```sh
sqlite3 packets.db 'SELECT version_num FROM alembic_version'
```

### Taking it over

1. Stop the Python Meshview, both `startdb.py` and the web server (or
   `mvrun.py`). Nothing else must have the database open.
2. Back the database up, with the side files if there are any:
   `packets.db`, `packets.db-wal`, `packets.db-shm`.
3. Convert the configuration, as [below](#converting-configini).
4. Run `meshview migrate`, or start Meshview, which does the same first.

```text
INFO meshview::db::sqlite: database is Python meshview at Alembic 44612e74a798, migrating
INFO meshview::db::sqlite: database migrated to meshview at migration 2
```

The data is not touched: the tables, their columns and their rows stay as they
are. What changes:

- The Python version's bookkeeping goes: `alembic_version`, `migration_status`,
  and `example`, a table an example migration added to every Python database and
  nothing used.
- An index the Python models declared but no migration ever created is added:
  `idx_traceroute_packet_id`, for looking traceroutes up by packet.
- A table of Meshview's own is added: `job_run`, the record of what the daily
  cleanup and snapshot did, which the Python version kept in a JSON file.
- Meshview records its own migrations in `_sqlx_migrations`.

### Going back

Once Meshview has taken a database over, the Python Meshview cannot open it
as it is: without `alembic_version` it would try to build its schema from
scratch over a database that already has one.

`scripts/revert-to-python.py` gives it back. It comes **with no warranty, and no
responsibility is taken for what it does to your data**: whatever it promises,
back the database up before running it.

```sh
scripts/revert-to-python.py packets.db        # only checks, changes nothing
scripts/revert-to-python.py --yes packets.db  # gives it back
```

It puts the Python version's bookkeeping back exactly as Python creates it, and
removes what Meshview added, in a single transaction: it either all happens
or none of it does. The data is not touched, so what Meshview ingested in the
meantime is kept. It needs Python 3 and nothing else.

It only works **as long as Meshview has not changed the schema** beyond
taking the database over. It refuses, and changes nothing, when:

- Meshview has applied a migration it does not know how to undo, which means
  the schema has moved on from the Python version's: the way back is then a
  backup taken before that version of Meshview;
- the database is in use, or Meshview was not stopped cleanly (its `-wal`
  file has data in it): stop every process using it first;
- the database is already the Python version's, or not meshview's at all.

Every Meshview release says in its notes whether it adds a migration, and
from which version on the script refuses.

## Converting `config.ini`

Meshview reads `config.toml`, not `config.ini`, and never writes either. The
sections and keys are the same, so converting is a matter of syntax, which
`scripts/convert-config.py` does:

```sh
scripts/convert-config.py config.ini -o config.toml
```

It reads the INI file as the Python version reads it and writes every key with
its type. What Meshview has no use for it leaves out, saying why on standard
error: the deprecated keys below, keys the Python version had stopped using
(`starting`), keys neither knows, with the one they look like (a misspelt
`firehose_interal`, which the Python version ignored, is left out rather than
corrected, so the configuration keeps doing what it did), empty values, and the
Python sample's channel colours, which Meshview has its own of.
`conversations` is written as `chat`, and `sqlite+aiosqlite://` as `sqlite://`.
Read its notes, and check the result by starting Meshview with it: anything
it does not take, it refuses.

By hand, the differences are these:

| | `config.ini` | `config.toml` |
|---|---|---|
| Strings get quotes | `title = Bay Area Mesh` | `title = "Bay Area Mesh"` |
| Booleans are lowercase | `nodes = True` | `nodes = true` |
| Numbers are bare | `port = 1883` | `port = 1883` |
| Leading zeros go | `minute = 00` | `minute = 0` |
| Topics were already a list | `topics = ["msh/US/bayarea/#"]` | `topics = ["msh/US/bayarea/#"]` |
| Node ids become a list of numbers | `skip_node_ids = 0xdeadbeef, 305419896` | `skip_node_ids = [0xdeadbeef, 305419896]` |
| Keys become a list of strings | `secondary_keys = AQ==, Ag==` | `secondary_keys = ["AQ==", "Ag=="]` |
| Colours are strings | `MyChannel = #18aa18` | `MyChannel = "#18aa18"` |
| Empty values can go | `announcement =` | `announcement = ""`, or nothing |
| Comments | `# ...` or `; ...` | `# ...` only |

The Python version's backup settings (`[cleanup] backup_enabled`, `backup_dir`,
`backup_hour`, `backup_minute`), `[logging] db_cleanup_logfile` and
`[server] tls_cert` are deprecated: they are accepted with a warning and
ignored, and will be removed in the next version. Leave them out. Backups are
made with `meshview backup` from a scheduler (see
[Deployment](deployment.md#backups)), the cleanup logs where everything else
does, and HTTPS is served by a reverse proxy in front (see
[Deployment](deployment.md#https)).

Two things that the INI file let pass are errors now:

- **Unknown keys.** The Python version warned about unknown keys in `[site]` and
  ignored everything else; Meshview refuses any key it does not know, in any
  section. A setting that was being silently ignored, such as a misspelt
  `firehose_interal`, has to be fixed or removed.
- **Values of the wrong type**, such as `port = "8081"` in quotes.

A value that ran over several lines in the INI file, with the continuation
lines indented, becomes a TOML multi-line string (`"""..."""`).

[sample.config.toml](../sample.config.toml) is the Python sample converted,
with every key, and [Configuration](configuration.md) has them all.

## What else is different

| | Python version | Meshview |
|---|---|---|
| Processes | `startdb.py` writes, `main.py` serves, `mvrun.py` starts both | one binary: `run`, or `ingest` and `serve` apart. See [Deployment](deployment.md). |
| Configuration | `config.ini` | `config.toml`, plus `MESHVIEW_<SECTION>__<KEY>` environment variables |
| API addresses | `/api/...` | `/api/v1/...`; the old ones redirect. See [API](api.md). |
| API reference | `static/api.html` | `/api/docs`, generated from the code; `/api.html` and `/api-chat.html` redirect to it |
| Web pages | rendered on every request | the same pages, rendered once at start, and their files built into the binary |
| Traceroute graph | `/graph/traceroute/{id}`, an SVG drawn by Graphviz | redirects to the traceroute's page, `/traceroute/{id}`, which draws it |
| Scripts and styles | sent as they are | compressed for browsers that take it |
| Channel colours on the map | the presets' in `config.ini`'s `[map_channel_colors]`, without the 2.8 ones and with `LongModerate` for the channel the firmware calls `LongMod` | built in for every preset, `[map_channel_colors]` only for what you change: the Python sample's can be left out |
| Container image | the database in `/var/lib/meshview`, UID 10001 | the database in `/data`, the working directory, as the same UID: mount the Python image's volume at `/data`, or keep it where it is and name it in `connection_string` |
| Coverage help | `docs/COVERAGE*.md`, fetched and rendered when it is opened, also served at `/docs/…` | part of the node page, translated with it |
| Probes | `/health` | `/livez` and `/readyz`, for orchestrators, beside `/health` |
| MQTT over TLS | no | yes |
| Coverage prediction | pyitm, installed apart | NTIA's model, built in (see [API](api.md#differences-from-the-python-version)) |
| Stopping | the database's side files can be left behind | stops in order and leaves only the database file |
| Backups | scheduled, copying the live file | `meshview backup`, a consistent copy, for your scheduler to run |
| Last cleanup | a JSON file beside the cleanup log | the `job_run` table |
| Cleanup log | a file of its own | standard output, with everything else |
| Stored packets | — | the same, byte for byte. See [Ingestion](ingestion.md). |

The port, 8081, and the database file, `packets.db`, are the same.

