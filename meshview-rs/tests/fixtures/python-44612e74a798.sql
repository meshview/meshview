-- The schema the Python meshview leaves at Alembic revision 44612e74a798, dumped
-- from a database it migrated from empty, plus the migration_status table its
-- writer adds at startup and one row in each table.
PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE alembic_version (
	version_num VARCHAR(32) NOT NULL, 
	CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num)
);
INSERT INTO alembic_version VALUES('44612e74a798');
CREATE TABLE example (
	id INTEGER NOT NULL, 
	name VARCHAR(100) NOT NULL, 
	description TEXT, 
	value FLOAT, 
	is_active BOOLEAN DEFAULT '1' NOT NULL, 
	created_at DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL, 
	updated_at DATETIME, 
	PRIMARY KEY (id)
);
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
INSERT INTO packet VALUES(1001,1,305419896,4294967295,x'0801',1794355200000000,'LongFast');
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
INSERT INTO packet_seen VALUES(1001,305419896,1794355200,3,3,'LongFast',6.5,-90,'msh/US/bayarea/2/e/LongFast/!12345678',1794355200000000);
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
INSERT INTO traceroute VALUES(1,1001,305419896,1,x'',1794355200000000,NULL);
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
	last_seen_us BIGINT, is_mqtt_gateway BOOLEAN, last_altitude BIGINT, 
	PRIMARY KEY (id), 
	UNIQUE (node_id)
);
INSERT INTO node VALUES('!12345678',305419896,'Test Node','TST','HELTEC_V3','2.7.15','CLIENT',374000000,-1220000000,'LongFast',1794268800000000,1794355200000000,1,12);
CREATE TABLE daily_snapshot (
	snapshot_date DATE NOT NULL, 
	node_count BIGINT NOT NULL, 
	packet_count BIGINT NOT NULL, 
	gateway_count BIGINT NOT NULL, 
	captured_at_us BIGINT NOT NULL, 
	PRIMARY KEY (snapshot_date)
);
INSERT INTO daily_snapshot VALUES('2026-11-10',1,1,1,1794355200000000);
CREATE TABLE migration_status (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                in_progress BOOLEAN NOT NULL DEFAULT FALSE,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
INSERT INTO migration_status VALUES(1,0,'2026-11-10 08:00:00');
CREATE INDEX idx_example_name ON example (name);
CREATE INDEX idx_packet_to_node_id ON packet (to_node_id);
CREATE INDEX idx_packet_from_node_id ON packet (from_node_id);
CREATE INDEX idx_packet_import_time_us ON packet (import_time_us);
CREATE INDEX idx_packet_from_node_time_us ON packet (from_node_id, import_time_us);
CREATE INDEX idx_packet_seen_packet_id ON packet_seen (packet_id);
CREATE INDEX idx_packet_seen_node_id ON packet_seen (node_id);
CREATE INDEX idx_packet_seen_import_time_us ON packet_seen (import_time_us);
CREATE INDEX idx_traceroute_import_time_us ON traceroute (import_time_us);
CREATE INDEX idx_node_first_seen_us ON node (first_seen_us);
CREATE INDEX idx_node_node_id ON node (node_id);
CREATE INDEX idx_node_last_seen_us ON node (last_seen_us);
CREATE INDEX idx_packet_from_node_portnum_time_us ON packet (from_node_id, portnum, import_time_us DESC);
COMMIT;
