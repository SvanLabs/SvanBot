#!/usr/bin/env bash
# Rust file-size limit (CONTRIBUTING.md, 0320): 500 physical lines is the hard maximum for a source
# file, 400 the point to consider splitting. Files over 500 when the rule was adopted are listed with
# their size in scripts/file-size-baseline.txt: they may shrink but never grow, and a file leaves the
# list once it is back under the limit. A new file over 500 lines fails.
#   scripts/check-file-size.sh             check (exit 1 on a new or grown oversized file)
#   scripts/check-file-size.sh --report    also list files over 400 lines
#   scripts/check-file-size.sh --baseline  rewrite the baseline from today's files (operator decision only)
set -euo pipefail
cd "$(dirname "$0")/.."
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
      echo "ERROR: $path has $lines lines; maximum is $max_lines (split it by responsibility, CONTRIBUTING.md)"
      failed=1
    elif [ "$lines" -gt "$allowed" ]; then
      echo "ERROR: $path grew to $lines lines (baseline $allowed); oversized files may only shrink"
      failed=1
    fi
  elif [ "$mode" = --report ] && [ "$lines" -gt "$warn_lines" ]; then
    echo "note: $path has $lines lines (over $warn_lines: consider splitting before adding more)"
  fi
done <<< "$sizes"
exit "$failed"
