#!/usr/bin/env bash
# Tests for scripts/patch-inbox.sh (design: issue #710). The whole landing flow runs against a
# temp bare origin and a clone ("box") holding the applier plus stub helpers (check.sh,
# build-lock.sh, provenance.py, gh), so nothing here touches the network, cargo or the real gate.
# Run: bash scripts/tests/patch-inbox.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
applier=$here/../patch-inbox.sh
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fails=0
check() { if ! "$@"; then echo "FAIL: $*" >&2; fails=1; fi; }

# --- fixture: bare origin + a box clone with the applier and stubs ---------------------------
git init -q --bare "$work/origin.git"
git clone -q "$work/origin.git" "$work/box"
cd "$work/box"
git config user.email t@example.invalid
git config user.name tester
mkdir -p scripts artifacts .scratch .github "$work/bin"
cp "$applier" scripts/patch-inbox.sh
printf '.scratch/\nartifacts/\n' > .gitignore
cat > scripts/check.sh <<'SH'
#!/usr/bin/env bash
# Stub gate: fast red on the marker word, green otherwise.
set -euo pipefail
if grep -rqwE "TO""DO" . --include='*.txt' 2>/dev/null; then echo "stub gate: marker found"; exit 1; fi
echo "stub gate ok"
SH
cat > scripts/build-lock.sh <<'SH'
#!/usr/bin/env bash
exit 0
SH
cat > scripts/provenance.py <<'PY'
import re, subprocess, sys
if sys.argv[1] == "check":
    out = subprocess.check_output(["git", "log", "--format=%B", sys.argv[2]])
    if not re.search(rb"^Generated-by: ", out, re.M):
        sys.exit("stub provenance: no trailer")
PY
printf '## What generated this?\n\nGenerated-by: <tool>/<model>\n\n## Motivation\n\nCloses #\n\n## Solution\n\n## Checklist\n' > .github/pull_request_template.md
cat > "$work/bin/gh" <<'SH'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >> "${GH_LOG:?}"
if [ "$1 $2" = "pr create" ]; then echo "https://example.invalid/pr/1"; fi
SH
chmod +x "$work/bin/gh" scripts/check.sh scripts/build-lock.sh
export PATH="$work/bin:$PATH" GH_LOG="$work/gh.log"
echo hi > README.txt
git add -A && git commit -qm "init" && git branch -M main && git push -q origin main

# mail <entry> <file-slug> <subject> [marker]: one commit on top of main as a mailbox file.
mail() {
  local entry=$1 slug=$2 subject=$3 marker=${4:-} issue=${1%%-*}
  mkdir -p "artifacts/patches/$entry"
  git checkout -q -B patchwork main
  echo "content $entry" >> "$slug.txt"
  [ -z "$marker" ] || echo "TO""DO" >> "$slug.txt"
  git add -A
  git commit -qm "$subject" -m "Closes #$issue" -m "Generated-by: test/harness"
  git format-patch -1 -q -o "artifacts/patches/$entry" >/dev/null
  git checkout -q main
}

# 1. empty tray
out=$(bash scripts/patch-inbox.sh list)
check grep -qE "tray empty|no tray" <<<"$out"

# 2. shape reject: bad subject
mail 1-bad badtxt "no area prefix"
bash scripts/patch-inbox.sh land 1-bad > "$work/out2"
check grep -q "rejected 1-bad" "$work/out2"
check test ! -d artifacts/patches/1-bad
check grep -q "rejected 1-bad" artifacts/patches/patch-inbox.log
check grep -q "issue comment 1" "$work/gh.log"

# 3. stale reject: origin/main moves under the patch
mail 3-stale README "docs: add a stale note"
echo moved > README.txt && git add -A && git commit -qm "docs: move the base" && git push -q origin main
bash scripts/patch-inbox.sh land 3-stale > "$work/out3" 2>/dev/null
check grep -q "rejected 3-stale" "$work/out3"
check grep -q "main moved under the patch" artifacts/patches/patch-inbox.log
check test ! -d artifacts/patches/3-stale

# 4. dry-run changes nothing
mail 4-dry drytxt "docs: a dry run note"
bash scripts/patch-inbox.sh land 4-dry --dry-run > "$work/out4"
check grep -q "dry-run 4-dry" "$work/out4"
check test -d artifacts/patches/4-dry
check test -z "$(git ls-remote origin 'docs/4-dry' 2>/dev/null)" || true
rm -rf artifacts/patches/4-dry

# 5. red gate keeps the entry with gate.log
mail 5-red redtxt "docs: a marked note" marker
if bash scripts/patch-inbox.sh land 5-red > "$work/out5" 2>&1; then echo "FAIL: red gate must exit nonzero" >&2; fails=1; fi
check grep -q "gate red for 5-red" "$work/out5"
check test -f artifacts/patches/5-red/gate.log
check grep -q "stub gate: marker found" artifacts/patches/5-red/gate.log
rm -rf artifacts/patches/5-red

# 6. lock refusal
mail 6-lock locktxt "docs: a locked note"
( flock -n 8; sleep 3 ) 8>artifacts/patches/.landing.lock &
locker=$!
sleep 0.3
if bash scripts/patch-inbox.sh land 6-lock > "$work/out6" 2>&1; then echo "FAIL: lock holder must refuse" >&2; fails=1; fi
check grep -q "another landing run holds the lock" "$work/out6"
wait "$locker" || true
rm -rf artifacts/patches/6-lock

# 7. success: lands, branches, opens the PR, deletes the entry
mail 7-clean cleantxt "docs: a clean note"
bash scripts/patch-inbox.sh land 7-clean > "$work/out7"
check grep -q "landed 7-clean: https://example.invalid/pr/1" "$work/out7"
check test ! -d artifacts/patches/7-clean
check test -n "$(git ls-remote origin 'docs/7-clean' 2>/dev/null)"
check grep -q "landed 7-clean" artifacts/patches/patch-inbox.log

# 8. --all lands every pending entry
mail 8-a atxt "docs: first"
mail 8-b btxt "docs: second"
bash scripts/patch-inbox.sh land --all > "$work/out8"
check grep -q "landed 8-a" "$work/out8"
check grep -q "landed 8-b" "$work/out8"
check test -z "$(ls -A artifacts/patches/ | grep -vE 'patch-inbox\.log|\.landing\.lock' || true)"

[ "$fails" = 0 ] || { echo "patch-inbox tests: failures above" >&2; exit 1; }
echo "patch-inbox tests: ok"
