#!/usr/bin/env bash
# Start the SvanBot fleet under a restart-on-crash supervisor.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts/logs
PIDFILE=artifacts/supervisor.pid
for supervisor_pidfile in "$PIDFILE" artifacts/head-supervisor.pid artifacts/worker-*-supervisor.pid; do
  [ -f "$supervisor_pidfile" ] || continue
  supervisor_pid=$(cat "$supervisor_pidfile")
  if kill -0 "$supervisor_pid" 2>/dev/null; then
    echo "Already running (supervisor pid $supervisor_pid)."
    exit 0
  fi
done
# Only scripts/release.sh writes target/release (LESSONS 31): a missing build, or REBUILD=1, goes
# through it (tests, then build, install with a recorded commit), never a bare cargo build (0239).
if [ ! -x target/release/sv10-bot ] || [ ! -x target/release/tables ] || [ ! -f web/dist/index.html ] || [ "${REBUILD:-0}" = "1" ]; then
  echo "No complete installed build: running scripts/release.sh (tests, build, install)."
  scripts/release.sh
fi
# Exact board-strength tables (rebuildable in ~30 s; see crates/libs/equity/src/tables.rs).
if [ ! -f artifacts/tables/strengths-turn.sv10tbl ]; then
  ./target/release/tables build
fi
rm -f artifacts/stop.flag artifacts/hold-until
# Split fleet (SVANBOT_FLEET=split, 0128): one head process (dashboard, background tasks,
# canonical models) plus one worker per bot, each under the same crash-loop supervisor.
# Default is the single all-in-one process. Switching modes needs an operator restart.
# The process layout comes from the environment, else from .env (only that line is read here).
if [ -z "${SVANBOT_FLEET:-}" ] && [ -f .env ]; then
  SVANBOT_FLEET=$(sed -n 's/^SVANBOT_FLEET=\([a-z]*\)[[:space:]]*$/\1/p' .env | tail -1)
fi
if [ "${SVANBOT_FLEET:-all}" = "split" ]; then
  # shellcheck disable=SC1091
  set -a; [ -f .env ] && . ./.env; set +a
  names="${SVANBOT_MAIN_NAME:-${SVANBOT_BOT_NAME:-bot1}}"
  for i in 2 3 4 5 6 7 8 9 10; do
    eval "k=\${OPENPOKER_API_KEY_${i}:-}"
    [ -n "${k:-}" ] && eval "names=\"\$names \${BOT_${i}_NAME:-bot${i}}\""
  done
  fleet_loop() {
    role="$1"; only="$2"; pidfile="$3"
    delay=5
    while true; do
      started=$(date +%s)
      # shellcheck disable=SC2086
      env SVANBOT_HEAD="$([ "$role" = head ] && echo 1 || echo 0)" \
          SVANBOT_WORKER="$([ "$role" = worker ] && echo 1 || echo 0)" \
          SVANBOT_ONLY="$only" \
          ./target/release/sv10-bot >> artifacts/logs/svanbot10.log 2>&1 &
      echo $! > "$pidfile"
      wait $!
      code=$?
      [ -f artifacts/stop.flag ] && break
      if [ "$code" = 75 ]; then delay=5; continue; fi
      if [ $(( $(date +%s) - started )) -ge 600 ]; then delay=5; fi
      sleep "$delay"
      delay=$(( delay * 2 > 300 ? 300 : delay * 2 ))
    done
  }
  export -f fleet_loop
  rm -f artifacts/fleet.pids
  nohup bash -c 'fleet_loop head "" artifacts/head.pid' > /dev/null 2>&1 &
  echo $! > artifacts/head-supervisor.pid
  echo "head:$!" >> artifacts/fleet.pids
  for n in $names; do
    safe="worker-$(echo "$n" | tr -c 'A-Za-z0-9' '_')"
    nohup bash -c "fleet_loop worker '$n' 'artifacts/$safe.pid'" > /dev/null 2>&1 &
    echo $! > "artifacts/$safe-supervisor.pid"
    echo "$n:$!" >> artifacts/fleet.pids
  done
  echo "Started split fleet (head + $(echo "$names" | wc -w) workers; see artifacts/fleet.pids)."
else
# Crash-loop backoff: a run shorter than 10 minutes doubles the restart delay (5 s .. 5 min), so a
# persistent failure (revoked key, bad config) cannot hammer the server or flood the log.
nohup bash -c '
  delay=5
  while true; do
    started=$(date +%s)
    ./target/release/sv10-bot >> artifacts/logs/svanbot10.log 2>&1 &
    echo $! > artifacts/bot.pid
    wait $!
    code=$?
    [ -f artifacts/stop.flag ] && break
    if [ "$code" = 75 ]; then
      # Hot swap to a new release (scripts/release.sh): restart at once, no backoff.
      echo "$(date +%FT%T%:z) sv10-bot swapping to the new release" >> artifacts/logs/svanbot10.log
      delay=5
      continue
    fi
    if [ $(( $(date +%s) - started )) -ge 600 ]; then delay=5; fi
    echo "$(date +%FT%T%:z) sv10-bot exited with $code; restarting in ${delay}s" >> artifacts/logs/svanbot10.log
    sleep "$delay"
    delay=$(( delay * 2 > 300 ? 300 : delay * 2 ))
  done
' > /dev/null 2>&1 &
echo $! > "$PIDFILE"
fi
# Size-based log rotation (copy-truncate keeps the running processes' append handles valid).
nohup bash -c '
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
if [ "${LEARNER:-1}" = "1" ]; then
  # The learner yields to live play (0115): lowest CPU nice, lowest best-effort I/O class, and the OOM
  # killer's first choice (+500; unprivileged processes may raise their own score), inherited by every run.
  nohup nice -n 15 bash -c 'echo 500 > /proc/self/oom_score_adj; ionice -c2 -n7 -p $$; while [ ! -f artifacts/stop.flag ]; do ./target/release/learner >> artifacts/logs/learner.log 2>&1; sleep 30; done' > /dev/null 2>&1 &
  echo $! > artifacts/learner-supervisor.pid
fi
if [ "${ANALYST:-1}" = "1" ]; then
  # The decision analyst re-solves every live decision with a deep search on every core: below live play
  # (nice 10) and above the learner (nice 15), exits for a new release like the learner.
  nohup nice -n 10 bash -c 'ionice -c2 -n6 -p $$; while [ ! -f artifacts/stop.flag ]; do ./target/release/analyst >> artifacts/logs/analyst.log 2>&1; sleep 30; done' > /dev/null 2>&1 &
  echo $! > artifacts/analyst-supervisor.pid
fi
# Results monitor (0104): read-only alerts (big losses, nemesis opponents, stalls, fleet errors) in
# artifacts/logs/monitor.log, restarted if it exits.
nohup bash -c 'while [ ! -f artifacts/stop.flag ]; do python3 scripts/monitor.py --interval 300 --summary-min 30 --big-loss-bb 250 >> artifacts/logs/monitor.log 2>&1; sleep 30; done' > /dev/null 2>&1 &
echo $! > artifacts/monitor-supervisor.pid
sleep 2
if [ -f artifacts/fleet.pids ]; then
  echo "Started learner, analyst and monitor alongside the split fleet (scripts/status.sh shows every process)."
else
  echo "Started (supervisor pid $(cat "$PIDFILE" 2>/dev/null || echo '?'), bot pid $(cat artifacts/bot.pid 2>/dev/null || echo '?'))."
fi
