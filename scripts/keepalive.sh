#!/usr/bin/env bash
# Fleet keepalive (0145). Meant to run every 5 minutes from svanbot10-keepalive.timer, which the
# operator installs. A stopped fleet costs ~50 chips a hand in missed play, and while it is
# disconnected something else plays our seats (0151: -24 chips/hand). Brings the fleet back
# unless the stop was meant to last:
#   artifacts/hold-until      "forever" or a unix time; stop.sh writes it (default 30 min), start.sh clears it
#   release-operation.lock    held during a release, snapshot or rollback
# Decisions go to artifacts/logs/keepalive.log. KEEPALIVE_DRY=1 decides and logs without restarting.
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts/logs
log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" >> artifacts/logs/keepalive.log; }
source scripts/supervisors.sh
# Read feature/layout expectations without exposing keys; explicit caller flags win.
caller_fleet=${SVANBOT_FLEET-}; caller_learner=${LEARNER-}; caller_analyst=${ANALYST-}
set -a; [ ! -f .env ] || source .env; set +a
[ -z "$caller_fleet" ] || export SVANBOT_FLEET="$caller_fleet"
[ -z "$caller_learner" ] || export LEARNER="$caller_learner"
[ -z "$caller_analyst" ] || export ANALYST="$caller_analyst"
hold=none
if [ -f artifacts/hold-until ]; then
  hold=$(cat artifacts/hold-until)
  if [ "$hold" = forever ]; then echo "fleet down, held forever"; exit 0; fi
  if [[ $hold =~ ^[0-9]+$ ]] && [ "$(date +%s)" -lt "$hold" ]; then echo "fleet down, held until $hold"; exit 0; fi
fi
missing=()
while IFS= read -r file; do pid_alive "$file" || missing+=("$file"); done < <(expected_supervisors)
[ "${#missing[@]}" -gt 0 ] || { echo "fleet up"; exit 0; }
fleet_present=0
for file in artifacts/supervisor.pid artifacts/head-supervisor.pid artifacts/worker-*-supervisor.pid; do
  pid_alive "$file" && fleet_present=1
done
exec 9> artifacts/release-operation.lock
if ! flock -n 9; then
  log "fleet incomplete; release operation holds the lock, waiting"
  echo "fleet down, release in progress"
  exit 0
fi
if [ "$fleet_present" = 1 ]; then
  log "partial fleet; repairing missing supervisors: ${missing[*]}"
  echo "fleet partial, repairing"
  [ "${KEEPALIVE_DRY:-0}" = 1 ] && exit 0
  # ExecReload puts repaired children in the persistent fleet service's cgroup. Release the
  # probe lock first: start.sh --repair acquires its own lock in that service.
  exec 9>&-
  if systemctl --user start svanbot10.service >> artifacts/logs/keepalive.log 2>&1; then
    systemctl --user reload svanbot10.service >> artifacts/logs/keepalive.log 2>&1 || log "partial repair failed"
  else
    log "fleet service could not be activated for partial repair"
  fi
else
  # systemd runs startup in the fleet service's cgroup, rather than killing detached children
  # when this oneshot finishes. Startup adopts surviving children instead of duplicating writers.
  exec 9>&-
  log "fleet down with no hold in force (hold-until: $hold); restarting svanbot10.service"
  echo "fleet down, restarting"
  [ "${KEEPALIVE_DRY:-0}" = 1 ] && exit 0
  systemctl --user restart svanbot10.service >> artifacts/logs/keepalive.log 2>&1 || log "restart failed"
fi
