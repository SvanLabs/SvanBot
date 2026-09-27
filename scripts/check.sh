#!/usr/bin/env bash
# Anti-regression gate (0110). Every commit and release passes it.
#   scripts/check.sh commit   pre-commit hook: placeholder markers, ai provenance, secret scan, rustfmt,
#                             clippy -D warnings, golden snapshot and property tests when crates/ is
#                             staged, tsc when web/ is
#   scripts/check.sh full     release gate: rustfmt, clippy, cargo-deny, docs drift, full workspace tests, tsc
#   scripts/check.sh deep     full + property/fuzz tests at 100k cases (weekly or before big changes)
#   scripts/check.sh lint     rustfmt, clippy, cargo-deny only (release.sh runs its own tests)
# Builds go to $CARGO_TARGET_DIR (default target/dev) so a live fleet's target/release is untouched.
# Tests build with the `gate` profile (0226, 0303: the release settings, `sv10-bot` at opt-level 1;
# results are bit-identical, the golden snapshot proves it on every run) and run in parallel.
# Compilers use every thread at idle CPU priority (0302/0303, operator 2026-09-27): the fleet always
# goes first; the test build overlaps clippy and the other checks.
#
# The AI-provenance step below is part of the gate, not a separate tool: this repository's history
# starts at a commit that carries a `Generated-by:` trailer, and every commit and pull request
# description since has to match it. Enforcing it here rather than in review is deliberate — a
# trailer is self-reported, so the convention only holds if a missing one stops the work.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/dev}"
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}"
mode="${1:-full}"
# Every step reports its time, and a step past two minutes is named (0334, operator 2026-09-27:
# "nothing is worth more than 2 minutes on a live running system").
BUDGET_SECS=120
check_t0=$(date +%s) step_name= step_t=$check_t0
step_end() {
  [ -n "$step_name" ] || return 0
  local took=$(($(date +%s) - step_t))
  [ "$took" -lt 2 ] || printf '   (%s: %d s)\n' "$step_name" "$took"
  [ "$took" -le "$BUDGET_SECS" ] || printf 'check.sh: %s took %d s, over the %d s budget (0334)\n' "$step_name" "$took" "$BUDGET_SECS" >&2
  step_name=
}
step() { step_end; printf '== %s\n' "$*"; step_name="$*"; step_t=$(date +%s); }
fail() { printf 'check.sh: %s\n' "$*" >&2; exit 1; }
# Idle CPU priority: compiles take only what live play leaves.
idle() { if command -v chrt >/dev/null; then nice -n 19 chrt -i 0 "$@"; else nice -n 19 "$@"; fi; }

# No placeholder markers in shipped sources (0238): the four marker words, whole words only (so
# mktemp's X-templates pass), in Rust, TypeScript, Python and shell. Split so this file passes itself.
step "placeholder markers"
markers=$(grep -rnwE "TO""DO|FIX""ME|X""XX|HA""CK" crates web/src scripts \
  --include='*.rs' --include='*.ts' --include='*.tsx' --include='*.py' --include='*.sh' 2>/dev/null || true)
[ -z "$markers" ] || { printf '%s\n' "$markers" | head -20 >&2; fail "placeholder markers in sources: finish the work or remove the marker"; }

# Every commit and every pull request description must name the AI system that produced it: a
# `Generated-by: <tool>/<model>` trailer in the commit footer, and the same line in the report body.
# The rule is stated in CONTRIBUTING.md section 0; this is the step that enforces it.
#
# It checks *declaration*, not authorship. A trailer is self-reported, and a person can add one to
# hand-written code; the gate makes the convention mandatory and visible, and review is what makes it
# true. Do not read a clean run as a guarantee about who wrote the code.
#
# The range is the tip locally, so the pre-commit hook and a plain `check.sh` agree, and the branch
# point in CI, so a pull request is checked over the commits it actually proposes. Both paths have to
# exist: a contributor who is green locally and red on push learns the rule in the worst place.
step "ai provenance"
provenance_range="${SVANBOT_PROVENANCE_RANGE:-HEAD}"
if [ -z "${SVANBOT_PROVENANCE_RANGE:-}" ] && [ -n "${GITHUB_BASE_REF:-}" ]; then
  provenance_range="origin/${GITHUB_BASE_REF}..HEAD"
fi
if [ -n "${SVANBOT_PR_BODY:-}" ]; then
  # CI writes the pull request description to a file and names it; the body is not a file in the tree.
  python3 scripts/provenance.py check --pr-body "$SVANBOT_PR_BODY" "$provenance_range" \
    || fail "provenance: every commit and pull request must name its generating system (CONTRIBUTING.md section 0)"
else
  python3 scripts/provenance.py check "$provenance_range" \
    || fail "provenance: every commit must name its generating system (CONTRIBUTING.md section 0)"
fi

staged_crates=1 staged_web=1
if [ "$mode" = commit ]; then
  step "secret scan (staged)"
  scripts/pre-commit-secret-scan.sh || fail "secret scan"
  git diff --cached --name-only | grep -qE '^(crates/|Cargo\.(toml|lock)|deny\.toml)' || staged_crates=0
  # Anything under web/, not just web/src/: the specs and the web root's config files are type-checked
  # too, and a change confined to them must not skip the check that covers them.
  git diff --cached --name-only | grep -q '^web/' || staged_web=0
fi

