#!/usr/bin/env bash
# Pre-commit hook: refuse commits whose staged changes contain a secret from .env or the .env file
# itself (keys named like KEY/TOKEN/SECRET/PASSWORD). Values are compared, never printed. Install: ln -sf ../../scripts/pre-commit-secret-scan.sh .git/hooks/pre-commit
set -uo pipefail
root="$(git rev-parse --show-toplevel)"
# Read whole, then searched: under pipefail a long list piped into `grep -q` ends in SIGPIPE, which
# read as "no .env staged" (#875).
staged="$(git diff --cached --name-only)"
if grep -vx '.env.example' <<<"$staged" | grep -xE '(.*/)?\.env(\..*)?' >/dev/null; then
  echo "pre-commit: refusing to commit an .env file" >&2
  exit 1
fi
[ -f "$root/.env" ] || exit 0
added="$(git diff --cached -U0 | grep '^+' || true)"
[ -n "$added" ] || exit 0
while IFS= read -r line; do
  case "$line" in ''|\#*) continue ;; esac
  name="${line%%=*}"
  case "$name" in *KEY*|*TOKEN*|*SECRET*|*PASSWORD*|*PASS*) ;; *) continue ;; esac
  value="${line#*=}"
  value="${value%\"}"; value="${value#\"}"; value="${value%\'}"; value="${value#\'}"
  if [ ${#value} -ge 16 ] && printf '%s' "$added" | grep -qF -- "$value"; then
    echo "pre-commit: staged changes contain the value of ${line%%=*} from .env; commit refused" >&2
    exit 1
  fi
done < "$root/.env"
exit 0
