#!/usr/bin/env bash
# Rolls a release out to the whole fleet in one run (V11-50): installs it when it is not installed
# yet, swaps the canary, then every other configured bot, one at a time, and stops at the first
# bot that does not take it. The operator or an agent runs it (deploy/README.md, "Updating the
# fleet").
#
#   scripts/update.sh [--canary-only] <version>
#
# Each swap is `svanbot-release update`, which takes the update lock, waits for the bot's gap
# between hands and watches it for a crash loop, so this never holds the lock itself.
set -euo pipefail

here=$(dirname -- "${BASH_SOURCE[0]}")
# shellcheck source=scripts/fleet.sh
source "$here/fleet.sh"

usage() {
    printf 'Usage: %s [--canary-only] <version>, for example: %s 11.1.7\n' "$0" "$0" >&2
    exit 2
}

canary_only=false
version=
for arg in "$@"; do
    case $arg in
        --canary-only) canary_only=true ;;
        -*) usage ;;
        *)
            [[ -z $version ]] || usage
            version=${arg#v}
            ;;
    esac
done
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage

tree=${SVANBOT_LIB:-"$HOME/.local/lib/svanbot"}

# The bots are every env file, as for start.sh; the canary is the one their files name.
shopt -s nullglob
bots=()
canary=
for file in "$HOME"/.config/svanbot/*.env; do
    name=${file##*/}
    name=${name%.env}
    [[ $name =~ ^$bot_name_pattern$ ]] || continue
    bots+=("$name")
    named=$(sed -n 's/^SVANBOT_CANARY=//p' "$file" | tail -n 1)
    canary=${canary:-$named}
done
canary=${canary:-SurSvan}
if ((${#bots[@]} == 0)); then
    printf 'No bot configurations found in %s.\n' "$HOME/.config/svanbot" >&2
    exit 1
fi
if [[ " ${bots[*]} " != *" $canary "* ]]; then
    printf 'The canary %s has no configuration in %s.\n' "$canary" "$HOME/.config/svanbot" >&2
    exit 1
fi

step() {
    printf '\n==> %s\n' "$*"
}

# Whether `bot` already runs this build: a rollout run again after a stop skips what it finished.
runs_it() {
    local pid
    pid=$("$supervisor" pid "$1" 2>/dev/null) || return 1
    [[ $(readlink "/proc/$pid/exe" 2>/dev/null) == "$tree/$version/svanbot" ]]
}

# One swap; a bot that is rolled back, or a swap that fails, ends the rollout here.
swap() {
    local bot=$1
    shift
    if runs_it "$bot"; then
        step "$bot already runs $version"
        return
    fi
    step "$bot → $version"
    local said
    if ! said=$("$@" 2>&1); then
        printf '%s\n' "$said"
        stopped "$bot"
    fi
    printf '%s\n' "$said"
    [[ $said == *"$bot now runs $version"* ]] || stopped "$bot"
}

stopped() {
    printf '\n%s did not take %s: the rollout stops here. Bots not yet swapped run what they ran.\n' \
        "$1" "$version" >&2
    printf 'See `svanbot-release status` and ~/.local/state/svanbot/%s.log.\n' "$1" >&2
    exit 1
}

if [[ ! -d $tree/$version ]]; then
    step "Installing $version"
    download=$(mktemp -d)
    trap 'rm -rf "$download"' EXIT
    gh release download "v$version" --repo SvanLabs/SvanBot --dir "$download"
    bundles=("$download"/svanbot-"$version"-*.tar.gz)
    ((${#bundles[@]} == 1)) || { printf 'v%s has no single bundle to install.\n' "$version" >&2; exit 1; }
    # install.sh checks the checksum and the CPU before anything is staged.
    bash "${SVANBOT_INSTALLER:-$here/../deploy/install.sh}" "${bundles[0]}"
    svanbot-release install "$tree/staged"
fi

swap "$canary" env SVANBOT_CANARY="$canary" SVANBOT_NAME="$canary" svanbot-release update "$version"
if $canary_only; then
    printf '\nThe canary %s runs %s. When its window is over, run this again without --canary-only:\n' \
        "$canary" "$version"
    printf 'it swaps the rest with --fleet.\n'
    exit 0
fi
for bot in "${bots[@]}"; do
    [[ $bot == "$canary" ]] && continue
    swap "$bot" env -u SVANBOT_CANARY SVANBOT_NAME="$bot" svanbot-release update --fleet "$version"
done

# The dashboard serves from current, so a running one is restarted onto the new build.
if "$supervisor" status dashboard >/dev/null; then
    step "Dashboard"
    "$supervisor" stop dashboard
    "$supervisor" start dashboard
fi

printf '\nEvery bot runs %s.\n' "$version"
