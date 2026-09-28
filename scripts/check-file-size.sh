#!/usr/bin/env bash
# Rust file-size limit (docs/CONTRIBUTING.md, 0320): 500 physical lines is the hard maximum for a source
# file, 400 the point to consider splitting. Files over 500 when the rule was adopted are listed with
# their size in scripts/file-size-baseline.txt: they may shrink but never grow, and a file leaves the
# list once it is back under the limit. A new file over 500 lines fails.
#
# The last of those is checked, not just stated (0390). An entry for a file that has come back under
# the limit is not harmless: the entry is a *ceiling*, so it silently re-grants the lines the file
# gave up — `learner.rs` sat at 380 lines with an entry of 931, and could have grown back to 930
# without the gate saying anything. The fix is to delete the line, which is one line, so the check
# asks for that rather than trusting someone to remember. Lowering an entry that is still needed is
# allowed but never required: an edit people have to make on every shrink is an edit that conflicts
# in every branch, and the size a file has today is not the size that matters here — the ceiling it
# may not cross is.
#
# The tree it reads is `SV10_FILE_SIZE_ROOT` (default: this checkout), which exists so the rule has
# tests to fail (`scripts/tests/file-size.sh`).
#   scripts/check-file-size.sh             check (exit 1 on a new, grown or stale entry)
#   scripts/check-file-size.sh --report    also list files over 400 lines
#   scripts/check-file-size.sh --baseline  rewrite the baseline from today's files (operator decision only)
set -euo pipefail
cd "${SV10_FILE_SIZE_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
max_lines=500
warn_lines=400
baseline=scripts/file-size-baseline.txt
mode="${1:-check}"
sizes=$(find crates -type f -name '*.rs' -not -path '*/target/*' -print0 | xargs -0 wc -l | grep -v ' total$' | awk '{print $2, $1}' | sort)
if [ "$mode" = --baseline ]; then
  { echo "# path lines — files over $max_lines lines when the limit was adopted (may shrink, never grow)"
    echo "$sizes" | awk -v m="$max_lines" '$2 > m'; } > "$baseline"
  echo "baseline written: $(grep -vc '^#' "$baseline") files"
  exit 0
fi
failed=0
while read -r path lines; do
  allowed=$(awk -v p="$path" '$1 == p {print $2}' "$baseline" 2>/dev/null)
  if [ "$lines" -gt "$max_lines" ]; then
    if [ -z "$allowed" ]; then
      echo "ERROR: $path has $lines lines; maximum is $max_lines (split it by responsibility, docs/CONTRIBUTING.md)"
      failed=1
    elif [ "$lines" -gt "$allowed" ]; then
      echo "ERROR: $path grew to $lines lines (baseline $allowed); oversized files may only shrink"
      failed=1
    fi
  elif [ -n "$allowed" ]; then
    echo "ERROR: $path has $lines lines and is listed in $baseline with $allowed: it is back under the $max_lines-line limit, so it has left the list — delete its line"
    failed=1
  elif [ "$mode" = --report ] && [ "$lines" -gt "$warn_lines" ]; then
    echo "note: $path has $lines lines (over $warn_lines: consider splitting before adding more)"
  fi
done <<< "$sizes"
exit "$failed"
