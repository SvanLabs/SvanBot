#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/../.." && pwd -P)
# Hermetic: release.sh runs this under its own lock and exports it; the fixture has its own root.
unset SV10_RELEASE_LOCK_FD SV10_RELEASE_ROOT SV10_HEALTH_URL SVANBOT_WEB_PORT
rollback="$repo_root/scripts/rollback.sh"
test_root=$(mktemp -d)
legacy_bot_pid=
health_pid=
cleanup_test() {
  [ -z "$legacy_bot_pid" ] || kill "$legacy_bot_pid" 2>/dev/null || true
  [ -z "$health_pid" ] || kill "$health_pid" 2>/dev/null || true
  rm -rf "$test_root"
}
trap cleanup_test EXIT

fail() {
  echo "release-rollback test: $*" >&2
  exit 1
}

write_binary() {
  local path=$1 commit=$2 name
  name=$(basename "$path")
  printf '#!/usr/bin/env bash\nprintf '\''%%s\\n'\'' '\''%s 10.0.0 %s'\''\n' "$name" "$commit" > "$path"
  chmod +x "$path"
}

write_legacy_binary() {
  local path=$1 name
  name=$(basename "$path")
  printf '#!/usr/bin/env bash\nprintf '\''%%s\\n'\'' '\''%s 10.0.0'\''\n' "$name" > "$path"
  chmod +x "$path"
}

start_health_server() {
  local commit=$1 port_file="$test_root/health-port"
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
}

write_release() {
  local commit=$1 label=$2 extra=$3
  rm -rf "$test_root/target/release" "$test_root/web/dist"
  mkdir -p "$test_root/target/release" "$test_root/web/dist/assets" "$test_root/artifacts"
  for name in sv10-bot learner analyst "$extra"; do
    write_binary "$test_root/target/release/$name" "$commit"
  done
  printf '%s\n' "$label dashboard" > "$test_root/web/dist/index.html"
  printf '%s\n' "$label asset" > "$test_root/web/dist/assets/app.js"
}

write_staged_release() {
  local commit=$1 label=$2 extra=$3 base
  base="$test_root/build-$label"
  mkdir -p "$base/bin" "$base/web/assets"
  for name in sv10-bot learner analyst "$extra"; do
    write_binary "$base/bin/$name" "$commit"
  done
  printf '%s\n' "$label dashboard" > "$base/web/index.html"
  printf '%s\n' "$label asset" > "$base/web/assets/app.js"
}

tree_digest() {
  (
    cd "$test_root"
    find target/release web/dist -type f -print0 |
      sort -z |
      xargs -0 sha256sum
  ) | sha256sum | cut -d' ' -f1
}

[ -x "$rollback" ] || fail "missing executable scripts/rollback.sh"

git -C "$test_root" init -q
git -C "$test_root" config user.email test@example.invalid
git -C "$test_root" config user.name "Release rollback test"
printf 'release A\n' > "$test_root/source.txt"
git -C "$test_root" add source.txt
git -C "$test_root" commit -qm "release A"
commit_a=$(git -C "$test_root" rev-parse --short HEAD)

mkdir -p "$test_root/crates"
printf 'untracked build input\n' > "$test_root/crates/untracked.rs"
if SV10_RELEASE_ROOT="$test_root" "$rollback" --validate-source-clean >/dev/null 2>&1; then
  fail "source cleanliness accepted an untracked Rust build input"
fi
rm "$test_root/crates/untracked.rs"
mkdir -p "$test_root/.cargo"
printf '[build]\nrustflags = ["-C", "target-cpu=native"]\n' > "$test_root/.cargo/config.toml"
if SV10_RELEASE_ROOT="$test_root" "$rollback" --validate-source-clean >/dev/null 2>&1; then
  fail "source cleanliness accepted an untracked Rust build-control input"
fi
rm "$test_root/.cargo/config.toml"
rmdir "$test_root/.cargo"
SV10_RELEASE_ROOT="$test_root" "$rollback" --validate-source-clean

# A true first install has the parent directories but neither managed release set.
mkdir -p "$test_root/target" "$test_root/web"
write_staged_release "$commit_a" BOOT extra-bootstrap
SV10_RELEASE_ROOT="$test_root" "$rollback" --install "$test_root/build-BOOT/bin" "$test_root/build-BOOT/web" "$commit_a"
[ "$("$test_root/target/release/extra-bootstrap" --version)" = "extra-bootstrap 10.0.0 $commit_a" ] ||
  fail "first install did not install the executable set"
grep -qx 'BOOT dashboard' "$test_root/web/dist/index.html" || fail "first install did not install the dashboard set"

write_release "$commit_a" A extra-a
printf '2026-09-19T10:00:00+00:00 %s release A\n' "$commit_a" > "$test_root/artifacts/releases.log"
for name in sv10-bot learner analyst; do
  write_legacy_binary "$test_root/target/release/$name"
