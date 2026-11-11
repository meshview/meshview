# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

# The versions this build rests on, written down once, here, because this is the
# only place the build itself can read, which is what lets the obvious thing
# work:
#
#   podman build -t meshview-rs .
#
# Both name a minor line rather than a patch, so fixes arrive without a commit.
ARG ALPINE_VERSION=3.24
ARG RUST_VERSION=1

# Which of the two stages below the binary comes from. Declared out here because
# global scope is the only one a FROM can read.
ARG BINARY_SOURCE=builder

# --- the binary, compiled here -----------------------------------------------

FROM docker.io/library/rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder

# gcc and musl-dev come with the base image, and they are all libsqlite3-sys
# needs to compile SQLite itself. The protobufs are compiled by protox, in Rust,
# so there is no protoc to install either.
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY meshview-proto ./meshview-proto
COPY meshview-rs ./meshview-rs

# No target is named: this image's own is a musl one, whichever architecture it
# runs on, so the same file works on x86_64 and aarch64 alike.
#
# The caches are what make a second build quick. They also mean the binary is
# produced inside a mount that does not survive the step, hence copying it out
# before the shell exits.
#
# What ships is one file that brings its own libc. A dynamically linked
# executable names an interpreter to load it, so finding one would mean the
# ground had moved — a patched compiler, a different base image — and that is
# worth hearing about here rather than in production.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    set -eux; \
    cargo build --release --locked; \
    install -D target/release/meshview-rs /out/meshview-rs; \
    if readelf -l /out/meshview-rs | grep -q INTERP; then \
        echo "meshview-rs is not statically linked" >&2; \
        exit 1; \
    fi

# --- the binary, built somewhere better --------------------------------------

# CI compiles with a proper cargo cache and publishes the binary as an artifact;
# repeating that work inside a container image would be slower and would prove
# nothing. Point the build at what it already made, under dist/:
#
#   podman build --build-arg BINARY_SOURCE=prebuilt -t meshview-rs .
#
# Only the stage that gets used is assembled, so dist/ need not exist for an
# ordinary build from source. The name carries the architecture in the build's
# own vocabulary, which is what lets one multi-platform build pick each binary
# out of the same directory.
FROM scratch AS prebuilt
ARG TARGETARCH
COPY dist/meshview-rs-${TARGETARCH} /out/meshview-rs

# --- whichever of those two it was -------------------------------------------

# COPY --from takes no variables, so the choice cannot be made where the copying
# happens. A stage whose only job is to have a fixed name resolves it.
FROM ${BINARY_SOURCE} AS binary

# --- the image that ships ----------------------------------------------------

FROM docker.io/library/alpine:${ALPINE_VERSION}

LABEL org.opencontainers.image.title="meshview-rs" \
      org.opencontainers.image.description="Real-time monitoring and diagnostics for Meshtastic mesh networks" \
      org.opencontainers.image.source="https://github.com/ogarcia/meshview-rs" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later"

# The mode is set rather than inherited because a binary that has travelled
# through a CI artifact arrives without its executable bit: zip does not carry
# permissions.
COPY --from=binary --chmod=0755 /out/meshview-rs /usr/local/bin/meshview-rs

# Never root. 10001 rather than 1000, which is the first user of most systems,
# so that the files it writes on a mounted volume belong to nobody in particular;
# and the same as the Python version's image, so that its volumes carry over as
# they are. /data is where the database lives, and the working directory, so a
# config.toml mounted there is found without saying where it is. It only needs
# to be readable: nothing is ever written to it.
RUN addgroup -g 10001 meshview \
 && adduser -D -H -G meshview -u 10001 meshview \
 && mkdir -p /data \
 && chown meshview:meshview /data

# The database is the default one, packets.db in the working directory: /data.
# It is not set in the environment, which would win over a connection_string in
# the configuration file and quietly open another database than the one named.

# No VOLUME for /data. It would look like protection for the database while
# actually creating an unnamed volume that goes away with the next container;
# where the state lives is the operator's decision to make and to name.
#
# No HEALTHCHECK either: /livez and /readyz are there to be asked, but how often
# and what a failure sets in motion belong to whoever runs the container.
EXPOSE 8081

# By number: a name cannot be checked to be anything but root, so Kubernetes'
# runAsNonRoot would refuse to start the container.
USER 10001:10001
WORKDIR /data
ENTRYPOINT ["/usr/local/bin/meshview-rs"]
CMD ["run"]
