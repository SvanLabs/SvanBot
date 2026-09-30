#!/usr/bin/env bash
# Project cleanliness (0116, 0329). Report by default; --apply removes only what a rebuild or a rerun
# recreates, or what nothing reads any more.
#   scripts/clean.sh            report: unknown files in artifacts/, untracked files, rotated logs,
#                               superseded build artifacts, leaked test dirs, session leftovers, disk
#   scripts/clean.sh --apply    + remove superseded build artifacts (scripts/prune-builds.py: variants in
#                                 target/dev/* and target/stage/* nothing has read for two days, never
#                                 the newest of a crate; skipped while cargo or rustc runs), leaked test
#                                 and release scratch dirs older than an hour, session screenshots in
#                                 artifacts/ older than a day; prune git worktrees, zstd the oldest
#                                 rotated logs, clear target/dev when the root filesystem has under 10 GB
#                                 free (left alone, and said so, while a build runs)
# Never touches databases, backups, tables, .env files, target/release (the installed build) or tracked
# files. Runs every 6 hours (svanbot10-clean.timer), independent of the nightly archive.
#
# The tree it reports on is `SV10_CLEAN_ROOT` (default: this checkout), which exists so the layout rule
# has a test to fail (`scripts/tests/clean.sh`).
set -uo pipefail
# `--apply` deletes, so an unusable root must not fall through to the caller's directory: without the
# `|| exit`, a mistyped SV10_CLEAN_ROOT reports the error and then prunes whatever tree it happened to
# be standing in.
cd "${SV10_CLEAN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}" || exit 1
apply=0; [ "${1:-}" = --apply ] && apply=1
say() { printf '%s\n' "$*"; }

say "== artifacts/: entries outside the known layout"
# Every entry the code writes (data-format: packed store marker; release-*/update-check: the updates panel;
# pacing-study: the learner; derived: fetch-data.sh --derived; unidentified-builds: rollback.sh
# --preserve-unidentified; review: local-review.py; bench-fixture: `learner bench-fixture`). The
# pidfiles are the whole set the control scripts
# leave behind — `head`, `worker-<name>` and their supervisors in split mode, `fleet.pids` beside them,
# and `hold-until`, which is how stop.sh tells the keepalive to stay down. Session screenshots (*.png)
# are reported and swept separately below.
known='^(svanbot10\.db(-wal|-shm)?|history\.db(-wal|-shm)?|backups|tables|logs|archive|quarantine|season-checks|derived|unidentified-builds|review|.env.previous|README\.md|releases\.log|release-snapshots|(bot|supervisor|learner-supervisor|analyst-supervisor|logrotate|monitor-supervisor|head|head-supervisor|worker-[A-Za-z0-9_-]+|fleet)\.pids?|release\.log|release\.lock|release-operation\.lock|stop\.flag|hold-until|data-format|release-progress\.json|release-timings\.json|update-check\.json|pacing-study\.json|bench-fixture\.json|[^/]+\.png)$'
unknown=$(ls -A artifacts | grep -vE "$known" || true)
[ -n "$unknown" ] && say "$unknown" || say "   none"
# A copied .env holds the API keys: never removed here, only named, so the operator deletes it once
# it is no longer the way back.
for f in artifacts/.env.*; do
  [ -e "$f" ] && [ "$f" != artifacts/.env.previous ] && say "   $f holds API keys: remove it by hand once it is no longer needed"
done

say "== artifacts/backups: files not written by the hourly/daily rotation"
odd=$(ls -A artifacts/backups | grep -vE '^(svanbot10-[0-9]{10}|daily-svanbot10-[0-9]{8})\.db(\.sha256)?$' || true)
[ -n "$odd" ] && say "$odd" || say "   none"

say "== untracked, not ignored"
untracked=$(git status --porcelain --untracked-files=all | grep '^??' || true)
[ -n "$untracked" ] && say "$untracked" || say "   none"

