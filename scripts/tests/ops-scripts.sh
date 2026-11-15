#!/usr/bin/env bash
# Regressions for the ops-script findings of #1006 (fix ticket #1028). Every case runs in a scratch
# tree; nothing here touches the live fleet or its artifacts/.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
t=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; rm -rf "$t"; }
trap cleanup EXIT
fail() { echo "ops-scripts test: $*" >&2; exit 1; }

# 1. pid_alive counts only a process whose working directory is the checkout (#1028). A pid file
# left behind survives a reboot, and the pid can name any unrelated live process.
mkdir -p "$t/root/artifacts" "$t/elsewhere"
(cd "$t/elsewhere" && exec sleep 60) & pids+=($!)
(cd "$t/root" && exec sleep 60) & pids+=($!)
sleep .3
echo "${pids[0]}" > "$t/root/artifacts/stranger.pid"
echo "${pids[1]}" > "$t/root/artifacts/ours.pid"
(cd "$t/root" && source "$repo/scripts/supervisors.sh"
  pid_alive artifacts/ours.pid || exit 11
  ! pid_alive artifacts/stranger.pid || exit 12) || fail "pid_alive identity check failed (exit $?): 11 = own process refused, 12 = unrelated process accepted"
