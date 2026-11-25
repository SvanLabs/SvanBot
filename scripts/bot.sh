#!/usr/bin/env bash
# The fleet's supervisor (V11-52): keeps one bot, the dashboard or the learner running in the
# background, in place of a systemd unit. Installed as `svanbot-bot`; the fleet scripts and the
# updater (`svanbot-release`) drive it. The operator or an agent runs it (deploy/README.md).
#
#   svanbot-bot start <name>    start it in the background; refused if it is already running
#   svanbot-bot stop <name>     stop it and wait until it has exited (TERM, then KILL after 90 s)
#   svanbot-bot status <name>   print running | restarting | stopped | failed; exit 0 only on running
#   svanbot-bot pid <name>      print the program's pid while it runs
#   svanbot-bot run <name>      the supervisor itself, in the foreground (what start runs)
#
# <name> is a bot with an env file in ~/.config/svanbot/, `dashboard` or `learn`.
#
# What the units gave, kept:
#   - a bot restarts 10 s after a crash, never after exit 0, 1 (the venue refused the key) or 2 (the
#     setup is missing), and at most 5 starts in 5 minutes: a crash loop must not spend the venue's
#     ten connections a minute per IP, which every bot on this box shares;
#   - a 1 GiB address-space cap, so a leak cannot reach the other bots;
#   - one log each, ~/.local/state/svanbot/<name>.log, rotated once past 64 MiB.
# The learner runs one step (`svanbot learn`) every half hour at idle priority, as its timer did.
set -euo pipefail

lib=${SVANBOT_LIB:-"$HOME/.local/lib/svanbot"}
state=${SVANBOT_STATE:-"$HOME/.local/state/svanbot"}
config="$HOME/.config/svanbot"
restart_sec=${SVANBOT_RESTART_SEC:-10}
learn_every=${SVANBOT_LEARN_EVERY:-1800}
stop_sec=${SVANBOT_STOP_SEC:-90}
start_limit=5
start_window=300
log_cap=$((64 * 1024 * 1024))

usage() {
    printf 'Usage: %s start|stop|status|pid|run <bot>|dashboard|learn\n' "${0##*/}" >&2
    exit 2
}

(($# == 2)) || usage
verb=$1
name=$2
case $verb in start | stop | status | pid | run) ;; *) usage ;; esac
case $name in
    dashboard | learn) ;;
    *) [[ $name =~ ^[A-Za-z0-9_]{3,32}$ ]] || usage ;;
esac

pidfile="$state/$name.pid"
childfile="$state/$name.child"
failedfile="$state/$name.failed"
startedfile="$state/$name.started"
logfile="$state/$name.log"

say() {
    printf '%s svanbot-bot: %s\n' "$(date '+%F %T')" "$*" >> "$logfile"
}

# The pid in `file` if that process is alive and is what the file says it is: a pid reused by
# another process after a crash is not ours.
alive() {
    local file=$1 want=$2 pid
    [[ -f $file ]] || return 1
    pid=$(< "$file")
    [[ $pid =~ ^[0-9]+$ ]] || return 1
    [[ -r /proc/$pid/cmdline ]] || return 1
    [[ $(tr '\0' ' ' < "/proc/$pid/cmdline") == *"$want"* ]] || return 1
    printf '%s' "$pid"
}

supervisor() { alive "$pidfile" "run $name"; }
program() {
    local sup
    sup=$(supervisor) || return 1
    alive "$childfile" "" > /dev/null || return 1
    # The program must be the supervisor's own child, not a stranger that took its pid.
    [[ $(awk '{print $4}' "/proc/$(< "$childfile")/stat" 2> /dev/null) == "$sup" ]] || return 1
    cat "$childfile"
}

