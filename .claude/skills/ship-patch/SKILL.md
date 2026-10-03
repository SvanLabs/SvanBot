---
name: ship-patch
description: Deliver every change as a patch file, never as freehand edits to the fleet checkout. Use when writing, reviewing, or landing any code, config, or doc change on a box where the fleet is running.
allowed-tools: Bash(git diff:*), Bash(git format-patch:*), Bash(git apply:*), Bash(git status:*), Bash(git log:*), Bash(git rev-parse:*), Bash(patch:*), Bash(ls:*), Read, Grep, Glob
---

# Ship patches, not freehand code

A running fleet hot-swaps from the checkout it was installed from, so an edit to that tree is a
deployment whether you meant it or not. The change always leaves your session as a patch file; the
fleet checkout is read-only to you. Landing is a separate, gated step that applies the patch in a
clean tree.

## Write the patch

1. Read, don't touch, the fleet checkout. Do all work in an isolated checkout or worktree cut from
   `main` — never in the tree the fleet runs from.
2. One logical change per patch, in exactly one crate layer (`AGENTS.md`). A fix plus a rename is
   two patches.
3. Export with history, not just a diff, so the message travels with the change:
   ```sh
   git format-patch -o artifacts/patches/<issue>-<slug>/ origin/main   # gitignored tray: one directory per submission
   ```
   A series stays in order inside its directory; never squash two decisions into one file by hand.
   `scripts/patch-inbox.sh land` (#710) is the machine that lands a submission: shape preflight,
   apply-check, the full gate in a landing worktree, provenance, branch and pull request — the
   manual protocol below remains for anything it refuses.
4. The patch message is the commit message: `<area>: <what it does>`, a body that says why,
   `Closes #<issue>` in its own paragraph, and `Generated-by: <tool>/<model>` last
   (`docs/CONTRIBUTING.md`).

## Patch hygiene

Before the patch leaves your session, each of these is true:

- `git apply --check` passes against a clean `main` — a patch that only applies to your dirty tree
  is not shippable. Rebase and re-export when `main` moved under you; never hand-edit someone
  else's patch to force it to fit.
- The behaviour or performance proof travels with it: the failing test first for a bug, the paired
  simulation for a behaviour change, the same-machine measurement for a performance claim. A patch
  that says "trust me" is freehand code with a filename.
- No `TODO`, `FIXME`, `XXX` or `HACK`; every backticked repository path names a file that exists
  (`scripts/docs-check.py` will reject the ones that don't).
- The golden snapshot is untouched unless the behaviour change is intended and each changed line is
  explained in the message (`.claude/commands/golden.md`).

## Land the patch

Landing is the only write path to anything the fleet reads:

1. `git apply --check` in a clean tree, then `scripts/check.sh full` on the applied result
   (`.claude/commands/gate.md`). Red here is red in CI — fix the patch, not the gate.
2. Commit in the message shape above and open the pull request into `main`. Small diffs a reviewer
   can verify beat large ones that have to be trusted.
3. Install only through the verified release path: `scripts/release.sh` builds, tests and installs
   atomically and the fleet hot-swaps between turns; `scripts/update.sh` is the same path from the
   dashboard; `scripts/rollback.sh` restores a verified snapshot (`docs/OPERATIONS.md`). A merge
   alone is not an install — confirm with `/api/health`.

## What a patch never bypasses

The patch is a delivery format, not a fast lane. It does not skip the paired simulation, the
same-machine measurement, the golden justification, the 95%-lower-bound promotion gate, or review.
`ALLOW_DIRTY=1 SKIP_TESTS=1` is for emergencies (see `scripts/release.sh`), and an emergency patch
says which emergency in its message.

## The traps this skill exists for

- **Editing the fleet checkout "just to try something."** Build trial binaries with
  `CARGO_TARGET_DIR=target/dev cargo build --profile release` — never into `target/release`,
  where a release build reaches live play untested.
- **A stale patch.** `git log` on `main` before landing; a patch exported yesterday that still
  applies can still be wrong. Re-check, don't assume.
- **Inbox hygiene.** `artifacts/patches/` is a tray, not storage: landed or rejected patches get
  deleted. What persists is the commit and the pull request, not the file.
- **Measuring against the wrong build.** After any install, confirm `/api/health` `commit` equals
  the landed commit before concluding anything (`.claude/skills/verify-install/SKILL.md`).
