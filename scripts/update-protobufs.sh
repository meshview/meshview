#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>
#
# Replaces the vendored Meshtastic protobufs with those of another upstream
# revision (a commit, tag or branch of github.com/meshtastic/protobufs):
#
#   scripts/update-protobufs.sh 569a2b3ae7b6bda1a7ab177463a55d8ede3bef53
#
# Vendored rather than a submodule so that a plain clone, the CI checkout and the
# container build all have them with nothing to remember. The .options files are
# nanopb's and mean nothing to prost, so only the .proto files come across.

set -euo pipefail

revision="${1:?usage: $0 <upstream revision>}"
here="$(cd "$(dirname "$0")/.." && pwd)"
target="$here/meshview-proto/protobufs"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

git clone --quiet https://github.com/meshtastic/protobufs.git "$work/protobufs"
git -C "$work/protobufs" checkout --quiet "$revision"
resolved="$(git -C "$work/protobufs" rev-parse HEAD)"

rm -rf "$target/meshtastic"
mkdir -p "$target/meshtastic"
cp "$work/protobufs"/meshtastic/*.proto "$target/meshtastic/"
cp "$work/protobufs/nanopb.proto" "$work/protobufs/LICENSE" "$target/"
echo "$resolved" > "$target/UPSTREAM_REV"

echo "protobufs now at $resolved; run cargo test -p meshview-proto"
