#!/usr/bin/env bash
# Tests for scripts/promote.sh (#370), hermetic: a bare "GitHub" origin holding `dev` and `main`, and
# a stub `gh` that records what the script asked it to do. No network.
#
# What matters here is not that the pull request is opened — that is a `gh` call — but the four
# decisions around it: that a red or caught-up `main` promotes nothing, that a diverged `main` is
# refused rather than merged, that an open promotion pull request is reused instead of duplicated,
# and that auto-merge is armed with a merge commit and never a squash.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail() { echo "promote test: $*" >&2; exit 1; }
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
unset SVANBOT_PROMOTE_REPO

git init -q --bare --initial-branch=main "$t/origin.git"
git clone -q "$t/origin.git" "$t/gen" 2>/dev/null
echo one > "$t/gen/a"
git -C "$t/gen" add -A
git -C "$t/gen" commit -qm one
git -C "$t/gen" -c push.negotiate=false push -q origin HEAD:main
git -C "$t/gen" checkout -q -b dev
echo two > "$t/gen/b"
git -C "$t/gen" add -A && git -C "$t/gen" commit -qm two
git -C "$t/gen" -c push.negotiate=false push -q origin dev
git clone -q "$t/origin.git" "$t/work"
# The script under test, in the checkout it runs from: it resolves its own root from `$0`, so it has
# to sit where a checkout of this repository would have it.
mkdir -p "$t/work/scripts"
cp "$repo/scripts/promote.sh" "$t/work/scripts/"

# A stub `gh`: it answers the four subcommands the script uses and appends every call to `$t/calls`
# so the test can assert what was asked for, not only what came back.
mkdir -p "$t/bin" "$t/state"
cat > "$t/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
echo "$*" >> "$TEST_CALLS"
case "$1 $2" in
  "repo view") echo "${TEST_REPO:-SvanLabs/SvanBot}" ;;
  "pr list")   [ -f "$TEST_STATE/pr" ] && cat "$TEST_STATE/pr" || true ;;
  "pr create")
    # GitHub's own rule, and the answer the real service is on record giving: a pull request whose
    # head has no commit the base lacks is refused outright, `No commits between main and dev`
    # (createPullRequest), observed on 2026-09-28 04:28. A stub that answers success for whatever it
    # is asked is a *more capable* GitHub than the one this script talks to, and a suite standing on
    # it cannot tell a script that opens a valid promotion pull request from one that asks for a
    # pull request it will never get. The fixture has both branches, so the stub can know what
    # GitHub knows. See docs/LESSONS.md 44 for the same mistake from the other side.
    body="" head="" base=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --body-file) body="$2" ;;
        --head) head="$2" ;;
        --base) base="$2" ;;
      esac
      shift
    done
    [ -n "$body" ] || { echo "stub gh: pr create without --body-file" >&2; exit 9; }
    [ -n "$head" ] && [ -n "$base" ] || { echo "stub gh: pr create without --head/--base" >&2; exit 9; }
    if [ "$(git -C "${TEST_GIT_DIR:-$PWD}" rev-list --count "origin/$head" --not "origin/$base")" = 0 ]; then
      echo "pull request create failed: GraphQL: No commits between $base and $head (createPullRequest)" >&2
      exit 1
    fi
    echo 377 > "$TEST_STATE/pr"
    cp "$body" "$TEST_STATE/body"
    echo "https://example.invalid/pull/377" ;;
  "pr view")   [ "$(cat "$TEST_STATE/armed" 2>/dev/null || echo false)" = true ] && echo true || echo false ;;
  "pr merge")  echo true > "$TEST_STATE/armed" ;;
  *) echo "stub gh: unhandled: $*" >&2; exit 9 ;;
esac
STUB
chmod +x "$t/bin/gh"

run() { (cd "$t/work" && PATH="$t/bin:$PATH" TEST_CALLS="$t/calls" TEST_STATE="$t/state" \
  TEST_GIT_DIR="$t/work" "${@:2}" bash scripts/promote.sh "${1:-}"); }
# Only the call log: `$t/state` is the stand-in for GitHub, and a pull request that was opened
# stays open across runs unless a test means to change it.
reset() { rm -f "$t/calls"; }
armed() { cat "$t/state/armed" 2>/dev/null || echo false; }
calls() { cat "$t/calls" 2>/dev/null || true; }
# The calls that change something. `gh repo view` and `gh pr list` are the script reading, and every
# run makes them.
mutations() { calls | grep -E '^(pr create|pr merge)' || true; }

# 1. `main` is at `dev`: nothing to promote, and nothing is changed.
git -C "$t/gen" -c push.negotiate=false push -qf origin dev:main
git -C "$t/work" fetch -q origin
run "" >/dev/null || fail "a caught-up main failed"
[ -z "$(mutations)" ] || fail "a caught-up main changed something: $(mutations)"

