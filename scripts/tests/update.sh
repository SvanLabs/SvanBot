#!/usr/bin/env bash
# Tests for scripts/update.sh and scripts/progress.py (0236), hermetic: a bare "GitHub" origin, a
# developer clone that pushes, and an operator checkout that updates with a stub release script.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
source "$repo/scripts/tests/lib-release-bin.sh"
t=$(mktemp -d)
health_pid=
bot_pid=
cleanup() {
  [ -z "${bot_pid:-}" ] || kill "$bot_pid" 2>/dev/null || true
  [ -z "${health_pid:-}" ] || kill "$health_pid" 2>/dev/null || true
  rm -rf "$t"
}
trap cleanup EXIT
fail() { echo "update test: $*" >&2; exit 1; }

write_binary() {
  local path=$1 commit=$2 name
  name=$(basename "$path")
  printf '#!/usr/bin/env bash\nprintf '\''%%s\\n'\'' '\''%s 10.0.0 %s'\''\n' "$name" "$commit" > "$path"
  chmod +x "$path"
}

# A health endpoint that answers once, like the real one: /api/health reporting `commit`.
start_health_server() {
  local commit=$1 port_file="$t/health-port"
  rm -f "$port_file"
  python3 - "$port_file" "$commit" <<'PY' &
import http.server, json, socketserver, sys
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({"commit": sys.argv[2]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *_):
        pass
with socketserver.TCPServer(("127.0.0.1", 0), Handler) as server:
    open(sys.argv[1], "w").write(str(server.server_address[1]))
    server.handle_request()
PY
  health_pid=$!
  for _ in $(seq 1 100); do [ -s "$port_file" ] && break; sleep 0.02; done
  [ -s "$port_file" ] || fail "health fixture did not start"
  health_url="http://127.0.0.1:$(< "$port_file")/api/health"
  export SV10_HEALTH_URL="$health_url"
}
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
unset SV10_RELEASE_LOCK_FD SV10_RELEASE_ROOT SV10_UPDATE_RUN SV10_PROGRESS_DIR SVANBOT_ADOPT_UPSTREAM
# The fixture's origin has only `main`, so an ambient update branch, remote or fetch timeout left in
# the environment would point update.sh somewhere the fixture does not have (#365). These are the
# operator's fleet settings, and the fleet that runs this gate is configured with them.
unset SVANBOT_UPDATE_BRANCH SVANBOT_UPDATE_REMOTE SVANBOT_UPDATE_FETCH_TIMEOUT

git init -q --bare --initial-branch=main "$t/origin.git"
git clone -q "$t/origin.git" "$t/dev" 2>/dev/null
mkdir -p "$t/dev/scripts" "$t/dev/crates"
cp "$repo/scripts/update.sh" "$repo/scripts/progress.py" "$repo/scripts/rollback.sh" "$t/dev/scripts/"
echo 'fn a() {}' > "$t/dev/crates/a.rs"
echo 'operator notes' > "$t/dev/operator-notes.txt"
# The fixture ignores what the real checkout ignores: the release test installs real sets of
# files into target/release and web/dist, and source cleanliness must not read them as edits.
printf 'artifacts/\ntarget/\nweb/\n' > "$t/dev/.gitignore"
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
python3 - "$box/artifacts/release-progress.json" <<'PY' || fail 'manual updater owner identity not recorded'
import json,sys
progress=json.load(open(sys.argv[1]))
assert isinstance(progress.get('pid'),int) and progress['pid'] > 0, progress
assert progress.get('process_start') and progress.get('boot_id'), progress
PY

# 2. New commits on the branch: fast-forward, release, installed commit is the new head.
push b && push c
[ "$(cd "$box" && bash scripts/update.sh --check)" = "2 $(git -C "$t/dev" rev-parse --short HEAD) $(git -C "$box" branch --show-current) 0" ] || fail "--check did not report 2 behind"
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

# A tracked non-build file can pass the source guard and still prevent git's fast-forward.
# That refusal must finish the progress record instead of leaving a permanent running card.
echo 'upstream notes' > "$t/dev/operator-notes.txt"
git -C "$t/dev" add operator-notes.txt
git -C "$t/dev" commit -qm notes
git -C "$t/dev" -c push.negotiate=false push -q origin HEAD:main
echo 'local notes to preserve' > "$box/operator-notes.txt"
echo '{}' > "$box/artifacts/release.lock"
if run "$t/release-ok.sh" >/dev/null 2>&1; then fail "a conflicting tracked edit was overwritten"; fi
[ "$(git -C "$box" rev-parse HEAD)" = "$before" ] || fail "a refused fast-forward moved the checkout"
[ "$(cat "$box/operator-notes.txt")" = 'local notes to preserve' ] || fail "local notes were lost"
[ ! -e "$box/artifacts/release.lock" ] || fail "release.lock left after a refused fast-forward"
[[ "$(state)" == "failed - fetch:failed" ]] || fail "a refused fast-forward left progress running: $(state)"
git -C "$box" checkout -q -- operator-notes.txt

# An unexpected Git error must also finish a run. A failing update check owns no run, so it must
# not change another operation's progress or remove the dashboard lock.
mkdir -p "$t/git-failure"
real_git=$(command -v git)
cat > "$t/git-failure/git" <<EOF
#!/usr/bin/env bash
if [ "\${1:-}" = rev-parse ] && [ "\${2:-}" = HEAD ]; then exit 42; fi
exec "$real_git" "\$@"
EOF
chmod +x "$t/git-failure/git"
if PATH="$t/git-failure:$PATH" run "$t/release-ok.sh" >/dev/null 2>&1; then fail "an unexpected git failure succeeded"; fi
[[ "$(state)" == "failed - fetch:failed" ]] || fail "an unexpected git failure left progress running: $(state)"
grep -q 'exit 42' "$box/artifacts/release-progress.json" || fail "unexpected failure lacks its exit status"
(cd "$box" && python3 scripts/progress.py start && python3 scripts/progress.py stage build)
cp "$box/artifacts/release-progress.json" "$t/active-progress.json"
echo '{}' > "$box/artifacts/release.lock"
git -C "$box" remote set-url origin "$t/missing.git"
if run "$t/release-ok.sh" --check >/dev/null 2>&1; then fail "a failed update check succeeded"; fi
cmp "$box/artifacts/release-progress.json" "$t/active-progress.json" || fail "update check changed another run's progress"
[ -e "$box/artifacts/release.lock" ] || fail "update check removed another run's lock"
git -C "$box" remote set-url origin "$t/origin.git"

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

# 9. A checkout that shares no history with the update branch — a fleet cloned from a private tree,
# pointed at the repository that tree publishes to. Without the opt-in it is refused like any other
# checkout ahead of its branch; with it, the checkout moves onto the branch, the runtime state it
# holds outside git is kept, the tracked files the branch does not have are reported and dropped, and
# the head it left is kept under a ref so nothing it had is only in the reflog.
git init -q --bare --initial-branch=main "$t/other.git"
git clone -q "$t/other.git" "$t/spun" 2>/dev/null
mkdir -p "$t/spun/scripts" "$t/spun/crates"
cp "$repo/scripts/update.sh" "$repo/scripts/progress.py" "$repo/scripts/rollback.sh" "$t/spun/scripts/"
echo 'fn other() {}' > "$t/spun/crates/other.rs"
echo 'notes that exist only here' > "$t/spun/private-notes.txt"
git -C "$t/spun" add -A && git -C "$t/spun" commit -qm other
git -C "$t/spun" -c push.negotiate=false push -q origin HEAD:main
git clone -q "$t/other.git" "$t/box2"
spun_head=$(git -C "$t/box2" rev-parse HEAD)
mkdir -p "$t/box2/artifacts" && echo '{"live":true}' > "$t/box2/artifacts/live-state.json"
git -C "$t/box2" remote set-url origin "$t/origin.git"

if (cd "$t/box2" && SV10_RELEASE_SCRIPT="$t/release-ok.sh" bash scripts/update.sh >/dev/null 2>&1); then
  fail "an unrelated checkout was adopted without the opt-in"
fi
[ "$(git -C "$t/box2" rev-parse HEAD)" = "$spun_head" ] || fail "a refused adoption moved the checkout"
grep -q "unrelated history" "$t/box2/artifacts/release-progress.json" || fail "no reason given for the adoption refusal"

# The move is git's own fast-forward, so its guards apply to it: a tracked file with uncommitted edits
# that the move would delete stops the adoption, where a hard reset would have thrown the edit away.
echo 'an uncommitted edit' >> "$t/box2/private-notes.txt"
if (cd "$t/box2" && SVANBOT_ADOPT_UPSTREAM=1 SV10_RELEASE_SCRIPT="$t/release-ok.sh" bash scripts/update.sh >/dev/null 2>&1); then
  fail "an adoption overwrote a tracked file with uncommitted edits"
fi
[ "$(git -C "$t/box2" rev-parse HEAD)" = "$spun_head" ] || fail "a refused adoption moved the checkout"
[ -z "$(git -C "$t/box2" for-each-ref --format='%(refname)' refs/replace)" ] || fail "a refused adoption left a graft behind"
git -C "$t/box2" checkout -q -- private-notes.txt

(cd "$t/box2" && SVANBOT_ADOPT_UPSTREAM=1 SV10_RELEASE_SCRIPT="$t/release-ok.sh" bash scripts/update.sh >/dev/null) || fail "adoption with the opt-in failed"
[ "$(git -C "$t/box2" rev-parse HEAD)" = "$(git -C "$t/dev" rev-parse HEAD)" ] || fail "adoption did not reach the update branch"
[ -f "$t/box2/artifacts/live-state.json" ] || fail "adoption dropped runtime state that git does not track"
[ ! -e "$t/box2/private-notes.txt" ] || fail "adoption kept a tracked file the branch does not have"
[ ! -e "$t/box2/crates/other.rs" ] || fail "adoption kept a tracked file the branch does not have"
[ -n "$(git -C "$t/box2" for-each-ref --format='%(refname)' refs/adopt)" ] || fail "the head the checkout left was not kept under a ref"
git -C "$t/box2" merge-base --is-ancestor "$spun_head" "$(git -C "$t/box2" for-each-ref --format='%(refname)' refs/adopt | head -1)" || fail "the kept ref does not point at the head that was left"
grep -q "recoverable from refs/adopt/" "$t/box2/artifacts/release.log" || fail "the log does not say where the dropped files went"

# The point of adopting rather than merging: every update after it is an ordinary fast-forward.
push f
(cd "$t/box2" && SV10_RELEASE_SCRIPT="$t/release-ok.sh" bash scripts/update.sh >/dev/null) || fail "the update after an adoption failed"
[ "$(git -C "$t/box2" rev-parse HEAD)" = "$(git -C "$t/dev" rev-parse HEAD)" ] || fail "the update after an adoption did not fast-forward"
grep -q "fast-forwarded" "$t/box2/artifacts/release.log" || fail "the update after an adoption was not a fast-forward"

# 10. A checkout ahead of the update branch with every commit on a remote branch (#394): the shape a
# checkout has when it sits on a branch ahead of the one `.env` names. Nothing is at risk and
# nothing needs installing, so it is neither a failure nor an install, and the message says which
# branch line the checkout is on and how to follow it.
git clone -q "$t/origin.git" "$t/ahead" 2>/dev/null
mkdir -p "$t/ahead/artifacts"
git -C "$t/ahead" checkout -q -b dev
echo 'fn only_on_dev() {}' > "$t/ahead/crates/only-dev.rs"
git -C "$t/ahead" add -A && git -C "$t/ahead" commit -qm "the next dev merge"
git -C "$t/ahead" -c push.negotiate=false push -q origin HEAD:dev
ahead_head=$(git -C "$t/ahead" rev-parse HEAD)
(cd "$t/ahead" && SV10_RELEASE_SCRIPT="$t/release-ok.sh" bash scripts/update.sh >/dev/null) || fail "a checkout ahead on its own branch was reported as failed"
[ "$(git -C "$t/ahead" rev-parse HEAD)" = "$ahead_head" ] || fail "a checkout ahead of the update branch moved"
ahead_state=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["state"], d.get("commit") or "-", ",".join(s["name"]+":"+s["state"] for s in d["stages"]))' "$t/ahead/artifacts/release-progress.json")
# The fetch ran; nothing after it did, and no build was installed.
[[ "$ahead_state" == "current - fetch:done" ]] || fail "progress after an ahead-of-branch run: $ahead_state"
grep -q "SVANBOT_UPDATE_BRANCH=dev" "$t/ahead/artifacts/release.log" || fail "the run does not say how to follow the checkout's own branch"
grep -q "== stub release" "$t/ahead/artifacts/release.log" && fail "an ahead-of-branch run built something"