# The env file's KEY=VALUE lines, exported. It is read, never sourced: a value is data.
load_env() {
    local file=$1 line key value
    while IFS= read -r line || [[ -n $line ]]; do
        [[ $line =~ ^[[:space:]]*(#|$) ]] && continue
        [[ $line =~ ^[[:space:]]*([A-Za-z_][A-Za-z0-9_]*)=(.*)$ ]] || continue
        key=${BASH_REMATCH[1]}
        value=${BASH_REMATCH[2]}
        if [[ $value =~ ^\"(.*)\"$ || $value =~ ^\'(.*)\'$ ]]; then
            value=${BASH_REMATCH[1]}
        fi
        export "$key=$value"
    done < "$file"
}

status() {
    if program > /dev/null; then
        echo running
        return 0
    elif supervisor > /dev/null; then
        echo restarting
    elif [[ -f $failedfile ]]; then
        echo failed
    else
        echo stopped
    fi
    return 3
}

# The pid of a process that already plays this bot outside a supervisor: an installed build run
# with no arguments (a bot, not `dashboard` or `learn`) whose environment names it. A second
# connection with a bot's key takes over its live session, so a bot that a systemd unit or a hand
# still runs is never started again here.
plays_elsewhere() {
    local dir pid word
    local -a argv
    for dir in /proc/[0-9]*; do
        pid=${dir#/proc/}
        mapfile -d '' -t argv < "$dir/cmdline" 2> /dev/null || continue
        ((${#argv[@]} > 0)) || continue
        # The program is the first word, or the second when an interpreter runs it.
        word=${argv[-1]}
        [[ $word == "$lib"/*/svanbot ]] || continue
        ((${#argv[@]} <= 2)) || continue
        tr '\0' '\n' < "$dir/environ" 2> /dev/null | grep -qx "SVANBOT_NAME=$name" || continue
        printf '%s' "$pid"
        return 0
    done
    return 1
}

start() {
    if [[ $name != dashboard && $name != learn && ! -f $config/$name.env ]]; then
        printf 'svanbot-bot: %s has no configuration in %s\n' "$name" "$config" >&2
        exit 2
    fi
    mkdir -p "$state"
    if supervisor > /dev/null; then
        printf 'svanbot-bot: %s is already running\n' "$name" >&2
        exit 1
    fi
    local other
    if [[ $name != dashboard && $name != learn ]] && other=$(plays_elsewhere); then
        printf 'svanbot-bot: %s already plays as pid %s, outside its supervisor; stop that first\n' \
            "$name" "$other" >&2
        exit 1
    fi
    rm -f "$failedfile" "$childfile" "$startedfile"
    if (($(stat -c %s "$logfile" 2> /dev/null || echo 0) > log_cap)); then
        mv -f "$logfile" "$logfile.1"
    fi
    setsid bash "${BASH_SOURCE[0]}" run "$name" < /dev/null >> "$logfile" 2>&1 &
    # Returns once the supervisor has executed the program, as `systemctl start` returns once the
    # unit's program has been executed, whether or not it is still running.
    local i
    for ((i = 0; i < 100; i++)); do
        [[ -f $startedfile ]] && return 0
        sleep 0.05
    done
    printf 'svanbot-bot: %s did not start; see %s\n' "$name" "$logfile" >&2
    exit 1
}

stop() {
    local sup i
    if ! sup=$(supervisor); then
        rm -f "$pidfile" "$childfile"
        return 0
    fi
    kill -TERM "$sup" 2> /dev/null || true
    for ((i = 0; i < stop_sec * 20; i++)); do
        supervisor > /dev/null || return 0
        sleep 0.05
    done
    printf 'svanbot-bot: %s did not stop in %s s: killed\n' "$name" "$stop_sec" >&2
    local child
    child=$(program) && kill -KILL "$child" 2> /dev/null
    kill -KILL "$sup" 2> /dev/null || true
    rm -f "$pidfile" "$childfile"
}

# Whether exit `code` is one no restart can fix: for a bot, 1 (the venue refused the key) and 2
# (the setup is missing); for the dashboard, 2.
final() {
    case $name in
        dashboard) (($1 == 2)) ;;
        *) (($1 == 1 || $1 == 2)) ;;
    esac
}

# The supervisor: starts the program, waits for it, and decides whether to start it again.
run() {
    # Whatever the starter held open stays with the starter: start.sh holds the update lock on a
    # descriptor, and a supervisor that kept it would hold the lock for as long as the fleet ran.
    local fd
    for fd in "/proc/$BASHPID/fd"/*; do
        fd=${fd##*/}
        if ((fd > 2 && fd != 255)); then eval "exec $fd>&-"; fi
    done
    mkdir -p "$state"
    printf '%s' "$BASHPID" > "$pidfile"
    local stopping=false child='' pause='' code
    local -a argv starts=()
    trap 'stopping=true; [[ -n $child ]] && kill -TERM "$child" 2> /dev/null; [[ -n $pause ]] && kill "$pause" 2> /dev/null' TERM INT
    local program="$lib/current/svanbot" memory=$((1024 * 1024)) delay=$restart_sec
    case $name in
        dashboard)
            export SVANBOT_DATA=${SVANBOT_DATA:-"$HOME/.local/share/svanbot"}
            export SVANBOT_LIB=$lib
            export SVANBOT_DASHBOARD_ADDR=${SVANBOT_DASHBOARD_ADDR:-127.0.0.1:8080}
            argv=("$program" dashboard)
            delay=$((restart_sec < 5 ? restart_sec : 5))
            ;;
        learn)
            # A search step plays paired sims on several threads, each with its own stack.
            memory=$((4 * 1024 * 1024))
            argv=(nice -n 19 ionice -c3 "$program" learn)
            command -v chrt > /dev/null && argv=(chrt -i 0 "${argv[@]}")
            ;;
        *)
            # A bot starts from its own file, as a unit did: nothing the starter's shell exported
            # under SVANBOT_ (another bot's key, name or roster) may reach it.
            unset "${!SVANBOT_@}"
            export SVANBOT_STORE="$HOME/.local/share/svanbot/$name.db"
            # The env file wins over the default, as a unit's EnvironmentFile did.
            load_env "$config/$name.env"
            argv=("$program")
            ;;
    esac
    say "supervising $name"
    while ! $stopping; do
        if [[ $name != learn ]]; then
            local now kept=() at
            now=$(date +%s)
            for at in "${starts[@]}"; do ((now - at < start_window)) && kept+=("$at"); done
            starts=("${kept[@]}")
            if ((${#starts[@]} >= start_limit)); then
                say "$start_limit starts in $start_window s: $name stays down"
                touch "$failedfile"
                break
            fi
            starts+=("$now")
        fi
        (
            ulimit -v "$memory"
            exec "${argv[@]}"
        ) &
        child=$!
        printf '%s' "$child" > "$childfile"
        touch "$startedfile"
        # A trap interrupts `wait`; wait again until the program has really exited.
        code=0
        wait "$child" || code=$?
        while kill -0 "$child" 2> /dev/null; do
            code=0
            wait "$child" || code=$?
        done
        child=
        rm -f "$childfile"
        $stopping && break
        if [[ $name == learn ]]; then
            ((code == 0)) || say "learn step exited $code"
            delay=$learn_every
        elif ((code == 0)); then
            say "$name exited 0: not restarted"
            break
        elif final "$code"; then
            say "$name exited $code: a restart cannot fix it"
            touch "$failedfile"
            break
        else
            say "$name exited $code: restarting in $delay s"
        fi
        sleep "$delay" &
        pause=$!
        wait "$pause" 2> /dev/null || true
        pause=
    done
    say "$name is down"
    rm -f "$pidfile"
}

case $verb in
    start) start ;;
    stop) stop ;;
    status) status ;;
    pid) program || exit 3 ;;
    run) run ;;
esac
