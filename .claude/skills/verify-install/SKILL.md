---
name: verify-install
description: Answer "what is actually running?" — the commit the fleet is playing, whether it matches the checkout, and which builds can be rolled back to. Use after an update, before reporting or fixing a bug, or when the dashboard and the checkout look like they disagree.
allowed-tools: Bash(curl -s localhost:*), Bash(git rev-parse:*), Bash(git log:*), Bash(git status:*), Bash(tail:*), Bash(ls:*), Read, Grep, Glob
---

# What is the fleet actually running?

Three things can disagree: the commit in the checkout, the build installed in `target/release`, and
the build the process was started from. Most "the fix did not work" reports are one of the three being
the wrong one, and only the third is what the dashboard is showing you.

## Find out

```sh
curl -s localhost:5000/api/health          # {"ok":true,"version":…,"commit":…}
git rev-parse --short HEAD                 # the checkout
tail -3 artifacts/releases.log             # the installs, newest last
ls -t artifacts/release-snapshots | head   # what a rollback could go back to
```

`/api/health` needs no operator token, and its `commit` is the build the running process was compiled
from — not the checkout, and not the file you just edited.

## Read the answer

- **`commit` == `HEAD`** — the fleet is playing what you are looking at. Any result you get from it is
  a result about this code.
- **`commit` != `HEAD`** — the checkout moved without a release (a pull, a branch, a merge) or a
  release failed after the checkout moved. The fleet is playing an older build, so nothing you measure
  against it says anything about the change under test. Install first, re-check, then conclude.
- **`commit` is not in `git log` at all** — the checkout was reset or re-cloned since the install, or
  the snapshot is from another line of work. `refs/adopt/before-upstream-*` holds a head a checkout
  left behind when it moved onto this repository (`docs/OPERATIONS.md`).

A release is `scripts/release.sh`: it builds, tests, installs atomically and the fleet hot-swaps
between turns. `scripts/update.sh --check` prints `<behind> <origin commit>` without touching anything.

## The trap this skill exists for

`target/release` is where a running fleet hot-swaps from, so a release build that lands there reaches
live play whether or not it was tested — this is the first rule in `CLAUDE.md` because it is the one
that is cheap to get wrong. When a measurement, a benchmark or a bug report needs a binary, build into
`target/dev`:

```sh
CARGO_TARGET_DIR=target/dev cargo build --profile release
```

and install deliberately with `scripts/release.sh`. Roll back with `scripts/update.sh --rollback
<commit>` (the dashboard's Roll back button), which restores a verified snapshot and hot-swaps like any
install; `scripts/rollback.sh` refuses a snapshot whose data format the installed build cannot read.
