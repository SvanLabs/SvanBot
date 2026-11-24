#!/usr/bin/env bash
# Stops the fleet: the learner, every bot its supervisor knows (a pid file under
# ~/.local/state/svanbot/), then the dashboard. Each stop waits for its program to exit.
set -euo pipefail

here=$(dirname -- "${BASH_SOURCE[0]}")
# shellcheck source=scripts/fleet.sh
source "$here/fleet.sh"

if (($# != 0)); then
    printf 'Usage: %s\n' "$0" >&2
    exit 2
fi

lock_install_tree
shopt -s nullglob
bots=()
for file in "${SVANBOT_STATE:-$HOME/.local/state/svanbot}"/*.pid; do
    name=${file##*/}
    name=${name%.pid}
    if [[ $name =~ ^$bot_name_pattern$ && $name != learn ]]; then
        bots+=("$name")
    fi
done

for name in learn "${bots[@]}" dashboard; do
    "$supervisor" stop "$name"
done
