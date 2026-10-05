#!/usr/bin/env bash
# Stop the fleet gracefully (models are saved on SIGTERM); seats are held 120s server-side.
set -uo pipefail
cd "$(dirname "$0")/.."
source scripts/supervisors.sh
# How long the keepalive (0145) leaves the fleet down: --hold 45m | 8h | forever (default 30m).
hold=30m
while [ $# -gt 0 ]; do
  case "$1" in
    --hold) hold="${2:-}"; shift 2 || shift ;;
    *) echo "usage: stop.sh [--hold 30m|8h|forever]" >&2; exit 2 ;;
  esac
done
mkdir -p artifacts
# The hold is written before anything is stopped, and a hold that cannot be written stops nothing:
# without it keepalive restarts the fleet within five minutes, under a restore or a rollback.
hold_until() {
  echo "$1" > artifacts/hold-until && [ "$(cat artifacts/hold-until 2>/dev/null)" = "$1" ] ||
    { echo "stop.sh: could not write artifacts/hold-until; nothing was stopped" >&2; exit 1; }
}
case "$hold" in
  forever) hold_until forever ;;
  [0-9]*m) [[ ${hold%m} =~ ^[0-9]+$ ]] || { echo "stop.sh: --hold takes Nm, Nh or forever" >&2; exit 2; }
           hold_until $(( $(date +%s) + ${hold%m} * 60 )) ;;
  [0-9]*h) [[ ${hold%h} =~ ^[0-9]+$ ]] || { echo "stop.sh: --hold takes Nm, Nh or forever" >&2; exit 2; }
           hold_until $(( $(date +%s) + ${hold%h} * 3600 )) ;;
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
# Supporting children belong to this checkout. Never use an unscoped pkill pattern, which can
# stop another deployment (or the live fleet when a scratch fault test runs).
for tool in learner analyst monitor; do
  file="artifacts/$tool-supervisor.pid"
  if pid_alive "$file"; then kill "$(cat "$file")" 2>/dev/null || true; fi
  term_pidfile "artifacts/$tool.pid"
  # Compatibility with supervisors installed before supporting child pid files were recorded.
  while true; do
    orphan=$(find_orphan "$tool")
    [ -n "$orphan" ] || break
    echo "$orphan" > "artifacts/$tool.pid"
    term_pidfile "artifacts/$tool.pid"
  done
done
[ -f artifacts/logrotate.pid ] && kill "$(cat artifacts/logrotate.pid)" 2>/dev/null || true
rm -f artifacts/bot.pid artifacts/head.pid artifacts/worker-*.pid artifacts/fleet.pids artifacts/supervisor.pid artifacts/head-supervisor.pid artifacts/worker-*-supervisor.pid artifacts/learner-supervisor.pid artifacts/analyst-supervisor.pid artifacts/logrotate.pid artifacts/monitor-supervisor.pid artifacts/learner.pid artifacts/analyst.pid artifacts/monitor.pid
echo "Stopped (keepalive hold: $hold)."
