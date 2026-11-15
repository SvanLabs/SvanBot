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

# 4. container-start.sh stops the fleet on a SIGTERM that arrives while start.sh is still running
# (#1028): the handler used to be installed only after start.sh returned, and a slow start is
# exactly when `docker stop` lands.
c="$t/container"
mkdir -p "$c/scripts"
cp "$repo/scripts/container-start.sh" "$c/scripts/"
touch "$c/.env"
printf '#!/bin/sh\necho started >> started\nsleep 2\n' > "$c/scripts/start.sh"
printf '#!/bin/sh\necho stopped >> stopped\n' > "$c/scripts/stop.sh"
chmod +x "$c/scripts/start.sh" "$c/scripts/stop.sh"
(cd "$c" && exec bash scripts/container-start.sh >/dev/null 2>&1) & cpid=$!
pids+=("$cpid")
for _ in $(seq 50); do [ -f "$c/started" ] && break; sleep .1; done
[ -f "$c/started" ] || fail "container-start.sh never ran start.sh"
kill -TERM "$cpid"
for _ in $(seq 100); do kill -0 "$cpid" 2>/dev/null || break; sleep .1; done
kill -0 "$cpid" 2>/dev/null && fail "container-start.sh ignored SIGTERM during start.sh"
[ -f "$c/stopped" ] || fail "SIGTERM during start.sh did not stop the fleet"
# Only a tail running from the scratch tree is ours to stop.
for p in $(pgrep -x tail); do
  [ "$(readlink "/proc/$p/cwd" 2>/dev/null)" != "$c" ] || kill "$p" 2>/dev/null || true
done

# 5. The fleet unit's stop budget is the 180 s the keepalive unit's comment counts on (#1028): a
# stop.sh that saves every bot must not be killed by systemd half way.
stop_budget=$(sed -n 's/^TimeoutStopSec=//p' "$repo/scripts/svanbot10.service")
[ "$stop_budget" = 180 ] || fail "svanbot10.service TimeoutStopSec=$stop_budget, keepalive.service budgets 180"

# 6. `start.sh --repair` refuses to start the other layout beside a live supervisor (#1028). The
# layout is read from the environment each run, so a fleet started with SVANBOT_FLEET=split by hand
# and repaired by the unit (which reads .env) would otherwise get a second set of database writers.
layout_case() { # <name> <layout in .env> <pid file of the running supervisor>
  local r="$t/$1" live
  mkdir -p "$r/scripts" "$r/artifacts/logs" "$r/target/release" "$r/web/dist"
  cp "$repo/scripts/start.sh" "$repo/scripts/supervisors.sh" "$r/scripts/"
  printf '#!/usr/bin/env bash\necho "$0" >> artifacts/launches\nexec sleep 60\n' > "$r/target/release/sv10-bot"
  chmod +x "$r/target/release/sv10-bot"
  printf 'SVANBOT_FLEET=%s\nLEARNER=0\nANALYST=0\n' "$2" > "$r/.env"
  (cd "$r" && exec sleep 60) & live=$!
  pids+=("$live")
  echo "$live" > "$r/artifacts/$3"
  sleep .2
  (cd "$r" && env -u SVANBOT_FLEET START_API_WAIT=1 SVANBOT_WEB_PORT=1 bash scripts/start.sh --repair >/dev/null 2>&1) || true
  sleep .5
  touch "$r/artifacts/stop.flag"
  for f in "$r"/artifacts/*.pid; do pids+=("$(cat "$f")"); done
  [ ! -e "$r/artifacts/launches" ] || fail "$1: --repair launched the $2 layout beside the live $3"
}
layout_case split-over-all split supervisor.pid
layout_case all-over-split all head-supervisor.pid
