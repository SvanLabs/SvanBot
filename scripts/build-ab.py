#!/usr/bin/env python3
"""Release-build wall time per `[profile.release]` knob, measured paired on a shared box (0337).

    scripts/build-ab.py [VARIANT ...] [--baseline off256] [--rounds 5] [--mode paired|simultaneous]

`lto` (false/thin/fat) x `codegen-units` (256/16/1) decides two separate things and only one of them
is a knob to turn: how long the one-file release rebuild the operator actually runs takes, and how fast
the shipped code is. This script answers the first. `ab` (crates/apps/core/src/bin/ab.rs) answers the second, on the
bench binaries this one leaves behind (`<root>/bins/<variant>/`).

Why interleaved. This box is never quiet (0337): the fleet, the learner, the analyst and other agents'
builds share four cores. One clean-touch build per variant measures the box's mood, not the flag, so
the variants are built round-robin (A, B, C, A, B, C, ...) with the order rotated each round, and the
verdict is on the *paired* per-round difference against the baseline — the same estimator
`ab` uses, so a load swing cancels instead of landing on whichever variant ran during it.
Each observation records the load average it was taken under.

The job is the shipped one: `cargo build --release --workspace --bins` with `SVANBOT_COMMIT` pinned
(release.sh builds exactly this), and the touch is one appended comment line in a crate that job
rebuilds, restored to the byte afterwards (`--touch`, default a bot source file; 0302 measured the
same loop). Distinct profiles need distinct `-C metadata`, so each variant gets its own target
directory under `--target-root` and its own cargo lock, which also keeps these builds from
serialising with the other agents' builds in `target/dev`.

Disk is the binding constraint (the root filesystem has a few GB free), hence `--mode paired`: the
baseline and one other variant are warm at a time and the rest of the variants' directories are
pruned between pairs. `--mode simultaneous` is the ticket's literal A,B,C,... round-robin and needs
every variant's directory at once; the script refuses to start it when the projection does not fit
`--min-free-gb`.

Adoption is not this script's call: a variant is adoptable only when its build time clears the
two-minute rule *and* `ab` shows a runtime gain whose 95% interval excludes 1 on identical
checksums (0335 rule 4). Verdicts here are about the build half only.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import shutil
import signal
import statistics
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_TOUCH = "crates/apps/bot/src/multiway.rs"
DEFAULT_JOB = ["build", "--release", "--workspace", "--bins"]
DEFAULT_BINS = ["bench", "sim", "sv10-bot"]
# `false` is not `off`: rustc gets `-C embed-bitcode=no` and no `-C lto` for `false` (what
# [profile.release] has today), while `off` passes `-C lto=off` and still embeds bitcode in every
# rlib — a different build from the shipped one, so it is not the baseline spelling.
LTO_VALUES = ("false", "thin", "fat")
CGU_VALUES = (256, 16, 1)
# Student's t at 95%, two-sided, by degrees of freedom (same table as sv10-stats `t975`).
T975 = {1: 12.71, 2: 4.31, 3: 3.19, 4: 2.78, 5: 2.58, 6: 2.45, 7: 2.37,
        8: 2.31, 9: 2.27, 10: 2.23, 14: 2.15, 19: 2.10, 29: 2.05}


def t975(df: int) -> float:
    # Rounded upward; a lower tabulated df keeps missing rows and the finite tail conservative.
    for k in sorted(T975, reverse=True):
        if df >= k:
            return T975[k]
    return math.inf


def parse_variant(spec: str):
    """`fat1` (preset), `off256` (preset) or `name=lto,cgu[,[+|-]inc]` (custom)."""
    if "=" in spec:
        name, rest = spec.split("=", 1)
        parts = rest.split(",")
        lto = parts[0] if parts[0] else "false"
        cgu = int(parts[1]) if len(parts) > 1 and parts[1] else 256
        inc = None
        if len(parts) > 2 and parts[2]:
            inc = parts[2] in ("1", "+", "inc", "true", "on")
        return name, lto, cgu, inc
    m = re.fullmatch(r"(off|false|thin|fat)(256|16|1)", spec)
    if not m:
        raise SystemExit(f"unknown variant {spec!r}: use off256|thin16|fat1|... or name=lto,cgu")
    lto = "false" if m.group(1) in ("off", "false") else m.group(1)
    return spec, lto, int(m.group(2)), None


def env_for(args, lto, cgu, inc):
    env = dict(os.environ)
    env["CARGO_PROFILE_RELEASE_LTO"] = lto
    env["CARGO_PROFILE_RELEASE_CODEGEN_UNITS"] = str(cgu)
    if inc is not None:
        env["CARGO_PROFILE_RELEASE_INCREMENTAL"] = "true" if inc else "false"
    # release.sh exports the commit so the shipped binaries carry it (lesson 37: an env var that
    # feeds env! recompiles everything when it changes). Pinned here so it never moves mid-run.
    if args.commit:
        env["SVANBOT_COMMIT"] = args.commit
    return env


def loadavg() -> str:
    try:
        with open("/proc/loadavg") as fh:
            return fh.read().split()[0]
    except OSError:
        return "?"


def concurrent_builds() -> int:
    """Other cargo/rustc processes right now (this one's own are not counted at call time)."""
    try:
        out = subprocess.run(["pgrep", "-c", "-x", "rustc"], capture_output=True, text=True)
        return int(out.stdout.strip() or 0)
    except (OSError, ValueError):
        return 0


def free_gb(path: str) -> float:
    probe = path
    while probe and not os.path.exists(probe):
        probe = os.path.dirname(probe)
    return shutil.disk_usage(probe or "/").free / 1e9


def dir_bytes(path: str) -> int:
    total = 0
    for root, _, files in os.walk(path):
        for f in files:
            try:
                total += os.lstat(os.path.join(root, f)).st_size
            except OSError:
                pass
    return total


def sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def crate_of(path: str) -> tuple[str, str]:
    """(package name, crate directory) of the package owning `path`."""
    d = os.path.dirname(os.path.abspath(path))
    while d != "/" and not os.path.exists(os.path.join(d, "Cargo.toml")):
        d = os.path.dirname(d)
    if d == "/":
        raise SystemExit(f"{path} is not inside a cargo package")
    name = None
    with open(os.path.join(d, "Cargo.toml")) as fh:
        for line in fh:
            m = re.match(r'\s*name\s*=\s*"([^"]+)"', line)
            if m:
                name = m.group(1)
                break
    if not name:
        raise SystemExit(f"no package name in {d}/Cargo.toml")
    return name, os.path.relpath(d, ROOT)


class Touch:
    """The one-file edit, with ownership: never write over someone else's change."""

    def __init__(self, rel_path: str):
        self.rel = rel_path
        self.path = os.path.join(ROOT, rel_path)
        with open(self.path, "rb") as fh:
            self.canonical = fh.read()
        self.expected = self.canonical
        self.crate, self.crate_dir = crate_of(self.path)

    def _current(self) -> bytes:
        with open(self.path, "rb") as fh:
            return fh.read()

    def check(self):
        got = self._current()
        if got != self.expected:
            raise SystemExit(
                f"{self.rel} changed under this run (not by it) — stopping rather than overwriting it.\n"
                f"  expected {len(self.expected)} B sha256 {hashlib.sha256(self.expected).hexdigest()[:16]}\n"
                f"  found    {len(got)} B sha256 {hashlib.sha256(got).hexdigest()[:16]}")
        return got

    def edit(self, variant: str, round_no: int):
        self.check()
        body = self.canonical
        if body and not body.endswith(b"\n"):
            body += b"\n"
        body += f"// 0337 build-ab: {variant} round {round_no}\n".encode()
        with open(self.path, "wb") as fh:
            fh.write(body)
        self.expected = body

    def restore(self):
        if self._current() == self.expected:
            with open(self.path, "wb") as fh:
                fh.write(self.canonical)
            self.expected = self.canonical
        elif sha256(self.path) != hashlib.sha256(self.canonical).hexdigest():
            # Someone edited it while this run's marker was in: their change is left alone.
            raise SystemExit(f"{self.rel} carries an edit that is not this run's — restored nothing; "
                             f"check it by hand")


class Runner:
    def __init__(self, args, touch: Touch):
        self.args = args
        self.touch = touch
        self.logs = os.path.join(ROOT, args.target_root, "logs")
        os.makedirs(self.logs, exist_ok=True)

    def target_dir(self, variant: str) -> str:
        return os.path.join(ROOT, self.args.target_root, variant)

    def build(self, variant: str, lto: str, cgu: int, inc, tag: str, timings: bool, touch: bool,
              round_no: int):
        """One cargo invocation. Returns (seconds, log path, crate recompiled, load before/after)."""
        args = self.args
        tdir = self.target_dir(variant)
        os.makedirs(tdir, exist_ok=True)
        if touch:
            self.touch.edit(variant, round_no)
        env = env_for(args, lto, cgu, inc)
        env["CARGO_TARGET_DIR"] = tdir
        cmd = [shutil.which("cargo") or "cargo", *args.job]
        if timings:
            cmd.append("--timings")
        log = os.path.join(self.logs, f"{variant}-{tag}.log")
        before = f"{loadavg()}/{concurrent_builds()}r"
        t0 = time.monotonic()
        try:
            proc = subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True,
                                  timeout=args.timeout)
            out = proc.stdout + proc.stderr
            rc = proc.returncode
        except subprocess.TimeoutExpired as exc:
            # TimeoutExpired retains bytes even when subprocess.run used text=True.
            out = "".join(
                stream.decode("utf-8", errors="replace") if isinstance(stream, bytes) else stream or ""
                for stream in (exc.stdout, exc.stderr)
            )
            rc = -9
        took = time.monotonic() - t0
        after = f"{loadavg()}/{concurrent_builds()}r"
        with open(log, "w") as fh:
            fh.write(f"# {' '.join(cmd)}\n# CARGO_TARGET_DIR={tdir}\n"
                     f"# CARGO_PROFILE_RELEASE_LTO={lto} CARGO_PROFILE_RELEASE_CODEGEN_UNITS={cgu}\n"
                     f"# exit {rc} in {took:.2f}s, load {before}->{after}\n\n{out}")
        if rc != 0:
            raise SystemExit(f"{variant} {tag}: cargo exited {rc} (log {log})")
        if "`release` profile" not in out:
            raise SystemExit(f"{variant} {tag}: cargo did not build the release profile — the "
                             f"[profile.release] knobs did not apply (log {log})")
        compiled = self.touch.crate if touch else None
        fresh = compiled is not None and f"Compiling {compiled} v" in out
        if touch and not fresh:
            raise SystemExit(
                f"{variant} {tag}: {self.touch.rel} did not dirty {compiled} — cargo had nothing to "
                f"rebuild, so this round is not a one-file rebuild (log {log})")
        if timings:
            for src in self.timing_files(tdir):
                shutil.copy2(src, os.path.join(self.logs, f"timing-{variant}-{tag}.html"))
        return took, log, fresh, before, after

    @staticmethod
    def timing_files(tdir: str):
        d = os.path.join(tdir, "cargo-timings")
        if not os.path.isdir(d):
            return []
        return sorted((os.path.join(d, f) for f in os.listdir(d) if f.endswith(".html")),
                      key=os.path.getmtime)[-1:]

    def keep_bins(self, variant: str):
        """Copy the bench binaries aside so ab can use them after the dir is pruned."""
        out = os.path.join(ROOT, self.args.target_root, "bins", variant)
        os.makedirs(out, exist_ok=True)
        kept = {}
        for name in self.args.bin:
            src = os.path.join(self.target_dir(variant), "release", name)
            if os.path.exists(src):
                dst = os.path.join(out, name)
                shutil.copy2(src, dst)
                kept[name] = {"bytes": os.path.getsize(dst), "sha256": sha256(dst)}
            else:
                print(f"  warning: no {name} in {variant}/release — the job did not build it",
                      file=sys.stderr)
        return kept


