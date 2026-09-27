#!/usr/bin/env python3
"""Superseded build artifacts in cargo target profiles (0329).

  scripts/prune-builds.py [--apply] [--days N] PROFILE_DIR ...

Cargo never deletes an artifact it has replaced: every flag, feature or source change leaves the old
`<crate>-<hash>` file or directory behind in `deps/`, `.fingerprint/`, `build/` and `incremental/`
(18 copies of `libsv10_bot` in one profile when this was written). Several of them can be live at once
— `-p sv10-bot` and `--workspace` unify features differently, so each keeps its own copy of a
dependency — which is why the newest-by-build-time is not the test: the first version kept the newest
two, deleted live variants and cost the next test build two minutes. The test is use. An entry nothing
has read or written for N days (default 2; the root filesystem is `relatime`, so a read shows up in the
access time within a day) is superseded, except the newest of its crate and suffix, which is always
kept. Superseded entries are reported, or removed with `--apply`; removing one is reversible by rebuild
(cargo sees the missing output and rebuilds that crate). Profile dirs that do not exist are skipped.
Prints one line per profile and a total. No third-party tools.
"""
from __future__ import annotations

import argparse
import collections
import os
import re
import shutil
import sys
import time

HASHED = re.compile(r"^(?P<stem>.+)-[0-9a-f]{16}(?P<ext>(\.[A-Za-z0-9_]+)*)$")
# rustc names incremental dirs `<crate>-<13 base-36 chars>`, not with cargo's metadata hash.
INCREMENTAL = re.compile(r"^(?P<stem>.+)-[0-9a-z]{13}(?P<ext>)$")
SUBDIRS = ("deps", ".fingerprint", "build", "incremental")


def size(path: str) -> int:
    if not os.path.isdir(path) or os.path.islink(path):
        return os.lstat(path).st_size
    total = 0
    for root, _, files in os.walk(path):
        for f in files:
            try:
                total += os.lstat(os.path.join(root, f)).st_size
            except OSError:
                pass
    return total


def last_used(path: str) -> float:
    """The latest access or modification of `path` or anything under it (Unix seconds)."""
    st = os.lstat(path)
    latest = max(st.st_atime, st.st_mtime)
    if os.path.isdir(path) and not os.path.islink(path):
        for root, dirs, files in os.walk(path):
            for f in dirs + files:
                try:
                    st = os.lstat(os.path.join(root, f))
                except OSError:
                    continue
                latest = max(latest, st.st_atime, st.st_mtime)
    return latest


def superseded(profile: str, cutoff: float) -> list[str]:
    """Every hashed entry in `profile` unused since `cutoff`, except the newest of its crate and suffix."""
    out = []
    for sub in SUBDIRS:
        d = os.path.join(profile, sub)
        if not os.path.isdir(d):
            continue
        groups: dict[tuple[str, str], list[tuple[float, str]]] = collections.defaultdict(list)
        for name in os.listdir(d):
            m = (INCREMENTAL if sub == "incremental" else HASHED).match(name)
            if not m:
                continue
            path = os.path.join(d, name)
            try:
                groups[(m["stem"], m["ext"])].append((last_used(path), path))
            except OSError:
                continue
        for entries in groups.values():
            entries.sort(reverse=True)
            out.extend(path for used, path in entries[1:] if used < cutoff)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--days", type=float, default=2.0)
    ap.add_argument("profiles", nargs="+")
    a = ap.parse_args()
    if a.days < 1:
        sys.exit("prune-builds.py: --days must be at least 1 (access times move once a day)")
    cutoff = time.time() - a.days * 86_400
    grand = 0
    for profile in a.profiles:
        if not os.path.isdir(profile):
            continue
        old = superseded(profile, cutoff)
        freed = 0
        for path in old:
            try:
                n = size(path)
                if a.apply:
                    if os.path.isdir(path) and not os.path.islink(path):
                        shutil.rmtree(path)
                    else:
                        os.remove(path)
                freed += n
            except OSError as e:
                print(f"   {path}: {e}", file=sys.stderr)
        grand += freed
        verb = "removed" if a.apply else "superseded"
        print(f"   {profile}: {verb} {len(old)} entries, {freed / 1e9:.1f} GB")
    print(f"   total {'freed' if a.apply else 'reclaimable'}: {grand / 1e9:.1f} GB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