# The same shape without a remote holding the commits is still the guard's case: work nothing else
# has must not be moved aside.
echo 'fn unpushed() {}' > "$t/ahead/crates/unpushed.rs"
git -C "$t/ahead" add -A && git -C "$t/ahead" commit -qm "not pushed anywhere"
unpushed_head=$(git -C "$t/ahead" rev-parse HEAD)
if (cd "$t/ahead" && SV10_RELEASE_SCRIPT="$t/release-ok.sh" bash scripts/update.sh >/dev/null 2>&1); then
  fail "a checkout with commits on no remote was updated"
fi
[ "$(git -C "$t/ahead" rev-parse HEAD)" = "$unpushed_head" ] || fail "a checkout with unpushed commits moved"
grep -q "local commits" "$t/ahead/artifacts/release-progress.json" || fail "no reason given for the refusal"

# 11. The health gate (issue #726): installing the files is not the same as the fleet running them.
# A build that answers `--version` and dies at startup passes every files-only check and then
# crash-loops; the update is complete only once /api/health reports the installed commit, and
# anything else restores the previous verified snapshot with the checkout and play intact.
gate_commit=$(git -C "$box" rev-parse --short HEAD)
mkdir -p "$box/target" "$box/web" "$box/artifacts" "$box/fixture/bin" "$box/fixture/web"
cat > "$t/fleet-bot.c" <<'EOF'
#include <stdio.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
  if (argc > 1 && strcmp(argv[1], "--version") == 0) { printf("sv10-bot 10.0.0 %s\n", COMMIT); return 0; }
  sleep(600);
  return 0;
}
EOF
cc -O2 -DCOMMIT="\"$gate_commit\"" -o "$box/fixture/bin/sv10-bot" "$t/fleet-bot.c"
for name in learner analyst; do write_binary "$box/fixture/bin/$name" "$gate_commit"; done
echo 'gate dashboard' > "$box/fixture/web/index.html"
(cd "$box" && SV10_RELEASE_ROOT="$box" bash scripts/rollback.sh --install "$box/fixture/bin" "$box/fixture/web" "$gate_commit") >/dev/null
(cd "$box" && SV10_RELEASE_ROOT="$box" bash scripts/rollback.sh --snapshot "$gate_commit") >/dev/null
# A bot playing the installed build: the gate asks /proc/<pid>/exe, so a stale pid file cannot
# answer for a dead fleet.
"$box/target/release/sv10-bot" &
bot_pid=$!
echo "$bot_pid" > "$box/artifacts/bot.pid"