def stages_from_timing(path: str) -> dict:
    """Per-stage summary of a `cargo --timings` report (lesson 37: name the stage, do not guess).

    Cargo's HTML carries its unit list as `const UNIT_DATA = [...]`, one entry per unit with the
    target kind and how long it took. Unit durations overlap (they run in parallel), so the sums are
    work, not wall time; `slowest` is what to read when a variant blows the budget.
    """
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            txt = fh.read()
    except OSError:
        return {}
    m = re.search(r"const UNIT_DATA = (\[.*?\]);", txt, re.S)
    if not m:
        return {}
    units = json.loads(m.group(1))
    total = re.search(r"Total time:</td><td>([^<]+)", txt)
    kinds: dict[str, float] = {}
    for u in units:
        # Empty `target` is a crate's own compile/metadata unit; a named one is a build script, a
        # binary (whose duration carries the LTO link for LTO variants) or a test.
        kind = (u.get("target") or "").strip() or "crate (lib/rmeta)"
        kinds[kind] = round(kinds.get(kind, 0.0) + u.get("duration", 0.0), 2)
    slowest = sorted(units, key=lambda u: -(u.get("duration") or 0))[:6]
    return {
        "units": len(units),
        "total_wall": total.group(1) if total else None,
        "work_by_kind_s": dict(sorted(kinds.items(), key=lambda kv: -kv[1])),
        "slowest": [{"unit": f"{u['name']} {u.get('version', '')}".strip(),
                     "kind": (u.get("target") or "").strip() or "crate (lib/rmeta)",
                     "s": round(u.get("duration") or 0, 2)} for u in slowest],
    }


