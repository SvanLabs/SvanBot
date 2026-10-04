#!/usr/bin/env bash
# Shared process expectations and repair: source from this checkout's root.
pid_alive() {
  local pid
  [ -f "$1" ] || return 1
  pid=$(cat "$1")
  [[ $pid =~ ^[1-9][0-9]*$ ]] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  [[ $(ps -o stat= -p "$pid" 2>/dev/null) != Z* ]]
}

fleet_names() {
  printf '%s\n' "${SVANBOT_MAIN_NAME:-${SVANBOT_BOT_NAME:-bot1}}"
  local i key name
  for i in {2..10}; do
    key="OPENPOKER_API_KEY_$i"; name="BOT_${i}_NAME"
    [ -z "${!key:-}" ] || printf '%s\n' "${!name:-bot$i}"
  done
}

worker_stem() { printf 'worker-%s' "$(printf '%s\n' "$1" | tr -c 'A-Za-z0-9' '_')"; }

# sv10-bot is the only tool the bots need to play. The rest are supporting: a missing one is skipped
# with a warning, never a reason to keep the bots down (#790). Only scripts/release.sh installs them.
has_tool() { [ -x "target/release/$1" ]; }

missing_supporting() {
  local tool
  for tool in monitor tables; do has_tool "$tool" || echo "$tool"; done
  [ "${LEARNER:-1}" != 1 ] || has_tool learner || echo learner
  [ "${ANALYST:-1}" != 1 ] || has_tool analyst || echo analyst
  [ -f web/dist/index.html ] || echo web/dist
}

fleet_playing() {
  local file
  for file in artifacts/supervisor.pid artifacts/head-supervisor.pid artifacts/worker-*-supervisor.pid; do
    pid_alive "$file" && return 0
  done
  return 1
}

expected_supervisors() {
  local name
  if [ "${SVANBOT_FLEET:-all}" = split ]; then
    echo artifacts/head-supervisor.pid
    while IFS= read -r name; do echo "artifacts/$(worker_stem "$name")-supervisor.pid"; done < <(fleet_names)
  else
    echo artifacts/supervisor.pid
  fi
  # A tool start.sh skipped has no supervisor to expect, or keepalive would reload for it forever.
  [ "${LEARNER:-1}" != 1 ] || ! has_tool learner || echo artifacts/learner-supervisor.pid
  [ "${ANALYST:-1}" != 1 ] || ! has_tool analyst || echo artifacts/analyst-supervisor.pid
  ! has_tool monitor || echo artifacts/monitor-supervisor.pid
  echo artifacts/logrotate.pid
}

# A supervisor may disappear while its child remains. Reattach supervision by observing that
# child until it exits; do not launch another database writer beside it.
find_orphan() {
  local tool="$1" pid process_root
  process_root=$(pwd -P)
  [ "$tool" != sv10-bot ] || return 0 # Fleet children have role-specific pid files.
  while read -r pid; do
    [ "$(readlink "/proc/$pid/cwd" 2>/dev/null)" = "$process_root" ] || continue
    [[ $(ps -o stat= -p "$pid" 2>/dev/null) != Z* ]] || continue
    printf '%s\n' "$pid"; return
  done < <(ps -C "$tool" -o pid=)
}

supervised_process() {
  local tool="$1" pidfile="$2" log="$3" role="${4:-}" only="${5:-}"
  local delay=5 started code child orphan missing=0
  [ "$tool" = sv10-bot ] || delay=30
  if ! pid_alive "$pidfile"; then
    orphan=$(find_orphan "$tool")
    [ -z "$orphan" ] || echo "$orphan" > "$pidfile"
  fi
  while [ ! -f artifacts/stop.flag ]; do
    started=$(date +%s)
    if pid_alive "$pidfile"; then
      child=$(cat "$pidfile")
      echo "$(date -u +%FT%TZ) adopting existing $tool child $child" >> "$log"
      while pid_alive "$pidfile" && [ ! -f artifacts/stop.flag ]; do sleep 2; done
      code=0
    elif [ "$tool" != sv10-bot ] && ! has_tool "$tool"; then
      # A supporting tool that stays gone is not worth a supervisor (#798). Five tries outlast a
      # release swap, which can remove the binary for a moment; keepalive respawns us once it exists.
      missing=$((missing + 1))
      if [ "$missing" -ge 5 ]; then
        echo "$(date -u +%FT%TZ) $tool is not installed after $missing tries; supervisor exiting (scripts/release.sh installs it)" >> "$log"
        return 0
      fi
      code=127
    else
      missing=0
      case "$tool" in
        sv10-bot)
          if [ -n "$role" ]; then
            env SVANBOT_HEAD="$([ "$role" = head ] && echo 1 || echo 0)" \
                SVANBOT_WORKER="$([ "$role" = worker ] && echo 1 || echo 0)" \
                SVANBOT_ONLY="$only" ./target/release/sv10-bot >> "$log" 2>&1 &
          else
            ./target/release/sv10-bot >> "$log" 2>&1 &
          fi ;;
        monitor) ./target/release/monitor --interval 300 --summary-min 30 --big-loss-bb 250 >> "$log" 2>&1 & ;;
        *) ./target/release/"$tool" >> "$log" 2>&1 & ;;
      esac
      child=$!
      echo "$child" > "$pidfile"
      wait "$child"; code=$?
    fi
    [ ! -f artifacts/stop.flag ] || break
    if [ "$tool" = sv10-bot ] && [ "$code" = 75 ]; then delay=5; continue; fi
    if [ $(($(date +%s) - started)) -ge 600 ]; then
      delay=5; [ "$tool" = sv10-bot ] || delay=30
    fi
    echo "$(date -u +%FT%TZ) $tool exited with $code; restarting in ${delay}s" >> "$log"
    sleep "$delay"
    delay=$((delay * 2 > 300 ? 300 : delay * 2))
  done
}

ensure_supervisor() {
  local supervisor="$1" tool="$2" pidfile="$3" log="$4" role="${5:-}" only="${6:-}"
  pid_alive "$supervisor" && return 0
  local priority=0 oom=0
  case "$tool" in learner) priority=15; oom=500 ;; analyst) priority=10 ;; esac
  nohup nice -n "$priority" bash -c '
    exec 8>&- 9>&-
    echo "$1" > /proc/self/oom_score_adj
    case "$2" in learner) ionice -c2 -n7 -p $$ ;; analyst) ionice -c2 -n6 -p $$ ;; esac
    shift
    source scripts/supervisors.sh
    supervised_process "$@"
  ' bash "$oom" "$tool" "$pidfile" "$log" "$role" "$only" > /dev/null 2>&1 &
  echo $! > "$supervisor"
  echo "Started missing $tool supervisor (pid $!)."
}