# 2. `dev` is ahead: the pull request is opened, its body lists the commits, and auto-merge is armed
#    with --merge. The squash check is the point — a squash promotion is what breaks the next one.
echo three > "$t/gen/d" && git -C "$t/gen" add -A && git -C "$t/gen" commit -qm three
git -C "$t/gen" -c push.negotiate=false push -q origin dev
git -C "$t/work" fetch -q origin
reset
out=$(run "") || fail "opening the promotion pull request failed"
[ "$(cat "$t/state/pr")" = 377 ] || fail "no pull request was opened"
[ "$(armed)" = true ] || fail "auto-merge was not armed"
grep -q -- "--auto --merge" "$t/calls" || fail "auto-merge was not armed with --merge: $(calls)"
if grep -q -- "--squash" "$t/calls"; then fail "the promotion was armed with --squash"; fi
[ "$(grep -c '^pr create' "$t/calls")" = 1 ] || fail "more than one pull request was opened"
body=$(cat "$t/state/body")
grep -q "commit(s) on \`dev\` that \`main\` does not have" <<<"$body" || fail "the body does not list the commits: $body"
grep -q "^Generated-by: " <<<"$body" || fail "the body names no generating system, so a person's run fails the gate: $body"
case "$out" in *"now merges itself when check passes"*) ;; *) fail "unexpected output: $out";; esac

# 3. Run again: the pull request is reused. A second one would be a second path to the same merge.
reset
out=$(run "") || fail "the second run failed"
[ -z "$(calls | grep '^pr create' || true)" ] || fail "a second pull request was opened: $(calls)"
case "$out" in *"already open: #377"*) ;; *) fail "the open pull request was not reported: $out";; esac

# 4. Open but not armed — a run that failed between the two calls — arms it.
reset
rm -f "$t/state/armed"
run "" >/dev/null || fail "arming an existing pull request failed"
[ "$(armed)" = true ] || fail "an unarmed open pull request was left unarmed"

# 5. `main` has a commit `dev` does not: this is not a promotion, and merging would be a real merge.
reset
git -C "$t/gen" fetch -q origin
git -C "$t/gen" checkout -q -B main origin/main && echo hotfix > "$t/gen/c" && git -C "$t/gen" add -A
git -C "$t/gen" commit -qm hotfix && git -C "$t/gen" -c push.negotiate=false push -q origin main
git -C "$t/gen" checkout -q dev
git -C "$t/work" fetch -q origin
if run "" >/dev/null 2>"$t/err"; then fail "a diverged main was promoted"; fi
grep -q "that dev does not" "$t/err" || fail "the refusal did not say why: $(cat "$t/err")"
[ -z "$(mutations)" ] || fail "a diverged main changed something: $(mutations)"

# 6. `--check` reports and changes nothing. First heal the divergence test 5 left, then move `dev`
#    on, so there is a plan to report.
git -C "$t/gen" checkout -q dev
git -C "$t/gen" -c push.negotiate=false push -qf origin dev:main
echo three-and-a-bit > "$t/gen/f" && git -C "$t/gen" add -A && git -C "$t/gen" commit -qm three-and-a-bit
git -C "$t/gen" -c push.negotiate=false push -q origin dev
git -C "$t/work" fetch -q origin
reset
before=$(armed)
out=$(run --check) || fail "--check failed"
case "$out" in *"would arm auto-merge on #377"*) ;; *) fail "--check did not report the plan: $out";; esac
[ "$(armed)" = "$before" ] || fail "--check changed whether auto-merge is armed"
[ -z "$(mutations)" ] || fail "--check changed something: $(mutations)"

# 7. The repository comes from `gh repo view` when `SVANBOT_PROMOTE_REPO` is unset — a person running
#    this by hand sets nothing — and every call carries it.
echo four > "$t/gen/e" && git -C "$t/gen" add -A && git -C "$t/gen" commit -qm four
git -C "$t/gen" -c push.negotiate=false push -q origin dev
git -C "$t/work" fetch -q origin
reset
run "" >/dev/null || fail "the run without the override failed"
grep -q "^repo view" "$t/calls" || fail "gh repo view was not consulted: $(calls)"
grep -q "^pr list --repo SvanLabs/SvanBot " "$t/calls" || fail "the repository gh repo view named was not used: $(calls)"

