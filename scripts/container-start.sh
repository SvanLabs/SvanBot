#!/usr/bin/env bash
# Container entrypoint: start the fleet exactly as scripts/start.sh does on a host, then stay in the
# foreground on the fleet log so the container lives as long as the fleet does.
#
# This shell stays PID 1 and stops the fleet when the container is stopped. With `exec tail` as PID 1
# nothing handled SIGTERM, so `docker stop` waited out its grace period and killed every process
# without the save the bots make on SIGTERM (#875).
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] || { echo "mount your .env at /svanbot/.env (see .env.example)" >&2; exit 1; }
mkdir -p artifacts/logs
scripts/start.sh
tail -F artifacts/logs/svanbot10.log &
tail_pid=$!
trap 'scripts/stop.sh || true; kill "$tail_pid" 2>/dev/null; exit 0' TERM INT
wait "$tail_pid"
