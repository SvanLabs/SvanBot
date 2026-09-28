#!/usr/bin/env bash
# Promote `dev` to `main` (#370).
#
# `main` is the released line: the branch a stranger installs from and the branch their Update
# fetches. It is meant to hold what `dev` holds whenever `dev` is green, and nothing made it so —
# promotion was a hand-opened pull request, so `main` moved when someone remembered. It moved once,
# on 2026-09-28, and was behind again within the hour.
#
# This keeps one promotion pull request open with auto-merge armed. The merge ref of a pull request
# is recomputed on every push to its head, so that one pull request always tests the head `dev` has
# now; its check goes green, auto-merge fires, and `main` moves. Nothing here can merge a red `dev`:
# the pull request is merged by the same `check` the ruleset requires.
#   scripts/promote.sh            open or reuse the promotion pull request, arm auto-merge
#   scripts/promote.sh --check    print what it would do and change nothing
#
# A merge commit, never a squash: a squash gives `main` a commit `dev` does not have, so the next
# promotion is a real merge with a conflict to resolve instead of a no-op. `docs/RELEASE.md` is the
# argument in full. `.github/workflows/promote.yml` runs this whenever a `check` on `dev` finishes,
# whatever it concluded: this script opens and arms, and the gate on the pull request it opens is what
# decides whether `main` moves.
#
# Repository: SVANBOT_PROMOTE_REPO, or what `gh repo view` reports. Needs `gh` with a token that can
# write contents and pull requests on it.
set -euo pipefail
cd "$(dirname "$0")/.."

dry=0
case "${1:-}" in
  "") ;;
  --check) dry=1 ;;
  *) echo "promote: usage: scripts/promote.sh [--check]" >&2; exit 2 ;;
esac

repo="${SVANBOT_PROMOTE_REPO:-$(gh repo view --json nameWithOwner -q .nameWithOwner)}"

git fetch --quiet origin dev main
dev=$(git rev-parse origin/dev)
main=$(git rev-parse origin/main)

# Content, not commit id. A promotion is a merge commit, so `main`'s tip is a commit `dev`'s tip
# never equals again after the first one, and comparing the two ids reads as "not current" on every
# run forever — including the run where `main` holds exactly what `dev` holds. What "nothing to
# promote" means is that the two trees are equal: a merge commit into this `main` would change no
# file and add nothing, wherever the two tips are. `git diff --quiet` is that comparison.
if git diff --quiet "$main" "$dev"; then
  echo "promote: main already holds what dev holds ($(git rev-parse --short "$dev")); nothing to promote"
  exit 0
fi

# The promotion is a merge commit that changes no file, and that is only true while `main` carries
# nothing of its own. One question settles it: has `main` added anything since the two lines last
# agreed?
#
#   [ "$(git rev-parse "$main^{tree}")" = "$(git rev-parse "$(git merge-base "$main" "$dev")^{tree}")" ]
#
# A promotion merge holds `dev`'s tree exactly — that is what "changes no file" means — and that `dev`
# is the next promotion's merge base, so a healthy `main` passes. Anything `main` carries that `dev`
# does not makes the trees differ, and it does not matter which shape it takes or which commit holds
# it: a non-merge commit `dev` lacks (a squash promotion, or a commit pushed straight to `main`), a
# conflict resolved on `main`'s side inside a merge (#386), or that same resolution with `dev` moved
# on since (#387). The commit-id answer would be wrong — the two tips are different commits forever
# after the first promotion — and so is an ancestor test, which refuses every promotion after the
# first, which is every promotion there will be.
#
# What reaches the refusal is that `main` holds content `dev`'s line never had, and only a person can
# say whether it belongs on `dev` (a fix that never went there) or on the floor (a resolution that
# should have gone the other way). The three messages below are that one refusal, split by shape, so
# each names what is actually there.
#
# `git merge-base` answers nothing for two branches with no common history at all, which is what
# replacing `main` with an unrelated line leaves. That is a divergence too, and it is refused here
# rather than left to fail on a `^{tree}` of the empty string.
base=$(git merge-base "$main" "$dev") || {
  echo "promote: main and dev share no commit at all, so this is not a promotion but a replacement of" >&2
  echo "promote: one line by another; nothing changed, and which of the two is the real one is a" >&2
  echo "promote: person's to say." >&2
  exit 1
}

# `grep -c .` and not `wc -l`: `printf '%s\n' ""` is one empty line, so an empty list counts as 1 and
# the pull request says "1 commit(s)" over nothing. The early return above makes that unreachable for
# a current `main`, and a count that is wrong on the empty list is still wrong to leave in.
commits=$(git log --oneline --no-decorate "$dev" --not "$main")
count=$(printf '%s' "$commits" | grep -c . || true)