# The stub release "installs" the checked-out commit by moving the marker, the way the real one
# installs a --version-only build that then crash-loops: the files are there, the fleet never is.
cat > "$t/release-crashloop.sh" <<'EOF'
#!/usr/bin/env bash
set -e
python3 scripts/progress.py stage snapshot
python3 scripts/progress.py stage install
printf '%s\n' "$(git rev-parse --short HEAD)" > target/release/.sv10-installed-commit
echo "stub: installed a build that answers --version and never serves /api/health"
EOF
chmod +x "$t/release-crashloop.sh"
export SV10_HEALTH_TIMEOUT=2

push g
before_gate=$(git -C "$box" rev-parse HEAD)
start_health_server "$gate_commit"
if run "$t/release-crashloop.sh" >/dev/null 2>&1; then
  fail "an update whose build never answered /api/health reported success"
fi
[ "$(cat "$box/target/release/.sv10-installed-commit")" = "$gate_commit" ] ||
  fail "the health gate did not roll back to the previous verified build"
[ "$(git -C "$box" rev-parse HEAD)" = "$before_gate" ] || fail "a rolled-back update left the checkout on the failed commit"
kill -0 "$bot_pid" || fail "the health gate stopped the fleet it was supposed to keep playing"
[[ "$(state)" == failed* ]] || fail "progress after a failed health gate: $(state)"
grep -q "did not answer /api/health" "$box/artifacts/release-progress.json" || fail "the progress record does not say the build failed to come up"
grep -q "rolled back to the verified build" "$box/artifacts/release.log" || fail "the release log does not report the rollback"
wait "$health_pid" || true
health_pid=

