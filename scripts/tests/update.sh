#!/usr/bin/env bash
# Tests for scripts/update.sh and scripts/progress.py (0236), hermetic: a bare "GitHub" origin, a
# developer clone that pushes, and an operator checkout that updates with a stub release script.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail() { echo "update test: $*" >&2; exit 1; }
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
unset SV10_RELEASE_LOCK_FD SV10_RELEASE_ROOT SV10_UPDATE_RUN SV10_PROGRESS_DIR

git init -q --bare --initial-branch=main "$t/origin.git"
git clone -q "$t/origin.git" "$t/dev" 2>/dev/null
mkdir -p "$t/dev/scripts" "$t/dev/crates"
cp "$repo/scripts/update.sh" "$repo/scripts/progress.py" "$repo/scripts/rollback.sh" "$t/dev/scripts/"
echo 'fn a() {}' > "$t/dev/crates/a.rs"
printf 'artifacts/\n' > "$t/dev/.gitignore"
git -C "$t/dev" add -A && git -C "$t/dev" commit -qm "first" && git -C "$t/dev" -c push.negotiate=false push -q origin HEAD:main
git clone -q "$t/origin.git" "$t/box"
box="$t/box"

# Stub releases: report stages like release.sh does, then succeed or fail.
cat > "$t/release-ok.sh" <<'EOF'
#!/usr/bin/env bash
set -e
for s in snapshot lint test build dashboard install; do python3 scripts/progress.py stage "$s"; done
echo "== stub release $(git rev-parse --short HEAD)"
EOF
cat > "$t/release-fail.sh" <<'EOF'
#!/usr/bin/env bash
python3 scripts/progress.py stage snapshot
python3 scripts/progress.py stage test
echo "stub: a test failed" >&2
exit 1
EOF
chmod +x "$t/release-ok.sh" "$t/release-fail.sh"
run() { (cd "$box" && SV10_RELEASE_SCRIPT="$1" bash scripts/update.sh "${@:2}"); }
state() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["state"], d.get("commit") or "-", ",".join(s["name"]+":"+s["state"] for s in d["stages"]))' "$box/artifacts/release-progress.json"; }
push() { echo "fn $1() {}" > "$t/dev/crates/$1.rs"; git -C "$t/dev" add -A; git -C "$t/dev" commit -qm "$1"; git -C "$t/dev" -c push.negotiate=false push -q origin HEAD:main; }

# 1. Up to date: the release still runs (it installs what is checked out) and the lock goes.
mkdir -p "$box/artifacts" && echo '{}' > "$box/artifacts/release.lock"
run "$t/release-ok.sh" >/dev/null || fail "up-to-date update failed"
[ ! -e "$box/artifacts/release.lock" ] || fail "release.lock left behind"
[[ "$(state)" == "installed $(git -C "$box" rev-parse --short HEAD) fetch:done,snapshot:done,lint:done,test:done,build:done,dashboard:done,install:done" ]] || fail "progress after an up-to-date run: $(state)"
[ -s "$box/artifacts/release-timings.json" ] || fail "no timings written"

# 2. New commits on the branch: fast-forward, release, installed commit is the new head.
push b && push c
[ "$(cd "$box" && bash scripts/update.sh --check)" = "2 $(git -C "$t/dev" rev-parse --short HEAD)" ] || fail "--check did not report 2 behind"
run "$t/release-ok.sh" >/dev/null || fail "fast-forward update failed"
[ "$(git -C "$box" rev-parse HEAD)" = "$(git -C "$t/dev" rev-parse HEAD)" ] || fail "checkout not fast-forwarded"
grep -q "fast-forwarded" "$box/artifacts/release.log" || fail "release.log lacks the fast-forward line"
grep -q "== stub release" "$box/artifacts/release.log" || fail "release.log lacks the release output"
[[ "$(state)" == installed* ]] || fail "progress: $(state)"

# 3. A failing release moves the checkout back and records the failed stage.
before=$(git -C "$box" rev-parse HEAD)
push d
echo '{}' > "$box/artifacts/release.lock"
if run "$t/release-fail.sh" >/dev/null 2>&1; then fail "a failing release reported success"; fi
[ "$(git -C "$box" rev-parse HEAD)" = "$before" ] || fail "checkout not restored after a failed release"
[ ! -e "$box/artifacts/release.lock" ] || fail "release.lock left after a failure"
[[ "$(state)" == "failed - fetch:done,snapshot:done,test:failed" ]] || fail "progress after a failure: $(state)"

# 4. Uncommitted build inputs: refused before anything moves.
echo 'fn dirty() {}' > "$box/crates/dirty.rs"
if run "$t/release-ok.sh" >/dev/null 2>&1; then fail "a dirty checkout was updated"; fi
[ "$(git -C "$box" rev-parse HEAD)" = "$before" ] || fail "dirty checkout moved"
rm "$box/crates/dirty.rs"

# 5. Local commits the branch lacks: refused (never merged, never rewritten).
echo 'fn local() {}' > "$box/crates/local.rs" && git -C "$box" add -A && git -C "$box" commit -qm local
local_head=$(git -C "$box" rev-parse HEAD)
if run "$t/release-ok.sh" >/dev/null 2>&1; then fail "diverged checkout was updated"; fi
[ "$(git -C "$box" rev-parse HEAD)" = "$local_head" ] || fail "diverged checkout moved"
grep -q "local commits" "$box/artifacts/release-progress.json" || fail "no reason given for the refusal"
git -C "$box" reset -q --hard "$before"

# 6. After the refusals, a good release catches up with everything pushed.
run "$t/release-ok.sh" >/dev/null || fail "catch-up update failed"
[ "$(git -C "$box" rev-parse HEAD)" = "$(git -C "$t/dev" rev-parse HEAD)" ] || fail "catch-up did not reach the branch head"

# 7. A fetch failure changes nothing.
git -C "$box" remote set-url origin "$t/missing.git"
if run "$t/release-ok.sh" >/dev/null 2>&1; then fail "update succeeded without a remote"; fi
[[ "$(state)" == "failed - fetch:failed" ]] || fail "progress after a fetch failure: $(state)"
# 8. Rollback: a malformed commit is refused before anything runs; a missing snapshot fails the run
# at its restore stage and leaves the install alone.
git -C "$box" remote set-url origin "$t/origin.git"
if (cd "$box" && bash scripts/update.sh --rollback 'x; rm -rf /' >/dev/null 2>&1); then fail "a malformed rollback commit was accepted"; fi
if (cd "$box" && bash scripts/update.sh --rollback deadbee >/dev/null 2>&1); then fail "a rollback without a snapshot succeeded"; fi
[[ "$(state)" == "failed - restore:failed" ]] || fail "progress after a failed rollback: $(state)"
echo "update tests: ok"
