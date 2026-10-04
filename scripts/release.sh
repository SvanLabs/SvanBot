#!/usr/bin/env bash
# Wrapper: the installer is crates/apps/release (sv10-release, #742); see docs/OPERATIONS.md.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$root"
export PATH="$HOME/.cargo/bin:$PATH" SV10_SCRIPT_ROOT="$root"
[ -z "${SV10_RELEASE_BIN:-}" ] || exec "$SV10_RELEASE_BIN" release "$@"
source scripts/sv10-release-bin.sh
exec "$(sv10_release_bin)" release "$@"
