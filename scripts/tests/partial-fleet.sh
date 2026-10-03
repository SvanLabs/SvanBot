#!/usr/bin/env bash
# Faults in a scratch deployment with sleeping tools; no live databases, keys or signals.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
fixture_python=$(command -v python3)
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
fail() { echo "partial fleet: $*" >&2; exit 1; }
await_file() {
  for _ in {1..100}; do [ ! -s "$1" ] || return 0; sleep .05; done
  fail "missing $1"
}
mkdir -p "$root"/{scripts,artifacts/logs,target/release,web/dist,fakebin}
# The scratch tree has no operator: caller-exported SVANBOT_FLEET/LEARNER/ANALYST (which
# scripts/start.sh restores over .env) must not leak the live tree's shape into it, or the
# split section repairs all-in-one and head-supervisor.pid never appears (#694).
unset SVANBOT_FLEET LEARNER ANALYST
cp "$repo/scripts/start.sh" "$repo/scripts/stop.sh" "$repo/scripts/keepalive.sh" "$root/scripts/"
[ ! -f "$repo/scripts/supervisors.sh" ] || cp "$repo/scripts/supervisors.sh" "$root/scripts/"
for tool in sv10-bot learner analyst tables; do
  cat > "$root/target/release/$tool" <<'SH'
#!/usr/bin/env bash
echo "${0##*/}:${SVANBOT_ONLY:-}" >> artifacts/launches
exec sleep 120
SH
  chmod +x "$root/target/release/$tool"
done
cat > "$root/fakebin/python3" <<'SH'
#!/usr/bin/env bash
echo monitor >> artifacts/launches
exec sleep 120
SH
chmod +x "$root/fakebin/python3"
touch "$root/web/dist/index.html" "$root/artifacts/tables-ready"
mkdir -p "$root/artifacts/tables"
touch "$root/artifacts/tables/strengths-turn.sv10tbl"
printf 'LEARNER=1\nANALYST=1\n' > "$root/.env"
(cd "$root"; exec sleep 120) &
original=$!
echo "$original" > "$root/artifacts/supervisor.pid"
echo "$original" > "$root/artifacts/bot.pid"
export PATH="$root/fakebin:$PATH"

"$root/scripts/start.sh" --repair > "$root/start-output"
for tool in learner analyst monitor; do
  await_file "$root/artifacts/$tool-supervisor.pid"
  await_file "$root/artifacts/$tool.pid"
done
await_file "$root/artifacts/logrotate.pid"
[ "$(cat "$root/artifacts/bot.pid")" = "$original" ] || fail "healthy bot was replaced"
[ "$(cat "$root/artifacts/supervisor.pid")" = "$original" ] || fail "healthy fleet supervisor was replaced"
before=$(cat "$root/artifacts/launches")
"$root/scripts/start.sh" --repair > /dev/null
[ "$(cat "$root/artifacts/launches")" = "$before" ] || fail "repeat repair launched duplicate tools"

# Kill only the learner's scratch supervisor. Its child survives and must be adopted.
child=$(cat "$root/artifacts/learner.pid")
parent=$(cat "$root/artifacts/learner-supervisor.pid")
kill "$parent"
"$root/scripts/start.sh" --repair > /dev/null
[ "$(cat "$root/artifacts/learner.pid")" = "$child" ] || fail "orphan learner was replaced"
[ "$(cat "$root/artifacts/learner-supervisor.pid")" != "$parent" ] || fail "missing learner supervisor was not repaired"
[ "$(cat "$root/artifacts/launches")" = "$before" ] || fail "adoption created a duplicate writer"

# The adopted learner must resume after its child exits, not just acquire a new supervisor.
kill "$child"
for _ in {1..800}; do
  replacement=$(cat "$root/artifacts/learner.pid")
  if [ "$replacement" != "$child" ] && kill -0 "$replacement" 2>/dev/null; then break; fi
  sleep .05
done
[ "$replacement" != "$child" ] || fail "adopted learner never resumed after its child exited"

