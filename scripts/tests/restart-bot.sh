#!/usr/bin/env bash
# restart-bot.sh signals every fleet process and kills the ones that ignore SIGTERM (#803).
# Scratch tree and throwaway sleepers; no live pidfiles, no signals to the real fleet.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
root=$(mktemp -d)
cleanup() {
  for file in "$root"/artifacts/*.pid; do
    [ -f "$file" ] || continue
    kill -KILL "$(cat "$file")" 2>/dev/null || true
  done
  rm -rf "$root"
}
trap cleanup EXIT
fail() { echo "restart-bot: $*" >&2; exit 1; }
mkdir -p "$root"/{scripts,artifacts}
cp "$repo/scripts/restart-bot.sh" "$root/scripts/"

# Nothing running: says so, exits 0.
out=$("$root/scripts/restart-bot.sh") || fail "failed with no fleet"
grep -q 'Fleet not running' <<<"$out" || fail "did not say the fleet is not running: $out"

# A well-behaved process exits on SIGTERM; a stubborn one ignores it and must be killed.
# Started from a subshell so init reaps them: a zombie child of this shell would still answer kill -0.
( setsid sleep 300 & echo $! > "$root/artifacts/head.pid" )
( setsid bash -c "trap '' TERM; while :; do sleep 1; done" & echo $! > "$root/artifacts/worker-Stubborn_.pid" )
polite=$(cat "$root/artifacts/head.pid"); stubborn=$(cat "$root/artifacts/worker-Stubborn_.pid")
sleep 0.5
start=$(date +%s)
out=$(RESTART_TERM_WAIT=2 "$root/scripts/restart-bot.sh") || fail "failed with a stubborn process"
[ $(( $(date +%s) - start )) -le 6 ] || fail "waited too long for a stubborn process"
if kill -0 "$polite" 2>/dev/null; then fail "the polite process survived SIGTERM"; fi
if kill -0 "$stubborn" 2>/dev/null; then fail "the stubborn process survived the KILL"; fi
grep -q "Killed $stubborn" <<<"$out" || fail "the kill was not reported: $out"
grep -q "Killed $polite" <<<"$out" && fail "killed a process that had already exited: $out"
echo "restart-bot tests passed"
