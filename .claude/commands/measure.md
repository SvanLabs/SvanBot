---
description: The measurement a performance change has to carry, on the same machine against the previous commit. Use for any speed-only or throughput change.
allowed-tools: Bash(scripts/bench-ab.py:*), Bash(python3 scripts/bench-ab.py:*), Bash(CARGO_TARGET_DIR=target/dev cargo build:*), Bash(git rev-parse:*), Bash(git log:*), Read, Grep, Glob
---

A performance change is not a change until it has a number.

`AGENTS.md` section 4:

> Every behaviour change passes a paired simulation, and every performance change carries a
> measurement taken on the same machine against the previous commit. A speed claim with no number
> behind it is not a change, and neither is a strategy change with no simulation.

`CONTRIBUTING.md` section 9 makes it a **MUST**, and the reason is `docs/LESSONS.md` 1, 3 and 6:
several optimisations that looked obviously right measured as no gain, and this machine is shared, so
a before and an after measured an hour apart differ by load rather than by code.

## The measurement

Build both revisions and compare them with the paired harness:

```
CARGO_TARGET_DIR=target/dev cargo build --profile release -p sv10-core --bin bench
python3 scripts/bench-ab.py BASE NEW --suite learner --repeat 10
```

`BASE` is the previous commit's `bench` binary and `NEW` is this change's — the same machine, the
same suite, back to back. The harness alternates them on each repeat and reports the paired ratio per
metric with a 95% t-interval. **A gain counts only when the interval excludes 1**, and the checksums
must match: a speed-only change computes the same thing, and a differing checksum means it does not
(`scripts/bench-ab.py` exits 2 and says so).

Suites are `learner` (table runs per wall and per CPU second), `live` (decision latency p50…p999 at
the live sample budget) and `micro` (evaluator, sampler, equity, RNG). `docs/OPERATIONS.md` has the
full procedure, the fixtures and the phase baselines to compare against.

## The guard that must not move

`docs/LESSONS.md` 3:

> speed-only changes must reproduce the guard `SIM_B='{"call_margin":0.005}' sim paired 12 600` exactly.

```
SIM_B='{"call_margin":0.005}' ./target/release/sim paired 12 600
```

A speed-only change must print the same result as before it. If the number moved, the change is not
speed-only, and it needs a paired simulation and a golden-snapshot justification instead (`/golden`).

## What the change has to say

State, in the commit or the pull request:

- the command that produced the numbers, and the suite and repeat count;
- the before and the after, with the interval, not a ratio on its own;
- that the checksums matched, and the guard result;
- if the interval does not exclude 1, then the honest conclusion is **no measurable change**. Say
  that, and say what it should not be adopted on that evidence.
