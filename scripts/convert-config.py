#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

"""Convert a Python meshview config.ini to a Meshview config.toml.

The INI file is read as the Python version reads it, with Python's own
configparser, and every key it has is written as Meshview expects it: with
its type (quoted strings, lowercase booleans, numbers without leading zeros,
lists), in its section. What Meshview has no use for is left out, with a
note on standard error saying why:

- the deprecated keys: [server] tls_cert, [cleanup] backup_*, [logging]
  db_cleanup_logfile, which Meshview would accept with a warning and ignore;
- keys the Python version no longer used either, such as [site] starting;
- keys neither knows, with the known one they look like, if any: Python ignored
  them, so leaving them out keeps the configuration doing what it did;
- empty values, which leave the default in place, as they did;
- [map_channel_colors] entries that are just the Python sample's colours:
  Meshview has colours of its own for the presets.

[site] conversations, the old name of chat, is written as chat, and a
connection string's driver, `+aiosqlite` in `sqlite+aiosqlite:///packets.db`,
which means nothing to Meshview, is taken out.

    scripts/convert-config.py config.ini > config.toml
    scripts/convert-config.py config.ini -o config.toml

Check the result before using it: read the notes, and start Meshview with
it, which refuses anything it does not take.
"""

import argparse
import configparser
import difflib
import json
import math
import sys
from datetime import date
from pathlib import Path

STRING, BOOLEAN, INTEGER, NUMBER, STRINGS, INTEGERS, JSON_STRINGS = (
    "string",
    "boolean",
    "whole number",
    "number",
    "list of strings",
    "list of whole numbers",
    "list of strings, as JSON",
)

# Meshview's keys, in its sections, as docs/configuration.md documents them.
SCHEMA = {
    "server": {"bind": STRING, "port": INTEGER},
    "site": {
        "domain": STRING,
        "language": STRING,
        "title": STRING,
        "message": STRING,
        "announcement": STRING,
        "nodes": BOOLEAN,
        "chat": BOOLEAN,
        "everything": BOOLEAN,
        "graphs": BOOLEAN,
        "stats": BOOLEAN,
        "net": BOOLEAN,
        "map": BOOLEAN,
        "top": BOOLEAN,
        "api": BOOLEAN,
        "meshviewworld": BOOLEAN,
        "map_top_left_lat": NUMBER,
        "map_top_left_lon": NUMBER,
        "map_bottom_right_lat": NUMBER,
        "map_bottom_right_lon": NUMBER,
        "map_tile_url": STRING,
        "map_tile_attribution": STRING,
        "map_tile_max_zoom": INTEGER,
        "map_tile_name": STRING,
        "map_interval": INTEGER,
        "firehose_interval": INTEGER,
        "weekly_net_message": STRING,
        "net_tag": STRING,
    },
    # Its keys are channel names: any key is a colour.
    "map_channel_colors": {},
    "mqtt": {
        "server": STRING,
        "port": INTEGER,
        "topics": JSON_STRINGS,
        "username": STRING,
        "password": STRING,
        "skip_node_ids": INTEGERS,
        "secondary_keys": STRINGS,
        "tls": BOOLEAN,
        "tls_ca_file": STRING,
        "tls_client_cert": STRING,
        "tls_client_key": STRING,
    },
    "database": {"connection_string": STRING},
    "cleanup": {
        "enabled": BOOLEAN,
        "days_to_keep": INTEGER,
        "hour": INTEGER,
        "minute": INTEGER,
        "vacuum": BOOLEAN,
    },
    "snapshot": {"hour": INTEGER, "minute": INTEGER},
    "logging": {"access_log": BOOLEAN},
}

RENAMED = {("site", "conversations"): "chat"}

LEFT_OUT = {
    ("server", "tls_cert"): "deprecated: serve HTTPS from a reverse proxy in front",
    ("server", "acme_challenge"): "not used: certificates are a reverse proxy's to get",
    (
        "site",
        "starting",
    ): "not used since the Python version dropped it: / is the dashboard",
    (
        "cleanup",
        "backup_enabled",
    ): "deprecated: run `meshview backup <file>` from a scheduler",
    ("cleanup", "backup_dir"): "deprecated, as backup_enabled",
    ("cleanup", "backup_hour"): "deprecated, as backup_enabled",
    ("cleanup", "backup_minute"): "deprecated, as backup_enabled",
    (
        "logging",
        "db_cleanup_logfile",
    ): "deprecated: the cleanup logs to standard output",
}

# The Python sample's channel colours, which Meshview replaces with its own.
PYTHON_SAMPLE_COLOURS = {
    "longfast": "#4363d8",
    "longslow": "#000075",
    "verylongslow": "#808000",
    "mediumslow": "#008080",
    "mediumfast": "#e6194b",
    "shortslow": "#911eb4",
    "shortfast": "#f58231",
    "longmoderate": "#46f0f0",
    "shortturbo": "#f032e6",
}


class Conversion:
    def __init__(self):
        self.notes = []
        self.errors = []

    def note(self, section, key, why):
        self.notes.append(f"[{section}] {key}: {why}")

    def error(self, section, key, why):
        self.errors.append(f"[{section}] {key}: {why}")


def toml_string(value):
    # A TOML basic string takes the same escapes as JSON.
    return json.dumps(value, ensure_ascii=False)


def boolean(raw):
    value = configparser.ConfigParser.BOOLEAN_STATES.get(raw.strip().lower())
    if value is None:
        raise ValueError("not a boolean")
    return "true" if value else "false"


def integer(raw):
    return str(int(raw.strip(), 10))


