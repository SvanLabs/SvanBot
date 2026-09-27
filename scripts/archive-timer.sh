#!/usr/bin/env bash
# Install (or refresh) the nightly archive timer as a systemd user unit.
set -euo pipefail
cd "$(dirname "$0")/.."
dest="$HOME/.config/systemd/user"
mkdir -p "$dest"
cp scripts/svanbot10-archive.service scripts/svanbot10-archive.timer "$dest/"
systemctl --user daemon-reload
systemctl --user enable --now svanbot10-archive.timer
systemctl --user list-timers svanbot10-archive.timer --no-pager
