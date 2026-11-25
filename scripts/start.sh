#!/usr/bin/env bash
# Starts the fleet: every bot with an env file in ~/.config/svanbot/, the dashboard and the learner,
# each under its supervisor (scripts/bot.sh, V11-52). One already running is left as it is. The
# operator or an agent runs it (deploy/README.md); a crontab `@reboot` line runs it on a reboot.
set -euo pipefail

here=$(dirname -- "${BASH_SOURCE[0]}")
# shellcheck source=scripts/fleet.sh
source "$here/fleet.sh"

if (($# != 0)); then
    printf 'Usage: %s\n' "$0" >&2
    exit 2
fi

shopt -s nullglob
bots=()
for file in "$HOME"/.config/svanbot/*.env; do
    [[ -f "$file" ]] || continue
    name=${file##*/}
    name=${name%.env}
    if [[ $name =~ ^$bot_name_pattern$ ]]; then
        bots+=("$name")
    fi
done
if ((${#bots[@]} == 0)); then
    printf 'No bot configurations found in %s.\n' "$HOME/.config/svanbot" >&2
    exit 1
fi

lock_install_tree
for name in "${bots[@]}" dashboard learn; do
    if ! said=$("$supervisor" start "$name" 2>&1); then
        # Already running is what start is for; anything else is reported and the rest still start.
        [[ $said == *"is already running"* ]] || { printf '%s\n' "$said" >&2; failed=1; }
    fi
done
exit "${failed:-0}"
