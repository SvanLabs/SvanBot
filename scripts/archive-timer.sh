#!/usr/bin/env bash
# Install (or refresh) the nightly archive timer and the 6-hourly cleanup timer as systemd user units.
set -euo pipefail
cd "$(dirname "$0")/.."
dest="$HOME/.config/systemd/user"
mkdir -p "$dest"
cp scripts/svanbot10-archive.service scripts/svanbot10-archive.timer scripts/svanbot10-clean.service scripts/svanbot10-clean.timer "$dest/"
systemctl --user daemon-reload
systemctl --user enable --now svanbot10-archive.timer svanbot10-clean.timer
systemctl --user list-timers svanbot10-archive.timer svanbot10-clean.timer --no-pager
