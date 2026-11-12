-- The schema the Python meshview leaves in PostgreSQL at Alembic revision
-- 44612e74a798, dumped with pg_dump --inserts from a database it migrated from
-- empty, plus the migration_status table its writer adds at startup and one row
-- in each table. pg_dump's psql commands are left out, so that it can be run as
-- plain SQL.

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

SET default_tablespace = '';

SET default_table_access_method = heap;

CREATE TABLE public.alembic_version (
    version_num character varying(32) NOT NULL
);

CREATE TABLE public.daily_snapshot (
    snapshot_date date NOT NULL,
    node_count bigint NOT NULL,
    packet_count bigint NOT NULL,
    gateway_count bigint NOT NULL,
    captured_at_us bigint NOT NULL
);

CREATE TABLE public.example (
    id integer NOT NULL,
    name character varying(100) NOT NULL,
    description text,
    value double precision,
    is_active boolean DEFAULT true NOT NULL,
    created_at timestamp without time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated_at timestamp without time zone
);

CREATE SEQUENCE public.example_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.example_id_seq OWNED BY public.example.id;

CREATE TABLE public.migration_status (
    id integer NOT NULL,
    in_progress boolean DEFAULT false NOT NULL,
    updated_at timestamp without time zone DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT migration_status_id_check CHECK ((id = 1))
);

CREATE TABLE public.node (
    id character varying NOT NULL,
    node_id bigint,
    long_name character varying,
    short_name character varying,
    hw_model character varying,
    firmware character varying,
    role character varying,
    last_lat bigint,
    last_long bigint,
    channel character varying,
    first_seen_us bigint,
    last_seen_us bigint,
    is_mqtt_gateway boolean,
    last_altitude bigint
);

CREATE TABLE public.packet (
    id bigint NOT NULL,
    portnum integer,
    from_node_id bigint,
    to_node_id bigint,
    payload bytea,
    import_time_us bigint,
    channel character varying
);

CREATE SEQUENCE public.packet_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.packet_id_seq OWNED BY public.packet.id;

CREATE TABLE public.packet_seen (
    packet_id bigint NOT NULL,
    node_id bigint NOT NULL,
    rx_time bigint NOT NULL,
    hop_limit integer,
    hop_start integer,
    channel character varying,
    rx_snr double precision,
    rx_rssi integer,
    topic character varying,
    import_time_us bigint
);

CREATE TABLE public.traceroute (
    id integer NOT NULL,
    packet_id bigint,
    gateway_node_id bigint,
    done boolean,
    route bytea,
    import_time_us bigint,
    route_return bytea
);

CREATE SEQUENCE public.traceroute_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.traceroute_id_seq OWNED BY public.traceroute.id;

ALTER TABLE ONLY public.example ALTER COLUMN id SET DEFAULT nextval('public.example_id_seq'::regclass);

ALTER TABLE ONLY public.packet ALTER COLUMN id SET DEFAULT nextval('public.packet_id_seq'::regclass);

ALTER TABLE ONLY public.traceroute ALTER COLUMN id SET DEFAULT nextval('public.traceroute_id_seq'::regclass);

INSERT INTO public.alembic_version VALUES ('44612e74a798');

INSERT INTO public.daily_snapshot VALUES ('2025-01-08', 1, 1, 1, 1736370123456789);

INSERT INTO public.example VALUES (1, 'example', NULL, NULL, true, '2026-11-12 14:22:32.274251', NULL);

INSERT INTO public.migration_status VALUES (1, false, '2026-11-12 14:04:38.987853');

INSERT INTO public.node VALUES ('!deadbeef', 3735928559, 'Alice', 'A', 'HELTEC_V3', '2.7.15', 'CLIENT', 423000000, -80000000, 'LongFast', 1736283723456789, 1736370123456789, true, 125);

INSERT INTO public.packet VALUES (1450000001, 1, 3735928559, 4294967295, '\x0d', 1736370123456789, 'LongFast');

INSERT INTO public.packet_seen VALUES (1450000001, 305419896, 1736370123, 3, 3, 'LongFast', 6.25, -90, 'msh/EU_868/2/e/LongFast/!12345678', 1736370123456789);

INSERT INTO public.traceroute VALUES (1, 1450000001, 305419896, true, '\x', 1736370123456789, NULL);

SELECT pg_catalog.setval('public.example_id_seq', 1, true);

SELECT pg_catalog.setval('public.packet_id_seq', 1, false);

SELECT pg_catalog.setval('public.traceroute_id_seq', 1, true);

ALTER TABLE ONLY public.alembic_version
    ADD CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num);

ALTER TABLE ONLY public.daily_snapshot
    ADD CONSTRAINT daily_snapshot_pkey PRIMARY KEY (snapshot_date);

ALTER TABLE ONLY public.example
    ADD CONSTRAINT example_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.migration_status
    ADD CONSTRAINT migration_status_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.node
    ADD CONSTRAINT node_node_id_key UNIQUE (node_id);

ALTER TABLE ONLY public.node
    ADD CONSTRAINT node_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.packet
    ADD CONSTRAINT packet_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.packet_seen
    ADD CONSTRAINT packet_seen_pkey PRIMARY KEY (packet_id, node_id, rx_time);

ALTER TABLE ONLY public.traceroute
    ADD CONSTRAINT traceroute_pkey PRIMARY KEY (id);

CREATE INDEX idx_example_name ON public.example USING btree (name);

CREATE INDEX idx_node_first_seen_us ON public.node USING btree (first_seen_us);

CREATE INDEX idx_node_last_seen_us ON public.node USING btree (last_seen_us);

CREATE INDEX idx_node_node_id ON public.node USING btree (node_id);

CREATE INDEX idx_packet_from_node_id ON public.packet USING btree (from_node_id);

CREATE INDEX idx_packet_from_node_portnum_time_us ON public.packet USING btree (from_node_id, portnum, import_time_us DESC);

CREATE INDEX idx_packet_from_node_time_us ON public.packet USING btree (from_node_id, import_time_us DESC);

CREATE INDEX idx_packet_import_time_us ON public.packet USING btree (import_time_us DESC);

CREATE INDEX idx_packet_seen_import_time_us ON public.packet_seen USING btree (import_time_us);

CREATE INDEX idx_packet_seen_node_id ON public.packet_seen USING btree (node_id);

CREATE INDEX idx_packet_seen_packet_id ON public.packet_seen USING btree (packet_id);

CREATE INDEX idx_packet_to_node_id ON public.packet USING btree (to_node_id);

CREATE INDEX idx_traceroute_import_time_us ON public.traceroute USING btree (import_time_us);

ALTER TABLE ONLY public.packet_seen
    ADD CONSTRAINT packet_seen_packet_id_fkey FOREIGN KEY (packet_id) REFERENCES public.packet(id);

ALTER TABLE ONLY public.traceroute
    ADD CONSTRAINT traceroute_packet_id_fkey FOREIGN KEY (packet_id) REFERENCES public.packet(id);

