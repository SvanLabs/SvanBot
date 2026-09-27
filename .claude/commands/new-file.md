---
description: Check the 500-line-per-Rust-file limit and the size baseline before adding to a file. Use when a source file is getting long or a size check fails.
allowed-tools: Bash(scripts/check-file-size.sh:*), Bash(bash scripts/check-file-size.sh:*), Bash(wc:*), Read, Grep, Glob
---

No Rust source file goes over **500 physical lines** — code, comments, blank lines and inline tests
all count. `CONTRIBUTING.md` section 4 makes it a **MUST**: agents and reviewers read this code in
pieces, and a file that has to be read whole to be understood is one neither can check.

## Check where a file stands

```
scripts/check-file-size.sh            # check: exits 1 on a new or grown oversized file
scripts/check-file-size.sh --report   # also list every file over 400 lines
```

The check compares against `scripts/file-size-baseline.txt`, which lists the files that were already
over the limit when the rule was adopted, with their size at that moment. **The baseline may only
shrink.** An oversized file may get smaller, never larger, and it leaves the list once it is back
under 500. A new file over 500 fails outright.

`scripts/check-file-size.sh --baseline` rewrites the baseline from the files on disk. That is an
operator decision, not a fix: running it to clear a red check erases the record of which files are
over the limit and how far over, which is the whole mechanism. Do not run it to make a check pass.

## Splitting

- Past **400** lines, split before adding functionality. The target is **300**.
- Split by responsibility, domain concept or subsystem. Do not split into fragments that have to be
  read together to make sense, and do not create files to dodge the count — a file that exists only
  to hold the overflow of another is worse than the long file.
- A large inline `#[cfg(test)] mod tests` moves first, into a sibling `tests.rs` behind
  `#[cfg(test)] mod tests;`, as `crates/libs/venue/src/tracker` does. That is usually most of the
  overshoot and it costs nothing to read.
- Re-export deliberately when a public path moves, so callers keep their paths.
- Functions are the same argument at a smaller scale: under **50** lines, and one over **100** needs
  a reason in review.

## Exceptions

Named in the file or in review, with the reason: generated code, large test fixtures, and naturally
cohesive units whose split would hurt clarity — a single state machine, a codec table. "It is
easier not to split it" is not one of them, and neither is a deadline.
