#!/usr/bin/env bash
# Stop the fleet gracefully (models are saved on SIGTERM); seats are held 120s server-side.
set -uo pipefail
cd "$(dirname "$0")/.."
# How long the keepalive (0145) leaves the fleet down: --hold 45m | 8h | forever (default 30m).
hold=30m
while [ $# -gt 0 ]; do
  case "$1" in
    --hold) hold="${2:-}"; shift 2 || shift ;;
    *) echo "usage: stop.sh [--hold 30m|8h|forever]" >&2; exit 2 ;;
  esac
done
mkdir -p artifacts
case "$hold" in
  forever) echo forever > artifacts/hold-until ;;
  [0-9]*m) [[ ${hold%m} =~ ^[0-9]+$ ]] || { echo "stop.sh: --hold takes Nm, Nh or forever" >&2; exit 2; }
           echo $(( $(date +%s) + ${hold%m} * 60 )) > artifacts/hold-until ;;
  [0-9]*h) [[ ${hold%h} =~ ^[0-9]+$ ]] || { echo "stop.sh: --hold takes Nm, Nh or forever" >&2; exit 2; }
           echo $(( $(date +%s) + ${hold%h} * 3600 )) > artifacts/hold-until ;;
  *) echo "stop.sh: --hold takes Nm, Nh or forever" >&2; exit 2 ;;
esac
touch artifacts/stop.flag
term_pidfile() {
  f="$1"
  if [ -f "$f" ] && kill -0 "$(cat "$f")" 2>/dev/null; then
    kill -TERM "$(cat "$f")"
    for _ in $(seq 1 20); do
      kill -0 "$(cat "$f")" 2>/dev/null || break
      sleep 0.5
    done
    kill -0 "$(cat "$f")" 2>/dev/null && kill -KILL "$(cat "$f")"
  fi
}
if [ -f artifacts/fleet.pids ]; then
  # Split fleet (0128): head + one worker per bot, each with its own supervisor.
  while IFS=: read -r _ sup; do kill "$sup" 2>/dev/null || true; done < artifacts/fleet.pids
  for p in artifacts/head.pid artifacts/worker-*.pid; do term_pidfile "$p"; done
fi
term_pidfile artifacts/bot.pid
for p in artifacts/supervisor.pid artifacts/head-supervisor.pid artifacts/worker-*-supervisor.pid artifacts/logrotate.pid; do
  # shellcheck disable=SC2086
  [ -f $p ] && kill "$(cat $p)" 2>/dev/null || true
done
if [ -f artifacts/learner-supervisor.pid ]; then
  kill "$(cat artifacts/learner-supervisor.pid)" 2>/dev/null || true
  pkill -f "target/release/learner" 2>/dev/null || true
fi
if [ -f artifacts/analyst-supervisor.pid ]; then
  kill "$(cat artifacts/analyst-supervisor.pid)" 2>/dev/null || true
  pkill -f "target/release/analyst" 2>/dev/null || true
fi
if [ -f artifacts/monitor-supervisor.pid ]; then
  kill "$(cat artifacts/monitor-supervisor.pid)" 2>/dev/null || true
  pkill -f "scripts/monitor.py" 2>/dev/null || true
fi
[ -f artifacts/logrotate.pid ] && kill "$(cat artifacts/logrotate.pid)" 2>/dev/null || true
rm -f artifacts/bot.pid artifacts/head.pid artifacts/worker-*.pid artifacts/fleet.pids artifacts/supervisor.pid artifacts/head-supervisor.pid artifacts/worker-*-supervisor.pid artifacts/learner-supervisor.pid artifacts/analyst-supervisor.pid artifacts/logrotate.pid artifacts/monitor-supervisor.pid
echo "Stopped (keepalive hold: $hold)."
