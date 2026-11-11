# Coming from the Python meshview

meshview-rs takes over from the Python meshview 4.0: its database, as it is,
and its configuration, rewritten in TOML with the same sections and keys.

## The database

meshview-rs takes over a database only at the Python version's last schema,
**Alembic revision `44612e74a798`**, which is what meshview 4.0 leaves. It does
not carry the Python version's migrations, so a database at an earlier revision
has to be brought up to date by the Python meshview 4.0 first: start it once
against the database and let it migrate. Until then meshview-rs refuses the
database, names the revision it found, and changes nothing.

To see which revision a database is at:

```sh
sqlite3 packets.db 'SELECT version_num FROM alembic_version'
```

### Taking it over

1. Stop the Python meshview, both `startdb.py` and the web server (or
   `mvrun.py`). Nothing else must have the database open.
2. Back the database up, with the side files if there are any:
   `packets.db`, `packets.db-wal`, `packets.db-shm`.
3. Convert the configuration, as [below](#converting-configini).
4. Run `meshview-rs migrate`, or start meshview-rs, which does the same first.

```text
INFO meshview_rs::db::sqlite: database is Python meshview at Alembic 44612e74a798, migrating
INFO meshview_rs::db::sqlite: database migrated to meshview-rs at migration 2
```

The data is not touched: the tables, their columns and their rows stay as they
are. What changes:

- The Python version's bookkeeping goes: `alembic_version`, `migration_status`,
  and `example`, a table an example migration added to every Python database and
  nothing used.
- An index the Python models declared but no migration ever created is added:
  `idx_traceroute_packet_id`, for looking traceroutes up by packet.
- A table of meshview-rs's own is added: `job_run`, the record of what the daily
  cleanup and snapshot did, which the Python version kept in a JSON file.
- meshview-rs records its own migrations in `_sqlx_migrations`.

### Going back

Once meshview-rs has taken a database over, the Python meshview cannot open it
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
removes what meshview-rs added, in a single transaction: it either all happens
or none of it does. The data is not touched, so what meshview-rs ingested in the
meantime is kept. It needs Python 3 and nothing else.

It only works **as long as meshview-rs has not changed the schema** beyond
taking the database over. It refuses, and changes nothing, when:

- meshview-rs has applied a migration it does not know how to undo, which means
  the schema has moved on from the Python version's: the way back is then a
  backup taken before that version of meshview-rs;
- the database is in use, or meshview-rs was not stopped cleanly (its `-wal`
  file has data in it): stop every process using it first;
- the database is already the Python version's, or not meshview's at all.

Every meshview-rs release says in its notes whether it adds a migration, and
from which version on the script refuses.

## Converting `config.ini`

meshview-rs reads `config.toml`, not `config.ini`, and never writes either. The
sections and keys are the same, so converting is a matter of syntax:

| | `config.ini` | `config.toml` |
|---|---|---|
| Strings get quotes | `title = Bay Area Mesh` | `title = "Bay Area Mesh"` |
| Booleans are lowercase | `nodes = True` | `nodes = true` |
| Numbers are bare | `port = 1883` | `port = 1883` |
| Leading zeros go | `minute = 00` | `minute = 0` |
| Topics were already a list | `topics = ["msh/US/bayarea/#"]` | `topics = ["msh/US/bayarea/#"]` |
| Node ids become a list of numbers | `skip_node_ids = 0xdeadbeef, 305419896` | `skip_node_ids = [0xdeadbeef, 305419896]` |
| Keys become a list of strings | `secondary_keys = AQ==, Ag==` | `secondary_keys = ["AQ==", "Ag=="]` |
| Colours are strings | `LongFast = #4363d8` | `LongFast = "#4363d8"` |
| Empty values can go | `announcement =` | `announcement = ""`, or nothing |
| Comments | `# ...` or `; ...` | `# ...` only |

The Python version's backup settings (`[cleanup] backup_enabled`, `backup_dir`,
`backup_hour`, `backup_minute`) and `[logging] db_cleanup_logfile` are
deprecated: they are accepted with a warning and ignored, and will be removed in
the next version. Leave them out. Backups are made with `meshview-rs backup`
from a scheduler (see [Deployment](deployment.md#backups)), and the cleanup logs
where everything else does.

Two things that the INI file let pass are errors now:

- **Unknown keys.** The Python version warned about unknown keys in `[site]` and
  ignored everything else; meshview-rs refuses any key it does not know, in any
  section. A setting that was being silently ignored, such as a misspelt
  `firehose_interal`, has to be fixed or removed.
- **Values of the wrong type**, such as `port = "8081"` in quotes.

A value that ran over several lines in the INI file, with the continuation
lines indented, becomes a TOML multi-line string (`"""..."""`).

[sample.config.toml](../sample.config.toml) is the Python sample converted,
with every key, and [Configuration](configuration.md) has them all.

## What else is different

| | Python meshview | meshview-rs |
|---|---|---|
| Processes | `startdb.py` writes, `main.py` serves, `mvrun.py` starts both | one binary: `run`, or `ingest` and `serve` apart. See [Deployment](deployment.md). |
| Configuration | `config.ini` | `config.toml`, plus `MESHVIEW_<SECTION>__<KEY>` environment variables |
| API addresses | `/api/...` | `/api/v1/...`; the old ones redirect. See [API](api.md). |
| API reference | `static/api.html` | `/api/docs`, generated from the code |
| Probes | `/health` | `/livez` and `/readyz`, for orchestrators; `/health` comes back with the API |
| MQTT over TLS | no | yes |
| Stopping | the database's side files can be left behind | stops in order and leaves only the database file |
| Backups | scheduled, copying the live file | `meshview-rs backup`, a consistent copy, for your scheduler to run |
| Last cleanup | a JSON file beside the cleanup log | the `job_run` table |
| Cleanup log | a file of its own | standard output, with everything else |
| Stored packets | — | the same, byte for byte. See [Ingestion](ingestion.md). |

The port, 8081, and the database file, `packets.db`, are the same.

Not ported yet, so missing for now: the web pages and the API endpoints, and
coverage prediction. Their settings
are already accepted, so the configuration does not need to change when they
arrive.
