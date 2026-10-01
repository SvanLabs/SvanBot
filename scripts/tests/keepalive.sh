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
cp "$repo_root/scripts/keepalive.sh" "$repo_root/scripts/stop.sh" "$repo_root/scripts/start.sh" "$repo_root/scripts/supervisors.sh" "$root/scripts/"
decide() { KEEPALIVE_DRY=1 "$root/scripts/keepalive.sh"; }

[ "$(decide)" = "fleet down, restarting" ] || fail "a down fleet with no hold was not restarted"
grep -q "restarting svanbot10.service" "$root/artifacts/logs/keepalive.log" || fail "restart decision not logged"

sleep 60 & sleeper=$!
for file in supervisor.pid learner-supervisor.pid analyst-supervisor.pid monitor-supervisor.pid logrotate.pid; do
  echo "$sleeper" > "$root/artifacts/$file"
done
[ "$(decide)" = "fleet up" ] || fail "a running supervisor was treated as down"
kill "$sleeper"; wait "$sleeper" 2>/dev/null || true; sleeper=
echo "$((1 << 22))" > "$root/artifacts/supervisor.pid"   # stale pid: nothing runs there
[ "$(decide)" = "fleet down, restarting" ] || fail "a stale supervisor pid kept the fleet down"
rm "$root/artifacts/supervisor.pid" "$root/artifacts/learner-supervisor.pid" "$root/artifacts/analyst-supervisor.pid" "$root/artifacts/monitor-supervisor.pid" "$root/artifacts/logrotate.pid"

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

# Start must recognize both fleet layouts before building or launching processes.
cat > "$root/scripts/release.sh" <<'SH'
#!/usr/bin/env bash
touch artifacts/unwanted-release
exit 17
SH
chmod +x "$root/scripts/release.sh"
rm -f "$root/artifacts/supervisor.pid"
sleep 60 & sleeper=$!
for pidfile in supervisor.pid head-supervisor.pid worker-bot1-supervisor.pid; do
  echo "$sleeper" > "$root/artifacts/$pidfile"
  "$root/scripts/start.sh" > "$root/start-output" 2>&1 || fail "start did not detect $pidfile"
  [ ! -f "$root/artifacts/unwanted-release" ] || fail "start rebuilt a running fleet"
  rm "$root/artifacts/$pidfile"
done
kill "$sleeper"; wait "$sleeper" 2>/dev/null || true; sleeper=
echo "$((1 << 22))" > "$root/artifacts/head-supervisor.pid"
if "$root/scripts/start.sh" > "$root/start-output" 2>&1; then
  fail "a stale split supervisor prevented startup"
else
  [ "$?" = 17 ] || fail "stale split startup failed before the release fixture"
fi
[ -f "$root/artifacts/unwanted-release" ] || fail "stale split supervisor prevented release"
echo "start supervisor detection: ok"
