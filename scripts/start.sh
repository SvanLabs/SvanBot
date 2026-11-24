#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/fleet.sh
source "$(dirname -- "${BASH_SOURCE[0]}")/fleet.sh"

if (($# != 0)); then
    printf 'Usage: %s\n' "$0" >&2
    exit 2
fi

shopt -s nullglob
units=()
for file in "$HOME"/.config/svanbot/*.env; do
    [[ -f "$file" ]] || continue
    name=${file##*/}
    name=${name%.env}
    if [[ $name =~ ^$bot_name_pattern$ ]]; then
        units+=("svanbot@$name.service")
    fi
done
if ((${#units[@]} == 0)); then
    printf 'No bot configurations found in %s.\n' "$HOME/.config/svanbot" >&2
    exit 1
fi

lock_install_tree
systemctl --user start "${units[@]}" svanbot-dashboard.service
