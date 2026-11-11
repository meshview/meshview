-- From here on the database belongs to meshview-rs, so what the Python version
-- kept for itself goes.
--
-- alembic_version is where Alembic remembered its revision; this program never
-- reads it again once a database has been adopted. migration_status was how the
-- Python writer told the Python reader to wait during a migration, and here the
-- applied migrations themselves answer that. example is a table an example
-- migration added to every Python database and nothing ever used.
--
-- Dropping alembic_version is also what stops the Python version from opening
-- this database afterwards and running its migrations over a schema it no longer
-- owns: going back means restoring a backup, not starting the old program.
DROP TABLE IF EXISTS alembic_version;
DROP TABLE IF EXISTS migration_status;
DROP TABLE IF EXISTS example;

-- The Python models declared this index, but no migration ever created it, so no
-- database has it. Looking a traceroute up by its packet is what the traceroute
-- page does.
CREATE INDEX IF NOT EXISTS idx_traceroute_packet_id ON traceroute (packet_id);
