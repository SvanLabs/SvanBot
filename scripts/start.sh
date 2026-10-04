#!/usr/bin/env bash
# Start the fleet, or --repair only missing supervisors without interrupting healthy play.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts/logs
source scripts/supervisors.sh
repair=0
case "${1:-}" in --repair) repair=1 ;; "") ;; *) echo "usage: start.sh [--repair]" >&2; exit 2 ;; esac
exec 8> artifacts/start.lock
flock -n 8 || { echo "Another fleet start or repair is active."; exit 0; }
if [ "$repair" = 0 ]; then
  for file in artifacts/supervisor.pid artifacts/head-supervisor.pid artifacts/worker-*-supervisor.pid; do
    if pid_alive "$file"; then echo "Already running (supervisor pid $(cat "$file"))."; exit 0; fi
  done
  # Running start.sh means "play now": clear the hold before any check that can exit. The unit's
  # ExecStop writes a forever hold, so a start that failed early would otherwise leave it (#798).
  rm -f artifacts/stop.flag artifacts/hold-until
else
  hold=$(cat artifacts/hold-until 2>/dev/null || true)
  if [ "$hold" = forever ] || { [[ $hold =~ ^[0-9]+$ ]] && [ "$(date +%s)" -lt "$hold" ]; }; then
    echo "Repair held by operator ($hold)."; exit 0
  fi
  exec 9> artifacts/release-operation.lock
  flock -n 9 || { echo "Repair held by release operation."; exit 0; }
fi
# Load process configuration in both layouts; explicit caller flags take precedence.
caller_fleet=${SVANBOT_FLEET-}; caller_learner=${LEARNER-}; caller_analyst=${ANALYST-}
set -a; [ ! -f .env ] || source .env; set +a
[ -z "$caller_fleet" ] || export SVANBOT_FLEET="$caller_fleet"
[ -z "$caller_learner" ] || export LEARNER="$caller_learner"
[ -z "$caller_analyst" ] || export ANALYST="$caller_analyst"
# start.sh never builds: only scripts/release.sh writes target/release (LESSONS 31), and a start that
# needs a release to pass stays down when the gate fails (#790). sv10-bot is essential; a missing
# supporting tool is skipped with a warning.
[ "${REBUILD:-0}" != 1 ] || { echo "REBUILD is gone: run scripts/release.sh, then scripts/start.sh." >&2; exit 2; }
has_tool sv10-bot || { echo "No installed sv10-bot; run scripts/release.sh first." >&2; exit 1; }
skipped=$(missing_supporting | paste -sd' ')
[ -z "$skipped" ] || echo "warning: starting without: $skipped (scripts/release.sh installs them)" >&2
# Exact board-strength tables (rebuildable in ~30 s; see crates/libs/equity/src/tables.rs).
if [ ! -f artifacts/tables/strengths-turn.sv10tbl ] && has_tool tables; then
  ./target/release/tables build || echo "warning: tables build failed; the bots play without them until restarted" >&2
fi
rm -f artifacts/stop.flag artifacts/hold-until
if [ "${SVANBOT_FLEET:-all}" = split ]; then
  ensure_supervisor artifacts/head-supervisor.pid sv10-bot artifacts/head.pid artifacts/logs/svanbot10.log head
  echo "head:$(cat artifacts/head-supervisor.pid)" > artifacts/fleet.pids.pending
  while IFS= read -r name; do
    stem=$(worker_stem "$name")
    ensure_supervisor "artifacts/$stem-supervisor.pid" sv10-bot "artifacts/$stem.pid" artifacts/logs/svanbot10.log worker "$name"
    echo "$name:$(cat "artifacts/$stem-supervisor.pid")" >> artifacts/fleet.pids.pending
  done < <(fleet_names)
  mv artifacts/fleet.pids.pending artifacts/fleet.pids
else
  ensure_supervisor artifacts/supervisor.pid sv10-bot artifacts/bot.pid artifacts/logs/svanbot10.log
fi
# Size-based log rotation (copy-truncate keeps the running processes' append handles valid).
if ! pid_alive artifacts/logrotate.pid; then
nohup bash -c '
  exec 8>&- 9>&-
  while [ ! -f artifacts/stop.flag ]; do
    for f in artifacts/logs/svanbot10.log artifacts/logs/learner.log artifacts/logs/analyst.log artifacts/logs/monitor.log; do
      if [ -f "$f" ] && [ "$(stat -c %s "$f")" -gt 52428800 ]; then
        for i in 2 1; do [ -f "$f.$i" ] && mv "$f.$i" "$f.$((i+1))"; done
        cp "$f" "$f.1" && : > "$f"
      fi
    done
    sleep 600
  done
' > /dev/null 2>&1 &
echo $! > artifacts/logrotate.pid
fi
if [ "${LEARNER:-1}" = 1 ] && has_tool learner; then
  ensure_supervisor artifacts/learner-supervisor.pid learner artifacts/learner.pid artifacts/logs/learner.log
fi
if [ "${ANALYST:-1}" = 1 ] && has_tool analyst; then
  ensure_supervisor artifacts/analyst-supervisor.pid analyst artifacts/analyst.pid artifacts/logs/analyst.log
fi
if has_tool monitor; then
  ensure_supervisor artifacts/monitor-supervisor.pid monitor artifacts/monitor.pid artifacts/logs/monitor.log
fi
sleep 2
if [ -f artifacts/fleet.pids ]; then
  echo "Started learner, analyst and monitor alongside the split fleet (scripts/status.sh shows every process)."
else
  echo "Started (supervisor pid $(cat artifacts/supervisor.pid 2>/dev/null || echo '?'), bot pid $(cat artifacts/bot.pid 2>/dev/null || echo '?'))."
fi
# Print-only check (#798): a slow boot must not fail the unit's ExecStart, so the exit code stays 0.
if command -v curl >/dev/null; then
  health=
  for _ in $(seq 1 "${START_API_WAIT:-20}"); do
    health=$(curl -s -m 2 "http://127.0.0.1:${SVANBOT_WEB_PORT:-5000}/api/health" 2>/dev/null || true)
    [[ $health == *'"ok":true'* ]] && break
    sleep 1
  done
  if [[ $health == *'"ok":true'* ]]; then
    echo "API up, commit $(sed -n 's/.*"commit":"\([^"]*\)".*/\1/p' <<< "$health")."
  else
    echo "API not reachable after ${START_API_WAIT:-20} s; see artifacts/logs/svanbot10.log" >&2
  fi
fi
