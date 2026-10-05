#!/usr/bin/env bash
# Restore runtime data from the newest `data-*` release on GitHub. Runtime data is not part of this
# source repository — databases, archives and screenshots are published as releases on a repository
# the operator names in SVANBOT_DATA_REPO, and gh must be authenticated for that repository.
# Default: the two live databases. `--all` also restores backups, release snapshots, tables/logs and
# screenshots. `--derived` instead fetches the public derived set (aggregates + schema, #17) into
# `artifacts/derived/` for analysis: it never touches the live databases and the fleet may run.
# Refuses while a known fleet writer or restart supervisor runs. FORCE=1 only bypasses existing data.
# Snapshots are made with sqlite `.backup` + zstd (see the release notes).
set -euo pipefail
cd "$(dirname "$0")/.."
# Named rather than defaulted: the repository holding the snapshots is the operator's, it is not
# this one, and a wrong default here restores nothing while looking like it should.
repo="${SVANBOT_DATA_REPO:?set SVANBOT_DATA_REPO to the owner/repo holding the data-* releases}"
all=0 derived=0
[ "${1:-}" = "--all" ] && all=1
[ "${1:-}" = "--derived" ] && derived=1

if [ "$derived" = 1 ]; then
  tag=$(gh release list --repo "$repo" --limit 50 --json tagName --jq '.[].tagName' | { grep '^data-' || true; } | sort | tail -1)
  [ -n "$tag" ] || { echo "no data-* release on $repo" >&2; exit 1; }
  echo "fetching derived set from $tag"
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  gh release download "$tag" --repo "$repo" --dir "$tmp" -p 'derived-*.tar.zst' -p 'SHA256SUMS*'
  (cd "$tmp" && cat SHA256SUMS* | sha256sum --check --ignore-missing --quiet)
  mkdir -p artifacts/derived
  zstd -dc "$tmp"/derived-*.tar.zst | tar -C artifacts/derived -xf -
  echo "derived set in artifacts/derived/ (aggregates + schema only; no databases touched)"
  exit 0
fi

for writer_pidfile in artifacts/bot.pid artifacts/head.pid artifacts/worker-*.pid     artifacts/supervisor.pid artifacts/head-supervisor.pid     artifacts/learner.pid artifacts/learner-supervisor.pid     artifacts/analyst.pid artifacts/analyst-supervisor.pid; do
  [ -f "$writer_pidfile" ] || continue
  writer_pid=$(cat "$writer_pidfile")
  [[ "$writer_pid" =~ ^[0-9]+$ ]] || continue
  if kill -0 "$writer_pid" 2>/dev/null; then
    echo "a fleet writer or supervisor is running ($writer_pidfile); stop it first" >&2
    exit 1
  fi
done
if [ -f artifacts/svanbot10.db ] && [ "${FORCE:-0}" != 1 ]; then
  echo "artifacts/svanbot10.db exists; rerun with FORCE=1 to overwrite" >&2
  exit 1
fi

tag=$(gh release list --repo "$repo" --limit 50 --json tagName --jq '.[].tagName' | { grep '^data-' || true; } | sort | tail -1)
[ -n "$tag" ] || { echo "no data-* release on $repo" >&2; exit 1; }
echo "restoring $tag"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
patterns=(-p 'svanbot10.db.zst' -p 'history.db.zst' -p 'misc.tar.zst' -p 'SHA256SUMS*')
[ "$all" = 1 ] && patterns+=(-p 'backups.tar.zst' -p 'release-snapshots.tar.zst' -p 'screenshots.tar.zst')
gh release download "$tag" --repo "$repo" --dir "$tmp" "${patterns[@]}"
(cd "$tmp" && cat SHA256SUMS* | sha256sum --check --ignore-missing --quiet)

mkdir -p artifacts
rm -f artifacts/svanbot10.db-wal artifacts/svanbot10.db-shm artifacts/history.db-wal artifacts/history.db-shm
zstd -dqf "$tmp/svanbot10.db.zst" -o artifacts/svanbot10.db
zstd -dqf "$tmp/history.db.zst" -o artifacts/history.db
for t in "$tmp"/*.tar.zst; do
  zstd -dc "$t" | tar -C artifacts -xf -
done
echo "restored into artifacts/: $(cd "$tmp" && ls ./*.zst | xargs -n1 basename | tr '\n' ' ')"
