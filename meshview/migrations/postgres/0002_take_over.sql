-- From here on the database belongs to Meshview, as in SQLite: what the Python
-- version kept for itself goes, and what Meshview keeps for itself arrives.
--
-- alembic_version is where Alembic remembered its revision, migration_status was
-- how the Python writer told the Python reader to wait during a migration, and
-- example is a table an example migration added to every Python database and
-- nothing ever used. Dropping alembic_version also stops the Python version from
-- running its migrations over a schema it no longer owns.
--
-- There is no way back to the Python version from here, unlike in SQLite: try
-- Meshview on a copy restored from a backup, beside the Python instance, first.
DROP TABLE IF EXISTS alembic_version;
DROP TABLE IF EXISTS migration_status;
DROP TABLE IF EXISTS example;

-- The Python models declared this index, but no migration ever created it.
-- Looking a traceroute up by its packet is what the traceroute page does.
CREATE INDEX IF NOT EXISTS idx_traceroute_packet_id ON traceroute (packet_id);

-- A node's packets and sightings in a recent window, the packets to a node and
-- those of a port, each read by its window alone; see the SQLite migration of
-- the same number. The indexes on the node alone go, as these begin with it.
CREATE INDEX IF NOT EXISTS idx_packet_seen_node_time_us ON packet_seen (node_id, import_time_us DESC);
CREATE INDEX IF NOT EXISTS idx_packet_to_node_time_us ON packet (to_node_id, import_time_us DESC);
CREATE INDEX IF NOT EXISTS idx_packet_portnum_time_us ON packet (portnum, import_time_us DESC);
DROP INDEX IF EXISTS idx_packet_seen_node_id;
DROP INDEX IF EXISTS idx_packet_to_node_id;
DROP INDEX IF EXISTS idx_packet_from_node_id;

-- What the scheduled jobs did, one row for each time one ran; see the SQLite
-- migration of the same number. `details` is JSON, kept as text as there.
CREATE TABLE IF NOT EXISTS job_run (
	id BIGSERIAL NOT NULL,
	job VARCHAR NOT NULL,
	status VARCHAR NOT NULL,
	started_us BIGINT NOT NULL,
	finished_us BIGINT,
	details TEXT,
	error TEXT,
	PRIMARY KEY (id)
);
CREATE INDEX IF NOT EXISTS idx_job_run_job_started_us ON job_run (job, started_us DESC);