# The full gate's test build starts now and overlaps everything up to the test run.
test_build_pid=
if [ "$mode" = full ] || [ "$mode" = deep ]; then
  mkdir -p "$CARGO_TARGET_DIR"
  idle cargo test --no-run --profile gate --workspace -q >"$CARGO_TARGET_DIR/check-build.log" 2>&1 &
  test_build_pid=$!
  # A check that fails early must not leave the build running behind it.
  trap '[ -z "$test_build_pid" ] || kill "$test_build_pid" 2>/dev/null || true' EXIT
fi

if [ "$staged_crates" = 1 ]; then
  step "rustfmt"
  cargo fmt --all --check >/dev/null || fail "rustfmt differs: run cargo fmt --all"
  step "file sizes (CONTRIBUTING.md: 500-line limit, baseline may only shrink)"
  scripts/check-file-size.sh || fail "a Rust file is over 500 lines or an oversized file grew"
  step "clippy -D warnings"
  idle cargo clippy --release --workspace --all-targets -q -- -D warnings || fail "clippy"
fi

case "$mode" in
  commit)
    if [ "$staged_crates" = 1 ]; then
      step "golden snapshot + property tests"
      idle cargo test --profile gate -q -p sv10-core --test characterization --test properties >/dev/null || fail "golden snapshot or property tests (rerun without -q to see)"
    fi
    ;;
  full|deep|lint)
    step "cargo-deny (licenses, bans, sources)"
    cargo deny check licenses bans sources 2>/dev/null || fail "cargo-deny"
    step "cargo-deny advisories"
    cargo deny check advisories 2>/dev/null || echo "   advisories failed or the RustSec database was unreachable; see: cargo deny check advisories"
    step "third-party notices current"
    python3 scripts/notices.py --check || fail "run scripts/notices.py and commit THIRD-PARTY-NOTICES.md"
    step "tickets lint + tool tests (tickets, codec vs zlib, release/rollback, keepalive, update)"
    python3 scripts/tickets.py lint >/dev/null || { python3 scripts/tickets.py lint | tail -20 >&2; fail "tickets lint (scripts/tickets.py lint --fix fixes edges and types)"; }
    # The codec test builds a small example with cargo: while the full gate's test build holds the
    # build directory it would wait on that lock (57 s of an 85 s gate, 0334), so it runs after it.
    pack_test=scripts/tests/test_pack.py
    [ -z "$test_build_pid" ] || pack_test=
    python3 -m unittest -q scripts/tests/test_tickets.py scripts/tests/test_fleet_check.py scripts/tests/test_monitor.py $pack_test scripts/tests/test_docs_check.py scripts/tests/test_progress.py scripts/tests/test_provenance.py 2>/dev/null || fail "ticket, fleet-check, codec, docs-check and provenance tool tests (python3 -m unittest scripts/tests/test_pack.py ...)"
    step "docs name only paths and commands that exist"
    python3 scripts/docs-check.py 2>/dev/null || { python3 scripts/docs-check.py | head -20 >&2; fail "docs drift (scripts/docs-check.py)"; }
    bash scripts/tests/release-rollback.sh >/dev/null 2>&1 || fail "release/rollback tests (run bash scripts/tests/release-rollback.sh)"
    bash scripts/tests/keepalive.sh >/dev/null 2>&1 || fail "keepalive tests (run bash scripts/tests/keepalive.sh)"
    bash scripts/tests/update.sh >/dev/null 2>&1 || fail "update tests (run bash scripts/tests/update.sh)"
    [ "$mode" = lint ] && { step_end; step "ok (lint, $(($(date +%s) - check_t0)) s)"; exit 0; }
    [ "$mode" = deep ] && export SV10_PROP_CASES=100000
    step "workspace tests${SV10_PROP_CASES:+ (property cases $SV10_PROP_CASES)}"
    log="$CARGO_TARGET_DIR/check-tests.log"
    mkdir -p "$CARGO_TARGET_DIR"
    if [ -n "$test_build_pid" ] && ! wait "$test_build_pid"; then
      tail -30 "$CARGO_TARGET_DIR/check-build.log" >&2
      fail "test build (full log: $CARGO_TARGET_DIR/check-build.log)"
    fi
    [ -n "$pack_test" ] || python3 -m unittest -q scripts/tests/test_pack.py 2>/dev/null || fail "codec tool test (python3 -m unittest scripts/tests/test_pack.py)"
    if ! idle python3 scripts/test.py --profile gate -q >"$log" 2>&1; then
      grep -E "^(---- |test result: FAILED|thread .* panicked|test.py: )" "$log" | head -20 >&2
      fail "tests (full log: $log)"
    fi
    tail -1 "$log"
    ;;
  *) fail "unknown mode $mode (commit | full | deep | lint)" ;;
esac

if [ "$staged_web" = 1 ] && [ -d web/node_modules ]; then
  step "tsc"
  # Both configs: tsconfig.json is the browser app (`src`), tsconfig.tests.json is the Playwright
  # specs and the web root's Node-side config files. Running only the first is what let the specs
  # drift unchecked, so the two are one step and one command.
  (cd web && npm run typecheck --silent) || fail "tsc"
fi
step_end
total=$(($(date +%s) - check_t0))
[ "$total" -le "$BUDGET_SECS" ] || printf 'check.sh: %s took %d s in all, over the %d s budget (0334)\n' "$mode" "$total" "$BUDGET_SECS" >&2
step "ok ($mode, $total s)"
