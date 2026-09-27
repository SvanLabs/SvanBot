#!/usr/bin/env bash
# Keepalive decisions and the stop.sh hold (0145), in a throwaway copy: KEEPALIVE_DRY=1 never
# restarts anything, and the copy has no pid files, so stop.sh signals nothing.
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/../.." && pwd -P)
root=$(mktemp -d)
sleeper=
cleanup() { [ -z "$sleeper" ] || kill "$sleeper" 2>/dev/null || true; rm -rf "$root"; }
trap cleanup EXIT
fail() { echo "keepalive test: $*" >&2; exit 1; }
mkdir -p "$root/scripts" "$root/artifacts"
cp "$repo_root/scripts/keepalive.sh" "$repo_root/scripts/stop.sh" "$root/scripts/"
decide() { KEEPALIVE_DRY=1 "$root/scripts/keepalive.sh"; }

[ "$(decide)" = "fleet down, restarting" ] || fail "a down fleet with no hold was not restarted"
grep -q "restarting svanbot10.service" "$root/artifacts/logs/keepalive.log" || fail "restart decision not logged"

sleep 60 & sleeper=$!
echo "$sleeper" > "$root/artifacts/supervisor.pid"
[ "$(decide)" = "fleet up" ] || fail "a running supervisor was treated as down"
kill "$sleeper"; wait "$sleeper" 2>/dev/null || true; sleeper=
echo "$((1 << 22))" > "$root/artifacts/supervisor.pid"   # stale pid: nothing runs there
[ "$(decide)" = "fleet down, restarting" ] || fail "a stale supervisor pid kept the fleet down"
rm "$root/artifacts/supervisor.pid"

"$root/scripts/stop.sh" >/dev/null
[[ $(decide) == "fleet down, held until "* ]] || fail "the default 30-minute stop hold was ignored"
held=$(cat "$root/artifacts/hold-until"); now=$(date +%s)
[ "$held" -gt "$((now + 29 * 60))" ] && [ "$held" -le "$((now + 30 * 60))" ] || fail "default hold is not 30 minutes"
"$root/scripts/stop.sh" --hold 8h >/dev/null
[ "$(cat "$root/artifacts/hold-until")" -gt "$((now + 8 * 3600 - 60))" ] || fail "--hold 8h not written"
"$root/scripts/stop.sh" --hold forever >/dev/null
[ "$(decide)" = "fleet down, held forever" ] || fail "--hold forever was ignored"
if "$root/scripts/stop.sh" --hold soon >/dev/null 2>&1; then fail "stop.sh accepted a bad hold"; fi
echo "$((now - 1))" > "$root/artifacts/hold-until"
[ "$(decide)" = "fleet down, restarting" ] || fail "an expired hold kept the fleet down"

exec 8> "$root/artifacts/release-operation.lock"; flock -n 8
[ "$(decide)" = "fleet down, release in progress" ] || fail "restarted during a release operation"
exec 8>&-
echo "keepalive: ok"
