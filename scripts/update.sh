#!/usr/bin/env bash
# One-click update (0236): fetch the update branch from GitHub, fast-forward this checkout, and run
# scripts/release.sh (build, test, install; the fleet hot-swaps between turns, play never stops).
#   scripts/update.sh            what the dashboard's Update button runs (detached)
#   scripts/update.sh --check    fetch only; print "<behind> <origin commit>" for the dashboard
#   scripts/update.sh --rollback <commit>
#                                restore a verified release snapshot (the dashboard's Roll back button,
#                                0239); the fleet hot-swaps to it like to any install
# Branch: SVANBOT_UPDATE_BRANCH (default main), remote: SVANBOT_UPDATE_REMOTE (default origin).
# Refuses a checkout with uncommitted build inputs, and one whose commits are on no remote at all.
# A checkout ahead of the update branch with every commit on a remote branch is not that: nothing is
# installed, nothing moves, and the run reports what is true rather than a failure (#394).
# A checkout that shares no history with the branch at all is a different repository, and moves onto
# it only with SVANBOT_ADOPT_UPSTREAM=1 (see adopt_upstream below).
# If the release fails, the checkout is moved back to the commit it was on, so the source on disk
# always matches what is installed. Progress: artifacts/release-progress.json (scripts/progress.py).
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH" GIT_TERMINAL_PROMPT=0
mkdir -p artifacts
update_started=0
update_exit() {
  local status=$?
  if [ "$update_started" = 1 ]; then
    if [ "$status" != 0 ]; then
      progress fail-running "" "update stopped unexpectedly (exit $status); see the release log"
    fi
    rm -f artifacts/release.lock
  fi
  return "$status"
}
trap update_exit EXIT
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
  update_started=1
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
  # Branch state rides along for the dashboard (issue #757): the parser reads the first two
  # fields, so older readers ignore the rest. NOTE: `cur`, not `branch` — that name is taken.
  cur=$(git branch --show-current 2>/dev/null || true); cur=${cur:-detached}
  echo "$(git rev-list --count "HEAD..$remote/$branch") $(git rev-parse --short "$remote/$branch") $cur $(git rev-list --count "$remote/$branch..HEAD")"
  exit 0
fi

: > artifacts/release.log
exec > >(tee -a artifacts/release.log) 2>&1
update_started=1
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

# This checkout is ahead of the update branch (#394). Two different situations wear that shape, and
# the guard below is about one of them:
#
#   - the commits are on a remote branch: the checkout is simply further along a line than the branch
#     the Update button follows — on another branch while `.env` still names that one, or already
#     ahead of the branch line it follows. Nothing is at risk and nothing needs installing: the branch
#     this install follows catches up on its own.
#   - the commits are on no remote at all: unfinished work, which a move onto the branch would leave
#     behind a branch tip. That is what "update by hand" was written for, and it still refuses.
#
# The two are told apart by asking the question directly. A checkout's own branch may not have been
# fetched since it last moved (the fleet fetches only the update branch), so when the commits look
# local and the checkout is on some other branch, that one branch is fetched once and asked again.
ahead_updates() {
  local ahead unshared current
  ahead=$(git rev-list --count "$target..$before")
  unshared=$(git rev-list --count "$target..$before" --not --remotes)
  current=$(git symbolic-ref --quiet --short HEAD || true)
  if [ "$unshared" != 0 ] && [ -n "$current" ] && [ "$current" != "$branch" ]; then
    timeout "${SVANBOT_UPDATE_FETCH_TIMEOUT:-120}" git fetch --quiet "$remote" "$current" || true
    unshared=$(git rev-list --count "$target..$before" --not --remotes)
  fi
  if [ "$unshared" != 0 ]; then
    echo "update: this checkout has commits that $remote/$branch does not contain; update by hand" >&2
    progress fail fetch "local commits not on $remote/$branch"
    exit 1
  fi
  echo "update: this checkout is $ahead commit(s) ahead of $remote/$branch; nothing to install (set SVANBOT_UPDATE_BRANCH=$current in .env to follow it here instead)"
  progress current "this checkout is $ahead commit(s) ahead of $remote/$branch; nothing to install (follow it with SVANBOT_UPDATE_BRANCH=$current)"
  exit 0
}

