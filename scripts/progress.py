#!/usr/bin/env python3
"""Release progress for the dashboard's progress bar (0236).

  progress.py start                   a new run (records the installed build's commit as the starting point)
  progress.py stage NAME              close the running stage, open NAME
  progress.py fail [STAGE] [MESSAGE]  the run failed (in STAGE, default the running one)
  progress.py fail-running [STAGE] [MESSAGE]  fail only a still-running record on unexpected exit
  progress.py installed COMMIT        the run installed COMMIT; stage durations become the next ETA
  progress.py current MESSAGE         there was nothing to install (the checkout is ahead of the
                                      update branch); MESSAGE says why, and no stage ever ran

State: artifacts/release-progress.json, written atomically. Timings of the last successful run per
stage: artifacts/release-timings.json (the dashboard weights the bar and estimates the time left
with them). Stages in order: fetch, snapshot, build (lint, test build and release build side by side,
0302), test, dashboard, install; a release run
by hand has no fetch; a rollback (update.sh --rollback) has the single stage restore. After install
the fleet swaps between turns; the API reports that part.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ART = Path(os.environ.get("SV10_PROGRESS_DIR", ROOT / "artifacts"))
STATE = ART / "release-progress.json"
TIMINGS = ART / "release-timings.json"
STAGES = ["fetch", "snapshot", "build", "test", "dashboard", "install", "restore"]


def load(path: Path) -> dict:
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return {}


def save(path: Path, data: dict) -> None:
    ART.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(data, indent=1))
    os.replace(tmp, path)


def head() -> str | None:
    try:
        return subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None


INSTALLED = Path(os.environ.get("SV10_INSTALLED_MARKER", ROOT / "target" / "release" / ".sv10-installed-commit"))


def installed() -> str | None:
    """The build the fleet runs now: the starting point of a run. The checkout's HEAD is not it —
    a release run by hand starts from the commit it is about to install, so the card read
    `078e871 → 078e871` (2026-09-27)."""
    try:
        return INSTALLED.read_text().strip() or head()
    except OSError:
        return head()


def close_running(st: dict, now: float) -> None:
    for s in st.get("stages", []):
        if s.get("state") == "running":
            s["state"] = "done"
            s["seconds"] = round(now - s["started"], 1)


def main(argv: list[str]) -> int:
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    now = time.time()
    cmd = argv[0]
    if cmd == "start":
        save(STATE, {"state": "running", "started": now, "updated": now, "from": installed(), "commit": None, "message": None, "stages": []})
        return 0
    st = load(STATE)
    if not st:
        st = {"state": "running", "started": now, "from": installed(), "commit": None, "message": None, "stages": []}
    if cmd == "stage" and len(argv) >= 2:
        close_running(st, now)
        st["stages"].append({"name": argv[1], "state": "running", "started": now, "seconds": None})
        st["state"] = "running"
    elif cmd in ("fail", "fail-running"):
        if cmd == "fail-running" and st.get("state") != "running":
            return 0
        stage = argv[1] if len(argv) > 1 and argv[1] else None
        running = next((s for s in st["stages"] if s["state"] == "running"), None)
        if running is not None:
            running["state"] = "failed"
            running["seconds"] = round(now - running["started"], 1)
        elif stage:
            st["stages"].append({"name": stage, "state": "failed", "started": now, "seconds": 0.0})
        st["state"] = "failed"
        st["message"] = argv[2] if len(argv) > 2 else st.get("message")
    elif cmd == "current" and len(argv) >= 2:
        # Nothing was installed and nothing failed (#394): the checkout is already ahead of the
        # branch this install follows, so the run has nothing to do. `installed` would claim a build,
        # `fail` would claim a fault, and the dashboard shows this one with its message and no bar.
        close_running(st, now)
        st["state"] = "current"
        st["commit"] = None
        st["message"] = argv[1]
    elif cmd == "installed" and len(argv) >= 2:
        close_running(st, now)
        st["state"] = "installed"
        st["commit"] = argv[1]
        timings = load(TIMINGS)
        for s in st["stages"]:
            if s["state"] == "done" and s.get("seconds") is not None:
                timings[s["name"]] = s["seconds"]
        save(TIMINGS, timings)
    else:
        print(f"progress.py: unknown command {argv}", file=sys.stderr)
        return 2
    st["updated"] = now
    save(STATE, st)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
