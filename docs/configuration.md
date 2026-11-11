# Configuration

Meshview is configured with a TOML file, environment variables, or both.
Every key has a default, so no file at all is a valid configuration as long as
the environment supplies what the role needs (an ingester needs a broker and
topics).

## Where it is read from

| How | File | If the file is missing |
|---|---|---|
| `--config <file>` | that file | error |
| `MESHVIEW_CONFIG=<file>` | that file | error |
| neither | `./config.toml`, or else `/etc/meshview/config.toml` | defaults and environment only |

Without a file named, the working directory comes first and the system's place
second: the first of the two that exists is read, and the log says which.

The file is only ever read, never written, so it can live somewhere Meshview
cannot write to, which is where it usually belongs: on a read-only mount, owned
by root, while the database directory is the one place the process writes. In
the container image that is `/etc/meshview`, apart from the data in `/data`.

When neither `--config` nor `MESHVIEW_CONFIG` is given, neither place has a
`config.toml` and the working directory has a `config.ini`, Meshview refuses to
start.
That is a Python meshview configuration that has not been converted yet, and
starting on the defaults instead would quietly create a new, empty database
somewhere else. Naming an `.ini` file explicitly is refused too. See
[Coming from the Python Meshview](migrating-from-python.md#converting-configini)
for converting it.

## The format

The sections and keys are the Python Meshview's, written as TOML: strings in
quotes, booleans as `true`/`false`, numbers bare, lists in brackets.
[sample.config.toml](../sample.config.toml) has every key.

Anything Meshview does not know is an error, whether it is a section or a
key, and so is a value of the wrong type:

```text
Error: in config.toml or the environment

Caused by:
    unknown field `pasword`, expected one of `server`, `port`, `topics`, `username`, `password`, ...
    in `mqtt`
```

## Environment variables

Any key can be set from the environment, and the environment wins over the
file. The name is built from the section and the key:

```text
MESHVIEW_<SECTION>__<KEY>
```

- `MESHVIEW_` is the prefix, with one underscore.
- Between the section and the key go **two** underscores. Sections and keys have
  underscores of their own (`map_channel_colors`, `connection_string`), and two
  is what tells where one ends and the other begins.
- Case does not matter: everything is lowercased.
- A variable with the prefix but no double underscore, such as
  `MESHVIEW_CONFIG`, is not a configuration key and is left alone.
- One with a section or key that does not exist is an error, as it would be in
  the file.
- At start, the names of the variables that set keys are logged, without their
  values, and the database is logged with where its connection string came
  from: the file, a variable, or the default. A variable left set somewhere
  wins over the file without anything else saying so.

```sh
MESHVIEW_DATABASE__CONNECTION_STRING=sqlite:////data/packets.db
MESHVIEW_MQTT__SERVER=mqtt.example.org
MESHVIEW_MQTT__TOPICS='["msh/EU_868/#", "msh/EU_N_868/#"]'
MESHVIEW_MQTT__TLS=true
MESHVIEW_MAP_CHANNEL_COLORS__LONGFAST="#4363d8"
```

A value from the environment is read as the type of the key it sets, so that a
password made of digits is still a string:

| Key type | Accepted |
|---|---|
| string | anything, as it is |
| boolean | `true`, `false`, `yes`, `no`, `on`, `off`, `1`, `0`, in any case |
| whole number | a TOML integer: `8883`, `0xdeadbeef` |
| number | a TOML number: `43`, `-8.5` |
| list | a TOML list: `["a", "b"]`, `[0xdeadbeef, 305419896]` |

## Database connection strings

`[database] connection_string` keeps the SQLAlchemy form of the Python version,
so existing values carry over unchanged:

| Value | Database |
|---|---|
| `sqlite:///packets.db` | `packets.db`, relative to the working directory |
| `sqlite:////var/lib/meshview/packets.db` | `/var/lib/meshview/packets.db` |
| `sqlite+aiosqlite:///packets.db` | the same as the first: the driver is ignored |

Three slashes and then the path: a relative path leaves three, an absolute one,
starting with its own slash, makes four. A query string (`?mode=ro`) is ignored.
`sqlite://packets.db`, with two, is refused rather than guessed at. PostgreSQL
strings are recognised and refused: it is not supported yet.

## Reference

### `[server]`

Where the web server, and the probes, listen. Every role listens.

| Key | Type | Default | Description |
|---|---|---|---|
| `bind` | string | `"*"` | Address to listen on. `*` means every interface. |
| `port` | whole number | `8081` | Port to listen on. |
| `tls_cert` | string | | **Deprecated**, ignored with a warning, and removed in the next version: serve HTTPS from a [reverse proxy](deployment.md#https) in front. |

### `[site]`

What the web pages show and how, also given to them, and to anyone, by
`/api/v1/config`. The pages are rendered when the server starts: a change shows
once it is restarted.

| Key | Type | Default | Description |
|---|---|---|---|
| `domain` | string | `""` | Domain metadata exposed by `/api/config`; does not configure binding or TLS. |
| `language` | string | `"en"` | Language of the pages and of the API reference: `en`, `es` or `ru`. |
| `title` | string | `""` | Site title, in the browser title bar and headers. |
| `message` | string | `""` | Tagline, shown in the footer and used as the page description. |
| `announcement` | string | `""` | Banner at the top of every page. Visitors can dismiss it; it shows again when the text changes. |
| `nodes` | boolean | `true` | Show the Nodes link. Pages and API routes stay available either way. |
| `chat` | boolean | `true` | Show the Chat link. |
| `everything` | boolean | `true` | Show the Everything link. |
| `graphs` | boolean | `true` | Show the Graphs link. |
| `stats` | boolean | `true` | Show the Stats link. |
| `net` | boolean | `true` | Show the Net link. |
| `map` | boolean | `true` | Show the Map link. |
| `top` | boolean | `true` | Show the Top link. |
| `api` | boolean | `true` | Show the API documentation link. |
| `meshviewworld` | boolean | `true` | Show the MeshviewWorld link. |
| `map_top_left_lat` | number | `39.0` | Map bounds: latitude of the top left corner. |
| `map_top_left_lon` | number | `-123.0` | Map bounds: longitude of the top left corner. |
| `map_bottom_right_lat` | number | `36.0` | Map bounds: latitude of the bottom right corner. |
| `map_bottom_right_lon` | number | `-121.0` | Map bounds: longitude of the bottom right corner. |
| `map_tile_url` | string | `""` | Tile server URL template. Empty means OpenStreetMap. |
| `map_tile_attribution` | string | `""` | Attribution for a custom tile server. |
| `map_tile_max_zoom` | whole number | `19` | Highest zoom level the tiles have. |
| `map_tile_name` | string | `""` | Name of the tile layer in the layer picker. |
| `map_interval` | whole number | `3` | Seconds between map refreshes. Zero or less disables polling. |
| `firehose_interval` | whole number | `3` | Seconds between firehose refreshes. Zero or less disables polling. |
| `weekly_net_message` | string | `"Weekly Mesh check-in message."` | Text of the weekly net page. |
| `net_tag` | string | `"#BayMeshNet"` | Tag the weekly net page looks for in messages. |

### `[map_channel_colors]`

A table of channel names to six digit hex colours, such as
`MyChannel = "#18aa18"`. Names are matched ignoring case, spaces, hyphens and
underscores; channels without one get a generated colour. Its keys are the
channel names, so there is no fixed list of them.

The channels of Meshtastic's presets, named as the firmware names them, have
colours built in, which a colour here replaces. They go by family, a hue each,
with the slow presets darker and the turbo ones lighter, and LongFast in the
site's own blue. `Custom` is the channel of a radio set up without a preset.

| Channel | Colour |
|---|---|
| LongSlow | `#133a6c` |
| LongFast | `#245ea8` |
| LongMod | `#617dd1` |
| LongTurbo | `#1f96d6` |
| MediumSlow | `#0b5a5f` |
| MediumFast | `#12858b` |
| MediumTurbo | `#1aa39c` |
| ShortSlow | `#9c4506` |
| ShortFast | `#e07012` |
| ShortTurbo | `#f29a3f` |
| NarrowSlow | `#4b2a8a` |
| NarrowFast | `#7a4cc8` |
| LiteSlow | `#8a1d51` |
| LiteFast | `#cf3d7e` |
| TinySlow | `#765800` |
| TinyFast | `#b38600` |
| Custom | `#6e6e6e` |

On the map every dot gets the border that stands out more from its colour,
white or near-black, so that light and dark colours both show on the day map
and on the night one.

### `[mqtt]`

The broker the ingester reads the mesh from. Needed by `run` and `ingest`.

| Key | Type | Default | Description |
|---|---|---|---|
| `server` | string | `""` | Broker host name or address. **Required** to ingest. |
| `port` | whole number | `1883`, or `8883` with `tls` | Broker port. |
| `topics` | list of strings | `[]` | Topics to subscribe to; `/#` includes everything below. **Required** to ingest. |
| `username` | string | `""` | User name, if the broker asks for one. |
| `password` | string | `""` | Password for that user. |
| `skip_node_ids` | list of whole numbers | `[]` | Nodes whose packets are not stored, in decimal or hex: `[0xdeadbeef, 305419896]`. |
| `secondary_keys` | list of strings | `[]` | Base64 AES keys, 16 or 32 bytes, tried after the default one. Meshtastic's one byte PSK shorthand is understood: `"Ag=="` is PSK 2. |
| `tls` | boolean | `false` | Connect over TLS. New in Meshview. |
| `tls_ca_file` | string | `""` | PEM file of CA certificates to trust instead of the public roots built into Meshview, for a broker with its own CA. |
| `tls_client_cert` | string | `""` | PEM client certificate, for a broker that asks for one. Needs `tls_client_key`. |
| `tls_client_key` | string | `""` | PEM private key of that certificate. Needs `tls_client_cert`. |

### `[database]`

| Key | Type | Default | Description |
|---|---|---|---|
| `connection_string` | string | `"sqlite:///packets.db"` | The database, in SQLAlchemy's form. See [connection strings](#database-connection-strings). |

### `[cleanup]`

The daily cleanup, run by the writer (`run` or `ingest`). See
[Ingestion](ingestion.md#daily-jobs).

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | boolean | `false` | Delete old data daily. |
| `days_to_keep` | whole number | `14` | How many days of data to keep. |
| `hour` | whole number | `2` | Hour of the daily cleanup, [local time](#local-time) (0 to 23). |
| `minute` | whole number | `0` | Minute of the daily cleanup (0 to 59). |
| `vacuum` | boolean | `false` | Run SQLite's `VACUUM` after cleaning up, giving the space freed back to the filesystem. |
| `backup_enabled` | boolean | | **Deprecated**, ignored with a warning, and removed in the next version: Meshview does not back up by itself. Use [`meshview backup`](deployment.md#backups). |
| `backup_dir` | string | | **Deprecated**, as `backup_enabled`. |
| `backup_hour` | whole number | | **Deprecated**, as `backup_enabled`. |
| `backup_minute` | whole number | | **Deprecated**, as `backup_enabled`. |

### `[snapshot]`

The daily snapshot, run by the writer whether or not cleanup is enabled. See
[Ingestion](ingestion.md#daily-jobs).

| Key | Type | Default | Description |
|---|---|---|---|
| `hour` | whole number | `1` | Hour, [local time](#local-time), at which the previous complete UTC day's figures are recorded. |
| `minute` | whole number | `0` | Minute of it. |

### Local time

The daily jobs run at a time of day in the local time zone, as in the Python
version. That is the `TZ` environment variable (`TZ=Europe/Madrid`), or the
system's time zone, or UTC when there is neither, which is the case in the
container image unless `TZ` is set. The time zone database is built into
Meshview, so `TZ` works even where the system has none installed. Daylight
saving time is followed: a job set inside the hour a clock skips runs at the
first time after it, and one inside the hour a clock repeats runs once.

### `[logging]`

Everything is logged to standard output, for the process's supervisor (systemd,
the container runtime) to collect; a log file is that supervisor's to keep. How
much is logged is controlled with `RUST_LOG`, as in `RUST_LOG=debug` or
`RUST_LOG=meshview::ingest=debug`; the default is `info`.

| Key | Type | Default | Description |
|---|---|---|---|
| `access_log` | boolean | `false` | Log every HTTP request, with its answer's status and how long it took, under `meshview::access`. |
| `db_cleanup_logfile` | string | | **Deprecated**, ignored with a warning, and removed in the next version: the cleanup logs where everything else does. |
