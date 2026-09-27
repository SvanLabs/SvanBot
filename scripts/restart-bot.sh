#!/usr/bin/env bash
# Restart only the fleet processes (their supervisors relaunch them); the learner keeps running.
# Handles the single all-in-one process (bot.pid) and the split fleet (head.pid + worker-*.pid).
cd "$(dirname "$0")/.."
sent=0
for pidfile in artifacts/bot.pid artifacts/head.pid artifacts/worker-*.pid; do
  [ -f "$pidfile" ] || continue
  pid=$(cat "$pidfile")
  if kill -0 "$pid" 2>/dev/null; then
    kill -TERM "$pid"
    echo "Sent SIGTERM to $pid ($(basename "$pidfile" .pid)); its supervisor restarts it in ~5s."
    sent=$((sent + 1))
  fi
done
[ "$sent" -gt 0 ] || echo "Fleet not running; use scripts/start.sh"
