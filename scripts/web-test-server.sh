#!/usr/bin/env bash
# Sandboxed dashboard for the Playwright tests: the real sv10-bot API and web/dist on port 5099,
# a throwaway artifacts directory, a fake bot key, bots stopped (dry run) and every network
# endpoint pointed at a closed local port, so nothing reaches openpoker.ai or the live databases.
set -euo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
bin="${SV10_TEST_BIN:-$repo/target/dev/release/sv10-bot}"
[ -x "$bin" ] || { echo "missing $bin (CARGO_TARGET_DIR=target/dev cargo build --release -p sv10-bot)" >&2; exit 1; }
sandbox="$(mktemp -d "${TMPDIR:-/tmp}/sv10-web-test.XXXXXX")"
trap 'rm -rf "$sandbox"' EXIT
mkdir -p "$sandbox/artifacts"
ln -s "$repo/web" "$sandbox/web"
# A checkout accumulates directories beside the code that the build never touches — a local tracker,
# a site build, measurement dumps. The sandbox is a throwaway root, so name any top-level directory
# the server should also see in SV10_TEST_EXTRA_DIRS (space-separated; empty by default). Each is
# linked only when it is present, so a tree without it gets no dangling link.
for extra in ${SV10_TEST_EXTRA_DIRS:-}; do
  if [ -e "$repo/$extra" ]; then ln -s "$repo/$extra" "$sandbox/$extra"; fi
done
env -i PATH="$PATH" HOME="$sandbox" SVANBOT10_ROOT="$sandbox" \
  SVANBOT_API_KEY=test-key SVANBOT_MAIN_NAME=TestBot SVANBOT_RUNTIME__DRY_RUN=true \
  SVANBOT_SERVER_URL=ws://127.0.0.1:9/ws SVANBOT_REST_BASE=http://127.0.0.1:9/api \
  SVANBOT_WEB__HOST=127.0.0.1 SVANBOT_WEB_PORT=5099 \
  "$bin" &
pid=$!
trap 'kill $pid 2>/dev/null; wait $pid 2>/dev/null; rm -rf "$sandbox"' EXIT INT TERM
wait $pid
