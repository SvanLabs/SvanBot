# v11 is written from scratch on its own history

v11.0.0 is a new codebase, not a refactor of v10. It lives on the orphan branch `v11`, starts from an
empty tree, and no v10 file is copied into it. v10 stays readable as a reference at
`archive/v10/final` and the other `archive/v10/*` tags.

The operator chose this over refactoring v10 in place. v10's layout, scripts and accreted modules
were the problem being solved, and a refactor would have carried them forward one move at a time.
Two things keep the rewrite honest:
- the ship bar, a paired sim that must beat the v10.0.2 champion
- tests written first for every behaviour v11 keeps

`main` stays v10 until the release, because the v10 updater follows `origin/main`, and a v11 commit
there would reach the live fleet. At the release, with the operator's approval, `v11` replaces `main`.

## Status

accepted (2026-11-15, map V11-00).

## Considered options

- **Refactor v10 in place**, in a series of PRs to `main`: rejected by the operator as not the full
  rework asked for. It would also have shipped every intermediate state to the live fleet.
- **A new repository**: rejected. The v11 map, its tickets and the project's labels live in this one.