def number(raw):
    value = float(raw.strip())
    if not math.isfinite(value):
        raise ValueError("not a finite number")
    return repr(value)


def split_list(raw):
    """Comma separated, as the Python version split them, quotes taken off."""
    items = []
    for item in raw.split(","):
        item = item.strip()
        if len(item) >= 2 and item[0] == item[-1] and item[0] in "'\"":
            item = item[1:-1]
        if item:
            items.append(item)
    return items


def convert_value(kind, raw):
    if kind == STRING:
        return toml_string(raw)
    if kind == BOOLEAN:
        return boolean(raw)
    if kind == INTEGER:
        return integer(raw)
    if kind == NUMBER:
        return number(raw)
    if kind == STRINGS:
        return "[" + ", ".join(toml_string(item) for item in split_list(raw)) + "]"
    if kind == INTEGERS:
        return "[" + ", ".join(str(int(item, 0)) for item in split_list(raw)) + "]"
    if kind == JSON_STRINGS:
        items = json.loads(raw)
        if not isinstance(items, list) or not all(
            isinstance(item, str) for item in items
        ):
            raise ValueError("not a JSON list of strings")
        return "[" + ", ".join(toml_string(item) for item in items) + "]"
    raise AssertionError(kind)


def convert(parser, source_name):
    conversion = Conversion()
    sections = {}

    for section in parser.sections():
        if section not in SCHEMA:
            close = difflib.get_close_matches(section, SCHEMA, n=1, cutoff=0.8)
            hint = f" (did you mean [{close[0]}]?)" if close else ""
            conversion.note(
                section, "*", f"a section Meshview does not know{hint}: left out"
            )
            continue
        out = sections.setdefault(section, {})
        for key, raw in parser.items(section, raw=False):
            if parser.defaults() and key in parser.defaults():
                continue
            if section == "map_channel_colors":
                colour = raw.strip()
                if not colour:
                    conversion.note(section, key, "empty: left out")
                elif PYTHON_SAMPLE_COLOURS.get(key) == colour.lower():
                    conversion.note(
                        section,
                        key,
                        "the Python sample's colour: left out, Meshview has its own",
                    )
                else:
                    out[key] = toml_string(colour)
                continue

            if (section, key) in LEFT_OUT:
                conversion.note(section, key, LEFT_OUT[(section, key)] + ": left out")
                continue
            if (section, key) in RENAMED:
                new = RENAMED[(section, key)]
                conversion.note(section, key, f"written as {new}, its name now")
                key = new
            kind = SCHEMA[section].get(key)
            if kind is None:
                close = difflib.get_close_matches(key, SCHEMA[section], n=1, cutoff=0.8)
                hint = f" (did you mean {close[0]}?)" if close else ""
                conversion.note(
                    section,
                    key,
                    f"not a key Meshview knows{hint}: left out, as the Python version "
                    "ignored it",
                )
                continue
            if not raw.strip():
                conversion.note(
                    section, key, "empty: left out, so it takes its default"
                )
                continue
            if (section, key) == ("database", "connection_string"):
                scheme, sep, rest = raw.strip().partition("://")
                if sep and scheme.startswith("sqlite+"):
                    conversion.note(
                        section, key, f"written without its driver, {scheme[7:]}"
                    )
                    raw = f"sqlite://{rest}"
            try:
                out[key] = convert_value(kind, raw)
            except (ValueError, json.JSONDecodeError) as e:
                conversion.error(section, key, f"{raw!r} is not a {kind} ({e})")

    lines = [
        f"# Meshview configuration, converted from {source_name}",
        f"# by scripts/convert-config.py on {date.today().isoformat()}.",
        "# See docs/configuration.md for every key.",
    ]
    for section, known in SCHEMA.items():
        values = sections.get(section)
        if not values:
            continue
        lines += ["", f"[{section}]"]
        keys = list(known) if known else sorted(values)
        for key in keys:
            if key in values:
                name = (
                    key
                    if key.replace("_", "").isalnum() and key.isascii()
                    else toml_string(key)
                )
                lines.append(f"{name} = {values[key]}")
    return "\n".join(lines) + "\n", conversion


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("config", type=Path, help="the Python Meshview's config.ini")
    parser.add_argument(
        "-o", "--output", type=Path, help="where to write it; standard output if not"
    )
    parser.add_argument(
        "-f", "--force", action="store_true", help="overwrite the output file"
    )
    args = parser.parse_args()

    ini = configparser.ConfigParser()
    try:
        with args.config.open(encoding="utf-8") as file:
            ini.read_file(file)
    except OSError as e:
        sys.exit(f"Cannot read {args.config}: {e.strerror}")
    except configparser.Error as e:
        sys.exit(f"{args.config} is not a valid INI file: {e}")

    try:
        toml, conversion = convert(ini, args.config.name)
    except configparser.InterpolationError as e:
        sys.exit(f"{args.config}: {e}")

    for note in conversion.notes:
        print(f"note: {note}", file=sys.stderr)
    for error in conversion.errors:
        print(f"error: {error}", file=sys.stderr)
    if conversion.errors:
        sys.exit(
            f"{len(conversion.errors)} value(s) could not be converted: fix them and run again"
        )

    if args.output is None:
        sys.stdout.write(toml)
        return
    if args.output.exists() and not args.force:
        sys.exit(f"{args.output} exists: pass --force to overwrite it")
    args.output.write_text(toml, encoding="utf-8")
    print(f"Written {args.output}", file=sys.stderr)


if __name__ == "__main__":
    main()
