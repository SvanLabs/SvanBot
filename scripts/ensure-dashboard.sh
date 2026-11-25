#!/usr/bin/env bash
# Keeps the dashboard up while any bot is (V11-57). A crontab `* * * * *` line runs it
# (deploy/README.md, "The dashboard"). When a bot is running, or waiting to restart, and the
# dashboard is stopped or failed, it starts the dashboard. A dashboard that runs, or is restarting
# under its supervisor, is left alone, and so is one with no bot to serve. The operator or an agent
# may run it by hand.
set -euo pipefail

here=$(dirname -- "${BASH_SOURCE[0]}")
# shellcheck source=scripts/fleet.sh
source "$here/fleet.sh"

state=${SVANBOT_STATE:-"$HOME/.local/state/svanbot"}

log() {
    printf '%s ensure-dashboard: %s\n' "$(date '+%F %T')" "$*"
}

# A mark of every run: one gone stale says cron has stopped running this.
mkdir -p "$state"
touch "$state/ensure-dashboard.ran"

# What the supervisor says of `name`: running, restarting, stopped or failed.
state_of() {
    "$supervisor" status "$1" 2> /dev/null || true
}

shopt -s nullglob
active=false
for file in "$HOME"/.config/svanbot/*.env; do
    name=${file##*/}
    name=${name%.env}
    [[ $name =~ ^$bot_name_pattern$ ]] || continue
    case $(state_of "$name") in
        running | restarting)
            active=true
            break
            ;;
    esac
done
$active || exit 0

case $(state_of dashboard) in
    running | restarting) exit 0 ;;
esac

log "a bot runs and the dashboard does not: starting it"
if said=$("$supervisor" start dashboard 2>&1); then
    exit 0
elif [[ $said == *"is already running"* ]]; then
    # `update.sh` restarts the dashboard at the end of a rollout, and may have started it a moment ago.
    exit 0
fi
printf '%s\n' "$said" >&2
exit 1