# Exercise the timer's actual partial-repair route through service activation and ExecReload.
cat > "$root/fakebin/systemctl" <<'SH'
#!/usr/bin/env bash
echo "$*" >> "$PARTIAL_ROOT/systemctl-calls"
case "$*" in
  '--user start svanbot10.service') "$PARTIAL_ROOT/scripts/start.sh" ;;
  '--user reload svanbot10.service') "$PARTIAL_ROOT/scripts/start.sh" --repair ;;
  *) exit 17 ;;
esac
SH
chmod +x "$root/fakebin/systemctl"
export PARTIAL_ROOT="$root"
parent=$(cat "$root/artifacts/monitor-supervisor.pid")
kill "$parent"
"$root/scripts/keepalive.sh" > "$root/keepalive-output"
grep -q 'fleet partial, repairing' "$root/keepalive-output" || fail "keepalive missed a partial fleet"
grep -q -- '--user reload svanbot10.service' "$root/systemctl-calls" || fail "partial repair skipped fleet-service reload"
[ "$(cat "$root/artifacts/bot.pid")" = "$original" ] || fail "keepalive interrupted healthy play"

# A hold and a release lock suppress partial repair as well as whole-fleet restart.
mv "$root/artifacts/analyst-supervisor.pid" "$root/artifacts/held-analyst.pid"
echo forever > "$root/artifacts/hold-until"
[[ $(KEEPALIVE_DRY=1 "$root/scripts/keepalive.sh") == *'held forever'* ]] || fail "repair ignored operator hold"
rm "$root/artifacts/hold-until"
exec 9> "$root/artifacts/release-operation.lock"; flock -n 9
[[ $(KEEPALIVE_DRY=1 "$root/scripts/keepalive.sh") == *'release in progress'* ]] || fail "repair ignored release lock"
exec 9>&-
mv "$root/artifacts/held-analyst.pid" "$root/artifacts/analyst-supervisor.pid"

# Split repair must reconstruct all configured workers and preserve healthy children.
kill "$original"; wait "$original" 2>/dev/null || true
rm "$root/artifacts/supervisor.pid" "$root/artifacts/bot.pid"
printf 'SVANBOT_FLEET=split\nSVANBOT_MAIN_NAME=A\nOPENPOKER_API_KEY_2=fixture\nBOT_2_NAME=B\nLEARNER=1\nANALYST=1\n' > "$root/.env"
"$root/scripts/start.sh" --repair > /dev/null
for stem in head worker-A_ worker-B_; do
  await_file "$root/artifacts/$stem-supervisor.pid"
  await_file "$root/artifacts/$stem.pid"
done
head=$(cat "$root/artifacts/head.pid")
worker=$(cat "$root/artifacts/worker-B_.pid")
parent=$(cat "$root/artifacts/worker-B_-supervisor.pid")
kill "$parent"
"$root/scripts/start.sh" --repair > /dev/null
[ "$(cat "$root/artifacts/head.pid")" = "$head" ] || fail "split repair replaced the healthy head"
[ "$(cat "$root/artifacts/worker-B_.pid")" = "$worker" ] || fail "split repair duplicated an orphan worker"
[ "$(cat "$root/artifacts/worker-B_-supervisor.pid")" != "$parent" ] || fail "missing split supervisor was not repaired"
# A configured opt-out is honored rather than silently ignored in the all-in-one layout.
# The operator can enable everything, but recovery must not override an explicit future hold.
source "$root/scripts/supervisors.sh"
LEARNER=0 ANALYST=0 SVANBOT_FLEET=all expected_supervisors > "$root/expected"
grep -q 'learner\|analyst' "$root/expected" && fail "feature expectations ignored configured flags"
# Checkout-scoped shutdown must leave a sibling deployment alone, even with a matching tool name.
mkdir -p "$root/sibling/target/release"
(cd "$root/sibling"; exec "$fixture_python" -c 'import ctypes,time; ctypes.CDLL(None).prctl(15,b"learner",0,0,0); time.sleep(120)') &
sibling=$!
echo "$sibling" > "$root/artifacts/outside-fixture.pid"
sleep .1
"$root/scripts/stop.sh" --hold forever > /dev/null
kill -0 "$sibling" 2>/dev/null || fail "scratch shutdown signalled a sibling deployment"
echo "partial fleet: ok"
