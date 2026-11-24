#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/fleet.sh
source "$(dirname -- "${BASH_SOURCE[0]}")/fleet.sh"

if (($# == 0)); then
    shopt -s nullglob
    for file in "$HOME"/.config/svanbot/*.env; do
        [[ -f "$file" ]] || continue
        name=${file##*/}
        name=${name%.env}
        if [[ $name =~ ^[a-zA-Z0-9_]{3,32}$ ]]; then
            set -- "$@" "$name"
        fi
    done
fi
if (($# == 0)); then
    printf 'No bot configurations found in %s.\n' "$HOME/.config/svanbot" >&2
    exit 1
fi

units=()
for name in "$@"; do
    if [[ ! $name =~ ^[a-zA-Z0-9_]{3,32}$ ]]; then
        printf 'Invalid bot name: %s\n' "$name" >&2
        exit 2
    fi
    units+=("svanbot@$name.service")
done

lock_install_tree
systemctl --user start "${units[@]}" svanbot-dashboard.service
