#!/usr/bin/env bash
# One-click update (0236): fetch the update branch from GitHub, fast-forward this checkout, and run
# scripts/release.sh (build, test, install; the fleet hot-swaps between turns, play never stops).
#   scripts/update.sh            what the dashboard's Update button runs (detached)
#   scripts/update.sh --check    fetch only; print "<behind> <origin commit>" for the dashboard
#   scripts/update.sh --rollback <commit>
#                                restore a verified release snapshot (the dashboard's Roll back button,
#                                0239); the fleet hot-swaps to it like to any install
# Branch: SVANBOT_UPDATE_BRANCH (default main), remote: SVANBOT_UPDATE_REMOTE (default origin).
# Refuses a checkout with uncommitted build inputs or local commits the branch does not contain.
# If the release fails, the checkout is moved back to the commit it was on, so the source on disk
# always matches what is installed. Progress: artifacts/release-progress.json (scripts/progress.py).
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH" GIT_TERMINAL_PROMPT=0
mkdir -p artifacts
# The dashboard wrote artifacts/release.lock before starting us; remove it however we exit.
trap 'rm -f artifacts/release.lock' EXIT
remote="${SVANBOT_UPDATE_REMOTE:-origin}"
branch="${SVANBOT_UPDATE_BRANCH:-main}"
progress() { python3 scripts/progress.py "$@" || true; }

fetch() {
  timeout "${SVANBOT_UPDATE_FETCH_TIMEOUT:-120}" git fetch --quiet "$remote" "$branch" || {
    echo "update: could not fetch $remote/$branch (network or credentials); nothing changed" >&2
    return 1
  }
}

if [ "${1:-}" = "--rollback" ]; then
  commit="${2:-}"
  [[ "$commit" =~ ^[0-9a-f]{7,40}$ ]] || { echo "update: usage: scripts/update.sh --rollback <commit>" >&2; exit 2; }
  : > artifacts/release.log
  exec > >(tee -a artifacts/release.log) 2>&1
  progress start
  echo "== restoring the saved build $commit"
  progress stage restore
  if ! scripts/rollback.sh "$commit"; then
    progress fail restore "rollback to $commit failed; the installed build is unchanged"
    exit 1
  fi
  progress installed "$commit"
  exit 0
fi

if [ "${1:-}" = "--check" ]; then
  fetch
  echo "$(git rev-list --count "HEAD..$remote/$branch") $(git rev-parse --short "$remote/$branch")"
  exit 0
fi

: > artifacts/release.log
exec > >(tee -a artifacts/release.log) 2>&1
progress start
echo "== fetching $remote/$branch"
progress stage fetch
fetch || { progress fail fetch "could not fetch $remote/$branch"; exit 1; }
before=$(git rev-parse HEAD)
target=$(git rev-parse "$remote/$branch")
if ! scripts/rollback.sh --validate-source-clean; then
  progress fail fetch "uncommitted build inputs in the checkout; commit or discard them first"
  exit 1
fi
if [ "$before" != "$target" ]; then
  if ! git merge-base --is-ancestor "$before" "$target"; then
    echo "update: this checkout has commits that $remote/$branch does not contain; update by hand" >&2
    progress fail fetch "local commits not on $remote/$branch"
    exit 1
  fi
  git merge --ff-only --quiet "$target"
  echo "fast-forwarded $(git rev-parse --short "$before") -> $(git rev-parse --short "$target") ($(git rev-list --count "$before..$target") commits)"
fi
# release.sh reports its own stages; a failure there moves the checkout back.
if ! SV10_UPDATE_RUN=1 "${SV10_RELEASE_SCRIPT:-scripts/release.sh}"; then
  if [ "$before" != "$target" ]; then
    git reset --quiet --keep "$before" && echo "update: release failed; checkout restored to $(git rev-parse --short "$before")"
  fi
  progress fail "" "release failed; the fleet keeps playing the installed build"
  exit 1
fi
progress installed "$(git rev-parse --short HEAD)"
