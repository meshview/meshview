#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>
"""Give a database meshview-rs has taken over back to the Python meshview 4.0.

    scripts/revert-to-python.py [--yes] packets.db

This is offered as is, with no warranty and no responsibility taken for what it
does to your data: back the database up before running it. Without --yes it
only says what it would do.

It works only while meshview-rs has not changed the schema beyond taking the
database over, which is migrations 1 and 2. From the first migration that does,
it refuses: there would be no telling what the Python version would make of the
result. It also refuses a database that is in use, or that was not stopped
cleanly, or that is already the Python version's.

What it does, in one transaction, so that it either all happens or none of it:

- puts back the Python version's bookkeeping (alembic_version at 44612e74a798,
  migration_status, and the unused example table), exactly as Python creates it;
- removes what meshview-rs added: the idx_traceroute_packet_id index and its
  own record of migrations, _sqlx_migrations.

The data, every row of every table, is not touched. Stop meshview-rs, every
process using the database, before running it.

Only the standard library is used.
"""

import argparse
import os
import sqlite3
import sys

# The migrations this knows how to undo, by version and the description sqlx
# records for them. Adding a migration to meshview-rs makes a test fail until
# this is taught to undo it, or retired.
REVERTIBLE = [(1, "baseline"), (2, "retire python bookkeeping")]

PYTHON_REVISION = "44612e74a798"

# What the Python meshview 4.0 creates, statement for statement.
RESTORE = [
    """CREATE TABLE alembic_version (
	version_num VARCHAR(32) NOT NULL,
	CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num)
)""",
    f"INSERT INTO alembic_version VALUES('{PYTHON_REVISION}')",
    """CREATE TABLE example (
	id INTEGER NOT NULL,
	name VARCHAR(100) NOT NULL,
	description TEXT,
	value FLOAT,
	is_active BOOLEAN DEFAULT '1' NOT NULL,
	created_at DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL,
	updated_at DATETIME,
	PRIMARY KEY (id)
)""",
    "CREATE INDEX idx_example_name ON example (name)",
    """CREATE TABLE migration_status (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                in_progress BOOLEAN NOT NULL DEFAULT FALSE,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )""",
    "INSERT INTO migration_status (id, in_progress) VALUES (1, 0)",
    "DROP INDEX IF EXISTS idx_traceroute_packet_id",
    "DROP TABLE _sqlx_migrations",
]

WARNING = """\
This gives the database back to the Python meshview 4.0. It comes with no
warranty and no responsibility is taken for what it does to your data: back the
database up first, along with any -wal and -shm files beside it.
"""


def refuse(message):
    sys.exit(f"refusing: {message}")


def tables(connection):
    return {
        name
        for (name,) in connection.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
        )
    }


def check(connection):
    """Refuses, with the reason, anything this should not touch."""
    present = tables(connection)
    if "alembic_version" in present:
        refuse("this database already belongs to the Python meshview (alembic_version is there)")
    if "_sqlx_migrations" not in present:
        refuse("this is not a database meshview-rs has taken over (no _sqlx_migrations)")
    missing = {"packet", "packet_seen", "node", "traceroute", "daily_snapshot"} - present
    if missing:
        refuse(f"tables are missing: {', '.join(sorted(missing))}")

    applied = connection.execute(
        "SELECT version, description, success FROM _sqlx_migrations ORDER BY version"
    ).fetchall()
    if any(not success for _, _, success in applied):
        refuse("a meshview-rs migration failed partway through; restore a backup instead")
    applied = [(version, description) for version, description, _ in applied]
    if applied != REVERTIBLE:
        refuse(
            f"meshview-rs has applied migrations {[v for v, _ in applied]}, and this only knows "
            f"how to undo {[v for v, _ in REVERTIBLE]}: the schema has changed since the Python "
            "version, and the way back is a backup taken before that"
        )


def main():
    parser = argparse.ArgumentParser(
        description="Give a database meshview-rs has taken over back to the Python meshview 4.0.",
        epilog="No warranty, no responsibility: back the database up first.",
    )
    parser.add_argument("database", help="the SQLite database file")
    parser.add_argument("--yes", action="store_true", help="do it, rather than only check")
    args = parser.parse_args()

    if not os.path.isfile(args.database):
        refuse(f"{args.database} is not a file")
    wal = f"{args.database}-wal"
    if os.path.exists(wal) and os.path.getsize(wal) > 0:
        refuse(
            f"{wal} has data in it: meshview-rs is still running, or was not stopped cleanly. "
            "Stop it, start and stop it again if need be, and try again"
        )

    print(WARNING)

    # Autocommit, so that the transaction below is exactly the one written here.
    connection = sqlite3.connect(args.database, isolation_level=None)
    try:
        try:
            # Takes the write lock at once: another process writing means it
            # is still in use.
            connection.execute("BEGIN IMMEDIATE")
        except sqlite3.OperationalError as error:
            refuse(f"the database is in use ({error}); stop every process using it")

        check(connection)

        if not args.yes:
            connection.execute("ROLLBACK")
            print("It can be given back. Nothing has been changed; run again with --yes to do it.")
            return

        for statement in RESTORE:
            connection.execute(statement)
        connection.execute("COMMIT")
    except BaseException:
        if connection.in_transaction:
            connection.execute("ROLLBACK")
        raise
    finally:
        connection.close()

    print(
        f"Done: {args.database} is at the Python meshview's Alembic revision {PYTHON_REVISION} "
        "again. Start the Python meshview 4.0 against it; meshview-rs would take it over again."
    )


if __name__ == "__main__":
    main()
