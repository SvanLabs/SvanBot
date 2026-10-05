#!/usr/bin/env bash
# pre-commit-secret-scan.sh refuses a staged .env however long the staged list is (#875). The list
# used to be piped into `grep -q`; with thousands of paths git died of SIGPIPE when grep left at its
# first match, and under pipefail that read as "no .env staged".
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail() { echo "secret-scan: $*" >&2; exit 1; }
git init -q "$t"
cd "$t"
: > kept.txt && git add kept.txt
bash "$repo/scripts/pre-commit-secret-scan.sh" || fail "refused a commit with no .env in it"
mkdir many
for i in $(seq 1 20000); do : > "many/a-path-long-enough-to-fill-the-pipe-several-times-over-$i.txt"; done
: > .env.example
: > .env
git add -A -f
if bash "$repo/scripts/pre-commit-secret-scan.sh" 2>/dev/null; then fail "a staged .env passed behind 20000 other paths"; fi
git rm -q --cached .env
bash "$repo/scripts/pre-commit-secret-scan.sh" || fail "refused .env.example, which may be committed"
echo "secret-scan tests passed"
