# Deployment

## Roles

One binary does everything, and what a process does is its role:

| Command | Ingests from MQTT | Serves the web and API | Migrates the database |
|---|---|---|---|
| `meshview-rs run` (the default) | yes | yes | yes |
| `meshview-rs ingest` | yes | no, probes only | yes |
| `meshview-rs serve` | no | yes | never |
| `meshview-rs migrate` | no | no | yes, then exits |

`run` is the simple case: one process, one database. Splitting it into one
`ingest` and any number of `serve` is what lets several web servers share what
a single ingester writes.

- There is **one writer**. Only `ingest` (or `run`) creates the database,
  migrates it and writes to it. Run exactly one of them per database.
- `serve` opens the database **read only** and never changes it. Started before
  the ingester has created or migrated the database, it waits, reporting itself
  alive but not ready, and becomes ready by itself once the schema is there.
  Given a database migrated by a newer meshview-rs than itself, it refuses to
  start rather than misread it.
- `migrate` brings the schema up to date and exits, for running it as a step of
  its own before anything else starts (a Kubernetes init container or job).

### One machine

SQLite in WAL mode shares memory between the processes using a database, which
only works when they run **on the same machine**. Several `serve` processes and
one `ingest` sharing a file on local disk is fine; the same file on a network
filesystem is not, and will end up corrupted: never NFS, CephFS, SMB or any
`ReadWriteMany` volume. Web servers spread over several machines will need
PostgreSQL, which is planned but not supported yet.

## Probes

Every role listens on `[server] port` and answers two probes, outside the API:

| Probe | Answers |
|---|---|
| `GET /livez` | 200 while the process runs. Checks nothing else, on purpose: failing it gets the process restarted, and nothing a restart cannot fix belongs in it. |
| `GET /readyz` | 200 once every check the role depends on passes, 503 until then, with the list of checks either way. |

What `/readyz` checks depends on the role:

| Role | Checks |
|---|---|
| `run` | `database` |
| `ingest` | `database`, `mqtt` |
| `serve` | `database` |

`database` passes once the database is at the schema this build knows. `mqtt`
passes while connected to the broker. `run` does not count the broker, because
the web can still be served while the broker is away, and taking the process out
of rotation for that would only make things worse.

```json
{"status":"unavailable","checks":{"database":true,"mqtt":false}}
```

## Stopping

`SIGTERM` and `SIGINT` both stop the process, cleanly and in order, giving each
step up to 5 seconds, which is comfortably within the 10 a container runtime
waits before it kills:

1. The web server stops accepting and finishes the requests in flight.
2. The ingester tells the broker it is leaving and stores the messages it had
   already received.
3. The read connections close.
4. The writer folds the write-ahead log back into the database and closes last.

The last connection to close is the one that removes the `-wal` and `-shm`
files beside the database, so a stopped `run`, or a stopped `ingest` with no
`serve` left, leaves only the database file. When `ingest` stops while a
`serve` still has the database open, everything is already in the database
file, but the empty `-wal` and the `-shm` stay: a read-only connection is not
allowed to remove them. They are harmless, and the next writer to start takes
them over.

A process that is killed instead (`SIGKILL`, or out of memory) loses at most the
messages it had not stored yet, and leaves the side files for the next start to
recover from. Copying a database while it is in use, or with side files beside
it, copies only part of it: back up a stopped database, or use SQLite's own
backup.

## The binary

Releases have a statically linked binary for Linux on `amd64` and `arm64`: it
brings its own libc and needs nothing installed beside it.

```sh
tar -xzf meshview-rs-<version>-amd64.tar.gz
./meshview-rs --config /etc/meshview-rs/config.toml run
```

The x86_64 build runs on any x86_64 CPU, including older ones without AVX.

## The container image

```sh
podman run -d --name meshview -p 8081:8081 \
  -v meshview-data:/data \
  -v ./config.toml:/data/config.toml:ro \
  ghcr.io/ogarcia/meshview-rs
```

