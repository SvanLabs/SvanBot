#!/usr/bin/env bash
# Tests for scripts/adopt-upstream.sh (#352), hermetic: a bare "public" origin, a developer clone of it
# that runs the bootstrap, and a deployment cloned from an unrelated "private" tree whose update.sh
# predates adoption, with runtime state, a .env and uncommitted edits.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
source "$repo/scripts/tests/lib-release-bin.sh"
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail() { echo "adopt-upstream test: $*" >&2; exit 1; }
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
unset SV10_RELEASE_LOCK_FD SV10_RELEASE_ROOT SV10_UPDATE_RUN SV10_PROGRESS_DIR SVANBOT_ADOPT_UPSTREAM SVANBOT_UPDATE_BRANCH

# The installer path the helper prints is used from another checkout, so it has to be absolute (#875).
# An executable newer than every source stands in for the built installer: nothing is compiled.
mkdir -p "$t/bin/target/dev/release" && : > "$t/bin/target/dev/release/sv10-release" && chmod +x "$t/bin/target/dev/release/sv10-release"
printed=$(cd "$t/bin" && unset SV10_RELEASE_BIN && source "$repo/scripts/sv10-release-bin.sh" && sv10_release_bin)
[ "$printed" = "$t/bin/target/dev/release/sv10-release" ] || fail "the installer path is not absolute: $printed"

scripts="update.sh progress.py rollback.sh adopt-upstream.sh"
git init -q --bare --initial-branch=main "$t/public.git"
git clone -q "$t/public.git" "$t/dev" 2>/dev/null
mkdir -p "$t/dev/scripts" "$t/dev/crates"
for s in $scripts; do cp "$repo/scripts/$s" "$t/dev/scripts/"; done
echo 'fn public() {}' > "$t/dev/crates/a.rs"
printf 'artifacts/\n.env\n' > "$t/dev/.gitignore"
git -C "$t/dev" add -A && git -C "$t/dev" commit -qm public && git -C "$t/dev" -c push.negotiate=false push -q origin HEAD:main

# The private tree: its own history, an update.sh that knows nothing of adoption.
git init -q --bare --initial-branch=main "$t/private.git"
git clone -q "$t/private.git" "$t/spun" 2>/dev/null
mkdir -p "$t/spun/scripts" "$t/spun/crates"
cp "$repo/scripts/progress.py" "$repo/scripts/rollback.sh" "$t/spun/scripts/"
echo 'echo "old update.sh: cannot adopt" >&2; exit 1' > "$t/spun/scripts/update.sh"
echo 'fn private() {}' > "$t/spun/crates/a.rs"
printf 'artifacts/\n.env\n' > "$t/spun/.gitignore"
mkdir "$t/spun/artifacts" && echo 'what lives here' > "$t/spun/artifacts/README.md"
git -C "$t/spun" add -A && git -C "$t/spun" add -f artifacts/README.md && git -C "$t/spun" commit -qm private
git -C "$t/spun" branch side && git -C "$t/spun" -c push.negotiate=false push -q origin main side

mkdir "$t/fleet"
git clone -q "$t/private.git" "$t/fleet/box"
box="$t/fleet/box"
private_head=$(git -C "$box" rev-parse HEAD)
mkdir -p "$box/artifacts" && echo '{"live":true}' > "$box/artifacts/live-state.json"
echo 'SECRET=1' > "$box/.env" && chmod 600 "$box/.env"
echo 'fn unscrubbed() {}' >> "$box/crates/a.rs"
echo 'scratch' > "$box/untracked-notes.txt"
sum() { sha256sum "$1" | awk '{ print $1 }'; }
env_sum=$(sum "$box/.env") live_sum=$(sum "$box/artifacts/live-state.json")

cat > "$t/release-ok.sh" <<'EOF'
#!/usr/bin/env bash
set -e
python3 scripts/progress.py stage install
mkdir -p target/release && git rev-parse --short HEAD > target/release/.sv10-installed-commit
EOF
chmod +x "$t/release-ok.sh"
adopt() { (cd "$t/dev" && SV10_RELEASE_SCRIPT="$t/release-ok.sh" bash scripts/adopt-upstream.sh --dir "$box" "$@"); }

