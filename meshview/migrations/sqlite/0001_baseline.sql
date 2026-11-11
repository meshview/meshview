-- The schema the Python meshview 4.0 leaves behind at its last Alembic revision,
-- 44612e74a798, copied from what `alembic upgrade head` actually produces rather
-- than from its models, which disagree with it about a few indexes.
--
-- Every statement is IF NOT EXISTS, and that is the whole trick of adopting a
-- database the Python version created: on one of those this migration changes
-- nothing and only gets recorded as applied, and on an empty file it builds the
-- same schema from scratch. Column order and type names are kept as Python
-- leaves them, so the two cannot be told apart afterwards.

CREATE TABLE IF NOT EXISTS "packet" (
	id BIGINT NOT NULL,
	portnum INTEGER,
	from_node_id BIGINT,
	to_node_id BIGINT,
	payload BLOB,
	import_time_us BIGINT,
	channel VARCHAR,
	PRIMARY KEY (id)
);
CREATE INDEX IF NOT EXISTS idx_packet_to_node_id ON packet (to_node_id);
CREATE INDEX IF NOT EXISTS idx_packet_from_node_id ON packet (from_node_id);
CREATE INDEX IF NOT EXISTS idx_packet_import_time_us ON packet (import_time_us);
CREATE INDEX IF NOT EXISTS idx_packet_from_node_time_us ON packet (from_node_id, import_time_us);
CREATE INDEX IF NOT EXISTS idx_packet_from_node_portnum_time_us
	ON packet (from_node_id, portnum, import_time_us DESC);

CREATE TABLE IF NOT EXISTS "packet_seen" (
	packet_id BIGINT NOT NULL,
	node_id BIGINT NOT NULL,
	rx_time BIGINT NOT NULL,
	hop_limit INTEGER,
	hop_start INTEGER,
	channel VARCHAR,
	rx_snr FLOAT,
	rx_rssi INTEGER,
	topic VARCHAR,
	import_time_us BIGINT,
	PRIMARY KEY (packet_id, node_id, rx_time),
	FOREIGN KEY(packet_id) REFERENCES packet (id)
);
CREATE INDEX IF NOT EXISTS idx_packet_seen_packet_id ON packet_seen (packet_id);
CREATE INDEX IF NOT EXISTS idx_packet_seen_node_id ON packet_seen (node_id);
CREATE INDEX IF NOT EXISTS idx_packet_seen_import_time_us ON packet_seen (import_time_us);

CREATE TABLE IF NOT EXISTS "traceroute" (
	id INTEGER NOT NULL,
	packet_id BIGINT,
	gateway_node_id BIGINT,
	done BOOLEAN,
	route BLOB,
	import_time_us BIGINT,
	route_return BLOB,
	PRIMARY KEY (id),
	FOREIGN KEY(packet_id) REFERENCES packet (id)
);
CREATE INDEX IF NOT EXISTS idx_traceroute_import_time_us ON traceroute (import_time_us);

-- The last two columns arrived through ALTER TABLE in Python, which is why they
-- come after the timestamps rather than beside the fields they belong with.
CREATE TABLE IF NOT EXISTS "node" (
	id VARCHAR NOT NULL,
	node_id BIGINT,
	long_name VARCHAR,
	short_name VARCHAR,
	hw_model VARCHAR,
	firmware VARCHAR,
	role VARCHAR,
	last_lat BIGINT,
	last_long BIGINT,
	channel VARCHAR,
	first_seen_us BIGINT,
	last_seen_us BIGINT,
	is_mqtt_gateway BOOLEAN,
	last_altitude BIGINT,
	PRIMARY KEY (id),
	UNIQUE (node_id)
);
CREATE INDEX IF NOT EXISTS idx_node_first_seen_us ON node (first_seen_us);
CREATE INDEX IF NOT EXISTS idx_node_node_id ON node (node_id);
CREATE INDEX IF NOT EXISTS idx_node_last_seen_us ON node (last_seen_us);

CREATE TABLE IF NOT EXISTS daily_snapshot (
	snapshot_date DATE NOT NULL,
	node_count BIGINT NOT NULL,
	packet_count BIGINT NOT NULL,
	gateway_count BIGINT NOT NULL,
	captured_at_us BIGINT NOT NULL,
	PRIMARY KEY (snapshot_date)
);
