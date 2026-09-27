#!/usr/bin/env bash
# The file-size baseline (0390), in a throwaway tree. The rule the real baseline cannot demonstrate
# is the one about an entry that is no longer needed: three files in this repository are in that
# state, and the way to see the check refuse is to put one of them back in code the rule is about.
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/../.." && pwd -P)
check="$repo_root/scripts/check-file-size.sh"
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
fail() { echo "file-size test: $*" >&2; exit 1; }
mkdir -p "$root/crates/libs/thing/src" "$root/scripts"
baseline="$root/scripts/file-size-baseline.txt"

lines() { awk -v n="$1" 'BEGIN { for (i = 0; i < n; i++) print "fn f() {}" }' > "$2"; }
expect() {  # expect ok|fail <needle or -> <what the case is>
  local want="$1" needle="$2" what="$3" code out
  out=$(SV10_FILE_SIZE_ROOT="$root" "$check" 2>&1) && code=0 || code=$?
  if [ "$want" = ok ]; then
    [ "$code" -eq 0 ] || fail "$what: exit $code — $out"
  else
    [ "$code" -ne 0 ] || fail "$what: the check passed it"
    [ "$needle" = - ] || grep -qF -- "$needle" <<< "$out" || fail "$what: no '$needle' in: $out"
  fi
}

# A listed file at its ceiling is what every entry was written as, and it passes.
lines 600 "$root/crates/libs/thing/src/big.rs"
echo "crates/libs/thing/src/big.rs 600" > "$baseline"
expect ok - "a listed file at its baseline"

# The ceiling holds: one line over is one line over.
lines 601 "$root/crates/libs/thing/src/big.rs"
expect fail "grew to 601 lines" "a listed file over its baseline"

# A file that came back under the limit has left the list. Its entry is a ceiling, so leaving it
# there would re-grant the lines the split gave back — which is the whole point of the check.
lines 480 "$root/crates/libs/thing/src/big.rs"
expect fail "has left the list" "a listed file back under the limit"

# With the line deleted it passes, and stays bounded by the limit like any other file.
: > "$baseline"
expect ok - "a file back under the limit, no longer listed"
lines 520 "$root/crates/libs/thing/src/big.rs"
expect fail "maximum is 500" "a file over the limit and not listed"

# An unlisted file under the limit is the ordinary case, and the note about 400 is not a failure.
lines 430 "$root/crates/libs/thing/src/middling.rs"
rm "$root/crates/libs/thing/src/big.rs"
expect ok - "unlisted files under the limit"

# --report adds the note; --baseline is not exercised here, because it is the operator's command.
out=$(SV10_FILE_SIZE_ROOT="$root" "$check" --report 2>&1) || fail "--report exited non-zero: $out"
grep -qF "consider splitting" <<< "$out" || fail "--report did not note a file over 400 lines: $out"
echo "file-size: ok"
