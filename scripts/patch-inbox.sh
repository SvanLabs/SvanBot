#!/usr/bin/env bash
# The patch inbox (design: issue #710; protocol: .claude/skills/ship-patch/SKILL.md). Agents drop
# one directory per submission under artifacts/patches/; this script is the machine that lands one:
# shape preflight, apply-check, apply, the full gate, provenance, branch and pull request — in a
# dedicated worktree cut from origin/main, never in the fleet's checkout.
#
#   scripts/patch-inbox.sh list
#   scripts/patch-inbox.sh land <entry>|--all [--dry-run]
#
# Tray:   artifacts/patches/<issue>-<slug>/*.patch   (a series stays in numeric order)
# Log:    artifacts/patches/patch-inbox.log          (append-only decisions)
# Failed: the entry is kept with gate.log next to it; landed and rejected entries are deleted.
# The gate command is scripts/check.sh full, run in the landing worktree — never overridable here.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD
tray=artifacts/patches
log=$tray/patch-inbox.log
landing=$root/.scratch/patch-inbox-landing
lock=$tray/.landing.lock

die() { printf 'patch-inbox: %s\n' "$*" >&2; exit 1; }
note() { printf '%s %s\n' "$(date +%FT%T%:z)" "$*" >> "$log"; }

usage() {
  cat >&2 <<'EOF'
usage: scripts/patch-inbox.sh list
       scripts/patch-inbox.sh land <entry>|--all [--dry-run]

Lands patch submissions from artifacts/patches/ (design: issue #710). The landing worktree is
.scratch/patch-inbox-landing, reset to origin/main per run; the gate is scripts/check.sh full.
EOF
  exit 2
}

# The issue a patch names, from its own closing paragraph; empty when the shape check has not run.
issue_of() { sed -n 's/^Closes #\([0-9]\{1,\}\)$/\1/p' "$1" | head -1; }

# Shape preflight on the patch text. `git mailinfo` writes the body to the msg file and the headers
# (Subject among them) to stdout; parsing into the caller's scratch dir. Reject-reason on stdout;
# exit 1 when the shape is wrong. Form only — the crate-area list stays CONTRIBUTING's.
preflight() {
  local file=$1 tmp=$2 msg subject last info
  if ! info=$(git -C "$landing" mailinfo "$tmp/msg" "$tmp/diff" < "$file" 2>/dev/null); then
    echo "not a git format-patch mailbox file"
    return 1
  fi
  msg=$tmp/msg
  subject=$(printf '%s\n' "$info" | sed -n 's/^Subject: //p' | head -1)
  if ! [[ $subject =~ ^[a-z][a-z0-9_-]*:\ .+ ]]; then
    echo "subject is not '<area>: <what it does>': $subject"
    return 1
  fi
  if [ "${#subject}" -gt 72 ]; then
    echo "subject is over 72 characters (${#subject})"
    return 1
  fi
  if ! grep -qE '^Closes #[0-9]+$' "$msg"; then
    echo "no 'Closes #<issue>' paragraph of its own"
    return 1
  fi
  last=$(grep -v '^[[:space:]]*$' "$msg" | tail -1)
  if [[ ! $last =~ ^(Generated-by|Co-Authored-By):\  ]] || ! grep -qE '^Generated-by: ' "$msg"; then
    echo "no 'Generated-by: <tool>/<model>' trailer in the footer"
    return 1
  fi
  if grep -qwE 'TO'"DO|FIX"'ME|X'"XX|HA"'CK' "$msg"; then
    echo "placeholder marker in the commit message"
    return 1
  fi
  return 0
}

reject() {
  local entry=$1 name=$2 why=$3 msg=$4 issue=""
  [ -f "$msg" ] && issue=$(sed -n 's/^Closes #\([0-9]\{1,\}\)$/\1/p' "$msg" | head -1)
  [ -n "$issue" ] && gh issue comment "$issue" --body "patch-inbox: rejected \`$name\` — $why. Rebase on origin/main and re-export; never hand-edit the patch to fit.

Generated-by: scripts/patch-inbox.sh" >/dev/null || true
  note "rejected $name — $why"
  rm -rf "$entry"
  printf 'rejected %s: %s\n' "$name" "$why"
}

land_one() {
  local name=$1 dry=$2 entry="$root/$tray/$1"
  [ -d "$entry" ] || die "no such entry: $name"
  local files=("$entry"/*.patch)
  [ -e "${files[0]}" ] || die "entry $name has no *.patch files"
  local tmp; tmp=$(mktemp -d)
  # Shape first, on every file: cheap, and a malformed message must not cost the gate.
  local f why
  for f in "${files[@]}"; do
    if ! why=$(preflight "$f" "$tmp"); then
      if [ "$dry" = 1 ]; then
        printf 'dry-run %s: would reject — %s\n' "$name" "$why"
        rm -rf "$tmp"
        return 0
      fi
      reject "$entry" "$name" "$(basename "$f"): $why" "$tmp/msg"
      rm -rf "$tmp"
      return 0
    fi
  done
  # The series names one issue and one branch: the first patch's message decides both.
  local issue subject area branch info
  info=$(git -C "$landing" mailinfo "$tmp/msg" "$tmp/diff" < "${files[0]}")
  subject=$(printf '%s\n' "$info" | sed -n 's/^Subject: //p' | head -1)
  issue=$(sed -n 's/^Closes #\([0-9]\{1,\}\)$/\1/p' "$tmp/msg" | head -1)
  area=${subject%%:*}
  branch="$area/$name"
  if [ "$dry" = 1 ]; then
    printf 'dry-run %s: git apply --check %s; git am %s; (cd %s && scripts/check.sh full); provenance; git checkout -b %s; git push origin %s; gh pr create --base main --head %s\n' \
      "$name" "${files[*]}" "${files[*]}" "$landing" "$branch" "$branch" "$branch"
    rm -rf "$tmp"
    return 0
  fi
  if ! git -C "$landing" apply --check "${files[@]}" 2>"$tmp/apply.log"; then
    reject "$entry" "$name" "main moved under the patch ($(head -1 "$tmp/apply.log")); rebase and re-export" "$tmp/msg"
    rm -rf "$tmp"
    return 0
  fi
  git -C "$landing" am -q "${files[@]}"
  # The gate runs in the landing worktree: same scripts, its own checkout and target dir.
  local took
  took=$(date +%s)
  if ! (cd "$landing" && bash scripts/check.sh full) >"$tmp/gate.log" 2>&1; then
    cp "$tmp/gate.log" "$entry/gate.log"
    tail -5 "$tmp/gate.log" >&2
    gh issue comment "$issue" --body "patch-inbox: the gate is red on \`$name\` (see the failing step above); the entry is kept with gate.log. Fix the patch, not the gate.

Generated-by: scripts/patch-inbox.sh" >/dev/null || true
    note "failed $name — gate red"
    rm -rf "$tmp"
    printf 'gate red for %s: entry kept with gate.log\n' "$name"
    return 1
  fi
  took=$(( $(date +%s) - took ))
  (cd "$landing" && python3 scripts/provenance.py check "origin/main..HEAD") >/dev/null
  git -C "$landing" checkout -q -b "$branch"
  git -C "$landing" push -q -u origin "$branch"
  git -C "$landing" log -1 --format=%B > "$tmp/full"
  local body="$tmp/body.md"
  python3 - "$landing/.github/pull_request_template.md" "$tmp/full" "$body" "$took" <<'PY'
import subprocess, sys
template, msg_path, out, took = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
msg = open(msg_path).read().strip("\n")
lines = msg.split("\n")
trailer = next(l for l in reversed(lines) if l.startswith("Generated-by: "))
body = open(template).read()
body = body.replace("Generated-by: <tool>/<model>", trailer)
body = body.replace("Closes #\n", "Closes #" + next(l.split("#")[1] for l in lines if l.startswith("Closes #")) + "\n", 1)
body = body.replace("## Motivation", "## Motivation\n\n" + lines[0] + "\n\nFrom the patch by its author:\n\n" + "\n".join(l for l in lines[1:] if not l.startswith(("Closes #", "Generated-by:", "Co-Authored-By:"))).strip(), 1)
body += f"\nLanded by `scripts/patch-inbox.sh`; `scripts/check.sh full` ran green in the landing worktree ({took} s).\n"
open(out, "w").write(body)
PY
  local url
  url=$(gh pr create --base main --head "$branch" --title "$subject" --body-file "$body" | tail -1)
  note "landed $name — $url"
  rm -rf "$entry" "$tmp"
  printf 'landed %s: %s\n' "$name" "$url"
  return 0
}

cmd_list() {
  [ -d "$tray" ] || { echo "no tray at $tray"; exit 0; }
  local found=0 d
  for d in "$tray"/*/; do
    [ -d "$d" ] || continue
    found=1
    if [ -f "$d/gate.log" ]; then
      printf '%s\tFAILED (gate.log)\n' "$(basename "$d")"
    else
      printf '%s\tpending\n' "$(basename "$d")"
    fi
  done
  [ "$found" = 1 ] || echo "tray empty"
  [ -f "$log" ] && { echo "---"; tail -5 "$log"; }
  return 0
}

cmd_land() {
  local target=${1:-} dry=0
  [ -n "$target" ] || usage
  shift
  for arg in "$@"; do
    case $arg in
      --dry-run) dry=1 ;;
      *) usage ;;
    esac
  done
  mkdir -p "$tray"
  exec 8>"$lock"
  flock -n 8 || die "another landing run holds the lock ($lock)"
  local held; held=$(scripts/build-lock.sh target/stage/release || true)
  [ -z "$held" ] || die "a release build is in flight; land later ($held)"
  git fetch -q origin main
  if [ ! -d "$landing" ]; then
    git worktree add -q --detach "$landing" origin/main
  else
    git -C "$landing" reset --hard -q origin/main
    git -C "$landing" clean -fd -q
  fi
  [ -z "$(git -C "$landing" status --porcelain)" ] || die "landing worktree is not clean after reset; inspect $landing"
  if [ "$target" = --all ]; then
    local d failed=0
    for d in "$tray"/*/; do
      [ -d "$d" ] || continue
      if ! land_one "$(basename "$d")" "$dry"; then failed=1; break; fi
    done
    [ "$failed" = 0 ] || exit 1
  else
    land_one "$target" "$dry"
  fi
}

case "${1:-}" in
  list) cmd_list ;;
  land) shift; cmd_land "$@" ;;
  -h|--help) usage ;;
  *) usage ;;
esac
