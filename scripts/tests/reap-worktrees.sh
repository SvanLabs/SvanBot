#!/usr/bin/env bash
# reap-worktrees.sh classifies worktrees and removes nothing (#763). A throwaway repository; no live
# worktree, branch or session is read or touched.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
fail() { echo "reap-worktrees: $*" >&2; exit 1; }
cd "$root"
git init -q -b main main && cd main
git config user.email t@t && git config user.name t
echo a > f && git add f && git commit -qm base
git worktree add -q ../merged -b merged                # same commit as main: merged once idle for a day
git worktree add -q ../fresh -b fresh                  # same commit, created just now: active
git worktree add -q ../unmerged -b unmerged && (cd ../unmerged && echo b > g && git add g && git commit -qm work)
git worktree add -q ../gone -b gone && rm -rf ../gone  # directory deleted behind git's back
git worktree add -q ../dirty -b dirty && echo x > ../dirty/untracked
git worktree add -q --lock ../locked -b locked
# Age the worktrees that should look idle.
old=$(( $(date +%s) - 20 * 86400 ))
for w in merged unmerged dirty locked; do
  touch -d "@$old" "../$w" "../$w/.git" "$(git -C "../$w" rev-parse --git-dir)/index" "$(git -C "../$w" rev-parse --git-dir)/HEAD" 2> /dev/null || true
done
before=$(git worktree list | wc -l)
out=$(REAP_ROOT="$root/main" REAP_BASE=main bash "$repo/scripts/reap-worktrees.sh" --no-size)
verdict() { grep -E "^[a-z]+ +-? *[0-9-]+d? +$1 " <<< "$out" | awk '{print $1}'; }
[ "$(verdict merged)" = merged ] || fail "merged branch not reported merged: $out"
[ "$(verdict fresh)" = active ] || fail "a new worktree on main was not active: $out"
[ "$(verdict unmerged)" = stale ] || fail "idle unmerged branch not stale: $out"
[ "$(verdict gone)" = prunable ] || fail "missing directory not prunable: $out"
[ "$(verdict dirty)" = dirty ] || fail "uncommitted changes not reported: $out"
[ "$(verdict locked)" = live ] || fail "a locked worktree was not live: $out"
grep -q 'git worktree remove .*merged' <<< "$out" || fail "no remove command for the merged worktree"
! grep -q 'git worktree remove .*locked' <<< "$out" || fail "offered to remove a locked worktree"
[ "$(git worktree list | wc -l)" = "$before" ] || fail "the report changed the worktree list"
[ -d ../merged ] && [ -d ../unmerged ] || fail "the report removed a directory"
echo "reap-worktrees tests passed"