# 8. The shape every real promotion leaves, and the one case 1–7 never built: `main`'s tip is itself
#    a merge commit of `dev`. Then `main` is *not* an ancestor of `dev` and never will be again, so a
#    guard phrased as "main must be an ancestor of dev" refuses every promotion after the first —
#    which is what #378 fixed. The fixture has to assert it reproduced that shape, or the test would
#    pass on the very guard it exists to catch.
reset
# A fresh GitHub: no promotion pull request open, so this run has to open one and its body is test 8's.
rm -f "$t/state/pr" "$t/state/armed"
git -C "$t/gen" fetch -q origin
git -C "$t/gen" checkout -q -B main origin/main
git -C "$t/gen" merge -q --no-ff --no-edit -m "Merge pull request #373 from SvanLabs/dev" origin/dev
git -C "$t/gen" -c push.negotiate=false push -q origin main
# `main` is now that merge commit, and `dev` moves on by one — the state this repository was in when
# promotion was first exercised for real.
git -C "$t/gen" checkout -q dev
echo five > "$t/gen/g" && git -C "$t/gen" add -A && git -C "$t/gen" commit -qm five
git -C "$t/gen" -c push.negotiate=false push -q origin dev
git -C "$t/work" fetch -q origin
if git -C "$t/work" merge-base --is-ancestor origin/main origin/dev; then
  fail "the fixture did not reproduce the real shape: main is still an ancestor of dev"
fi
if ! out=$(run ""); then fail "a promotion merge commit on main was refused: $out"; fi
[ "$(armed)" = true ] || fail "auto-merge was not armed from the real main shape"
grep -q "commit(s) on \`dev\` that \`main\` does not have" "$t/state/body" ||
  fail "the body does not list the commits: $(cat "$t/state/body")"

# 9. `main` holds what `dev` holds — the state a promotion leaves — and the two tips are different
#    commits. That is what makes the check a tree comparison and not an id one: by id this state reads
#    as "not current" on every run forever, so the script opened a promotion pull request with an empty
#    commit list in it (the empty list counted as one line) every time nothing had moved.
reset
# A fresh GitHub, as a run after the last promotion has merged finds it: the previous run's pull
# request is closed, so a script that does not recognise this state opens a new one rather than
# reusing it, and the mutation is the thing the test is looking for.
rm -f "$t/state/pr" "$t/state/armed"
git -C "$t/gen" fetch -q origin
git -C "$t/gen" checkout -q -B main origin/main
git -C "$t/gen" merge -q --no-ff --no-edit -m "Merge pull request #380 from SvanLabs/dev" origin/dev
git -C "$t/gen" -c push.negotiate=false push -q origin main
git -C "$t/gen" checkout -q dev
git -C "$t/work" fetch -q origin
if [ "$(git -C "$t/work" rev-parse origin/main)" = "$(git -C "$t/work" rev-parse origin/dev)" ]; then
  fail "the fixture did not reproduce the real shape: the two tips are the same commit"
fi
[ -z "$(git -C "$t/work" diff origin/main origin/dev)" ] ||
  fail "the fixture did not reproduce the real shape: the two trees differ"
out=$(run "") || fail "a main holding what dev holds failed: $out"
[ -z "$(mutations)" ] || fail "an already-current main changed something: $(mutations)"
case "$out" in *"already holds what dev holds"*) ;; *) fail "an already-current main was not recognised: $out";; esac

# 10. `main` carries content of its own, and every commit on `dev` is already on it. That is a
#     conflict resolved on `main` instead of on `dev`: the merge commit itself holds the resolution,
#     so the commit that carries the content is a merge and the stray check — which counts non-merge
#     commits only — cannot see it. The trees differ, so the currency check does not fire either.
#     There is nothing to promote: `dev` has no commit `main` lacks, which is precisely the state
#     GitHub refuses with `No commits between main and dev`. A script that asks anyway gets a red
#     run and no information, and before this case the stub answered it as success, so the suite
#     could not have told.
reset
rm -f "$t/state/pr" "$t/state/armed"
git -C "$t/gen" fetch -q origin
# `dev` moves on, so the merge below has something to bring in and really is a merge.
git -C "$t/gen" checkout -q dev
echo six > "$t/gen/i" && git -C "$t/gen" add -A && git -C "$t/gen" commit -qm six
git -C "$t/gen" -c push.negotiate=false push -q origin dev
# `main` merges `dev`, and the merge commit carries a file `dev` does not have — what resolving the
# conflict on the wrong side leaves behind.
git -C "$t/gen" checkout -q -B main origin/main
git -C "$t/gen" merge -q --no-ff --no-commit origin/dev
echo resolved-on-main > "$t/gen/both" && git -C "$t/gen" add -A
git -C "$t/gen" commit -qm "Merge branch 'dev' into main"
git -C "$t/gen" -c push.negotiate=false push -q origin main
git -C "$t/gen" checkout -q dev
git -C "$t/work" fetch -q origin
# The fixture asserts it built that shape before anything is said about the script: `dev` on `main`
# (nothing to promote), no stray commit, and two trees that still differ. Any one of the three being
# wrong means this case would pass without exercising the state it exists for.
[ "$(git -C "$t/work" rev-list --count origin/dev --not origin/main)" = 0 ] ||
  fail "the fixture did not reproduce the real shape: dev has a commit main does not"
