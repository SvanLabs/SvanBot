#!/usr/bin/env bash
# Restart only the fleet processes (their supervisors relaunch them); the learner keeps running.
# Handles the single all-in-one process (bot.pid) and the split fleet (head.pid + worker-*.pid).
# A process still alive RESTART_TERM_WAIT seconds (default 10) after SIGTERM is killed, as stop.sh does:
# builds before the exit-on-SIGTERM fix stayed alive after "shutting down" and the restart never happened (#803).
cd "$(dirname "$0")/.."
wait_s=${RESTART_TERM_WAIT:-10}
pids=()
for pidfile in artifacts/bot.pid artifacts/head.pid artifacts/worker-*.pid; do
  [ -f "$pidfile" ] || continue
  pid=$(cat "$pidfile")
  if kill -0 "$pid" 2>/dev/null; then
    kill -TERM "$pid"
    echo "Sent SIGTERM to $pid ($(basename "$pidfile" .pid)); its supervisor restarts it in ~5s."
    pids+=("$pid")
  fi
done
[ "${#pids[@]}" -gt 0 ] || { echo "Fleet not running; use scripts/start.sh"; exit 0; }
for _ in $(seq 1 $((wait_s * 2))); do
  alive=0
  for pid in "${pids[@]}"; do kill -0 "$pid" 2>/dev/null && alive=1; done
  [ "$alive" = 1 ] || break
  sleep 0.5
done
for pid in "${pids[@]}"; do
  if kill -0 "$pid" 2>/dev/null; then
    kill -KILL "$pid" 2>/dev/null && echo "Killed $pid: still alive ${wait_s}s after SIGTERM."
  fi
done
