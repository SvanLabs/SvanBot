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
alive() { [ -f "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null; }

for p in artifacts/supervisor.pid artifacts/head-supervisor.pid; do
  if alive "$p"; then echo "fleet up"; exit 0; fi
done
hold=none
if [ -f artifacts/hold-until ]; then
  hold=$(cat artifacts/hold-until)
  if [ "$hold" = forever ]; then echo "fleet down, held forever"; exit 0; fi
  if [[ $hold =~ ^[0-9]+$ ]] && [ "$(date +%s)" -lt "$hold" ]; then echo "fleet down, held until $hold"; exit 0; fi
fi
exec 9> artifacts/release-operation.lock
if ! flock -n 9; then
  log "fleet down; a release operation holds the lock, waiting"
  echo "fleet down, release in progress"
  exit 0
fi
exec 9>&-
log "fleet down with no hold in force (hold-until: $hold); restarting svanbot10.service"
echo "fleet down, restarting"
[ "${KEEPALIVE_DRY:-0}" = 1 ] && exit 0
systemctl --user restart svanbot10.service >> artifacts/logs/keepalive.log 2>&1 || log "restart failed"
