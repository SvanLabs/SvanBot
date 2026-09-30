#!/usr/bin/env bash
# The artifact layout scripts/clean.sh reports against (0396).
#
# The regression this guards is a stale list, not a wrong program: `known` is a hand-written regex of
# what the project's own scripts write under `artifacts/`, so the day a script starts writing a new
# entry the list is behind it and a healthy box reads as untidy. The split fleet did exactly that —
# `head.pid`, `worker-*.pid`, `fleet.pids` and one supervisor pidfile per worker are what start.sh
# leaves behind in `SVANBOT_FLEET=split`, and none of them were in the list, so `scripts/clean.sh`
# named twelve entries of a running fleet as "outside the known layout". The fixture below is that
# layout as the scripts name it; the last case proves the report still names something that is
# genuinely not ours, so the section cannot pass by going blind.
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/../.." && pwd -P)
check="$repo_root/scripts/clean.sh"
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
fail() { echo "clean test: $*" >&2; exit 1; }

# Every directory the project creates there: runtime state, outputs of the report tools, and the
# two a one-off command makes (fetch-data.sh --derived, rollback.sh --preserve-unidentified).
mkdir -p "$root/artifacts"/{logs,backups,tables,archive,release-snapshots,season-checks,quarantine,derived,unidentified-builds,review}
# The single-process fleet and the split fleet are one layout: a box runs one or the other, and a
# stopped fleet keeps its pidfiles until stop.sh removes them.
for f in bot.pid supervisor.pid learner-supervisor.pid analyst-supervisor.pid monitor-supervisor.pid \
         logrotate.pid head.pid head-supervisor.pid worker-bot1.pid worker-bot2.pid \
         worker-bot1-supervisor.pid worker-bot2-supervisor.pid fleet.pids stop.flag hold-until; do
  : > "$root/artifacts/$f"
done
for f in svanbot10.db svanbot10.db-wal svanbot10.db-shm history.db history.db-wal history.db-shm \
         data-format release.lock release.log releases.log release-progress.json release-timings.json \
         update-check.json pacing-study.json bench-fixture.json release-operation.lock .env.previous \
         dashboard.png; do
  : > "$root/artifacts/$f"
done

# The section, without the header line and without the one that follows it. A clean.sh that fails
# outright is not this test's subject — the assertion below then sees empty output and says so,
# which beats a `set -e` abort that would swallow the message and the reason with it.
section() {
  { SV10_CLEAN_ROOT="$root" "$check" 2>/dev/null || true; } |
    awk -v want="$1" '$0 == want { on = 1; next } /^== / { on = 0 } on' | sed 's/^ *//'
}
layout=$(section "== artifacts/: entries outside the known layout")
[ "$layout" = none ] || fail "the entries this project's scripts write read as unknown: $layout"

# The control: an entry nothing in the tree writes is still named, so a report that always said
# "none" would fail here rather than pass everything above.
: > "$root/artifacts/not-ours.dat"
layout=$(section "== artifacts/: entries outside the known layout")
[ "$layout" = not-ours.dat ] || fail "an entry nothing writes was not reported: $layout"
rm "$root/artifacts/not-ours.dat"

# The other half of the report keeps working on the same root: the rotation's own names are known
# to it, and a backup it did not write is not.
: > "$root/artifacts/backups/svanbot10-1759000000.db"
: > "$root/artifacts/backups/daily-svanbot10-20260930.db.sha256"
[ "$(section "== artifacts/backups: files not written by the hourly/daily rotation")" = none ] ||
  fail "a rotation backup read as unknown"
: > "$root/artifacts/backups/from-the-operator"
[ "$(section "== artifacts/backups: files not written by the hourly/daily rotation")" = from-the-operator ] ||
  fail "a file the rotation did not write was not reported"

echo "clean: ok"
