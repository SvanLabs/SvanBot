#!/usr/bin/env bash
# Tests for scripts/build-lock.sh (#363), hermetic: a throwaway crate in a temp dir whose build
# script sleeps long enough for the probe to be asked while the build is running.
#
# The premise is an external interface — *which* file cargo flocks, and that a held lock is visible
# to `flock -n` at all — so a stub cannot stand in for it: a fixture that locks a file this test
# chose would pass on a probe that no cargo build would ever trip, which is the shape LESSONS 44 is
# about. The test asks real cargo, and the crate exists only to make cargo hold its lock for a
# moment. The holder is killed rather than waited for; the sleep only has to outlast the probe.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
t=$(mktemp -d)
holder=
cleanup() {
  [ -z "$holder" ] || kill "$holder" 2>/dev/null || true
  rm -rf "$t"
}
trap cleanup EXIT
fail() { echo "build-lock test: $*" >&2; exit 1; }

probe="$repo/scripts/build-lock.sh"
[ -x "$probe" ] || fail "$probe is not executable"
command -v cargo >/dev/null 2>&1 || fail "cargo is not on PATH; this test asks real cargo"
command -v flock >/dev/null 2>&1 || fail "flock is not on PATH; the probe cannot answer without it"

# 1. A profile directory nothing has built in, and one that does not exist: nothing to report. The
#    release's first run finds exactly this, and a probe that reported a build there would put a
#    wait in the log of every first release.
mkdir -p "$t/empty"
out=$("$probe" "$t/empty")
[ -z "$out" ] || fail "an unbuilt profile directory reported a build: $out"
out=$("$probe" "$t/absent")
[ -z "$out" ] || fail "a missing profile directory reported a build: $out"

# 2. Real cargo, holding its own lock while its build script sleeps.
mkdir -p "$t/crate/src"
cat > "$t/crate/Cargo.toml" <<'EOF'
[package]
name = "lockholder"
version = "0.1.0"
edition = "2021"
EOF
# 45 s, and the fixture is killed rather than waited for, so the sleep costs the suite nothing: it
# only has to outlast the probe's bounded wait on a machine where cargo is slow to start (the gate's
# own test build can hold the package cache while this runs).
cat > "$t/crate/build.rs" <<'EOF'
fn main() { std::thread::sleep(std::time::Duration::from_secs(45)); }
EOF
echo 'fn main() {}' > "$t/crate/src/main.rs"

CARGO_TARGET_DIR="$t/target" cargo build --offline -q --manifest-path "$t/crate/Cargo.toml" &
holder=$!

# Bounded wait: a probe that never fires is the failure this asserts, and reporting it as one beats
# hanging the suite. Cargo's own startup is the variable part, so the bound is generous and the
# fixture's sleep is longer than the whole wait.
out=""
for _ in $(seq 1 400); do
  out=$("$probe" "$t/target/debug")
  if [ -n "$out" ]; then break; fi
  kill -0 "$holder" 2>/dev/null || fail "cargo exited before the probe ever saw a held lock"
  sleep 0.05
done
[ -n "$out" ] || fail "a running cargo build never showed up as a held lock in $t/target/debug"
case "$out" in *"$t/target/debug"*) ;; *) fail "the message does not name the held directory: $out";; esac

# 3. The holder gone is the lock free: the probe takes the lock and gives it straight back, so it
#    cannot report itself, and the release after a killed build is not told a build is running.
kill "$holder" 2>/dev/null || true
wait "$holder" 2>/dev/null || true
holder=
out=$("$probe" "$t/target/debug")
[ -z "$out" ] || fail "the lock was still reported held after the holder was gone: $out"

echo "build-lock tests: ok"
