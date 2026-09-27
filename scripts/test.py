#!/usr/bin/env python3
"""Workspace tests, built once and run in parallel (0226).

  scripts/test.py [--profile gate|release] [--jobs N] [-p CRATE ...] [--slowest N] [-- TEST-ARGS]

`cargo test` runs its test executables one after another. This runner builds them the same way
(`cargo test --no-run --message-format=json`), then runs every executable concurrently, each in its
package directory as cargo does, with `RUST_TEST_THREADS` threads (default 2). Only artifacts cargo
built as tests (`profile.test`) are run, so the `sim`, `probe` and `tables` binaries never start.

Default profile `gate` (no LTO, incremental; bit-identical results, see Cargo.toml). Jobs default
to what the machine can hold: logical cores / test threads, capped by available memory over the
largest peak RSS a test binary has shown (stored in `$CARGO_TARGET_DIR/test-rss.json`, 600 MB before
the first run; a parallel suite was OOM-killed once, release.sh). Prints the slowest binaries and
every failure's output; exit 1 on any failure. No third-party tools.

Each run gets its own `TMPDIR`, removed when the run ends: the tests make `temp_dir()/sv10-<name>-<pid>`
directories and most never remove them, and `/tmp` is RAM (tmpfs) — 11,328 of them held 2.5 GB when
this was found (0329).
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def mem_available() -> int:
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) * 1024
    except OSError:
        pass
    return 4 << 30


def build(profile: str, packages: list[str], env: dict) -> list[tuple[str, str, Path]]:
    """Build every test executable; returns (label, executable, cwd) for each."""
    cmd = ["cargo", "test", "--no-run", "--message-format=json-render-diagnostics"]
    cmd += ["--release"] if profile == "release" else ["--profile", profile]
    cmd += [x for p in packages for x in ("-p", p)] or ["--workspace"]
    proc = subprocess.run(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE, text=True)
    if proc.returncode != 0:
        sys.exit("test.py: build failed")
    out = []
    for line in proc.stdout.splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") != "compiler-artifact" or not msg.get("executable") or not msg.get("profile", {}).get("test"):
            continue
        target = msg["target"]
        name = target["name"]
        kind = target["kind"][0]
        label = f"{name}" if kind == "lib" else f"{name} ({kind})"
        out.append((label, msg["executable"], Path(msg["manifest_path"]).parent))
    return out


def exit_note(code: int) -> str:
    """How a test binary ended, in words (#140)."""
    if code < 0:
        try:
            return f"killed by {signal.Signals(-code).name}"
        except ValueError:
            return f"killed by signal {-code}"
    return f"exit status {code}"


def failure_report(label: str, out: str, code: int) -> str:
    """The block printed for a test binary that did not exit 0 (#140).

    A binary can end without printing anything: a signal it does not handle, or an exit before
    its buffered stdout reaches the pipe. The block was printed empty, so a run that failed this
    way said only that something went wrong — twice it cost a rerun to find out whether the diff
    or the harness was at fault (249 tests of 564 ran, and 315 of the missing ones were one
    binary's). Which is why the status is named here, and the silence is said out loud.
    """
    body = out.rstrip()
    head = f"---- {label} ({exit_note(code)}) ----"
    return f"{head}\n{body}\n" if body else f"{head}\n(no output: it ended before its first line)\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--profile", default="gate")
    ap.add_argument("--jobs", type=int, default=0)
    ap.add_argument("-p", dest="packages", action="append", default=[])
    ap.add_argument("--slowest", type=int, default=8)
    ap.add_argument("--quiet", "-q", action="store_true")
    ap.add_argument("rest", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    test_args = a.rest[1:] if a.rest[:1] == ["--"] else a.rest

    env = dict(os.environ)
    env["PATH"] = str(Path.home() / ".cargo/bin") + ":" + env.get("PATH", "")
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target/dev"))
    threads = int(env.setdefault("RUST_TEST_THREADS", "2"))
    target = Path(env["CARGO_TARGET_DIR"])
    rss_file = target / "test-rss.json"

    t0 = time.monotonic()
    exes = build(a.profile, a.packages, env)
    built = time.monotonic() - t0
    try:
        known = json.loads(rss_file.read_text())
    except (OSError, ValueError):
        known = {}
    peak = max(known.values(), default=600 << 20)
    cores = os.cpu_count() or 2
    jobs = a.jobs or max(1, min(cores // max(1, threads) or 1, int(mem_available() * 0.6 // max(peak, 1)), len(exes)))

    run_env = dict(env, TMPDIR=tempfile.mkdtemp(prefix="sv10-testrun-"))

    def run(item):
        label, exe, cwd = item
        start = time.monotonic()
        p = subprocess.Popen([exe, *test_args], cwd=cwd, env=run_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        out = p.stdout.read()
        _, status, usage = os.wait4(p.pid, 0)
        p.returncode = os.waitstatus_to_exitcode(status)
        return label, exe, p.returncode, time.monotonic() - start, usage.ru_maxrss * 1024, out

    t1 = time.monotonic()
    try:
        with ThreadPoolExecutor(max_workers=jobs) as pool:
            results = list(pool.map(run, exes))
    finally:
        shutil.rmtree(run_env["TMPDIR"], ignore_errors=True)
    ran = time.monotonic() - t1

    passed = failed = ignored = 0
    failures = []
    for label, exe, code, secs, rss, out in results:
        known[Path(exe).name.rsplit("-", 1)[0]] = rss
        for line in out.splitlines():
            if line.startswith("test result:"):
                parts = line.replace(";", "").split()
                passed += int(parts[3])
                failed += int(parts[5])
                ignored += int(parts[7])
        if code != 0:
            failures.append((label, out, code))
    try:
        target.mkdir(parents=True, exist_ok=True)
        rss_file.write_text(json.dumps(known, indent=0, sort_keys=True))
    except OSError:
        pass

    for label, out, code in failures:
        print(failure_report(label, out, code), file=sys.stderr)
    if not a.quiet or failures:
        slow = sorted(results, key=lambda r: -r[3])[: a.slowest]
        print("slowest: " + ", ".join(f"{r[0]} {r[3]:.1f}s" for r in slow))
    print(
        f"test.py: {passed} passed, {failed} failed, {ignored} ignored in {len(results)} binaries; "
        f"build {built:.1f}s, run {ran:.1f}s ({jobs} jobs x {threads} threads, profile {a.profile}, "
        f"peak test RSS {max((r[4] for r in results), default=0) / 2**20:.0f} MB)"
    )
    if failures or any(r[2] != 0 for r in results):
        return 1
    if not results:
        print("test.py: no test executables were built", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
