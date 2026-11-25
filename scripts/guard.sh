#!/usr/bin/env bash
# Check assigned policies between fleet runs.
set -euo pipefail

state=${SVANBOT_STATE:-"$HOME/.local/state/svanbot"}
lib=${SVANBOT_LIB:-"$HOME/.local/lib/svanbot"}
export PATH="$lib/current:$HOME/.local/bin:$PATH"

log() {
    printf '%s guard: %s\n' "$(date '+%F %T')" "$*"
}

mkdir -p "$state"
exec 8>>"$state/guard.lock"
if ! flock -n 8; then
    log "another guard is already running"
    exit 0
fi

log "checking assigned policies"
nice -n 19 ionice -c3 svanbot play guard
