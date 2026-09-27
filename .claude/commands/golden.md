---
description: The golden-snapshot workflow for a change that is meant to alter behaviour. Regenerate only after reading and justifying every changed line.
allowed-tools: Bash(python3 scripts/test.py:*), Bash(UPDATE_GOLDEN=1 python3 scripts/test.py:*), Bash(git diff:*), Read, Grep, Glob
---

The golden snapshot is this project's determinism check. It is
`crates/apps/core/tests/golden/core.json`, written by the `characterization` test in `sv10-core`, and
it records the decision core's output over a fixed set of hands. Any change that alters a decision
moves it, which is the point: the snapshot is how a behaviour change announces itself instead of
arriving quietly.

## The rule

`AGENTS.md` section 4:

> The golden snapshot is the determinism check. If your change is intended to alter behaviour, it
> will fail, and that is it working.

> Never regenerate the golden file to make a red test go green. If you cannot explain why each
> changed line should change, the change is wrong.

## Before you regenerate

Answer these, in writing, in the pull request body or the commit:

1. Was altering behaviour the intent of this change? A speed-only change must reproduce the snapshot
   exactly. If it moved and you did not mean it to, that is a bug — find it, do not snapshot it.
2. For **every** changed entry: why should *this* line differ? "It changed because I regenerated it"
   is not an answer.
3. If the change is a strategy change, where is its paired simulation? `AGENTS.md` section 4 requires
   one for every behaviour change (`/measure`).

If you cannot answer all three, **stop and do not regenerate.** Say what you cannot explain. A
snapshot regenerated to clear a red run destroys the only record of what the change did, and the
diff a reviewer would have used to check it is gone.

## Regenerating

```
python3 scripts/test.py characterization          # see it fail, and read the failure
UPDATE_GOLDEN=1 python3 scripts/test.py           # write the new snapshot
python3 scripts/test.py                           # confirm it is green without the flag
```

Then read the diff of `crates/apps/core/tests/golden/core.json` and account for each change before
committing it. Regenerating without reading the diff is the same as not having the check.

`UPDATE_GOLDEN` is read by the test itself (`crates/apps/core/tests/characterization.rs`), so it
works through `scripts/test.py` and through a direct `cargo test` alike.
