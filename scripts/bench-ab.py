#!/usr/bin/env python3
"""Compare two `bench` builds on paired, alternating repeats (0335).

    scripts/bench-ab.py BASE_BIN NEW_BIN [--suite learner|live|micro] [--repeat 10] [-- extra bench args]

The machine is shared (the fleet, the analyst, the learner and other users' programs), so a before
and an after measured an hour apart differ by load, not by code. Each repeat runs A then B (or B
then A, alternating) back to back, and the verdict is on the paired ratio B/A per metric: its mean
and a 95% t-interval. A gain counts only when the interval excludes 1. Checksums must match: a
speed-only change computes the same thing.
"""
import json, math, statistics, subprocess, sys

# Metrics per suite and whether higher is better.
METRICS = {
    "learner": {"table_runs_per_cpu_s": True, "table_runs_per_s": True},
    "live": {"p50_ms": False, "p95_ms": False, "p99_ms": False, "p999_ms": False, "mean_ms": False, "cpu_s": False},
    "micro": {"eval_ns_per_hand": False, "sample_ns_per_combo": False, "deal_ns_per_sample_1opp": False,
              "deal_ns_per_sample_2opp": False, "reweight_ns_per_sample_1opp": False, "reweight_ns_per_sample_2opp": False,
              "equity_hu_samples_per_s": True, "rng_ns_per_u64": False},
}
T975 = {1: 12.71, 2: 4.30, 3: 3.18, 4: 2.78, 5: 2.57, 6: 2.45, 7: 2.36, 8: 2.31, 9: 2.26, 10: 2.23, 14: 2.14, 19: 2.09, 29: 2.05}


def t975(df):
    keys = sorted(T975)
    for k in keys:
        if df <= k:
            return T975[k]
    return 1.96


def run(binary, suite, extra):
    out = subprocess.run([binary, suite, *extra], capture_output=True, text=True, check=True).stdout
    return [json.loads(l) for l in out.splitlines() if l.startswith("{")][0]


def main():
    args = sys.argv[1:]
    extra = []
    if "--" in args:
        i = args.index("--")
        args, extra = args[:i], args[i + 1:]
    base, new = args[0], args[1]
    suite = args[args.index("--suite") + 1] if "--suite" in args else "micro"
    repeat = int(args[args.index("--repeat") + 1]) if "--repeat" in args else 10
    ratios = {m: [] for m in METRICS[suite]}
    sums = set()
    for i in range(repeat):
        order = [(base, "A"), (new, "B")] if i % 2 == 0 else [(new, "B"), (base, "A")]
        got = {}
        for binary, tag in order:
            got[tag] = run(binary, suite, extra)
        sums.add((got["A"]["checksum"], got["B"]["checksum"]))
        for m in ratios:
            if m in got["A"] and m in got["B"] and got["A"][m]:
                ratios[m].append(got["B"][m] / got["A"][m])
        print(f"repeat {i + 1}/{repeat}: " + ", ".join(f"{m} {got['A'].get(m, 0):.4g}->{got['B'].get(m, 0):.4g}" for m in ratios), file=sys.stderr)
    same = all(a == b for a, b in sums)
    report = {"suite": suite, "repeats": repeat, "checksums_match": same, "metrics": {}}
    for m, r in ratios.items():
        if len(r) < 2:
            continue
        mean = statistics.mean(r)
        half = t975(len(r) - 1) * statistics.stdev(r) / math.sqrt(len(r))
        higher_better = METRICS[suite][m]
        lo, hi = mean - half, mean + half
        better = (lo > 1) if higher_better else (hi < 1)
        worse = (hi < 1) if higher_better else (lo > 1)
        report["metrics"][m] = {"ratio": round(mean, 4), "ci95": [round(lo, 4), round(hi, 4)],
                                "verdict": "faster" if better else "slower" if worse else "no measurable change"}
    print(json.dumps(report, indent=1))
    if not same:
        print("CHECKSUMS DIFFER: the two builds do not compute the same thing", file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
