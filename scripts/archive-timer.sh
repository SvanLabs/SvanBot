#!/usr/bin/env bash
# Install (or refresh) the nightly archive timer and the 6-hourly cleanup timer as systemd user units.
# scripts/units.sh renders every unit for this checkout (the units name no fixed directory), so this
# is the archive half of it: render, then enable the two timers.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/units.sh
systemctl --user enable --now svanbot10-archive.timer svanbot10-clean.timer
systemctl --user list-timers svanbot10-archive.timer svanbot10-clean.timer --no-pager