| | |
|---|---|
| Image | `ghcr.io/ogarcia/meshview-rs`: `latest` is the last release, `master` the development branch, and each release has its version as a tag. Both `amd64` and `arm64`. |
| Command | `run` by default; pass `ingest`, `serve` or `migrate` to choose another role. |
| User | UID and GID 1000, never root. |
| Working directory | `/data`, so a `config.toml` there is found without saying where it is. It only needs to be readable. |
| Database | `MESHVIEW_DATABASE__CONNECTION_STRING=sqlite:////data/packets.db` is set in the image. |
| Port | 8081. |
| Volumes | None declared: where the database lives is yours to decide and to name. Without a volume it lives and dies with the container. |
| Health check | None built in: point your runtime at `/livez` and `/readyz`. |
| CA certificates | None needed: the roots for MQTT over TLS are built into the binary. |

The whole configuration can also come from the environment, with no file:

```sh
podman run -d --name meshview -p 8081:8081 -v meshview-data:/data \
  -e MESHVIEW_MQTT__SERVER=mqtt.example.org \
  -e MESHVIEW_MQTT__TOPICS='["msh/EU_868/#"]' \
  ghcr.io/ogarcia/meshview-rs
```

## Kubernetes

The rules above translate into:

- One `ingest`, never more: `replicas: 1` and `strategy: Recreate`, so that an
  update stops the old one before it starts the new one.
- As many `serve` as wanted, **on the same node** as the ingester, sharing its
  volume. A `ReadWriteOnce` volume can be mounted by several pods as long as
  they are on one node; pod affinity keeps them there.
- `terminationGracePeriodSeconds` of at least 10, so that the clean stop has
  time to finish.

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: meshview-ingest
spec:
  replicas: 1
  strategy:
    type: Recreate
  selector:
    matchLabels: {app: meshview, role: ingest}
  template:
    metadata:
      labels: {app: meshview, role: ingest}
    spec:
      terminationGracePeriodSeconds: 15
      securityContext: {fsGroup: 1000}
      containers:
        - name: meshview
          image: ghcr.io/ogarcia/meshview-rs:latest
          args: [ingest]
          ports: [{containerPort: 8081}]
          livenessProbe: {httpGet: {path: /livez, port: 8081}}
          readinessProbe: {httpGet: {path: /readyz, port: 8081}}
          volumeMounts:
            - {name: data, mountPath: /data}
            - {name: config, mountPath: /data/config.toml, subPath: config.toml, readOnly: true}
      volumes:
        - name: data
          persistentVolumeClaim: {claimName: meshview-data}
        - name: config
          secret: {secretName: meshview-config}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: meshview-serve
spec:
  replicas: 3
  selector:
    matchLabels: {app: meshview, role: serve}
  template:
    metadata:
      labels: {app: meshview, role: serve}
    spec:
      terminationGracePeriodSeconds: 15
      securityContext: {fsGroup: 1000}
      affinity:
        podAffinity:
          requiredDuringSchedulingIgnoredDuringExecution:
            - labelSelector:
                matchLabels: {app: meshview, role: ingest}
              topologyKey: kubernetes.io/hostname
      containers:
        - name: meshview
          image: ghcr.io/ogarcia/meshview-rs:latest
          args: [serve]
          ports: [{containerPort: 8081}]
          livenessProbe: {httpGet: {path: /livez, port: 8081}}
          readinessProbe: {httpGet: {path: /readyz, port: 8081}}
          volumeMounts:
            - {name: data, mountPath: /data}
            - {name: config, mountPath: /data/config.toml, subPath: config.toml, readOnly: true}
      volumes:
        - name: data
          persistentVolumeClaim: {claimName: meshview-data}
        - name: config
          secret: {secretName: meshview-config}
```

The `serve` pods mount the volume read-write even though they only read: SQLite
readers still need the directory to be writable for the `-shm` file. The
`meshview-config` secret holds the `config.toml`; a `ConfigMap` does as well if
nothing in it is secret, and environment variables can replace it altogether.

Updating to a version with a new migration needs no coordination: the new
`ingest` migrates on start, and `serve` pods of the new version wait, not ready,
until it has. `serve` pods of the old version that are already running keep
serving the migrated database until they are replaced, which works as long as
the migration only adds to the schema, as migrations are meant to. One of them
that restarts in the meantime finds a schema newer than it knows and refuses to
start, which is what replacing it fixes.