if [ "$before" != "$target" ]; then
  if git merge-base --is-ancestor "$before" "$target"; then
    if ! git merge --ff-only --quiet "$target"; then
      progress fail fetch "fast-forward refused; local edits are preserved, see the release log"
      exit 1
    fi
    echo "fast-forwarded $(git rev-parse --short "$before") -> $(git rev-parse --short "$target") ($(git rev-list --count "$before..$target") commits)"
  elif git merge-base "$before" "$target" >/dev/null 2>&1; then
    ahead_updates
  else
    adopt_upstream
  fi
fi
# What the health gate below needs, read before the install moves anything: which verified build is
# installed now, and whether a bot is actually running it. After a hot swap the pid files briefly name
# a process that has already exited, so the same question asked afterwards reads the fleet as down.
fleet_running=0
scripts/rollback.sh --fleet-running && fleet_running=1
previous_installed=$(scripts/rollback.sh --installed-commit 2>/dev/null || true)

# release.sh reports its own stages; a failure there moves the checkout back.
if ! SV10_UPDATE_RUN=1 "${SV10_RELEASE_SCRIPT:-scripts/release.sh}"; then
  if [ "$before" != "$target" ]; then
    git reset --quiet --keep "$before" && echo "update: release failed; checkout restored to $(git rev-parse --short "$before")"
  fi
  progress fail "" "release failed; the fleet keeps playing the installed build"
  exit 1
fi

# Installing the files is not the same as the fleet running them (issue #726). A binary that answers
# `--version` and dies at startup passes every files-only check, gets installed, and then crash-loops
# while keepalive counts supervisors and still says "fleet up". Wait, bounded, for /api/health to
# report the installed commit; anything else restores the previous verified build, loudly, instead of
# leaving play down. A box with no fleet running has nothing to verify and skips the gate.
if [ "$fleet_running" = 1 ]; then
  installed=$(scripts/rollback.sh --installed-commit 2>/dev/null || true)
  if [ -z "$installed" ]; then
    echo "update: warning: the release installed no commit identity; the fleet cannot be verified" >&2
  elif ! scripts/rollback.sh --await-health "$installed"; then
    if ! scripts/rollback.sh --fleet-running && ! scripts/rollback.sh --fleet-supervisors; then
      # The fleet stopped while the release ran (stop.sh, a service stop): nothing is left to verify
      # and nothing is broken, so keep the install instead of undoing a good update.
      echo "update: installed $installed, but the fleet stopped during the release; the install is unverified — start the fleet and check /api/health" >&2
    else
      rolled_back=0
      # Name only a previous commit that can actually be restored: a markerless or adopted install has
      # a commit in releases.log but no verified snapshot, and handing the operator that command would
      # fail the same way the automatic rollback just did.
      rollback_target=
      if [ -n "$previous_installed" ] && scripts/rollback.sh --verify "$previous_installed" >/dev/null 2>&1; then
        rollback_target=$previous_installed
      fi
      if [ -n "$rollback_target" ] && scripts/rollback.sh "$rollback_target"; then
        rolled_back=1
      fi
      if [ "$rolled_back" = 1 ]; then
        echo "update: rolled back to the verified build $rollback_target; the fleet keeps playing it" >&2
        progress fail install "the new build did not answer /api/health with its commit; rolled back to $rollback_target"
      elif [ -n "$rollback_target" ]; then
        echo "update: the automatic rollback to $rollback_target failed; the fleet may still be on $installed" >&2
        progress fail install "the new build did not answer /api/health and the rollback failed; run scripts/rollback.sh $rollback_target by hand"
      else
        echo "update: the new build did not answer /api/health and no verified rollback target exists; the fleet may still be on $installed" >&2
        progress fail install "the new build did not answer /api/health and no verified rollback target exists; fix the build, release again, or restore a snapshot by hand"
      fi
      if [ "$rolled_back" = 1 ] && [ "$before" != "$target" ]; then
        git reset --quiet --keep "$before" && echo "update: checkout restored to $(git rev-parse --short "$before"), so the source matches the build the fleet runs"
      fi
      exit 1
    fi
  fi
fi
progress installed "$(git rev-parse --short HEAD)"
