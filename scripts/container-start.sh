#!/usr/bin/env bash
# Container entrypoint: start the fleet exactly as scripts/start.sh does on a host, then stay in the
# foreground on the fleet log so the container lives as long as the fleet does.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] || { echo "mount your .env at /svanbot/.env (see .env.example)" >&2; exit 1; }
mkdir -p artifacts/logs
scripts/start.sh
exec tail -F artifacts/logs/svanbot10.log