done
printf '%s\n' '#include <stdio.h>' '#include <string.h>' '#include <unistd.h>' \
  'int main(int argc, char **argv) { if (argc > 1 && strcmp(argv[1], "--version") == 0) { puts("sv10-bot 10.0.0"); return 0; } sleep(60); return 0; }' \
  > "$test_root/legacy-bot.c"
cc -O2 -o "$test_root/target/release/sv10-bot" "$test_root/legacy-bot.c"
"$test_root/target/release/sv10-bot" &
legacy_bot_pid=$!
printf '%s\n' "$legacy_bot_pid" > "$test_root/artifacts/bot.pid"
start_health_server 0000000
if SV10_RELEASE_ROOT="$test_root" SV10_HEALTH_URL="$health_url" "$rollback" --adopt-legacy "$commit_a" >/dev/null 2>&1; then
  fail "legacy adoption accepted mismatched health identity"
fi
wait "$health_pid" || true
health_pid=
start_health_server "$commit_a"
SV10_RELEASE_ROOT="$test_root" SV10_HEALTH_URL="$health_url" "$rollback" --adopt-legacy "$commit_a"
wait "$health_pid" || true
health_pid=
kill "$legacy_bot_pid"
wait "$legacy_bot_pid" || true
legacy_bot_pid=

write_binary "$test_root/target/release/analyst" 0000000
if SV10_RELEASE_ROOT="$test_root" "$rollback" --snapshot "$commit_a" >/dev/null 2>&1; then
  fail "snapshot accepted binaries from a different build identity"
fi
write_legacy_binary "$test_root/target/release/analyst"
ln -s "$test_root/source.txt" "$test_root/web/dist/assets/linked.js"
if SV10_RELEASE_ROOT="$test_root" "$rollback" --snapshot "$commit_a" >/dev/null 2>&1; then
  fail "snapshot accepted a dashboard symlink"
fi
rm "$test_root/web/dist/assets/linked.js"
exec 8> "$test_root/artifacts/release-operation.lock"
flock -n 8
if SV10_RELEASE_ROOT="$test_root" "$rollback" --snapshot "$commit_a" >/dev/null 2>&1; then
  fail "snapshot ignored the global operation lock"
fi
# Older snapshots beyond SV10_KEEP_SNAPSHOTS are pruned oldest first; the new one always stays.
for i in 1 2 3 4; do
  mkdir -p "$test_root/artifacts/release-snapshots/old$i"
  touch -d "2026-01-0$i" "$test_root/artifacts/release-snapshots/old$i"
done
mkdir -p "$test_root/artifacts/release-snapshots/.stage.tmp"
SV10_RELEASE_ROOT="$test_root" SV10_RELEASE_LOCK_FD=8 SV10_KEEP_SNAPSHOTS=3 "$rollback" --snapshot "$commit_a"
exec 8>&-
[ -d "$test_root/artifacts/release-snapshots/$commit_a" ] || fail "prune removed the new snapshot"
[ -d "$test_root/artifacts/release-snapshots/old4" ] && [ -d "$test_root/artifacts/release-snapshots/old3" ] || fail "prune removed a newer snapshot"
[ ! -e "$test_root/artifacts/release-snapshots/old2" ] && [ ! -e "$test_root/artifacts/release-snapshots/old1" ] || fail "prune kept snapshots beyond the limit"
[ -d "$test_root/artifacts/release-snapshots/.stage.tmp" ] || fail "prune touched a staging directory"
rm -rf "$test_root/artifacts/release-snapshots"/old* "$test_root/artifacts/release-snapshots/.stage.tmp"
snapshot="$test_root/artifacts/release-snapshots/$commit_a"
[ -f "$snapshot/SHA256SUMS" ] || fail "snapshot manifest missing"
grep -q "target/release/extra-a" "$snapshot/SHA256SUMS" || fail "snapshot omitted an installed executable"
grep -q "web/dist/assets/app.js" "$snapshot/SHA256SUMS" || fail "snapshot omitted dashboard content"

printf 'release B\n' >> "$test_root/source.txt"
git -C "$test_root" add source.txt
git -C "$test_root" commit -qm "release B"
commit_b=$(git -C "$test_root" rev-parse --short HEAD)
write_staged_release "$commit_b" B extra-b
before_rejected_install=$(tree_digest)
write_legacy_binary "$test_root/build-B/bin/analyst"
if SV10_RELEASE_ROOT="$test_root" "$rollback" --install "$test_root/build-B/bin" "$test_root/build-B/web" "$commit_b" >/dev/null 2>&1; then
  fail "fresh install accepted a binary without build identity"
fi
[ "$(tree_digest)" = "$before_rejected_install" ] || fail "rejected fresh install changed release A"
write_binary "$test_root/build-B/bin/analyst" "$commit_b"
SV10_RELEASE_ROOT="$test_root" "$rollback" --install "$test_root/build-B/bin" "$test_root/build-B/web" "$commit_b"
printf '2026-09-19T11:00:00+00:00 %s release B\n' "$commit_b" >> "$test_root/artifacts/releases.log"
before_failed_rollback=$(tree_digest)