# With no fleet playing there is nothing to verify: a stopped box (or a first install) is not blocked
# by a gate that could only ever time out.
kill "$bot_pid"
wait "$bot_pid" 2>/dev/null || true
bot_pid=
rm "$box/artifacts/bot.pid"
run "$t/release-crashloop.sh" >/dev/null || fail "a release on a stopped fleet was refused"
[[ "$(state)" == installed* ]] || fail "a stopped fleet skipped the gate but failed the run: $(state)"

# A fleet that does report the installed commit passes the gate and the run completes.
push h
export SV10_HEALTH_TIMEOUT=2
start_health_server "$(git -C "$t/dev" rev-parse --short HEAD)"
"$box/target/release/sv10-bot" &
bot_pid=$!
echo "$bot_pid" > "$box/artifacts/bot.pid"
run "$t/release-crashloop.sh" >/dev/null || fail "the health gate refused a fleet reporting the installed commit"
[[ "$(state)" == installed* ]] || fail "progress after a verified install: $(state)"
wait "$health_pid" || true
health_pid=
unset SV10_HEALTH_TIMEOUT SV10_HEALTH_URL

# 12. The fleet can stop while the release runs (a stop.sh, a service stop). Then there is nothing
# left to verify and nothing broken, so the install is kept and reported unverified: rolling back
# would undo a good update, and the old build was no more verified than the new one.
push i
stop_commit=$(git -C "$box" rev-parse --short HEAD)    # the build the fixture binaries below carry
run_commit=$(git -C "$t/dev" rev-parse --short HEAD)
previous_marker=$(cat "$box/target/release/.sv10-installed-commit")
cat > "$t/release-fleet-stops.sh" <<'EOF'
#!/usr/bin/env bash
set -e
python3 scripts/progress.py stage snapshot
python3 scripts/progress.py stage install
printf '%s\n' "$(git rev-parse --short HEAD)" > target/release/.sv10-installed-commit
kill "$(cat artifacts/bot.pid)" 2>/dev/null || true
rm -f artifacts/bot.pid
echo "stub: installed a build, then the fleet stopped"
EOF
chmod +x "$t/release-fleet-stops.sh"
export SV10_HEALTH_TIMEOUT=2
"$box/target/release/sv10-bot" &
bot_pid=$!
echo "$bot_pid" > "$box/artifacts/bot.pid"
start_health_server 0000000
run "$t/release-fleet-stops.sh" >/dev/null || fail "a release whose fleet stopped mid-run was refused"
wait "$bot_pid" 2>/dev/null || true
bot_pid=
[ "$(cat "$box/target/release/.sv10-installed-commit")" = "$run_commit" ] ||
  fail "a fleet that stopped during the release had its install rolled back"
