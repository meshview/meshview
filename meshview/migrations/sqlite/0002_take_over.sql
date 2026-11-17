-- From here on the database belongs to meshview-rs: what the Python version kept
-- for itself goes, and what meshview-rs keeps for itself arrives.
--
-- alembic_version is where Alembic remembered its revision; this program never
-- reads it again once a database has been adopted. migration_status was how the
-- Python writer told the Python reader to wait during a migration, and here the
-- applied migrations themselves answer that. example is a table an example
-- migration added to every Python database and nothing ever used.
--
-- Dropping alembic_version is also what stops the Python version from opening
-- this database afterwards and running its migrations over a schema it no longer
-- owns. scripts/revert-to-python.py undoes exactly this migration, and nothing
-- else, to give a database back.
DROP TABLE IF EXISTS alembic_version;
DROP TABLE IF EXISTS migration_status;
DROP TABLE IF EXISTS example;

-- The Python models declared this index, but no migration ever created it, so no
-- database has it. Looking a traceroute up by its packet is what the traceroute
-- page does.
CREATE INDEX IF NOT EXISTS idx_traceroute_packet_id ON traceroute (packet_id);

-- What a node page asks for is a node's packets or sightings in a recent
-- window, and the Python indexes on a node alone made that read the node's
-- whole history to keep the window: a busy gateway's sightings, for its direct
-- receptions, took minutes on a cold cache. With the time beside the node, only
-- the window is read. The same for the packets to a node, and for the packets
-- of a port, which the chat and the statistics by port ask for: without it they
-- walk every packet back in time looking for those of their port.
--
-- The indexes on the node alone go, as these begin with it and serve whatever
-- those did. The one on the sender alone was already the start of
-- idx_packet_from_node_time_us.
CREATE INDEX IF NOT EXISTS idx_packet_seen_node_time_us ON packet_seen (node_id, import_time_us);
CREATE INDEX IF NOT EXISTS idx_packet_to_node_time_us ON packet (to_node_id, import_time_us);
CREATE INDEX IF NOT EXISTS idx_packet_portnum_time_us ON packet (portnum, import_time_us);
DROP INDEX IF EXISTS idx_packet_seen_node_id;
DROP INDEX IF EXISTS idx_packet_to_node_id;
DROP INDEX IF EXISTS idx_packet_from_node_id;

-- A gateway's direct receptions, the packets it heard at no hops, are a small
-- part of its sightings, and this index keeps only those, with what is shown of
-- them: the node page's chart of them is answered from it alone, without
-- reading each sighting to find out whether it was one. The conditions are the
-- query's own, word for word, as SQLite needs to know it can use it.
CREATE INDEX IF NOT EXISTS idx_packet_seen_direct
	ON packet_seen (node_id, import_time_us, packet_id, rx_rssi, rx_snr)
	WHERE hop_start > 0 AND hop_limit = hop_start AND rx_rssi <> 0;

-- What the scheduled jobs did, one row for each time one ran: the daily cleanup
-- and the daily snapshot, and whatever comes after them.
--
-- The Python version kept the last cleanup in a JSON file beside its log, for
-- its web server, another process, to read back. A table is where that belongs:
-- every process that can open the database can read it, and nothing else has to
-- be shared between them.
--
-- `details` is JSON, its shape the job's own: for a cleanup the cutoff and the
-- rows deleted from each table, for a snapshot the figures recorded. Only a
-- bounded number of runs is kept for each job.
CREATE TABLE IF NOT EXISTS job_run (
	id INTEGER NOT NULL,
	job VARCHAR NOT NULL,
	status VARCHAR NOT NULL,
	started_us BIGINT NOT NULL,
	finished_us BIGINT,
	details TEXT,
	error TEXT,
	PRIMARY KEY (id)
);
CREATE INDEX IF NOT EXISTS idx_job_run_job_started_us ON job_run (job, started_us DESC);
