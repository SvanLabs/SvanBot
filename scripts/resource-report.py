#!/usr/bin/env python3
"""Disk and memory report for the target machine (0227): database bytes per table, backups, tables,
snapshots, logs, and free space per mount. Read-only: live databases are opened `mode=ro` with a WAL-aware read transaction.

  scripts/resource-report.py [ROOT]      default: the repository root
  scripts/resource-report.py --json      machine-readable (the dashboard's host check reads it, 0241)
"""
from __future__ import annotations

import json
import os
import shutil
import sqlite3
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def tree_bytes(path: Path) -> int:
    if path.is_file():
        return path.stat().st_size
    total = 0
    for dirpath, _, files in os.walk(path):
        for f in files:
            try:
                total += (Path(dirpath) / f).lstat().st_size
            except OSError:
                pass
    return total


def db_tables(path: Path) -> list[tuple[str, int]]:
    try:
        c = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
        c.execute("BEGIN")
        rows = c.execute("SELECT name, SUM(pgsize) FROM dbstat GROUP BY name ORDER BY 2 DESC").fetchall()
        free = c.execute("PRAGMA freelist_count").fetchone()[0] * c.execute("PRAGMA page_size").fetchone()[0]
        c.close()
        return [(n, int(b)) for n, b in rows] + ([("(free pages)", free)] if free else [])
    except sqlite3.Error as e:
        return [(f"unreadable: {e}", 0)]


def report(root: Path) -> dict:
    art = root / "artifacts"
    out = {"databases": {}, "files": {}, "mounts": {}}
    for name in ("svanbot10.db", "history.db"):
        p = art / name
        if p.exists():
            out["databases"][name] = {"bytes": p.stat().st_size, "tables": db_tables(p)}
    for label, rel in [("backups", "backups"), ("release snapshots", "release-snapshots"), ("strength tables", "tables"), ("logs", "logs"), ("archive", "archive")]:
        p = art / rel
        if p.exists():
            out["files"][label] = tree_bytes(p)
    mirror = os.environ.get("SVANBOT_ARCHIVE_DIR")
    for label, path in [("repository", root), ("archive mirror", Path(mirror) if mirror else None)]:
        if path and path.exists():
            u = shutil.disk_usage(path)
            out["mounts"][label] = {"path": str(path), "total": u.total, "free": u.free}
    return out


def mb(b: int) -> str:
    return f"{b / 1e6:9.1f} MB"


def main(argv: list[str]) -> int:
    as_json = "--json" in argv
    args = [a for a in argv if a != "--json"]
    root = Path(args[0]) if args else ROOT
    r = report(root)
    if as_json:
        print(json.dumps(r))
        return 0
    for name, db in r["databases"].items():
        print(f"{name}: {mb(db['bytes'])}")
        for table, b in db["tables"][:12]:
            print(f"   {table:32s} {mb(b)}")
    for label, b in r["files"].items():
        print(f"{label + ':':22s} {mb(b)}")
    for label, m in r["mounts"].items():
        print(f"{label} ({m['path']}): {m['free'] / 1e9:.1f} GB free of {m['total'] / 1e9:.1f} GB")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