[ "$(cat "$box/target/release/.sv10-installed-commit")" != "$previous_marker" ] || fail "the update installed nothing"
[ "$(git -C "$box" rev-parse --short HEAD)" = "$run_commit" ] || fail "an unverified install moved the checkout back"
[[ "$(state)" == installed* ]] || fail "progress after an unverified install: $(state)"
grep -q "the install is unverified" "$box/artifacts/release.log" || fail "the run did not report the install as unverified"
wait "$health_pid" || true
health_pid=

# 13. The same shape with a supervisor still alive is a build that never came up, not a fleet stopped
# on purpose: the gate must still roll back to the verified snapshot.
push j
cc -O2 -DCOMMIT="\"$stop_commit\"" -o "$box/fixture/bin/sv10-bot" "$t/fleet-bot.c"
for name in learner analyst; do write_binary "$box/fixture/bin/$name" "$stop_commit"; done
(cd "$box" && SV10_RELEASE_ROOT="$box" bash scripts/rollback.sh --install "$box/fixture/bin" "$box/fixture/web" "$stop_commit") >/dev/null
(cd "$box" && SV10_RELEASE_ROOT="$box" bash scripts/rollback.sh --snapshot "$stop_commit") >/dev/null
cat > "$t/release-crashloop-supervised.sh" <<'EOF'
#!/usr/bin/env bash
set -e
python3 scripts/progress.py stage snapshot
python3 scripts/progress.py stage install
printf '%s\n' "$(git rev-parse --short HEAD)" > target/release/.sv10-installed-commit
kill "$(cat artifacts/bot.pid)" 2>/dev/null || true
rm -f artifacts/bot.pid
echo "stub: installed a build that crash-loops while the supervisor restarts it"
EOF
chmod +x "$t/release-crashloop-supervised.sh"
# From the release root, the way start.sh launches one: --fleet-supervisors only counts a
# supervisor process that is really this checkout's.
( cd "$box" && exec sleep 60 ) &
supervisor_pid=$!
echo "$supervisor_pid" > "$box/artifacts/supervisor.pid"
"$box/target/release/sv10-bot" &
bot_pid=$!
echo "$bot_pid" > "$box/artifacts/bot.pid"
start_health_server 0000000
before_supervised=$(git -C "$box" rev-parse HEAD)
if run "$t/release-crashloop-supervised.sh" >/dev/null 2>&1; then fail "a crash-looping build reported success"; fi
wait "$bot_pid" 2>/dev/null || true
bot_pid=
[ "$(cat "$box/target/release/.sv10-installed-commit")" = "$stop_commit" ] ||
  fail "a live supervisor did not make the gate roll back the crash-looping build"
