#!/usr/bin/env bash
# Removes build output nobody has touched in 14 days from this clone's target/dev, at low CPU and
# idle I/O priority, so the clone stays under its disk budget (tools/disk.ts). cargo-sweep is
# pinned in mise.box.toml. A crontab `@daily` line runs it (deploy/README.md).
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
export MISE_ENV=box CARGO_TARGET_DIR=target/dev
exec nice -n 15 ionice -c3 "${MISE:-mise}" exec -- cargo sweep --time 14