def main():
    argv = sys.argv[1:]
    passthrough = []
    if "--" in argv:
        i = argv.index("--")
        argv, passthrough = argv[:i], argv[i + 1:]
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("variants", nargs="*", default=None,
                   help="off256|off16|off1|thin256|...|fat1, all, or name=lto,cgu,inc")
    p.add_argument("--baseline", default="off256", help="variant every delta is paired against")
    p.add_argument("--rounds", type=int, default=5, help="interleaved rounds (default 5)")
    p.add_argument("--mode", choices=("paired", "simultaneous"), default="paired")
    p.add_argument("--budget", type=float, default=120.0, help="the 2-minute rule, seconds")
    p.add_argument("--touch", default=DEFAULT_TOUCH, help="file whose one-line edit is timed")
    p.add_argument("--target-root", default="target/dev/lto-ab")
    p.add_argument("--bins", dest="bin", action="append", default=None,
                   help=f"binaries to keep per variant (default {' '.join(DEFAULT_BINS)})")
    p.add_argument("--min-free-gb", type=float, default=3.0,
                   help="refuse to warm a variant below this much free space")
    p.add_argument("--estimate-gb", type=float, default=1.2,
                   help="projected footprint of one variant, for the fit check")
    p.add_argument("--timings", default="warm", choices=("warm", "rounds", "all", "off"))
    p.add_argument("--commit", default=None, help="SVANBOT_COMMIT to pin (default: HEAD)")
    p.add_argument("--timeout", type=float, default=1800.0, help="per-build timeout, seconds")
    p.add_argument("--keep-dirs", action="store_true", help="do not prune between pairs")
    p.add_argument("--report", default=None, help="write the JSON report here too")
    p.add_argument("--plan", action="store_true", help="print the plan and exit")
    args = p.parse_args(argv)
    args.bin = args.bin or list(DEFAULT_BINS)
    if passthrough:
        # The knobs are `[profile.release]` settings, so the job must build that profile: a dev
        # profile build would ignore every one of them and quietly measure the wrong binary.
        named = [a for a in passthrough if a in ("--dev", "--debug", "--profile", "--testing", "--tests")]
        if named:
            raise SystemExit(f"`-- {' '.join(named)}` builds another profile; the knobs are "
                             f"[profile.release] settings. Drop it.")
        args.job = ["build", "--release", *[a for a in passthrough if a != "--release"]]
    else:
        args.job = list(DEFAULT_JOB)
    if args.commit is None:
        args.commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT,
                                     capture_output=True, text=True).stdout.strip()
    root = os.path.abspath(os.path.join(ROOT, args.target_root))
    for forbidden in ("target/release", "target/stage"):
        if root.startswith(os.path.join(ROOT, forbidden)):
            raise SystemExit(f"--target-root {args.target_root} would write the live build; refusing")

    specs = args.variants or []
    if not specs or specs == ["all"]:
        specs = [f"{l}{c}" for l in ("off", "thin", "fat") for c in (256, 16, 1)]
    variants = [parse_variant(s) for s in specs]
    base = parse_variant(args.baseline)
    if base[0] not in [v[0] for v in variants]:
        variants.insert(0, base)
    if args.mode == "simultaneous":
        need = args.estimate_gb * len(variants)
        if free_gb(ROOT) - need < args.min_free_gb:
            raise SystemExit(
                f"simultaneous mode projects {need:.1f} GB for {len(variants)} variants but only "
                f"{free_gb(ROOT):.1f} GB is free (keeping {args.min_free_gb:.1f} GB back). "
                f"Use --mode paired, lower --estimate-gb, or free disk first.")

    touch = Touch(args.touch)
    runner = Runner(args, touch)
    print(f"0337 build-ab: job `cargo {' '.join(args.job)}`, touch {args.touch} "
          f"({touch.crate} in {touch.crate_dir}), commit {args.commit}", file=sys.stderr)
    print(f"  baseline {base[0]}  variants {[v[0] for v in variants if v[0] != base[0]]}  "
          f"rounds {args.rounds}  mode {args.mode}  budget {args.budget:.0f}s", file=sys.stderr)
    for name, lto, cgu, inc in variants:
        print(f"    {name:10s} lto={lto:5s} codegen-units={cgu:3d} "
              f"incremental={'manifest' if inc is None else inc}  dir {args.target_root}/{name}",
              file=sys.stderr)
    if args.plan:
        return

    report = {"ticket": "0337", "job": args.job, "touch": args.touch, "crate": touch.crate,
              "commit": args.commit, "mode": args.mode, "rounds": args.rounds,
              "budget_s": args.budget, "baseline": base[0], "load_avg_at_start": loadavg(),
              "variants": {}, "binaries": {}, "started": time.strftime("%FT%T%z")}
    times = {v[0]: [] for v in variants}
    loads = {v[0]: [] for v in variants}
    # Paired observations live here, not in `times`: in paired mode the baseline is built in every
    # pair, so its rounds list is longer than any other variant's and the pairs must be taken from
    # the slice each pair produced.
    paired = {v[0]: {"deltas": [], "ratios": [], "pairs": 0} for v in variants}
    done: set[str] = set()

    def warm(v):
        name, lto, cgu, inc = v
        tdir = runner.target_dir(name)
        if not os.path.isdir(tdir):
            if free_gb(ROOT) < args.min_free_gb:
                raise SystemExit(f"{free_gb(ROOT):.1f} GB free, under --min-free-gb "
                                 f"{args.min_free_gb}: prune before warming {name}")
            print(f"  warming {name} (cold; several minutes under load) ...", file=sys.stderr)
        took, log, _, before, after = runner.build(
            name, lto, cgu, inc, "warm", args.timings in ("warm", "all"), touch=False, round_no=0)
        report["variants"].setdefault(name, {})
        report["variants"][name].update(
            {"lto": lto, "codegen_units": cgu, "incremental": inc, "warm_s": round(took, 2),
             "target_dir": os.path.relpath(tdir, ROOT), "load": [before, after], "log": os.path.relpath(log, ROOT)})
        report["binaries"].setdefault(name, runner.keep_bins(name))
        done.add(name)
        print(f"  {name} warm {took:.1f}s, dir {dir_bytes(tdir) / 1e9:.2f} GB, "
              f"load {before}->{after}", file=sys.stderr)

    def prune(keep: set[str]):
        for v in variants:
            name = v[0]
            if name in keep or name == base[0] or name not in done:
                continue
            if args.keep_dirs:
                continue
            d = runner.target_dir(name)
            if os.path.isdir(d):
                shutil.rmtree(d)
                print(f"  pruned {os.path.relpath(d, ROOT)}", file=sys.stderr)
                done.discard(name)

    def rounds_for(active):
        start = {v[0]: len(times[v[0]]) for v in active}
        for r in range(args.rounds):
            order = active[r % len(active):] + active[:r % len(active)]
            for name, lto, cgu, inc in order:
                took, log, _, before, after = runner.build(
                    name, lto, cgu, inc, f"r{r + 1}", args.timings in ("rounds", "all"),
                    touch=True, round_no=r + 1)
                times[name].append(took)
                loads[name].append(f"{before}->{after}")
                print(f"  round {r + 1}/{args.rounds} {name:10s} {took:7.1f}s  "
                      f"(load {before}->{after})", file=sys.stderr)
        # Pair the rounds this pass just recorded against the baseline's, round by round.
        if any(v[0] == base[0] for v in active):
            for name, _, _, _ in active:
                if name == base[0]:
                    continue
                mine = times[name][start[name]:]
                theirs = times[base[0]][start[base[0]]:]
                for x, y in zip(mine, theirs):
                    paired[name]["deltas"].append(x - y)
                    paired[name]["ratios"].append(x / y)
                if mine and len(mine) == len(theirs):
                    paired[name]["pairs"] += 1

    try:
        if args.mode == "simultaneous":
            for v in variants:
                warm(v)
            rounds_for(variants)
        else:
            warm(base)
            for v in variants:
                if v[0] == base[0]:
                    continue
                prune({v[0]})
                warm(v)
                rounds_for([base, v])
                prune({v[0]})
    finally:
        touch.restore()

    # Paired per-round deltas against the baseline: the estimator the ticket asks for.
    for name, _, _, _ in variants:
        entry = report["variants"][name]
        own = times[name]
        entry["rounds_s"] = [round(t, 2) for t in own]
        entry["loads"] = loads[name]
        if not own:
            continue
        entry["median_s"] = round(statistics.median(own), 2)
        timing = os.path.join(ROOT, args.target_root, "logs", f"timing-{name}-warm.html")
        if os.path.exists(timing):
            entry["stages"] = stages_from_timing(timing)
        half = t975(len(own) - 1) * statistics.stdev(own) / math.sqrt(len(own)) if len(own) > 1 else 0
        entry["mean_ci95"] = [round(statistics.mean(own) - half, 2), round(statistics.mean(own) + half, 2)]
        lo, hi = entry["mean_ci95"]
        entry["build_within_budget"] = ("yes" if hi <= args.budget else
                                        "no" if lo > args.budget else "unclear")
        if name != base[0] and paired[name]["deltas"]:
            deltas = paired[name]["deltas"]
            ratios = paired[name]["ratios"]
            dh = t975(len(deltas) - 1) * statistics.stdev(deltas) / math.sqrt(len(deltas)) if len(deltas) > 1 else 0
            rh = t975(len(ratios) - 1) * statistics.stdev(ratios) / math.sqrt(len(ratios)) if len(ratios) > 1 else 0
            entry["paired_delta_s"] = {"pairs": paired[name]["pairs"], "per_round": [round(d, 2) for d in deltas],
                                       "mean": round(statistics.mean(deltas), 2),
                                       "ci95": [round(statistics.mean(deltas) - dh, 2),
                                                round(statistics.mean(deltas) + dh, 2)]}
            entry["paired_ratio"] = {"mean": round(statistics.mean(ratios), 4),
                                     "ci95": [round(statistics.mean(ratios) - rh, 4),
                                              round(statistics.mean(ratios) + rh, 4)]}
    print(f"{'variant':10s} {'median':>8s} {'paired delta vs ' + base[0]:>26s}  {'2 min rule':>10s}",
          file=sys.stderr)
    for name, _, _, _ in variants:
        e = report["variants"][name]
        if "median_s" not in e:
            continue
        d = e.get("paired_delta_s", {}).get("mean")
        ds = "—" if d is None else f"{d:+.1f}s ({e['paired_ratio']['ci95'][0]:.2f}–{e['paired_ratio']['ci95'][1]:.2f})"
        print(f"{name:10s} {e['median_s']:7.1f}s {ds:>26s}  {e['build_within_budget']:>10s}",
              file=sys.stderr)
    text = json.dumps(report, indent=1)
    print(text)
    if args.report:
        with open(args.report, "w") as fh:
            fh.write(text + "\n")
    # The build half is not an adoption decision: the runtime half goes through ab on the
    # kept binaries, against the frozen 0335 fixture, and a gain counts only if the interval
    # excludes 1 with matching checksums.
    bins = os.path.join(ROOT, args.target_root, "bins")
    base_bin = os.path.join(bins, base[0], "bench")
    for name, _, _, _ in variants:
        if name == base[0] or not os.path.exists(os.path.join(bins, name, "bench")):
            continue
        print(f"next: target/dev/release/ab {os.path.relpath(base_bin, ROOT)} "
              f"{os.path.relpath(os.path.join(bins, name, 'bench'), ROOT)} "
              f"--suite learner --repeat 10   # then --suite live", file=sys.stderr)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(130))
    main()
