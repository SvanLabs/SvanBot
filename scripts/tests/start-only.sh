#!/usr/bin/env bash
# start.sh starts the bots whenever sv10-bot is installed and never builds (#790). Scratch tree,
# sleeping tools and no release.sh to divert into; no live databases, keys or signals.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
root=$(mktemp -d)
cleanup() {
  touch "$root/artifacts/stop.flag"
  for file in "$root"/artifacts/*.pid; do
    [ -f "$file" ] || continue
    pid=$(cat "$file")
    [[ $pid =~ ^[1-9][0-9]*$ ]] && kill "$pid" 2>/dev/null || true
  done
  rm -rf "$root"
}
trap cleanup EXIT
fail() { echo "start only: $*" >&2; exit 1; }
mkdir -p "$root"/{scripts,artifacts/logs,target/release}
unset SVANBOT_FLEET LEARNER ANALYST REBUILD
export START_API_WAIT=1 SVANBOT_WEB_PORT=1   # never the live fleet's API
cp "$repo/scripts/start.sh" "$repo/scripts/stop.sh" "$repo/scripts/keepalive.sh" "$repo/scripts/supervisors.sh" "$root/scripts/"
tool() { printf '#!/usr/bin/env bash\n%s\n' "${2:-exec sleep 120}" > "$root/target/release/$1"; chmod +x "$root/target/release/$1"; }
exit_of() { "$@" > "$root/out" 2>&1 && echo 0 || echo $?; }

# The unit's ExecStop leaves a forever hold; a start that fails early must not keep it (#798).
touch "$root/artifacts/stop.flag"; echo forever > "$root/artifacts/hold-until"
[ "$(exit_of "$root/scripts/start.sh")" = 1 ] || fail "started without sv10-bot"
[ ! -e "$root/artifacts/stop.flag" ] && [ ! -e "$root/artifacts/hold-until" ] || fail "a failed start kept the hold"
grep -q 'scripts/release.sh' "$root/out" || fail "missing sv10-bot did not name release.sh"
[ ! -e "$root/artifacts/supervisor.pid" ] || fail "a supervisor launched without sv10-bot"

tool sv10-bot
[ "$(REBUILD=1 exit_of "$root/scripts/start.sh")" = 2 ] || fail "REBUILD=1 was accepted"

# Only sv10-bot installed: the bots start, every missing supporting tool is named, none is launched.
[ "$(exit_of "$root/scripts/start.sh")" = 0 ] || fail "start refused a bundle without supporting tools"
grep -q 'starting without: monitor tables learner analyst web/dist' "$root/out" || fail "missing tools not named: $(cat "$root/out")"
[ -s "$root/artifacts/supervisor.pid" ] || fail "no sv10-bot supervisor"
for name in monitor learner analyst; do
  [ ! -e "$root/artifacts/$name-supervisor.pid" ] || fail "$name supervisor started without its tool"
done
[ "$(KEEPALIVE_DRY=1 "$root/scripts/keepalive.sh")" = "fleet up" ] || fail "keepalive expects a supervisor for a tool that is not installed"
[ "$(exit_of "$root/scripts/start.sh" --repair)" = 0 ] || fail "repair failed on a bundle without supporting tools"

grep -q 'API not reachable after 1 s' "$root/out" || fail "start did not report the unreachable API"
mkdir -p "$root/fakebin"
printf '#!/bin/sh\necho "{\\"commit\\":\\"abc1234\\",\\"ok\\":true}"\n' > "$root/fakebin/curl"; chmod +x "$root/fakebin/curl"
PATH="$root/fakebin:$PATH" "$root/scripts/start.sh" --repair > "$root/out" 2>&1 || fail "repair failed with a healthy API"
grep -q 'API up, commit abc1234' "$root/out" || fail "start did not print the API check: $(cat "$root/out")"
# Repair honours a hold; only a plain start ends it.
echo forever > "$root/artifacts/hold-until"
[ "$(exit_of "$root/scripts/start.sh" --repair)" = 0 ] && [ -e "$root/artifacts/hold-until" ] || fail "repair ignored the hold"
rm "$root/artifacts/hold-until"

# A supporting tool that stays gone ends its supervisor after five tries; sv10-bot never does (#798).
# sleep is stubbed so the backoff costs nothing; the real supervisors run without errexit.
( set +e; cd "$root"; source scripts/supervisors.sh; sleep() { :; }
  supervised_process monitor artifacts/gone.pid artifacts/gone.log )
grep -q 'monitor is not installed after 5 tries; supervisor exiting' "$root/artifacts/gone.log" || fail "supervisor kept retrying a missing monitor"
[ "$(grep -c 'exited with 127' "$root/artifacts/gone.log")" = 4 ] || fail "supervisor gave up after the wrong number of tries"
mv "$root/target/release/sv10-bot" "$root/sv10-bot.keep"
( set +e; cd "$root"; source scripts/supervisors.sh; n=0; sleep() { n=$((n + 1)); [ "$n" -lt 8 ] || touch artifacts/stop.flag; }
  supervised_process sv10-bot artifacts/bots.pid artifacts/bots.log )
mv "$root/sv10-bot.keep" "$root/target/release/sv10-bot"; rm -f "$root/artifacts/stop.flag"
! grep -q 'supervisor exiting' "$root/artifacts/bots.log" || fail "sv10-bot supervisor gave up"
[ "$(grep -c 'exited with 127' "$root/artifacts/bots.log")" -ge 6 ] || fail "sv10-bot supervisor did not keep retrying"

# A failing tables build is a warning, not a reason to stay down.
tool tables 'exit 1'
[ "$(exit_of "$root/scripts/start.sh" --repair)" = 0 ] || fail "a failing tables build stopped start"
grep -q 'tables build failed' "$root/out" || fail "tables build failure not reported"
echo "start only: ok"
