# Deployment

## Roles

One binary does everything, and what a process does is its role:

| Command | Ingests from MQTT | Serves the web and API | Migrates the database |
|---|---|---|---|
| `meshview-rs run` (the default) | yes | yes | yes |
| `meshview-rs ingest` | yes | no, probes only | yes |
| `meshview-rs serve` | no | yes | never |
| `meshview-rs migrate` | no | no | yes, then exits |

And `meshview-rs backup <file>`, which is not a role but a tool: it copies the
database, consistently, while it is in use, and exits. See [Backups](#backups).

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

## HTTPS

meshview-rs serves plain HTTP: HTTPS is for a reverse proxy in front of it to
serve, as nginx, Caddy, Traefik or a Kubernetes ingress do, getting and renewing
the certificates themselves. The Python version's `[server] tls_cert` is
deprecated: it is accepted with a warning and ignored, and will be removed in
the next version.

With a proxy on the same machine, bind meshview-rs to the loopback interface
only (`[server] bind = "127.0.0.1"`), so that it is reached through the proxy
alone. Caddy, which gets its certificate by itself:

```caddyfile
mesh.example.org {
    reverse_proxy 127.0.0.1:8081
}
```

nginx, with the certificate from elsewhere (certbot, for one):

```nginx
server {
    listen 443 ssl;
    server_name mesh.example.org;
    ssl_certificate     /etc/letsencrypt/live/mesh.example.org/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/mesh.example.org/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8081;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

meshview-rs compresses what is worth compressing itself, so the proxy need not.

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
recover from.

Anything else that opens the database also makes the side files, and leaves them
if it is the last to close and cannot remove them: the `sqlite3` shell, a script
reading it, even read only. They are as harmless as the ones above.

## Backups

meshview-rs does not schedule backups: when, where to, how many to keep and how
to get them off the machine are a backup system's job, and there are good ones.
What it does is the one part such a system cannot do well by itself, which is
take a **consistent copy of a database that is being written**:

```sh
meshview-rs backup /backups/packets-$(date +%Y%m%d-%H%M%S).db
```

It copies the database the configuration names, with SQLite's `VACUUM INTO`: a
complete, compacted copy as of one moment, made while `ingest` goes on writing,
with nothing stopped. Copying the file itself, with `cp`, `rsync` or a volume
snapshot, does not give that: with WAL the latest writes may not even be in the
file yet, and a copy taken mid-write may not open at all.

- The copy is written to `<file>.partial` and moved into place once whole, so a
  file with the name asked for is always a complete copy.
- An existing file is never overwritten: naming each copy is the scheduler's
  part, and so is deleting old ones.
- It has to run on the machine that has the database, like any other process
  using it (see [One machine](#one-machine)).
- It needs no running meshview-rs: it works as well on a stopped database.

The copy is a plain SQLite database that meshview-rs opens as it is: restoring
is stopping meshview-rs, putting the copy where the database was, and starting
it again. Compressing and encrypting it are the backup system's part too.

With cron, keeping a week:

```cron
15 3 * * *  meshview-rs --config /etc/meshview-rs/config.toml backup /var/backups/meshview/packets-$(date +\%u).db.new && mv /var/backups/meshview/packets-$(date +\%u).db.new /var/backups/meshview/packets-$(date +\%u).db
```

or as a step before restic, borg or whatever already backs the machine up, so
that what it picks up is a consistent copy rather than the live file.

Continuous replication, with [Litestream](https://litestream.io) for one, is the
other way to go, for losing minutes rather than a day. It replicates the
write-ahead log as it is written, beside meshview-rs. meshview-rs only asks for a
checkpoint of its own when it stops, which should leave Litestream in charge of
them, but this combination has not been tested yet: check Litestream's
documentation for what it needs from the applications writing the database.

The Python version's own backups (`[cleanup] backup_enabled` and the keys that go
with it) are deprecated: they are accepted with a warning, do nothing, and will
be removed in the next version.

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
| User | UID and GID 10001, never root: no user a system already has, and the Python version's image's, so its volumes carry over without a `chown`. |
| Working directory | `/data`, so a `config.toml` there is found without saying where it is. It only needs to be readable. |
| Database | `/data/packets.db` unless the configuration names another: the default, `packets.db`, is relative to the working directory. |
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
- Backups as a `CronJob` on that same node, writing to a volume the backup
  system takes them from (see [below](#backing-up-from-kubernetes)).

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
      securityContext: {fsGroup: 10001}
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
      securityContext: {fsGroup: 10001}
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

### Backing up from Kubernetes

A `CronJob` running `meshview-rs backup`, kept on the ingester's node, writing to
a volume of its own for the backup system to collect:

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: meshview-backup
spec:
  schedule: "15 3 * * *"
  concurrencyPolicy: Forbid
  jobTemplate:
    spec:
      template:
        spec:
          restartPolicy: OnFailure
          securityContext: {fsGroup: 10001}
          affinity:
            podAffinity:
              requiredDuringSchedulingIgnoredDuringExecution:
                - labelSelector:
                    matchLabels: {app: meshview, role: ingest}
                  topologyKey: kubernetes.io/hostname
          containers:
            - name: backup
              image: ghcr.io/ogarcia/meshview-rs:latest
              command: [sh, -c]
              args: ["meshview-rs backup /backups/packets-$(date +%Y%m%d).db"]
              volumeMounts:
                - {name: data, mountPath: /data}
                - {name: config, mountPath: /data/config.toml, subPath: config.toml, readOnly: true}
                - {name: backups, mountPath: /backups}
          volumes:
            - name: data
              persistentVolumeClaim: {claimName: meshview-data}
            - name: config
              secret: {secretName: meshview-config}
            - name: backups
              persistentVolumeClaim: {claimName: meshview-backups}
```

### Updating

Updating to a version with a new migration needs no coordination: the new
`ingest` migrates on start, and `serve` pods of the new version wait, not ready,
until it has. `serve` pods of the old version that are already running keep
serving the migrated database until they are replaced, which works as long as
the migration only adds to the schema, as migrations are meant to. One of them
that restarts in the meantime finds a schema newer than it knows and refuses to
start, which is what replacing it fixes.
