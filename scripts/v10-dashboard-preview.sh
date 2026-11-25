#!/usr/bin/env bash
# Serve the archived Control Room with demo data.
set -euo pipefail

root=$(git -C "$(dirname -- "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
archive=daf1a8e1007b47d93db2eddf1e1b3db65d2553ae
port=${1:-5555}

if ! git -C "$root" cat-file -e "$archive^{tree}" 2>/dev/null; then
    git -C "$root" fetch --no-tags origin refs/tags/archive/v10/final
fi

preview=$(mktemp -d "${TMPDIR:-/tmp}/svanbot-v10-preview.XXXXXX")
trap 'rm -rf -- "$preview"' EXIT
git -C "$root" archive "$archive" web docs/GUIDE.md | tar -x -C "$preview"
cp "$root/deploy/v10-preview.config.ts" "$preview/web/preview.config.ts"

cd "$preview/web"
npm ci --cache "$preview/npm-cache" --no-audit --no-fund
./node_modules/.bin/vite --config preview.config.ts --host 127.0.0.1 --port "$port" --strictPort