[ "$(git -C "$t/work" rev-list --no-merges --count origin/main --not origin/dev)" = 0 ] ||
  fail "the fixture did not reproduce the real shape: main carries a stray non-merge commit"
[ -n "$(git -C "$t/work" diff origin/main origin/dev)" ] ||
  fail "the fixture did not reproduce the real shape: the two trees are equal"
# And that the stub now stands in for a GitHub that refuses this pull request, which is the half of
# this case that is about the fixture rather than the script.
if (cd "$t/work" && PATH="$t/bin:$PATH" TEST_CALLS="$t/direct-calls" TEST_STATE="$t/state" \
    TEST_GIT_DIR="$t/work" gh pr create --repo SvanLabs/SvanBot --base main --head dev \
    --title t --body-file /dev/null) 2>"$t/err"; then
  fail "the stub accepted a pull request GitHub refuses"
fi
grep -q "No commits between main and dev" "$t/err" || fail "the stub refused with the wrong reason: $(cat "$t/err")"
if run "" >/dev/null 2>"$t/err"; then fail "a main carrying its own content was promoted"; fi
[ -z "$(mutations)" ] || fail "a main carrying its own content was sent to pr create: $(mutations)"
grep -q "content of its own" "$t/err" || fail "the refusal did not say why: $(cat "$t/err")"
grep -q "both" "$t/err" || fail "the refusal did not name the file main carries: $(cat "$t/err")"

# 11. The same state as 10, one push later: `main` still carries the resolution inside its merge, and
#     `dev` has moved on since. Now both of the old guards pass — `dev` has a commit `main` lacks, so
#     the count is nonzero, and the content sits in a merge, so the stray count is zero — and the
#     script promoted (#387): a real merge that keeps `main`'s resolution on `main`'s side alone and
#     leaves the two trees different for good, however green `dev` is. Unlike 10, GitHub does not
#     refuse this pull request either, so the guard is the only thing standing between this state and
#     a bad promotion, and its refusal is the whole of the evidence.
reset
rm -f "$t/state/pr" "$t/state/armed"
git -C "$t/gen" fetch -q origin
git -C "$t/gen" checkout -q dev
echo seven > "$t/gen/j" && git -C "$t/gen" add -A && git -C "$t/gen" commit -qm seven
git -C "$t/gen" -c push.negotiate=false push -q origin dev
git -C "$t/work" fetch -q origin
# The fixture asserts it built that shape before anything is said about the script: `dev` exactly one
# commit ahead of `main`, no stray non-merge commit on `main`, two trees that differ, and a file
# `main` holds that `dev` has never had. Any one of the four being wrong means this case would pass
# without exercising the state it exists for.
[ "$(git -C "$t/work" rev-list --count origin/dev --not origin/main)" = 1 ] ||
  fail "the fixture did not reproduce the real shape: dev is not one commit ahead of main"
[ "$(git -C "$t/work" rev-list --no-merges --count origin/main --not origin/dev)" = 0 ] ||
  fail "the fixture did not reproduce the real shape: main carries a stray non-merge commit"
[ -n "$(git -C "$t/work" diff origin/main origin/dev)" ] ||
  fail "the fixture did not reproduce the real shape: the two trees are equal"
git -C "$t/work" cat-file -e origin/main:both 2>/dev/null ||
  fail "the fixture did not reproduce the real shape: main does not carry the resolved file"
if git -C "$t/work" cat-file -e origin/dev:both 2>/dev/null; then
  fail "the fixture did not reproduce the real shape: dev has the file main resolved on its own side"
fi
# And the half of this case that is about the fixture rather than the script: this is a pull request
# GitHub accepts, so a red `promote` run is not what stops it.
if (cd "$t/work" && PATH="$t/bin:$PATH" TEST_CALLS="$t/direct-calls" TEST_STATE="$t/state" \
    TEST_GIT_DIR="$t/work" gh pr create --repo SvanLabs/SvanBot --base main --head dev \
    --title t --body-file /dev/null) >/dev/null 2>"$t/err"; then
  rm -f "$t/state/pr"
else
  fail "the stub refused a pull request GitHub accepts, so this case is not the real one: $(cat "$t/err")"
fi
if run "" >/dev/null 2>"$t/err"; then
  fail "a main carrying its own content was promoted once dev had moved on"
fi
[ -z "$(mutations)" ] || fail "a main carrying its own content was sent to pr create: $(mutations)"
grep -q "content of its own" "$t/err" || fail "the refusal did not say why: $(cat "$t/err")"
grep -q "both" "$t/err" || fail "the refusal did not name the file main carries: $(cat "$t/err")"

echo "promote tests: ok"
