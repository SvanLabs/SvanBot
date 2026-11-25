#!/usr/bin/env bash
# Keeps every bot on the newest release (V11-56). A crontab `*/30` line runs it (deploy/README.md,
# "Always the newest release"). When a running bot runs an older build than the newest GitHub
# release, it hands the release to scripts/update.sh: the canary first, then the rest one at a time,
# and a bot that does not come up is rolled back. The operator or an agent may run it by hand.
#
# A release that installed but did not take is left alone for six hours, so a bad build is not
# swapped in and out all day; a newer release is tried at once. One that could not be installed
# (no network) is tried again next time.
set -euo pipefail

here=$(dirname -- "${BASH_SOURCE[0]}")
# shellcheck source=scripts/fleet.sh
source "$here/fleet.sh"

# cron's PATH has neither svanbot-bot nor svanbot-release.
export PATH="$HOME/.local/bin:$PATH"

tree=${SVANBOT_LIB:-"$HOME/.local/lib/svanbot"}
state=${SVANBOT_STATE:-"$HOME/.local/state/svanbot"}
failed_file=$state/auto-update.failed
retry_after=21600

log() {
    printf '%s auto-update: %s\n' "$(date '+%F %T')" "$*"
}

# `touch` the pause file to stop it, for example after a `svanbot-release rollback`; remove it to
# resume. One run at a time: a rollout waits for each bot's gap between hands and takes a while.
mkdir -p "$state"
if [[ -e $state/auto-update.pause ]]; then
    log "paused: remove $state/auto-update.pause to resume"
    exit 0
fi
exec 8>>"$state/auto-update.lock"
if ! flock -n 8; then
    log "another auto-update is already running"
    exit 0
fi

# A rollout by hand holds the fleet's update lock while it swaps a bot. Leave it to finish: an update
# started on top of it is refused, and that refusal is not a bad release.
if ! (flock -n 9) 9>>"$tree/update.lock"; then
    log "an update is in progress; trying again next time"
    exit 0
fi

# The build each running bot runs, from /proc. A bot that is down has none, and is left alone: it
# starts on `current`, which the next rollout points at the newest build.
shopt -s nullglob
builds=()
for file in "$HOME"/.config/svanbot/*.env; do
    name=${file##*/}
    name=${name%.env}
    [[ $name =~ ^$bot_name_pattern$ ]] || continue
    pid=$("$supervisor" pid "$name" 2>/dev/null) || continue
    exe=$(readlink "/proc/$pid/exe" 2>/dev/null) || exe=
    builds+=("$(basename -- "$(dirname -- "$exe")")")
done
((${#builds[@]} > 0)) || exit 0

# `gh` on this box is logged in as the operator. Unattended, it acts as the App. The token is given
# to the two calls that need it and is exported to nothing else: the bots that update.sh restarts
# live for days, and must not carry it in their environment.
helper=${SVANBOT_APP_TOKEN:-"$HOME/.config/svanbot/app-token.sh"}
token=$("$helper")

tag=$(GH_TOKEN=$token gh release view --repo SvanLabs/SvanBot --json tagName --jq .tagName)
latest=${tag#v}
if [[ ! $latest =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    log "the newest release is '$tag', which is not a version"
    exit 1
fi

# A bot is older when the release sorts after the build it runs. One is enough.
older=false
for have in "${builds[@]}"; do
    if [[ $have != "$latest" && $(printf '%s\n%s\n' "$have" "$latest" | sort -V | tail -n 1) == "$latest" ]]; then
        older=true
    fi
done
$older || exit 0

if [[ -f $failed_file ]]; then
    read -r failed_version failed_at <"$failed_file" || true
    if [[ ${failed_version:-} == "$latest" ]] && (($(date +%s) - ${failed_at:-0} < retry_after)); then
        log "$latest did not take earlier; trying again after $(date -d "@$((failed_at + retry_after))" '+%F %T')"
        exit 0
    fi
fi

log "rolling out $latest"
# Closing the lock's descriptor (8>&-) keeps it out of the supervisors update.sh starts: they would
# hold the lock for as long as they live, and no later run could take it.
if SVANBOT_GH_TOKEN=$token "${SVANBOT_UPDATE:-$here/update.sh}" --running-only "$latest" 8>&-; then
    rm -f "$failed_file"
    log "every running bot runs $latest"
    exit 0
fi
if [[ -d $tree/$latest ]]; then
    printf '%s %s\n' "$latest" "$(date +%s)" >"$failed_file"
    log "$latest did not take; leaving it alone for $((retry_after / 3600)) hours"
else
    log "$latest could not be installed; trying again next time"
fi
exit 1
