#!/usr/bin/env python3
"""One-release shim (#717): a monitor supervisor started before the port still relaunches
`python3 scripts/monitor.py` every 30 s. This forwards to the binary in the same process, so the
supervisor keeps its pid and signals reach the monitor directly. Delete once every fleet has
restarted supervision."""
import os
import sys

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.execv(os.path.join(root, "target", "release", "monitor"), ["monitor", *sys.argv[1:]])
