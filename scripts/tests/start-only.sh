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
cp "$repo/scripts/start.sh" "$repo/scripts/stop.sh" "$repo/scripts/keepalive.sh" "$repo/scripts/supervisors.sh" "$root/scripts/"
tool() { printf '#!/usr/bin/env bash\n%s\n' "${2:-exec sleep 120}" > "$root/target/release/$1"; chmod +x "$root/target/release/$1"; }
exit_of() { "$@" > "$root/out" 2>&1 && echo 0 || echo $?; }

[ "$(exit_of "$root/scripts/start.sh")" = 1 ] || fail "started without sv10-bot"
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

# A failing tables build is a warning, not a reason to stay down.
tool tables 'exit 1'
[ "$(exit_of "$root/scripts/start.sh" --repair)" = 0 ] || fail "a failing tables build stopped start"
grep -q 'tables build failed' "$root/out" || fail "tables build failure not reported"
echo "start only: ok"
