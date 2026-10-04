#!/usr/bin/env bash
# Bring a fleet's checkout onto this repository: the bootstrap for a deployment whose own
# `scripts/update.sh` predates the adoption path (issue #352).
#
#   scripts/adopt-upstream.sh --dir /path/to/fleet [--backup DIR] [--dry-run]
#
# Run from a checkout of this repository; its `origin` is what the target is moved onto. In order:
#
#   1. refuses unless the target is a git checkout with `artifacts/` and no release run in flight;
#   2. preserves the target into `--backup` (default beside it): every ref as a bundle, the working
#      tree's source as a tar (tracked and untracked files, ignored paths left out), the uncommitted
#      diff, `.env`, and an inventory of `artifacts/` — the runtime state itself is never copied,
#      because adopting in place never moves it;
#   3. reverts the target's uncommitted *tracked* edits, which would otherwise make git refuse the
#      move (untracked files are left alone), only after checking the backup landed;
#   4. re-points `origin` at this repository, keeping the old remote as `private`, so `update.sh`'s
#      default remote is the new one with no environment change;
#   5. runs the real adoption path — this checkout's `scripts/update.sh`, copied into the target under
#      an untracked name — which grafts the two histories together, fast-forwards, and then runs
#      `scripts/release.sh` (build, test, install; the fleet hot-swaps, play does not stop);
#   6. verifies the result with the commands `docs/OPERATIONS.md` lists, and reports where the backup
#      is.
#
# Nothing here is a second copy of update.sh's adoption logic: the graft, its guards and the ref it
# keeps are the script's own, and this only sets the stage it cannot set for itself. The untracked name
# the copy runs under is load-bearing — a modified *tracked* path would make git refuse its own
# fast-forward.
set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd -P)
stamp=$(date -u +%Y%m%dT%H%M%SZ)
branch=${SVANBOT_UPDATE_BRANCH:-main}

die() { echo "adopt: $*" >&2; exit 1; }
say() { echo "== $*"; }
usage() { echo "usage: scripts/adopt-upstream.sh --dir <checkout> [--backup <dir>] [--dry-run]" >&2; }

dir= backup= dry=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dir) [ $# -ge 2 ] || { usage; exit 2; }; dir=$2; shift 2 ;;
    --backup) [ $# -ge 2 ] || { usage; exit 2; }; backup=$2; shift 2 ;;
    --dry-run) dry=1; shift ;;
    --help | -h) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
done
[ -n "$dir" ] || { usage; exit 2; }
[ -d "$dir" ] || die "no such directory: $dir"
dir=$(cd "$dir" && pwd -P)
[ "$dir" != "$here" ] || die "--dir is this checkout; adoption moves *another* deployment onto it"
[ -e "$dir/.git" ] || die "$dir is not a git checkout"
[ -d "$dir/artifacts" ] || die "$dir has no artifacts/ — point --dir at the deployment's own checkout"
if [ -f "$dir/artifacts/release.lock" ]; then
  die "a release is running ($dir/artifacts/release.lock); wait for it to finish, then run this again"
fi
git -C "$dir" rev-parse --verify --quiet HEAD >/dev/null || die "$dir has no commits"
before=$(git -C "$dir" rev-parse HEAD)
src=$(git -C "$here" remote get-url origin 2>/dev/null) || die "this checkout ($here) has no origin to adopt onto"

# Where the target is going, and whether it is already there. `ls-remote` is the whole proof the
# credentials and the network work, and it writes nothing; if the tip is already in the target's object
# store the move is a plain fast-forward, which is update.sh's job, not this script's.
tip=$(git -C "$dir" ls-remote "$src" "$branch" 2>/dev/null | awk 'NR == 1 { print $1 }' || true)
[ -n "$tip" ] || die "could not read $branch from $src (network, or credentials for it)"
if git -C "$dir" cat-file -e "$tip" 2>/dev/null && git -C "$dir" merge-base --is-ancestor "$before" "$tip"; then
  say "$dir is already on $src/$branch (its HEAD is an ancestor of $tip)"
  say "nothing to adopt: use scripts/update.sh in $dir for a normal update"
  exit 0
fi

backup=${backup:-$(dirname "$dir")/adopt-backup-$stamp}
say "target    $dir at $(git -C "$dir" rev-parse --short HEAD)"
say "source    $src $branch ($(git -C "$dir" rev-parse --short "$tip" 2>/dev/null || echo "${tip:0:7}"), not in the target's history)"
say "backup    $backup"
if [ "$dry" = 1 ]; then
  say "the target's uncommitted tracked edits would be preserved there and then reverted:"
  git -C "$dir" status --porcelain --untracked-files=no | sed 's/^/   /'
  say "dry run: nothing was fetched, written or moved"
  exit 0
fi

# 2. Preserve. Every ref the target has, the source it is built from, and the settings beside it; the
# runtime state is not copied because nothing below touches it.
say "preserving"
mkdir -p "$backup"
chmod 700 "$backup"
git -C "$dir" bundle create "$backup/repo.bundle" --all >/dev/null 2>&1
git -C "$dir" diff HEAD --binary >"$backup/uncommitted.patch"
git -C "$dir" ls-files --cached --others --exclude-standard -z |
  tar -C "$dir" --null -T - --ignore-failed-read -czf "$backup/worktree.tar.gz"