# 1. Refusals before anything is read: this checkout itself, a directory with no artifacts/, a
# release in flight.
if (cd "$t/dev" && bash scripts/adopt-upstream.sh --dir "$t/dev" >/dev/null 2>&1); then fail "adopted its own checkout"; fi
mkdir "$t/plain" && git init -q "$t/plain"
if (cd "$t/dev" && bash scripts/adopt-upstream.sh --dir "$t/plain" >/dev/null 2>&1); then fail "adopted a directory without artifacts/"; fi
echo '{}' > "$box/artifacts/release.lock"
if adopt >/dev/null 2>&1; then fail "adopted while a release was running"; fi
rm "$box/artifacts/release.lock"

# 2. A dry run names the edits it would revert and changes nothing.
out=$(adopt --dry-run --backup "$t/dry-backup") || fail "dry run failed"
grep -q "crates/a.rs" <<<"$out" || fail "dry run did not list the uncommitted edit"
[ ! -e "$t/dry-backup" ] || fail "dry run wrote a backup"
[ "$(git -C "$box" rev-parse HEAD)" = "$private_head" ] || fail "dry run moved the checkout"
grep -q unscrubbed "$box/crates/a.rs" || fail "dry run reverted an edit"
[ "$(git -C "$box" remote get-url origin)" = "$t/private.git" ] || fail "dry run re-pointed origin"

# 3. The real run: onto the public branch, runtime state and .env untouched, old remote kept.
out=$(adopt --backup "$t/backup") || fail "adoption failed"
[ "$(git -C "$box" rev-parse HEAD)" = "$(git -C "$t/dev" rev-parse HEAD)" ] || fail "did not reach the public branch"
[ "$(sum "$box/.env")" = "$env_sum" ] || fail ".env changed"
[ "$(sum "$box/artifacts/live-state.json")" = "$live_sum" ] || fail "artifacts/ changed"
[ "$(git -C "$box" remote get-url origin)" = "$t/public.git" ] || fail "origin not re-pointed"
[ "$(git -C "$box" remote get-url private)" = "$t/private.git" ] || fail "the old remote was not kept as private"
[ -n "$(git -C "$box" for-each-ref --format='%(refname)' refs/adopt)" ] || fail "no refs/adopt/* kept"
[ -z "$(git -C "$box" for-each-ref --format='%(refname)' refs/replace)" ] || fail "a graft was left behind"
[ -z "$(git -C "$box" status --porcelain --untracked-files=no)" ] || fail "tracked tree not clean"
[ -f "$box/untracked-notes.txt" ] || fail "an untracked file was removed"
[ -z "$(find "$box/scripts" -name '.adopt-upstream-*')" ] || fail "the temporary update.sh was left behind"
[ "$(cat "$box/target/release/.sv10-installed-commit")" = "$(git -C "$box" rev-parse --short HEAD)" ] || fail "release did not run"
! grep -q "did not install" <<<"$out" || fail "a short installed hash was reported as a failed install"
grep -A1 "the old head tracked" <<<"$out" | grep -q README.md || fail "a tracked artifacts/ file the move removed was not reported as such"
! grep -q "lost these untracked" <<<"$out" || fail "a tracked artifacts/ file was reported as lost: $out"

# 4. The backup restores what the move replaced: every ref, the edit, the .env.
git clone -q "$t/backup/repo.bundle" "$t/restored" 2>/dev/null || fail "bundle does not clone"
git -C "$t/restored" cat-file -e "$private_head" || fail "bundle lacks the private head"
git bundle list-heads "$t/backup/repo.bundle" | grep -q 'refs/remotes/origin/side' || fail "bundle lacks a remote branch"
grep -q unscrubbed "$t/backup/uncommitted.patch" || fail "the uncommitted edit is not in the backup"
tar -xzf "$t/backup/worktree.tar.gz" -O crates/a.rs | grep -q unscrubbed || fail "worktree tar lacks the edited file"
[ "$(sum "$t/backup/env")" = "$env_sum" ] || fail ".env not backed up"
[ "$(stat -c %a "$t/backup")" = 700 ] || fail "backup directory is not private"
[ "$(cat "$t/backup/artifacts-missing.txt")" = README.md ] || fail "missing artifacts: $(cat "$t/backup/artifacts-missing.txt")"

# 5. A second run has nothing to adopt and says so, changing nothing.
head=$(git -C "$box" rev-parse HEAD)
out=$(adopt --backup "$t/backup2") || fail "second run failed"
grep -q "nothing to adopt" <<<"$out" || fail "second run did not say nothing to adopt"
[ ! -e "$t/backup2" ] && [ "$(git -C "$box" rev-parse HEAD)" = "$head" ] || fail "second run changed something"

echo "adopt-upstream tests: ok"