[ "$(git -C "$box" rev-parse HEAD)" = "$before_supervised" ] || fail "the rolled-back crash-loop left the checkout on the failed commit"
grep -q "rolled back to the verified build" "$box/artifacts/release.log" || fail "the supervised crash-loop was not reported as rolled back"
kill "$supervisor_pid" 2>/dev/null || true
wait "$supervisor_pid" 2>/dev/null || true
rm -f "$box/artifacts/supervisor.pid"
wait "$health_pid" || true
health_pid=

# 14. A previous install with no verified snapshot cannot be a rollback target: naming it as the
# manual fallback would hand the operator a command that fails the same way.
push k
unverified_commit=$(git -C "$t/dev" rev-parse --short HEAD)
# The installed marker names a commit with no snapshot: adopt-legacy and markerless installs leave
# exactly this shape, and the manual fallback must not name a commit that cannot be restored.
printf '%s\n' "$run_commit" > "$box/target/release/.sv10-installed-commit"
# From the release root, the way start.sh launches one: --fleet-supervisors only counts a
# supervisor process that is really this checkout's.
( cd "$box" && exec sleep 60 ) &
supervisor_pid=$!
echo "$supervisor_pid" > "$box/artifacts/supervisor.pid"
"$box/target/release/sv10-bot" &
bot_pid=$!
echo "$bot_pid" > "$box/artifacts/bot.pid"
start_health_server 0000000
if run "$t/release-crashloop-supervised.sh" >/dev/null 2>&1; then fail "a build with no rollback target reported success"; fi
wait "$bot_pid" 2>/dev/null || true
bot_pid=
[ "$(cat "$box/target/release/.sv10-installed-commit")" = "$unverified_commit" ] || fail "a rollback ran without a verified target"
[ "$(git -C "$box" rev-parse --short HEAD)" = "$unverified_commit" ] || fail "a missing rollback target moved the checkout"
grep -q "no verified rollback target" "$box/artifacts/release.log" || fail "the run did not name the missing rollback target"
kill "$supervisor_pid" 2>/dev/null || true
wait "$supervisor_pid" 2>/dev/null || true
rm -f "$box/artifacts/supervisor.pid"
wait "$health_pid" || true
health_pid=
unset SV10_HEALTH_TIMEOUT SV10_HEALTH_URL

echo "update tests: ok"
