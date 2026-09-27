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
# A checkout that shares no history with the branch at all is a different repository, and moves onto
# it only with SVANBOT_ADOPT_UPSTREAM=1 (see adopt_upstream below).
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
# Move a checkout that shares no history with the update branch onto it. This is the shape a fleet has
# when it was cloned from a private tree and is being moved onto the repository that tree publishes to:
# there is no common commit to build on, so no merge can leave `--ff-only` working afterwards — a merge
# commit would sit off the branch forever and every later update would refuse. The only way onto the
# branch is to put the checkout on it, which means the working tree becomes the branch's tree.
#
# Git will do that through its own fast-forward if it is told, for the length of one command, that the
# two histories are related: `git replace --graft` gives the branch's root commit this checkout's head
# as its parent, which makes that head an ancestor of the branch. The move then runs the ordinary
# `--ff-only` path — the one every other update runs — so its guards stay in force: git refuses rather
# than discards uncommitted edits to tracked files it would overwrite, where a `reset --hard` would have
# thrown them away. The replace ref is deleted as soon as the checkout is on the branch, so what is
# left behind is an ordinary clone whose history really is the branch's, and the next update is an
# ordinary fast-forward with no graft involved.
#
# Destructive by nature, so it happens only when an operator said so ahead of time, never on a button
# press alone. `SVANBOT_ADOPT_UPSTREAM=1` is that statement, and it is read here rather than passed on
# the command line because the dashboard starts this script.
adopt_upstream() {
  if [ "${SVANBOT_ADOPT_UPSTREAM:-}" != "1" ]; then
    echo "update: this checkout shares no history with $remote/$branch — a different repository, not a diverged one" >&2
    echo "update: set SVANBOT_ADOPT_UPSTREAM=1 to move this checkout onto $remote/$branch; see docs/OPERATIONS.md" >&2
    progress fail fetch "unrelated history: set SVANBOT_ADOPT_UPSTREAM=1 to move this checkout onto $remote/$branch"
    exit 1
  fi
  local kept dropped root
  kept="refs/adopt/before-upstream-$(date -u +%Y%m%dT%H%M%SZ)"
  # First, before anything moves: the old head is reachable from a ref of its own, so every commit this
  # checkout has — including the build that is installed — survives a move that would otherwise leave it
  # to the reflog. Rollback resolves commits in this repository, and this is what keeps that true. The
  # ref has to outlive the command: a replace ref does not count for reachability, so the graft below
  # cannot stand in for it — `git gc` would collect the commits the moment it went.
  git update-ref "$kept" "$before"
  root=$(git rev-list --max-parents=0 "$target" | tail -1)
  dropped=$(git diff --name-only --diff-filter=D "$before" "$target")
  echo "== adopting $remote/$branch: this checkout's history is unrelated to it"
  echo "kept this checkout's head $before as $kept"
  echo "tracked files here that $remote/$branch does not have: $(printf '%s' "$dropped" | grep -c . ) (all recoverable from $kept)"
  printf '%s\n' "$dropped" | head -20 | sed 's/^/  - /'
  if ! git replace --graft "$root" "$before"; then
    echo "update: could not graft $root onto $before; this checkout is unchanged" >&2
    progress fail fetch "adoption failed: could not relate the two histories"
    exit 1
  fi
  # The fast-forward — not a reset — moves the working tree, and only tracked files at that: `artifacts/`
  # (the store, the snapshots, the hourly backups, the logs) is ignored and is left exactly as it is.
  # That is the whole point of adopting in place rather than cloning: the runtime state stays where the
  # running fleet expects it, and the old head stays installed until the release below replaces it.
  if ! git merge --ff-only --quiet "$target"; then
    git replace -d "$root" || true
    echo "update: adopting $remote/$branch failed; this checkout is unchanged" >&2
    echo "update: commit, stash or discard the local changes git named above, then update again" >&2
    progress fail fetch "adoption refused: local changes would be overwritten"
    exit 1
  fi
  # The checkout is on the branch now, so the graft has done its work and the repository should not keep
  # a story about the two histories being one. Deleting it leaves a plain clone.
  git replace -d "$root" || echo "update: warning: left a replace ref on $root; history will read as joined" >&2
  echo "adopted $remote/$branch at $(git rev-parse --short "$target")"
}

if [ "$before" != "$target" ]; then
  if git merge-base --is-ancestor "$before" "$target"; then
    git merge --ff-only --quiet "$target"
    echo "fast-forwarded $(git rev-parse --short "$before") -> $(git rev-parse --short "$target") ($(git rev-list --count "$before..$target") commits)"
  elif git merge-base "$before" "$target" >/dev/null 2>&1; then
    echo "update: this checkout has commits that $remote/$branch does not contain; update by hand" >&2
    progress fail fetch "local commits not on $remote/$branch"
    exit 1
  else
    adopt_upstream
  fi
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
