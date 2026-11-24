#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/fleet.sh
source "$(dirname -- "${BASH_SOURCE[0]}")/fleet.sh"

if (($# != 0)); then
    printf 'Usage: %s\n' "$0" >&2
    exit 2
fi

lock_install_tree
listed=$(systemctl --user list-units 'svanbot@*.service' --all --plain --full --no-legend --no-pager)
units=()
while read -r unit _; do
    if [[ $unit =~ ^svanbot@[a-zA-Z0-9_]{3,32}\.service$ ]]; then
        units+=("$unit")
    fi
done <<< "$listed"

systemctl --user stop "${units[@]}" svanbot-dashboard.service