say "== rotated logs"
# Only the oldest generation: start.sh shifts .1 -> .2 -> .3, so a compressed .3.zst is overwritten by the
# next one and stays bounded, while compressing .1/.2 would escape the rotation.
rotated=$(ls artifacts/logs/*.log.3 2>/dev/null || true)
[ -n "$rotated" ] && say "$rotated" || say "   none to compress"

say "== superseded build artifacts (cargo never deletes them)"
profiles=$(ls -d target/dev/*/ target/stage/*/ 2>/dev/null | grep -vE '/(tmp|doc|cargo-timings)/$' || true)
[ -n "$profiles" ] && python3 scripts/prune-builds.py $profiles || say "   no build profiles"

# Scratch dirs older than an hour: a test run, release or web test still in progress is younger than that.
tmp="${TMPDIR:-/tmp}"
say "== leaked test and release scratch dirs older than an hour (${tmp} is $(findmnt -no FSTYPE --target "$tmp" 2>/dev/null || echo '?'))"
leaked=$(find "$tmp" -maxdepth 1 -name 'sv10-*' -mmin +60 2>/dev/null; find target -maxdepth 1 -name '.web-stage-*' -mmin +60 2>/dev/null)
if [ -n "$leaked" ]; then
  say "   $(printf '%s\n' "$leaked" | wc -l) entries, $(printf '%s\0' $leaked | du -sch --files0-from=- 2>/dev/null | tail -1 | cut -f1)"
else
  say "   none"
fi

say "== session screenshots in artifacts/ older than a day"
shots=$(find artifacts -maxdepth 1 -name '*.png' -mmin +1440 2>/dev/null || true)
[ -n "$shots" ] && say "$shots" || say "   none"

free_gb=$(df -BG --output=avail . | tail -1 | tr -dc 0-9)
say "== disk: root ${free_gb} GB free; target/ $(du -sh target 2>/dev/null | cut -f1); artifacts/ $(du -sh artifacts | cut -f1)"

if [ "$apply" = 1 ]; then
  say "== applying"
  git worktree prune
  if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; then
    say "   build artifacts left: a cargo build is running"
    # A cleanup that declines to free space never does it silently (0360): the nightly timer is
    # unattended, so this line is the only thing that tells anyone the disk is still under 10 GB.
    [ "${free_gb:-99}" -lt 10 ] && say "   target/dev left too: ${free_gb} GB free, and a build owns it"
  else
    # target/tmp is cargo's own scratch inside the target dir it was handed — a release rebuilt
    # target/stage/tmp at 05:35 on 2026-09-27 — so it is a build artifact like the others and waits for
    # the same guard. target/udeps is a CARGO_TARGET_DIR of OPERATIONS.md's unused-deps check.
    rm -rf target/tmp target/udeps
    # A whole profile no project command builds (`debug`: only a bare `cargo build`/`cargo test` makes it;
    # the tests use `gate`, releases `release`) goes once a day has passed without a write to it: it was
    # 5.7 GB of the SSD on 2026-09-27.
    for d in target/dev/debug target/debug; do
      if [ -d "$d" ] && [ -z "$(find "$d" -mmin -1440 -print -quit 2>/dev/null)" ]; then
        rm -rf "$d" && say "   removed unused build profile $d"
      fi
    done
    [ -n "$profiles" ] && python3 scripts/prune-builds.py --apply $profiles
    # The low-disk branch removes more than the pruner above — the whole tree a running build has open,
    # rebuilt from nothing afterwards. It sat outside this guard by oversight, not by design: the guard is
    # here because pruning under a live build is unsafe, and deleting target/dev under one is that same
    # hazard with none of the caution. It refuses rather than waits — the timer is unattended, and a wait
    # would sit on the build that is itself starving for the space. The refusal is said out loud above.
    if [ "${free_gb:-99}" -lt 10 ]; then
      rm -rf target/dev && say "   removed target/dev (low disk)"
    fi
  fi
  if [ -n "$leaked" ]; then
    printf '%s\n' "$leaked" | while IFS= read -r d; do rm -rf -- "$d"; done
    say "   removed $(printf '%s\n' "$leaked" | wc -l) leaked scratch dirs"
  fi
  for f in $shots; do rm -f -- "$f" && say "   removed $f"; done
  for f in $rotated; do zstd -q --rm -f "$f" && say "   compressed $f"; done
fi
