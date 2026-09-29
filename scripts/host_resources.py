#!/usr/bin/env python3
"""Linux host budgets: CPU affinity, cgroup-v2 ceilings, available RAM and filesystem space."""
import argparse
import os
from pathlib import Path

CGROUP = Path("/sys/fs/cgroup")


def read(path):
    try:
        return path.read_text().strip()
    except OSError:
        return ""


def cgroup():
    for line in read(Path("/proc/self/cgroup")).splitlines():
        if line.startswith("0::"):
            path = Path(line[3:].lstrip("/"))
            if ".." not in path.parts:
                return CGROUP / path
    return CGROUP


def ancestors(leaf, root):
    if leaf != root and root not in leaf.parents:
        return
    while True:
        yield leaf
        if leaf == root:
            return
        leaf = leaf.parent


def memory_available(host=None, leaf=None, root=CGROUP):
    if host is None:
        host = 256 << 20  # Unknown capacity: use conservative serial scheduling.
        for line in read(Path("/proc/meminfo")).splitlines():
            if line.startswith("MemAvailable:"):
                host = int(line.split()[1]) * 1024
                break
    for path in ancestors(leaf or cgroup(), root):
        maximum, current = read(path / "memory.max"), read(path / "memory.current")
        if maximum.isdigit():
            used = int(current) if current.isdigit() else int(maximum)
            host = min(host, max(0, int(maximum) - used))
    return host


def cpu_available(host=None, leaf=None, root=CGROUP):
    if host is None:
        try:
            host = len(os.sched_getaffinity(0))
        except (AttributeError, OSError):
            host = os.cpu_count() or 1
    for path in ancestors(leaf or cgroup(), root):
        fields = read(path / "cpu.max").split()
        if len(fields) == 2 and fields[0].isdigit() and fields[1].isdigit() and int(fields[1]):
            host = min(host, max(1, int(fields[0]) // int(fields[1])))
    return max(1, host)


def build_jobs(cores, memory):
    # Leave 40% of available memory for live play, the linker and the user's other processes.
    return max(1, min(cores, int(memory * 0.6) // (768 << 20)))


def check_disk(path, reserve):
    probe = path.resolve()
    while not probe.exists():
        probe = probe.parent
    stats = os.statvfs(probe)
    free = stats.f_bavail * stats.f_frsize
    if free < reserve:
        raise ValueError(f"{path}: {free / 2**20:.0f} MiB free, need {reserve / 2**20:.0f} MiB reserve; "
                         "choose another filesystem or free space explicitly")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", action="store_true")
    parser.add_argument("--check-disk", nargs="+", type=Path)
    args = parser.parse_args()
    if args.jobs:
        print(build_jobs(cpu_available(), memory_available()))
    if args.check_disk:
        reserve = int(os.environ.get("SVANBOT_MIN_FREE_MB", "512"))
        if reserve < 0:
            raise ValueError("SVANBOT_MIN_FREE_MB must be nonnegative")
        for path in args.check_disk:
            check_disk(path, reserve << 20)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        raise SystemExit(f"host-resources: {error}")