if [ -f "$dir/.env" ]; then cp -p "$dir/.env" "$backup/env"; fi
git -C "$dir" status --porcelain >"$backup/uncommitted-files.txt"
{
  echo "target       $dir"
  echo "head         $before ($(git -C "$dir" log -1 --format=%s))"
  echo "source       $src $branch $tip"
  echo "installed    $(cat "$dir/target/release/.sv10-installed-commit" 2>/dev/null || echo none)"
  git -C "$dir" remote -v | sed 's/^/remote       /'
} >"$backup/pre-state.txt"
inventory() { find "$1" -maxdepth 1 -printf '%y %f\n' | sort; }
snapshots() { find "$1/artifacts/release-snapshots" -mindepth 1 -printf '%y %P\n' 2>/dev/null | sort || true; }
inventory "$dir/artifacts" >"$backup/artifacts-before.txt"
snapshots "$dir" >"$backup/snapshots-before.txt"
du -sh "$dir/artifacts" >"$backup/artifacts-facts.txt"
for f in repo.bundle worktree.tar.gz; do
  [ -s "$backup/$f" ] || die "the backup did not produce $backup/$f; nothing was changed"
done

# 3. Make the tracked tree match its head, so the fast-forward has nothing to refuse. The untracked
# files stay: the merge only refuses when one of them sits where the branch has a file, and it names
# that file rather than discarding it.
dirty=$(git -C "$dir" status --porcelain --untracked-files=no | wc -l)
if [ "$dirty" != 0 ]; then
  say "reverting $dirty uncommitted tracked path(s); the diff and the working tree are in the backup"
  git -C "$dir" reset --quiet --hard HEAD
fi

# 4. Point the default remote at this repository without losing the old one.
if [ "$(git -C "$dir" remote get-url origin 2>/dev/null || true)" = "$src" ]; then
  say "origin already points at $src"
else
  keep=private
  if git -C "$dir" remote | grep -qx "$keep"; then keep="private-$stamp"; fi
  git -C "$dir" remote rename origin "$keep"
  git -C "$dir" remote add origin "$src"
  say "origin -> $src (the previous remote is kept as $keep)"
fi

# 5. The real adoption path, run from an untracked copy inside the target so that `$(dirname $0)/..` is
# the deployment and not this checkout.
# The copy is a wrapper around the Rust installer, and the target has no Rust sources of the right history
# yet: hand it this checkout's installer.
if [ -z "${SV10_RELEASE_BIN:-}" ]; then
  SV10_RELEASE_BIN=$(cd "$here" && source scripts/sv10-release-bin.sh && sv10_release_bin) || die "could not build the installer in $here"
fi
export SV10_RELEASE_BIN
tmp="scripts/.adopt-upstream-$stamp.sh"
cp "$here/scripts/update.sh" "$dir/$tmp"
say "adopting: update.sh's own SVANBOT_ADOPT_UPSTREAM path, then release.sh (build, test, install)"
status=0
(cd "$dir" && SVANBOT_ADOPT_UPSTREAM=1 bash "$tmp") || status=$?
rm -f "$dir/$tmp"
if [ "$status" != 0 ]; then
  die "adoption failed (exit $status): read $dir/artifacts/release.log; the backup of the source is $backup"
fi

# 6. Verify with the commands docs/OPERATIONS.md lists, and compare the runtime state's file set: the
# fleet is writing to those files the whole time, so sizes move, but nothing may go missing.
say "verifying"
find "$dir/artifacts" -maxdepth 1 -printf '%y %10s %f\n' | sort >"$backup/artifacts-after.txt"
comm -23 <(awk '{ print $NF }' "$backup/artifacts-before.txt" | sort) <(awk '{ print $NF }' "$backup/artifacts-after.txt" | sort) | grep -v '^release-snapshots:$' >"$backup/artifacts-missing.txt" || true
git -C "$dir" remote -v | sed 's/^/   /'
git -C "$dir" log --oneline -1 | sed 's/^/   /'
echo "   refs/adopt:   $(git -C "$dir" for-each-ref --format='%(refname) %(objectname:short)' refs/adopt | tr '\n' ' ')"
echo "   refs/replace: $(git -C "$dir" for-each-ref --format='%(refname)' refs/replace | wc -l) ref(s)"
if [ -n "$(git -C "$dir" status --porcelain)" ]; then
  say "the tree is not clean: the lines below are files the branch does not have"
  git -C "$dir" status --porcelain | sed 's/^/   /'
fi
# release.sh records the short hash, so both sides are resolved to a full one before comparing.
installed=$(cat "$dir/target/release/.sv10-installed-commit" 2>/dev/null || echo none)
head=$(git -C "$dir" rev-parse HEAD)
if [ "$(git -C "$dir" rev-parse --verify --quiet "$installed^{commit}" || true)" != "$head" ]; then
  say "installed build is $installed, the checkout is on ${head:0:7} — the release did not install"
fi
# A file the old head tracked under artifacts/ and the branch does not is removed by the move itself,
# like any other tracked file the branch lacks; only an entry git never owned is a real loss.
git -C "$dir" ls-tree --name-only "$before" -- artifacts/ | sed 's|^artifacts/||' | sort >"$backup/artifacts-tracked.txt"
if [ -s "$backup/artifacts-missing.txt" ]; then
  if grep -qxFf "$backup/artifacts-tracked.txt" "$backup/artifacts-missing.txt"; then
    say "artifacts/ files the old head tracked and the branch does not (recoverable from refs/adopt/):"
    grep -xFf "$backup/artifacts-tracked.txt" "$backup/artifacts-missing.txt" | sed 's/^/   /'
  fi
  if grep -qvxFf "$backup/artifacts-tracked.txt" "$backup/artifacts-missing.txt"; then
    say "artifacts/ lost these untracked entries during the move, which never touches them — check by hand:"
    grep -vxFf "$backup/artifacts-tracked.txt" "$backup/artifacts-missing.txt" | sed 's/^/   /'
  fi
else
  say "artifacts/ intact: the same $(wc -l <"$backup/artifacts-after.txt") top-level entries as before the move"
fi
say "delete the backup only once the fleet has run a while on the new source: $backup"
