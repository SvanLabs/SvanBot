---
description: Run the SvanBot pre-commit gate and report any failure. Use before committing anything under crates/ or web/.
allowed-tools: Bash(scripts/check.sh commit:*), Bash(bash scripts/check.sh commit:*), Read, Grep, Glob
---

Run the pre-commit gate:

```
scripts/check.sh commit
```

That is the subset of the gate the git pre-commit hook runs: placeholder markers, AI provenance, the
staged secret scan, rustfmt, the 500-line file check, clippy `-D warnings`, and the golden snapshot
plus the property tests when `crates/` is staged. It defaults `CARGO_TARGET_DIR` to `target/dev`, so
running it cannot disturb a live fleet's `target/release`.

If it passes, say so and stop. `scripts/check.sh full` is the gate CI runs — run it before pushing.

## If it fails

Report the failing step and the cause, then fix the cause. Do not adjust the check.

- **Golden snapshot.** A failure here is the determinism check working. If the change was meant to
  alter behaviour, the snapshot is *supposed* to fail. Regenerating is a deliberate act with its own
  procedure and its own justification (`/golden`), never a way to clear a red run.
- **File size.** A file over 500 lines gets split, not exempted, and the baseline may only shrink
  (`/new-file`).
- **rustfmt.** The fix is `cargo fmt --all`. rustfmt is the source of truth; never hand-format
  against it.
- **clippy.** Fix the lint. An `#[allow(...)]` needs a comment on the same or the previous line
  saying why it is safe, and a crate-wide or workspace-wide one needs a ticket reference as well.
- **AI provenance.** The commit needs its `Generated-by: <tool>/<model>` trailer. The step exists to
  stop work, not to be skipped.
- **Placeholder markers.** `TODO`, `FIXME`, `XXX` and `HACK` fail the gate. Finish the work or open
  an issue; do not reword the marker.

## The rule that matters most

`AGENTS.md` section 4, in its own words:

> Never regenerate the golden file to make a red test go green. If you cannot explain why each
> changed line should change, the change is wrong.

And `AGENTS.md` section 7:

> If a test fails and you do not understand why, say so in the pull request rather than adjusting the
> test until it passes.

So: when a test fails for a reason you cannot explain, leave it failing and say so. Editing the test
until it agrees with the code is the one move that always makes a change worse, and it is the failure
a reviewer cannot see in the diff afterwards.
