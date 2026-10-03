#!/usr/bin/env bash
# Build, test and install a release. Running processes hot-swap to it by themselves: the fleet at
# the next moment no bot is mid-turn (seats resync within seconds), the learner between steps
# (at most about two minutes, 0334).
#   scripts/release.sh            committed tree only; full workspace tests
#   ALLOW_DIRTY=1 SKIP_TESTS=1    for emergencies
#   RELEASE_ADOPT_UNIDENTIFIED=1  operator-approved, once: release over a build with no commit identity
# The dashboard can trigger this detached (POST /api/releases/update): output is tee'd to
# artifacts/release.log for the progress tail, and artifacts/release.lock is removed on exit.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
# Node lives in ~/.local/bin on the reference box, and the fleet's own PATH (which a
# dashboard-triggered release inherits) does not include it — without this the dashboard
# build stage dies with `npm: command not found` after lint and tests pass (#696).
[ -d "$HOME/.local/bin" ] && export PATH="$HOME/.local/bin:$PATH"
SVANBOT_BUILD_STAGES=2 source scripts/resources.sh

# Progress for the dashboard's bar (0236). Under scripts/update.sh (SV10_UPDATE_RUN=1) the run, its
# log and its outcome belong to update.sh; by hand this script records them itself.
progress() { python3 scripts/progress.py "$@" || true; }
own_run() { [ "${SV10_UPDATE_RUN:-0}" != 1 ]; }
WEB_STAGE=
release_exit() {
  local status=$?
  [ -z "${WEB_STAGE:-}" ] || rm -rf -- "$WEB_STAGE"
  # A dashboard update's lock goes however this ends, even before the checks below (0236: an
  # early failure used to leave it, and the button read "running" for two hours).
  if own_run; then rm -f artifacts/release.lock; fi
  if [ "$status" != 0 ] && own_run; then progress fail "" "release failed (exit $status); the fleet keeps playing the installed build"; fi
}
trap release_exit EXIT
mkdir -p artifacts
if own_run; then progress start; fi
scripts/rollback.sh --validate-layout
exec 9> artifacts/release-operation.lock
flock -n 9 || { echo "Another release, snapshot, or rollback operation is active." >&2; exit 1; }
export SV10_RELEASE_LOCK_FD=9
# Console and dashboard tail see the same output. The run log is fresh per release (install records
# live in releases.log, untouched); under update.sh it is already open and already fresh.
if own_run; then
  : > artifacts/release.log
  exec > >(tee -a artifacts/release.log) 2>&1
fi
STAGE=target/stage
# Every thread at idle CPU priority (0302, operator 2026-09-27): the fleet always goes first. The test
# runner still sizes its parallelism by free memory (the suite was OOM-killed once, 2026-09-15).
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}"
idle() { if command -v chrt >/dev/null; then nice -n 19 chrt -i 0 "$@"; else nice -n 19 "$@"; fi; }
# Stage times, and a stage past two minutes named (0334, operator 2026-09-27: "nothing is worth more
# than 2 minutes on a live running system").
BUDGET_SECS=120
release_t0=$(date +%s)
over_budget() { [ "$2" -le "$BUDGET_SECS" ] || echo "WARNING: $1 took $2 s, over the $BUDGET_SECS s budget (0334)" >&2; }
timed() { local name=$1 t=$(date +%s); shift; "$@"; local took=$(($(date +%s) - t)); echo "   $name: $took s"; over_budget "$name" "$took"; }
if [ "${ALLOW_DIRTY:-0}" != 1 ]; then
  scripts/rollback.sh --validate-source-clean || {
    echo "Commit the listed build inputs first (or use ALLOW_DIRTY=1 only for an emergency)." >&2
    exit 1
  }
fi
commit=$(git rev-parse --short HEAD)
# Baked into the binaries (`sv10_bot::BUILD_COMMIT`, surfaced on /api/health).
export SVANBOT_COMMIT="$commit"
# Free space first: the snapshot below copies the installed sets (~250 MB), the build stages a whole
# release, and a root that ran out half-way through either leaves junk or fails late (LESSONS 22).
# Fail closed here, before anything is written.
echo "== free space"
scripts/rollback.sh --check-space
# Snapshot the currently installed release before lint/build can replace binaries or web/dist. A
# partial or unidentified existing installation fails closed; only a true first install may proceed
# without a recovery point.
progress stage snapshot
if [ -d target/release ] || [ -d web/dist ]; then
  if [ ! -f target/release/.sv10-installed-commit ] && [ "${RELEASE_ADOPT_UNIDENTIFIED:-0}" = 1 ]; then
    # Operator-approved, one-time: the installed build has no commit identity (built outside
    # release.sh), so it is copied aside with checksums rather than snapshotted by commit.
    echo "== preserving the unidentified installed build (RELEASE_ADOPT_UNIDENTIFIED=1)"
    scripts/rollback.sh --preserve-unidentified
  else
    installed_commit=$(scripts/rollback.sh --installed-commit) || {
      echo "Existing installation has no verifiable commit identity; refusing to overwrite it." >&2
      exit 1
    }
    if [ ! -f target/release/.sv10-installed-commit ]; then
      echo "== verifying one-time legacy installed identity $installed_commit"
      scripts/rollback.sh --adopt-legacy "$installed_commit" || {
        echo "If the installed build was made outside release.sh (e.g. '<name> <version> dev'), rerun with" >&2
        echo "RELEASE_ADOPT_UNIDENTIFIED=1 to keep a verified copy aside and release over it." >&2
        exit 1
      }
    fi
    echo "== snapshotting installed release $installed_commit"
    scripts/rollback.sh --snapshot "$installed_commit"
  fi
