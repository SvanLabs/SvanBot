#!/usr/bin/env bash
# Sourced by the fleet scripts: what it sets is theirs to use.
# shellcheck disable=SC2034

bot_name_pattern='[A-Za-z0-9_]{3,32}'

# The supervisor every fleet script drives (scripts/bot.sh, V11-52).
supervisor=${SVANBOT_BOT:-"$(dirname -- "${BASH_SOURCE[0]}")/bot.sh"}

lock_install_tree() {
    local tree=${SVANBOT_LIB:-"$HOME/.local/lib/svanbot"}
    if ! exec 9>>"$tree/update.lock"; then
        printf 'Cannot open the fleet update lock in %s.\n' "$tree" >&2
        return 1
    fi
    if ! flock -n 9; then
        printf 'A fleet operation or release update is already in progress.\n' >&2
        return 1
    fi
}
