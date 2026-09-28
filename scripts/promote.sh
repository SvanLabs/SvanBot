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
# argument in full. `.github/workflows/promote.yml` runs this on a green `check` on `dev`.
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

if [ "$main" = "$dev" ]; then
  echo "promote: main is already at dev ($(git rev-parse --short "$dev")); nothing to promote"
  exit 0
fi

# The promotion is a merge commit that changes no file, and that is only true while `main` is an
# ancestor of `dev`. The moment the two lines have a commit the other does not — a squash promotion
# would do it, and so would a commit pushed straight to `main` — the merge is a real merge with a
# conflict, which is a person's decision and not this script's.
if ! git merge-base --is-ancestor "$main" "$dev"; then
  echo "promote: main is not an ancestor of dev, so the two lines have diverged and this is not a" >&2
  echo "promote: promotion but a merge to resolve by hand; nothing changed" >&2
  exit 1
fi

commits=$(git log --oneline --no-decorate "$main..$dev")
count=$(printf '%s\n' "$commits" | wc -l)

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
    echo 'Opened by `scripts/promote.sh` (#370), which `.github/workflows/promote.yml` runs on a'
    echo 'green `check` on `dev`. Auto-merge is armed, so this merges itself once `check` passes on'
    echo 'it; a red `dev` never gets here.'
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