printf 'corrupt\n' >> "$snapshot/target/release/learner"
if SV10_RELEASE_ROOT="$test_root" "$rollback" "$commit_a" >/dev/null 2>&1; then
  fail "rollback accepted a corrupt snapshot"
fi
[ "$(tree_digest)" = "$before_failed_rollback" ] || fail "failed verification changed installed files"

write_legacy_binary "$snapshot/target/release/learner"
if SV10_RELEASE_ROOT="$test_root" SV10_RELEASE_TEST_FAIL_AFTER_BIN_SWAP=1 "$rollback" "$commit_a" >/dev/null 2>&1; then
  fail "injected install failure unexpectedly succeeded"
fi
[ "$(tree_digest)" = "$before_failed_rollback" ] || fail "mid-install failure did not restore release B"

# Data-format floor (0229): release A's source has no store codec (format 1), so it is refused while
# the databases hold compressed columns (format 2), and accepted once `archive unpack` lowered it.
printf '2\n' > "$test_root/artifacts/data-format"
[ "$(SV10_RELEASE_ROOT="$test_root" "$rollback" --data-format "$commit_a")" = "1 2" ] || fail "data formats misread"
if SV10_RELEASE_ROOT="$test_root" "$rollback" "$commit_a" >/dev/null 2>"$test_root/floor.err"; then
  fail "rollback installed a build that cannot read the compressed store"
fi
grep -q "archive unpack" "$test_root/floor.err" || fail "the refusal does not name archive unpack"
[ "$(tree_digest)" = "$before_failed_rollback" ] || fail "a refused rollback changed installed files"
printf '1\n' > "$test_root/artifacts/data-format"

SV10_RELEASE_ROOT="$test_root" "$rollback" "$commit_a"
for name in sv10-bot learner analyst; do
  [ "$("$test_root/target/release/$name" --version 2>/dev/null || "$test_root/target/release/$name")" = "$name 10.0.0" ] ||
    fail "$name was not restored"
done
[ "$("$test_root/target/release/extra-a" --version)" = "extra-a 10.0.0 $commit_a" ] || fail "extra-a was not restored"
[ ! -e "$test_root/target/release/extra-b" ] || fail "rollback retained a release-B-only executable"
grep -qx 'A dashboard' "$test_root/web/dist/index.html" || fail "dashboard was not restored"
grep -Eq " $commit_a rollback from $commit_b$" "$test_root/artifacts/releases.log" || fail "rollback record missing"

# An unidentified build ("11.0.0 dev", built straight into target/release) cannot be snapshotted
# by commit; --preserve-unidentified keeps a verified copy aside instead, only while a verified
# rollback snapshot exists, and never touches the installed tree.
if SV10_RELEASE_ROOT="$test_root" "$rollback" --preserve-unidentified >/dev/null 2>&1; then
  fail "preserve-unidentified accepted an identified install"
fi
rm "$test_root/target/release/.sv10-installed-commit"
for name in sv10-bot learner analyst; do
  printf '#!/usr/bin/env bash\necho "%s 11.0.0 dev"\n' "$name" > "$test_root/target/release/$name"
  chmod +x "$test_root/target/release/$name"
done
before_preserve=$(tree_digest)
mv "$snapshot" "$test_root/snapshot-aside"
if SV10_RELEASE_ROOT="$test_root" "$rollback" --preserve-unidentified >/dev/null 2>&1; then
  fail "preserve-unidentified proceeded without a verified rollback snapshot"
fi
mv "$test_root/snapshot-aside" "$snapshot"
preserved=$(SV10_RELEASE_ROOT="$test_root" "$rollback" --preserve-unidentified | sed -n 's/^Preserved unidentified build at //p')
[ -n "$preserved" ] && [ -f "$preserved/SHA256SUMS" ] || fail "preserve-unidentified wrote no manifest"
grep -q "target/release/sv10-bot" "$preserved/SHA256SUMS" || fail "preserved copy omitted sv10-bot"
grep -q "web/dist/index.html" "$preserved/SHA256SUMS" || fail "preserved copy omitted the dashboard"
grep -q "sv10-bot 11.0.0 dev" "$preserved/VERSIONS" || fail "preserved copy did not record versions"
(cd "$preserved" && sha256sum --check --strict --quiet SHA256SUMS) || fail "preserved copy does not verify"
[ "$(tree_digest)" = "$before_preserve" ] || fail "preserve-unidentified changed the installed tree"

if SV10_RELEASE_ROOT=/ "$rollback" "$commit_a" >/dev/null 2>&1; then
  fail "unsafe root / was accepted"
fi

mv "$test_root/target" "$test_root/target-real"
ln -s "$test_root/target-real" "$test_root/target"
if SV10_RELEASE_ROOT="$test_root" "$rollback" --snapshot "$commit_a" >/dev/null 2>&1; then
  fail "symlinked managed target was accepted"
fi

echo "release rollback integration: ok"