else
  echo "== first install: no prior executable or dashboard set to snapshot"
fi
# Lint, the test build and the release build run at once (0302). Lint and tests use target/dev, the
# cache the pre-commit hook and check.sh keep warm for this very tree (a separate stage cache made the
# release re-lint from cold: 73 s of a one-file release); the shipped binaries build in target/stage,
# never in target/release (LESSONS 9). Each job writes its own log; a failure prints its tail.
echo "== checking and building $commit (lint, tests and release in parallel)"
progress stage build
mkdir -p "$STAGE"
pids=() names=() logs=()
DEV=target/dev
mkdir -p "$DEV"
# Only the shipped build carries the commit id: cargo tracks SVANBOT_COMMIT, so exporting it to lint
# and the test build recompiled sv10-bot in both on every release (60 s of a one-file release).
# `scripts/tests/test_build_env.py` pins those call sites, so a job added here that builds into the
# gate's cache cannot quietly carry the commit (#329).
start_job() {
  local name=$1 dir=$2 commit_id=$3; shift 3
  (
    export CARGO_TARGET_DIR="$dir"
    if [ "$commit_id" = with-commit ]; then export SVANBOT_COMMIT="$commit"; else unset SVANBOT_COMMIT; fi
    started=$(date +%s)
    idle "$@"
    rc=$?
    echo $(($(date +%s) - started)) >"$STAGE/$name.secs"
    exit $rc
  ) >"$STAGE/$name.log" 2>&1 &
  pids+=($!) names+=("$name") logs+=("$STAGE/$name.log")
}
if [ "${SKIP_TESTS:-0}" != 1 ]; then
  start_job lint "$DEV" no-commit scripts/check.sh lint
  start_job test-build "$DEV" no-commit cargo test --no-run --profile gate --workspace -q
fi
# The job below shares cargo's build lock with anything else building in this target directory.
# Another cargo in this tree's `target/stage` — a benchmark, a one-off `--profile release` build —
# holds `release/.cargo-lock`, and the job then reports that wait as build time having compiled
# nothing at all: 498 s of the 2026-09-28 05:02 release, which no cache explains. Cargo names the
# wait itself in one line, into a log the next release truncates; say it here, in the release log
# the dashboard tails, so the wait is named while it is happening and not reconstructed afterwards.
scripts/build-lock.sh "$STAGE/release" || true
start_job release-build "$STAGE" with-commit cargo build --release --workspace --bins -q
failed=0
for i in "${!pids[@]}"; do
  if wait "${pids[$i]}"; then
    secs=$(cat "$STAGE/${names[$i]}.secs" 2>/dev/null || echo 0)
    echo "   ${names[$i]} ok ($secs s)"
    over_budget "${names[$i]}" "$secs"
    if [ "$secs" -gt "$BUDGET_SECS" ]; then
      # The next release truncates this job's log, and that log is where cargo's own `Blocking
      # waiting for file lock on build directory` lands — the one line that names a wait rather than
      # a slow compile. A stage over budget keeps its log, so the reason outlives the release that
      # suffered it (docs/LESSONS.md 46).
      cp "${logs[$i]}" "$STAGE/${names[$i]}.over-budget.log"
      echo "   ${names[$i]}: over budget, log kept at $STAGE/${names[$i]}.over-budget.log" >&2
    fi
  else
    echo "== ${names[$i]} failed (log: ${logs[$i]})"; tail -30 "${logs[$i]}"; failed=1
  fi
done
[ "$failed" = 0 ] || exit 1
if [ "${SKIP_TESTS:-0}" != 1 ]; then
  echo "== testing"
  progress stage test
  run_tests() (export CARGO_TARGET_DIR="$DEV"; unset SVANBOT_COMMIT; idle python3 scripts/test.py --profile gate -q)
  timed tests run_tests
fi
for b in sv10-bot learner analyst; do
  "$STAGE/release/$b" --version | grep -q "^$b " || { echo "$b --version failed; not installing" >&2; exit 1; }
done
echo "== dashboard"
progress stage dashboard
WEB_STAGE=$(mktemp -d "$PWD/target/.web-stage-${commit}.XXXXXX")
build_web() (cd web && npm run build -- --outDir "$WEB_STAGE" --emptyOutDir >/dev/null)
timed dashboard build_web
echo "== installing"
progress stage install
timed install scripts/rollback.sh --install "$STAGE/release" "$WEB_STAGE" "$commit"
took=$(($(date +%s) - release_t0))
# The new build is live from here: a failure in the bookkeeping below is not a failed release (issue
# #726). With `set -e` a full disk at this point reported "release failed" for a fleet already playing
# the new build, and update.sh moved a checkout the installed build had come from.
rm -rf -- "$WEB_STAGE" || echo "warning: could not remove the dashboard staging $WEB_STAGE" >&2
WEB_STAGE=
mkdir -p artifacts 2>/dev/null || echo "warning: could not create artifacts/" >&2
echo "$(date +%FT%T%:z) $commit $(git log -1 --format=%s "$commit" | cut -c1-120)" >> artifacts/releases.log ||
  echo "warning: could not append to artifacts/releases.log" >&2
echo "Installed $commit in $took s. The fleet swaps within ~1 minute, the learner after its current step (watch artifacts/logs/svanbot10.log for 'hot swap')."
over_budget release "$took"
if own_run; then progress installed "$commit"; fi