if [ "$(git rev-parse "$main^{tree}")" != "$(git rev-parse "$base^{tree}")" ]; then
  # A non-merge commit on `main` that `dev` does not have: a squash promotion, or a commit pushed
  # straight to `main`. The two lines have diverged, and the next promotion is a real merge with a
  # conflict to resolve, which is a person's decision and not this script's.
  strays=$(git rev-list --no-merges --count "$main" --not "$dev")
  if [ "$strays" != 0 ]; then
    echo "promote: main carries $strays commit(s) that dev does not (a squash promotion, or a commit" >&2
    echo "promote: pushed straight to main), so the two lines have diverged and this is not a promotion" >&2
    echo "promote: but a merge to resolve by hand; nothing changed. They are:" >&2
    git log --no-merges --format='promote:   %h %s' "$main" --not "$dev" | head -5 >&2
    exit 1
  fi

  # No non-merge commit of `main`'s own, so the content is inside a merge: a conflict resolved on
  # `main`'s side. Every commit on `dev` is already on `main` in the first shape — which is also the
  # state GitHub refuses outright, `No commits between main and dev` (createPullRequest), a red
  # `promote` run and no information — and `dev` has moved on since in the second, which is an
  # ordinary-looking pull request whose merge keeps `main`'s content on `main`'s side alone.
  if [ "$count" = 0 ]; then
    echo "promote: every commit on dev is already on main and the two trees still differ, so main" >&2
    echo "promote: carries content of its own and this is not a promotion but a divergence to resolve" >&2
    echo "promote: by hand; nothing changed. The two trees differ on:" >&2
    git diff --name-only "$main" "$dev" | head -5 | sed 's/^/promote:   /' >&2
    exit 1
  fi
  echo "promote: main carries content of its own — a conflict resolved on main's side — and dev has" >&2
  echo "promote: moved on since, so this is not a promotion: the merge would keep that content on" >&2
  echo "promote: main's side alone, where dev never reaches it, and leave the two trees different" >&2
  echo "promote: for good. Nothing changed. main added, since $(git rev-parse --short "$base"):" >&2
  git diff --name-only "$base" "$main" | head -5 | sed 's/^/promote:   /' >&2
  exit 1
fi

# `// empty`, because `.[0].number` alone prints the four characters `null` for an empty list, which
# everything below would read as a pull request number.
open_pr() {
  gh pr list --repo "$repo" --head dev --base main --state open --json number --jq '.[0].number // empty'
}

number=$(open_pr)
if [ -z "$number" ]; then
  if [ "$dry" = 1 ]; then
    echo "promote: would open the promotion pull request for $count commit(s)"
    exit 0
  fi
  body=$(mktemp)
  {
    echo 'Promotes `dev` to `main`: a merge commit, which by definition changes no file.'
    echo
    echo "$count commit(s) on \`dev\` that \`main\` does not have:"
    echo
    echo '```'
    printf '%s\n' "$commits" | head -30
    if [ "$count" -gt 30 ]; then
      echo "... and $((count - 30)) more"
    fi
    echo '```'
    echo
    echo 'Opened by `scripts/promote.sh` (#370), which `.github/workflows/promote.yml` runs whenever a'
    echo '`check` on `dev` finishes. Auto-merge is armed, so this merges itself once `check` passes on'
    echo 'it — the ruleset on `main` requires that check, so a `dev` that failed the gate never lands.'
    echo
    # A person running this by hand authors the pull request, and the provenance gate exempts only an
    # author whose name ends in `[bot]`. Without this line the promotion pull request they opened
    # fails the gate on its description, which is the one artifact here a script wrote.
    echo "Generated-by: scripts/promote.sh"
  } > "$body"
  gh pr create --repo "$repo" --base main --head dev --title "release: promote dev to main" --body-file "$body"
  rm -f "$body"
  number=$(open_pr)
  [ -n "$number" ] || { echo "promote: the promotion pull request was created but cannot be found; nothing armed" >&2; exit 1; }
else
  echo "promote: the promotion pull request is already open: #$number"
fi

if [ "$dry" = 1 ]; then
  echo "promote: would arm auto-merge on #$number"
  exit 0
fi

if [ "$(gh pr view "$number" --repo "$repo" --json autoMergeRequest --jq '.autoMergeRequest != null')" = "true" ]; then
  echo "promote: #$number already merges itself when check passes"
else
  gh pr merge "$number" --repo "$repo" --auto --merge
  echo "promote: #$number now merges itself when check passes"
fi
