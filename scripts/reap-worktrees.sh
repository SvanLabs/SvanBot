#!/usr/bin/env bash
# Report which registered git worktrees are dead weight (#763). Report only: it never removes a worktree
# or a branch, it prints the command. Push archive/<branch> before removing a branch (recoverability rule).
#   scripts/reap-worktrees.sh [--no-size]
# Verdicts: live (locked, or a process runs inside), prunable (directory gone), merged (HEAD is in the base
# and idle for a day), stale (idle REAP_STALE_DAYS, default 14, not merged), dirty (merged or stale but with
# uncommitted changes), active (everything else).
# Env: REAP_BASE (default origin/main, else main), REAP_ROOT (the repository; default this checkout).
set -uo pipefail
root=${REAP_ROOT:-$(cd "$(dirname "$0")/.." && pwd -P)}
cd "$root" || exit 1
size=1; [ "${1:-}" = --no-size ] && size=0
base=${REAP_BASE:-}
if [ -z "$base" ]; then git rev-parse -q --verify origin/main > /dev/null && base=origin/main || base=main; fi
stale_days=${REAP_STALE_DAYS:-14}
now=$(date +%s)
cwds=$(for p in /proc/[0-9]*; do readlink "$p/cwd" 2> /dev/null; done | sort -u)
in_use() { grep -qxF -- "$1" <<< "$cwds" || grep -qF -- "$1/" <<< "$cwds"; }

top=$(git rev-parse --show-toplevel)
git worktree list --porcelain | awk -v RS= -F'\n' '{
  path = ""; head = ""; branch = "(detached)"; lock = 0; prun = 0
  for (i = 1; i <= NF; i++) {
    if ($i ~ /^worktree /) path = substr($i, 10)
    else if ($i ~ /^HEAD /) head = substr($i, 6)
    else if ($i ~ /^branch refs\/heads\//) branch = substr($i, 19)
    else if ($i ~ /^locked/) lock = 1
    else if ($i ~ /^prunable/) prun = 1
  }
  print path "|" head "|" branch "|" lock "|" prun
}' | while IFS='|' read -r path head branch lock prun; do
  [ "$path" = "$top" ] && continue
  if [ "$prun" = 1 ] || [ ! -d "$path" ]; then verdict=prunable; reason="directory is gone"; age=-; sz=-
  else
    gitdir=$(git -C "$path" rev-parse --git-dir 2> /dev/null)
    newest=$(stat -c %Y "$path" "$gitdir/index" "$gitdir/HEAD" 2> /dev/null | sort -n | tail -1)
    age=$(( (now - ${newest:-$now}) / 86400 ))
    sz=-; [ "$size" = 1 ] && sz=$(du -sh "$path" 2> /dev/null | cut -f1)
    if [ "$lock" = 1 ]; then verdict=live; reason="locked by a session"
    elif in_use "$path"; then verdict=live; reason="a process runs inside"
    elif git merge-base --is-ancestor "$head" "$base" 2> /dev/null && [ "$age" -ge 1 ]; then verdict=merged; reason="HEAD is in $base"
    elif [ "$age" -ge "$stale_days" ]; then verdict=stale; reason="idle ${age}d, not in $base"
    else verdict=active; reason="idle ${age}d"
    fi
    # Uncommitted work is never "safe to remove": git worktree remove would refuse it, so say why.
    case "$verdict" in merged | stale) [ -z "$(git -C "$path" status --porcelain 2> /dev/null | head -1)" ] || { verdict=dirty; reason="$reason; uncommitted changes"; } ;; esac
  fi
  printf '%-9s %-6s %5sd  %-34s %s (%s)\n' "$verdict" "$sz" "$age" "${branch:0:34}" "${path#$top/}" "$reason"
  case "$verdict" in
    merged | stale) hint="push archive/$branch first if the branch has work"; [ "$branch" != "(detached)" ] || hint="detached HEAD, nothing to archive"
      printf '          git worktree remove %q   # %s\n' "$path" "$hint" ;;
    prunable) printf '          git worktree prune\n' ;;
  esac
done
[ ! -d .scratch ] || [ "$size" = 0 ] || printf '.scratch: %s\n' "$(du -sh .scratch 2> /dev/null | cut -f1)"
