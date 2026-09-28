# Trunk on `main` — Phase 0 and Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Get the repository onto a single branch, `main`, with a buildable tree, and fix the one line that makes the promotion gate's documented error rate false.

**Architecture:** Phase 0 is almost entirely deletion and text: the tree does not compile, three open pieces of work need landing, and the two-branch model with its promotion machinery (`dev`, `scripts/promote.sh`, `.github/workflows/promote.yml`) is replaced by trunk-based work on `main`. Phase 1 is a one-line change to the promotion gate plus the document that misdescribes it.

**Tech Stack:** Rust 2024 workspace, bash, GitHub Actions, `gh` CLI, Python 3 helper scripts (`scripts/check.sh`, `scripts/docs-check.py`).

**Spec:** `docs/superpowers/specs/2026-09-28-trunk-on-main-and-the-strength-program-design.md`

## Global Constraints

- **Build into `target/dev`, never `target/release`.** A fleet is running and hot-swaps from `target/release`. Every script defaults `CARGO_TARGET_DIR` to `target/dev`; a bare `cargo` invocation does not. Use `CARGO_TARGET_DIR=target/dev cargo …`.
- **Every commit carries a `Generated-by:` trailer** in the footer, plus `Co-Authored-By: Claude Code <noreply@anthropic.com>`. `scripts/provenance.py check` enforces it and `scripts/check.sh` runs that check.
- **Every pull request and issue body carries `Generated-by:`** in the body, not the title.
- **`PATH` needs `$HOME/.cargo/bin` and `$HOME/.local/bin`.** The shell here does not inherit `.bashrc`; prefix commands with `export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH" &&`.
- **`scripts/check.sh full` is the gate.** Run it before every push. `scripts/check.sh commit` is the fast subset.
- **No placeholder markers** — `TODO`, `FIXME`, `XXX`, `HACK` fail the gate.
- **500 lines per Rust file**, enforced by `scripts/check-file-size.sh` against a baseline that may only shrink.
- **Documents name only paths that exist**, enforced by `scripts/docs-check.py` over the list in `scripts/docs-check.live`. Deleting a file and leaving a document that names it is a gate failure. `docs/superpowers/` is exempt by not being listed.
- **Never regenerate the golden snapshot to make a red test pass.** Phase 1 changes no behaviour, so the golden file must not move.
- **`main`'s ruleset allows `merge` commits only** (`allowed_merge_methods: ["merge"]`); `dev`'s allows `squash` only. Until the ruleset is changed, pull requests into `main` merge with `--merge`.
- **A red `claude-review` job is not a blocker.** `.github/workflows/claude-code-review.yml` authenticates with an `ANTHROPIC_API_KEY` that is currently empty or quota-exhausted, so the job fails with `"api_error_status": 429` (`You've hit your session limit`) without ever reading the diff. It is **not** a required status — the ruleset on `main` requires only `check` — and it cannot re-run on its own, because it triggers on `[opened, synchronize, ready_for_review, reopened]` and a base retarget is an `edited` event. Merge when `check`, `gate` and `web` pass, and note the red job in your report as an infrastructure observation. **Stop** if any of those three fails, or if `claude-review` reports a finding about the diff rather than an API error — the distinction matters, and reading the job log is how you tell them apart.

## Review Focus

The spec implies these conditions and no task's tests exercise them. Each is pinned to the task that owns the code.

1. **A document still naming a deleted path.** `docs/LESSONS.md` entries 44 and 45 quote `scripts/promote.sh` in backticks, and `LESSONS.md` is a live document. Deleting the script without touching those entries fails `docs-check`. → Task 4.
2. **`scripts/check.sh` invoking a deleted test suite.** Line 165 runs `scripts/tests/promote.sh` and line 150 names it in a step description. The gate would fail on a missing file. → Task 4.
3. **A commit landing on `main` that the gate never saw.** The ruleset requires the `check` status, but only for pull requests; a direct `git push origin main` bypasses the merge path entirely. → Tasks 2, 3, 4 all push to feature branches only; Task 7 verifies nothing was pushed straight to `main`.
4. **The interim gate clause going inert again.** The existing tests pass both before and after the fix, which is precisely how the defect survived. → Task 8 adds the regression case that fails on the old code.
5. **A stale local ref reported as truth.** Local `main` and `dev` are behind `origin`; every comparison in this plan fetches first. A plan step that reads a local ref without fetching is reading history. → Tasks 2, 5, 7.

---

## Phase 0

### Task 1: Make the tree build again

The working tree does not compile. Every other task is blocked on it.

**Files:**
- Modify: `crates/libs/store/src/packed.rs:308`

**Interfaces:**
- Consumes: nothing.
- Produces: a compiling workspace. No signature changes.

- [ ] **Step 1: Confirm the failure**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /home/administrator/SvanBot
CARGO_TARGET_DIR=target/dev cargo check -p sv10-store --message-format short 2>&1 | tail -5
```

Expected: `crates/libs/store/src/packed.rs:308:30: error[E0277]: &&[(i64, std::string::String)] is not an iterator`

- [ ] **Step 2: Read the current line**

```bash
sed -n '299,314p' crates/libs/store/src/packed.rs
```

Expected: `pub fn compact_rows(conn: &Connection, codec: &Codec, column: Column, rows: &[(i64, String)]) -> Result<(usize, Option<i64>)>` at line 299, and `for (rowid, text) in &rows {` at line 308. `rows` is already a slice reference, so `&rows` is `&&[..]`.

- [ ] **Step 3: Apply the one-character fix**

Replace line 308:

```rust
        for (rowid, text) in &rows {
```

with:

```rust
        for (rowid, text) in rows {
```

`rows` is `&[(i64, String)]`, so this yields `&(i64, String)`, which destructures under match ergonomics into `rowid: &i64` and `text: &String`. The existing body needs no change: `codec.pack(&tx, column, text)` takes `&str` and `&String` deref-coerces, and `params![…, rowid]` takes `&i64` because `ToSql` is implemented for references.

- [ ] **Step 4: Verify the crate compiles**

```bash
CARGO_TARGET_DIR=target/dev cargo check -p sv10-store --message-format short 2>&1 | tail -5
```

Expected: no `error[E0277]`; the command exits 0.

- [ ] **Step 5: Run the store's own tests**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
python3 scripts/test.py store
```

Expected: PASS. `compact_rows` and `compact_candidates` are covered by the store's existing compaction tests; if any fail, the split changed behaviour and the fix is wrong — read the failure rather than adjusting the test.

- [ ] **Step 6: Cut the branch, commit, push and merge**

```bash
git checkout -b fix/store-compact-rows-iteration origin/main
git add crates/libs/store/src/packed.rs
git commit -F - <<'EOF'
store: compact_rows iterates its slice, not a reference to it

`compact_rows` takes `rows: &[(i64, String)]` and iterated `&rows`, which is
`&&[(i64, String)]` and is not an iterator, so sv10-store did not compile and
neither did anything above it. Deleting the `&` yields `&(i64, String)`, which
destructures under match ergonomics; the body needs no change.

Generated-by: claude-code/deepseek-flash
Co-Authored-By: Claude Code <noreply@anthropic.com>
EOF
git push -u origin fix/store-compact-rows-iteration
gh pr create --base main --title "store: compact_rows iterates its slice, not a reference to it" --body "Makes \`sv10-store\` compile. The uncommitted change in this checkout splits \`compact_batch\` into \`compact_candidates\` and \`compact_rows\`; the split did not build.

Generated-by: claude-code/deepseek-flash"
gh pr merge --merge
```

Then confirm it landed:

```bash
git fetch origin --prune && git log --oneline -1 origin/main
```

Expected: a merge commit naming this pull request.

Note: this branch is cut from `origin/main`, not from the current working branch, because the current working branch carries two commits already in `main` and Phase 0 is moving everything onto `main` anyway. The uncommitted `packed.rs` modification in the working tree is the change being fixed and committed here — do not discard it.

---

### Task 2: Land the two open pull requests onto `main`

PRs #456 (`scripts/setup.sh` builds through `scripts/release.sh`) and #457 (the wiring-table `installed` predicate, closing #315) both target `dev`. Retarget them before anything deletes `dev`.

**Files:**
- Modify (via PR #456): `scripts/setup.sh`
- Modify (via PR #457): `crates/apps/bot/src/api/wiring.rs`, `crates/apps/bot/src/review_wiring.rs`, `crates/apps/bot/src/review_wiring/tests.rs`, `web/src/games.tsx`, `web/src/types.ts`

**Interfaces:**
- Consumes: Task 1's compiling tree (the gate builds the workspace).
- Produces: nothing later tasks import. #457 changes the `wiring.v1` report shape; Task 4's documents do not depend on it.

- [ ] **Step 1: Confirm both are still open, and against which base**

```bash
cd /home/administrator/SvanBot
for n in 456 457; do gh pr view $n --json number,title,baseRefName,headRefName,mergeable,mergeStateStatus | jq -c .; done
```

Expected: both `"baseRefName":"dev"` and `"mergeable":"MERGEABLE"`.

- [ ] **Step 2: Retarget both to `main`**

```bash
gh pr edit 456 --base main
gh pr edit 457 --base main
```

Expected: no error. If `gh` reports `Resource not accessible by integration`, the token lacks write on pull requests and this task is a human action in the GitHub UI; the remaining steps are unchanged.

- [ ] **Step 3: Wait for the `check` run the retarget re-triggers, then read it**

```bash
sleep 30
gh pr checks 456
gh pr checks 457
```

Expected: a `check` run appears for each (the workflow triggers on `edited`). Wait for it to conclude before merging.

- [ ] **Step 4: Merge both with a merge commit**

```bash
gh pr merge 456 --merge
gh pr merge 457 --merge
```

`--merge` and not `--squash`: `main`'s ruleset allows `merge` commits only. Squash becomes available once the ruleset is changed, which needs repository admin and is not in this plan.

- [ ] **Step 5: Verify both merged and `main` moved**

```bash
git fetch origin --prune
git log --oneline -3 origin/main
gh issue view 315 --json state --jq .state
```

Expected: two new merge commits at the tip of `origin/main`; `#315` reads `CLOSED` (the pull request body closes it). `#15` remains open — its pull request body may not carry a closing keyword; if it is still open, close it in Task 6.

- [ ] **Step 6: Commit nothing**

This task changes no file locally. It is recorded as done when `origin/main` holds both merges.

---

### Task 3: Land the design spec on `main`

The spec is written and untracked. It lands on `main` as the first commit of the new trunk model.

**Files:**
- Create: `docs/superpowers/specs/2026-09-28-trunk-on-main-and-the-strength-program-design.md`
- Create: `docs/superpowers/plans/2026-09-28-trunk-on-main-phase-0-and-1.md` (this document)

**Interfaces:**
- Consumes: Task 2 — the branch must be cut from a `main` that already holds #457, so the spec's description of the wiring fix matches what is in the tree.
- Produces: the two documents every later task cites.

- [ ] **Step 1: Verify the spec is not tracked and no live document needs registering**

```bash
cd /home/administrator/SvanBot
git status --short docs/
grep -n "superpowers" scripts/docs-check.live || echo "not listed - exempt, as intended"
```

Expected: both spec files show as untracked (`??`), and `docs-check.live` does not mention `superpowers`. Leave it that way: `scripts/docs-check.py` exempts `docs/superpowers` by not listing it, which is the documented intent.

- [ ] **Step 2: Cut a branch from `origin/main` and stage both documents**

```bash
git fetch origin --prune
git checkout -b docs/strength-program-spec origin/main
git add docs/superpowers/specs/2026-09-28-trunk-on-main-and-the-strength-program-design.md
git add docs/superpowers/plans/2026-09-28-trunk-on-main-phase-0-and-1.md
git status --short
```

Expected: exactly two added files. If `crates/libs/store/src/packed.rs` appears, do not stage it — it belongs to Task 1's branch.

- [ ] **Step 3: Run the gate's documentation checks**

```bash
python3 scripts/docs-check.py && echo "docs-check ok"
bash scripts/check-file-size.sh
```

Expected: `docs-check: 26 documents, 0 problem(s)`. The count is the list length and does not change — `docs/superpowers` is exempt.

Note the interpreter: `scripts/docs-check.py` is Python and `scripts/check-file-size.sh` is bash. Running the second under `python3` dies with a `SyntaxError` at its line 21.

- [ ] **Step 4: Commit and push**

```bash
git commit -F - <<'EOF'
docs: the trunk-on-main and strength-program design

Records the decisions taken on 2026-09-28 and the code facts they rest on:
the policy has no game-theoretic machinery, the river errors have three
mechanical explanations in the pricing path, more Monte Carlo samples is
already known to gain nothing, the release is inside its 120 s budget, the
promotion gate's interim z clause is inert, and the store's write
transactions are all deferred.

Generated-by: claude-code/deepseek-flash
Co-Authored-By: Claude Code <noreply@anthropic.com>
EOF
git push -u origin docs/strength-program-spec
```

- [ ] **Step 5: Open the pull request and merge it**

```bash
gh pr create --base main --title "docs: the trunk-on-main and strength-program design" --body "$(cat <<'EOF'
Design document for the branch model change and the strength program, plus the
implementation plan for Phase 0 and Phase 1.

Both live under `docs/superpowers/`, which `scripts/docs-check.py` exempts by
not listing it, so their paths are not checked against the tree.

Generated-by: claude-code/deepseek-flash
EOF
)"
gh pr merge --merge
```

Expected: merged. Note the pull request number for Step 6.

- [ ] **Step 6: Confirm the documents are on `main`**

```bash
git fetch origin --prune
git cat-file -e origin/main:docs/superpowers/specs/2026-09-28-trunk-on-main-and-the-strength-program-design.md && echo "spec on main"
git cat-file -e origin/main:docs/superpowers/plans/2026-09-28-trunk-on-main-phase-0-and-1.md && echo "plan on main"
```

Expected: both lines print.

---

### Task 4: Delete the promotion machinery and rewrite the branch model

One change, because `scripts/docs-check.py` couples them: deleting `scripts/promote.sh` while any live document still names it fails the gate. This is the largest task in the plan and it is almost entirely deletion and text.

**Files:**
- Delete: `.github/workflows/promote.yml`, `scripts/promote.sh`, `scripts/tests/promote.sh`
- Modify: `scripts/check.sh:150,165`; `.github/workflows/check.yml` (push trigger); `.github/workflows/close-linked-issues.yml:37-40`; `.github/workflows/claude-maintainer.yml:3-7,47-51,139,141-143`; `AGENTS.md:20-34`; `CONTRIBUTING.md:23-26`; `README.md:330-337,376-379`; `llms.txt:29-34,78-89`; `docs/RELEASE.md:10-46`; `docs/LESSONS.md:203-235`; `.env.example:40-43`; `scripts/update.sh:156-157`
- **Not** modified, verified during prework: `docs/OPERATIONS.md` (no branch-model text), `docs/README.md` (branch-agnostic), `scripts/tests/update.sh` (its assertion is about the fixture's own branch, not the repository's)

**Interfaces:**
- Consumes: Task 3's `main`.
- Produces: a repository with one long-lived branch. Task 5 depends on `promote.yml` being gone.

**The canonical new branch text.** Every document below gets this idea, at its own length:

> There is one branch, `main`. It is the default branch, the released line — what a clone lands on,
> what the Update button fetches, and what every pull request targets. Work happens on a short-lived
> branch cut from `main`, opened as a pull request into `main`, with the branch deleted on merge.
> There is no integration branch between a merge and the released line.

- [ ] **Step 1: Cut the branch and confirm the current state**

```bash
cd /home/administrator/SvanBot
git fetch origin --prune
git checkout -b chore/trunk-on-main origin/main
git rev-list --count origin/main..origin/dev
```

Expected: `0` — `dev` holds nothing `main` lacks, so nothing is lost by this change.

- [ ] **Step 2: Delete the three files**

```bash
git rm .github/workflows/promote.yml scripts/promote.sh scripts/tests/promote.sh
```

- [ ] **Step 3: Fix `scripts/check.sh`, which invokes the deleted suite**

Two edits. First, delete this line entirely (line 165):

```bash
    run_suite "promote tests (run bash scripts/tests/promote.sh)" bash scripts/tests/promote.sh
```

Second, on line 150, drop `, promote` from the step description, which currently reads:

```bash
    step "tickets lint + tool tests (tickets, codec vs zlib, test runner, release/rollback, keepalive, update, adopt-upstream, file-size, promote, build lock, systemd units)"
```

so that it becomes:

```bash
    step "tickets lint + tool tests (tickets, codec vs zlib, test runner, release/rollback, keepalive, update, adopt-upstream, file-size, build lock, systemd units)"
```

Then confirm the file no longer invokes the deleted suite:

```bash
grep -n "promote" scripts/check.sh
```

Expected: no output.

- [ ] **Step 4: Narrow the `check` workflow's push trigger**

In `.github/workflows/check.yml`, change:

```yaml
  push:
    branches: [main, dev]
```

to:

```yaml
  push:
    branches: [main]
```

- [ ] **Step 5: Rewrite `AGENTS.md` section "Branches"**

Replace the whole section (**lines 20-34**, from the `## Branches` heading through `is played before it is promoted.`, leaving the blank line at 35 and the `## Which issue to take` heading that follows) with:

```markdown
## Branches

There is one branch: `main`. It is the repository's default branch, the released line — what a fresh
`git clone` lands on, what the Update button fetches, and what every pull request targets.

Cut a short-lived branch from `main` for your change, open the pull request into `main` with
`gh pr create --base main`, and let GitHub delete the branch on merge (`deleteBranchOnMerge` is on).
Nothing else is long-lived: a merge is live as soon as a fleet's next Update runs, so the gate on the
pull request is the only thing between a change and live play. Never push to `main` directly.

The reference fleet follows `main` with `SVANBOT_UPDATE_BRANCH` unset, so the default applies and
every change is played.
```

This removes the sentence at line 33 that claimed the reference fleet sets `SVANBOT_UPDATE_BRANCH=dev` — this box's `.env` does not set it, so the claim was false.

- [ ] **Step 6: Rewrite `CONTRIBUTING.md` step 2**

Replace lines 23-26, which currently read:

```markdown
2. **Fork, and make one branch per change off `dev`** (where work lands; every pull request goes
   into `dev`, and `main` is only ever updated from it), named for it: `fix/split-pots-all-folded`,
   `fix/localstorage-guard`. `main` is the default branch, so a pull request opened without a base
   targets the released line: **set the base to `dev`**.
```

with:

```markdown
2. **Fork, and make one short-lived branch per change off `main`**, named for it:
   `fix/split-pots-all-folded`, `fix/localstorage-guard`. `main` is the default branch and the
   released line, and a pull request opened without a base already targets it, so the base needs no
   setting. The branch is deleted when the pull request merges.
```

- [ ] **Step 7: Rewrite `README.md`**

Replace the paragraph spanning **lines 330-337**, from `The button fast-forwards the checkout onto **one branch**` through `installed build (#394).` Line 338 is blank and line 339 is `</details>`; leave both. The replacement keeps the `#394` sentence's meaning:

Then make a **second** edit further down the same file. Lines **376-379** are a separate contribution-instructions paragraph that names the branch model too:

```markdown
2. **Fork, and make a branch off `dev`** named for the change — `fix/split-pots-all-folded`,
   `docs/…`. Every pull request goes into `dev`, where work lands; `main` is the default branch and
   the released line, and moves only when `dev` is promoted to it. **Open the pull request against
   `dev`** — the base GitHub offers by default is `main`.
```

Replace those four lines with:

```markdown
2. **Fork, and make a short-lived branch off `main`** named for the change —
   `fix/split-pots-all-folded`, `docs/…`. `main` is the default branch and the released line, and
   every pull request goes into it, so the base needs no setting.
```

Both edits are in the first replacement below; here is that first one:

```markdown
The button fast-forwards the checkout onto **one branch**: `SVANBOT_UPDATE_BRANCH`, `main` by
default. `main` is the default branch and the released line, so a fresh `git clone` lands on it and
keeps updating from it, and every merge reaches live play at the next Update. An install that must
not follow the released line — a staging box, a fork — sets `SVANBOT_UPDATE_BRANCH=<branch>` in
`.env` to follow that branch instead.
```

Leave the `SVANBOT_UPDATE_BRANCH` row of the configuration table (line 351) unchanged: its default is still `main`, which is what it says.

- [ ] **Step 8: Rewrite `llms.txt`**

Replace lines 29-34 so they read:

```markdown
The clone lands on `main`, the default branch and the released line. Every pull request targets
`main`, and a pull request opened without a base already does. Updates follow one branch,
`SVANBOT_UPDATE_BRANCH` (`main` by default, `SVANBOT_UPDATE_REMOTE` default `origin`).
```

Then replace every `blob/dev/` in the link list (lines 81-89) with `blob/main/`, and replace the two lines beginning `These are the docs as \`dev\` has them` and `request goes. \`main\`, the released line the clone landed on, is promoted from it.` with:

```markdown
These are the docs as `main` has them — the branch the clone landed on, where a change is made
against, and where your pull request goes.
```

- [ ] **Step 9: Rewrite the promotion section of `docs/RELEASE.md`**

`docs/RELEASE.md` is 115 lines. Its section headed `## Promoting \`dev\` to \`main\`` runs from **line 10** to **line 46**: line 10 is `## Promoting \`dev\` to \`main\``, line 42 is the reference-fleet sentence, line 45 is `After it merges, every fleet following \`main\` installs it at its next Update (or \`scripts/update.sh\`).`, and line 46 is `Tags are cut on \`main\`.` Everything in it — the merge-commit-never-squash reasoning, the auto-merge description, the hand-run `scripts/promote.sh` command block, and the sentence `The reference fleet tracks \`dev\` (\`SVANBOT_UPDATE_BRANCH=dev\`) rather than \`main\`, so every change is played before it is promoted.` — describes machinery that no longer exists.

Replace lines 10-46 in full with:

```markdown
## Cutting a version

Work lands on `main`. It is the released line: what everyone who installs SvanBot runs, and where
their Update fetches from. `main` carries only what the gate passed, because the ruleset on it
requires the `check` status on every pull request into it, and a change reaches live play at a
fleet's next Update rather than at a promotion.

To cut a version, tag the commit on `main` that the gate passed.
```

Leave everything above line 10 and everything below line 46 untouched: the `> **Read this when**` header's `**Related:**` link and the "Before this" line stay true, and the `## What ships` section at line 48 still follows the blank line at 47.

Two sentences further down are inside the replaced range and are carried over in meaning: line 45, `After it merges, every fleet following \`main\` installs it at its next Update (or \`scripts/update.sh\`).`, becomes the new first paragraph's "a fleet's next Update" clause, and line 46, `Tags are cut on \`main\`.`, becomes the section's closing tag sentence.

Confirm nothing downstream still names the removed machinery:

```bash
grep -n "promote\|dev" docs/RELEASE.md
```

Expected: no hit that refers to a branch or to `scripts/promote.sh`.

- [ ] **Step 10: `docs/OPERATIONS.md` needs no change — verify that, then skip it**

This step replaces an earlier instruction to rewrite this file, which was wrong. Verified during prework:

```bash
grep -n "promot\|\bdev\b\|promoted" docs/OPERATIONS.md
```

Every hit is unrelated to the branch model: `npm run dev`, `target/dev`, `scripts/web-test-server.sh`, and four uses of *promotion* that mean the learner's **parameter** promotion (`analyst.drift`, the champion's +1 bb/100 gate), never a branch. Lines 23 and 385 both name `SVANBOT_UPDATE_BRANCH` with its default `main`, which stays true.

Expected: no edit. If you find a `dev`-branch or promotion-branch sentence that this list does not cover, the list is wrong — edit that sentence and say so in your report rather than rewording anything else. Do not search for the word "promotion" and act on its hits; in this file it usually means the learner.

- [ ] **Step 11: Neutralise the two `LESSONS.md` entries that name the deleted script**

Entries 44 and 45 (lines 203-235) each open by quoting `scripts/promote.sh` in backticks, and `docs/LESSONS.md` is in `scripts/docs-check.live`, so those backticks fail the check once the file is gone. The entries are a historical record and must not be rewritten — wrap the path references in the check's off-block and add one line above entry 44:

```markdown
The entries below describe `scripts/promote.sh` and its tests, deleted on 2026-09-28 when the
repository moved to trunk-based work on `main`. The lessons are about the mistakes, not the script.
<!-- docs-check: off -->
```

and close it after entry 45 with:

```markdown
<!-- docs-check: on -->
```

The boundary is exact: entry 45's last line of prose is **line 235** (`the answer: containment of *content* is \`git rev-list --count dev --not main\` being zero.`), line 236 is blank, and line 237 begins entry 46. Close the off-block between lines 235 and 236 — before the blank line, not after it — so entry 46 is checked again. Verify by re-reading the boundary and running the check at Step 15.

- [ ] **Step 12: Fix the two shell sites that hardcode the branch line**

`scripts/update.sh:156-157` prints a hint that names the promoted line. Replace both `echo` lines with:

```bash
  echo "update: this checkout is $ahead commit(s) ahead of $remote/$branch; nothing to install (set SVANBOT_UPDATE_BRANCH=$current in .env to follow it here instead)"
```

Keep the `progress current` call on the following line exactly as it is. It is not an `echo`, and `scripts/tests/update.sh:163` asserts the progress state it produces (`current - fetch:done`); folding it into the echo breaks a different test.

**Leave `scripts/tests/update.sh:164` unchanged.** It reads:

```bash
grep -q "SVANBOT_UPDATE_BRANCH=dev" "$t/ahead/artifacts/release.log" || fail "the run does not say how to follow the checkout's own branch"
```

The `dev` there is not the repository's `dev` branch. The fixture creates its own ahead-branch and names it `dev` (`scripts/tests/update.sh:154`, `git -C "$t/ahead" checkout -q -b dev`, pushed as `HEAD:dev` at line 157), and the assertion checks that the run names *the checkout's own branch* — whatever it is called. The replacement echo above keeps `SVANBOT_UPDATE_BRANCH=$current`, so the assertion still passes. Changing the expected name would break the test rather than update it. Verify by running it:

```bash
bash scripts/tests/update.sh && echo "update suite ok"
```

- [ ] **Step 13: Repoint the two workflows that pin themselves to `dev`**

Three workflows check out `dev` by name. One is deleted in Step 2. The other two fail at checkout the moment the branch is gone, and `claude-maintainer.yml` additionally instructs an agent to open pull requests against `dev` and to merge them with a method `main` does not allow.

**`.github/workflows/close-linked-issues.yml`** — change `ref: dev` (line 40) to `ref: main`, and replace the three-line comment above it (lines 37-39) with:

```yaml
          # Name the branch the script is fixed on. A checkout with no `ref:` would take the
          # repository default, which is `main` — the same branch — so this is belt and braces,
          # kept because a fork can move its own default.
```

**`.github/workflows/claude-maintainer.yml`** — five sites:

1. Lines 47-51. A comment above the same `ref: dev` reasons about promotion merges:

```yaml
          # The job works `dev`'s issues and opens its pull request into `dev`. A scheduled run checks
          # out the default branch, which is `main` — the released line — so the branch to work is
          # named here rather than inherited: a branch cut from `main` would carry `main`'s promotion
          # merge commits into the pull request with it.
          ref: dev
```

Replace all five lines with:

```yaml
          # A scheduled run checks out the repository default, which is `main` — the branch the job
          # works and the branch its pull request targets, so this pin is belt and braces rather than
          # a correction. The note that used to be here reasoned about promotion merges, which no
          # longer exist.
          ref: main
```
2. The header comment, lines 3-7, currently reads *"fixes it on a branch cut from `dev`, runs the gate and opens a pull request into `dev` — the branch work lands on, which a scheduled run has to check out by name because the repository's default branch is `main`, the released line. Auto-merge is on, and the ruleset on `dev` and `main` requires the `check` job, so nothing lands unless the full gate is green."* Rewrite those five lines as:

```markdown
# Twice a day Claude takes the oldest open `agent-friendly` issue that no pull request closes yet,
# fixes it on a branch cut from `main`, runs the gate and opens a pull request into `main` — the
# default branch and the released line. Auto-merge is on, and the ruleset on `main` requires the
# `check` job, so nothing lands unless the full gate is green.
```

3. Line 139, inside the agent's own instructions: `push, and open a pull request into \`dev\`` becomes `push, and open a pull request into \`main\``.
4. Lines 141-143. Currently:

```
               line — `gh pr create --base dev`, because the base GitHub offers by default is `main`,
               the released line, which takes no feature pull request.
            5. Turn on auto-merge: `gh pr merge <number> --auto --squash`.
```

becomes:

```
               line — plain `gh pr create`, since the base GitHub offers by default is `main`, the
               branch the pull request belongs against.
            5. Turn on auto-merge: `gh pr merge <number> --auto --merge`.
```

That last edit matters as much as the branch change: `main`'s ruleset allows `merge` commits only, so `--squash` would arm auto-merge and then never fire.

Then verify no workflow still names the branch or the wrong merge method:

```bash
grep -rn "ref: dev\|--base dev\|into \`dev\`\|--auto --squash" .github/workflows/
```

Expected: no output.

- [ ] **Step 14: Fix the `.env.example` comment**

Lines 40-43 describe the branch as the released line that moves when `dev` is promoted. Replace with:

```
# Branch the dashboard's Update button fetches and fast-forwards this checkout to, and the branch its
# "behind by N" check reads (`SVANBOT_UPDATE_REMOTE`, default origin). main = the released line, and
# the default; set it only to follow a different line here.
# SVANBOT_UPDATE_BRANCH=main
```

- [ ] **Step 15: Run the coupling checks before the full gate**

```bash
cd /home/administrator/SvanBot
python3 scripts/docs-check.py && echo "docs-check ok"
grep -rn "promote\.sh\|promote\.yml" --include='*.md' --include='*.sh' --include='*.yml' . | grep -v '^./target' | grep -v '^./docs/superpowers' | grep -v CHANGELOG
```

Expected: `docs-check: 26 documents, 0 problem(s)`, and the grep returns only the off-blocked `LESSONS.md` entries and comments that are not backticked paths.

- [ ] **Step 16: Run the full gate**

```bash
scripts/check.sh full
```

Expected: PASS. If it fails on a path, the document naming it was missed in steps 5-14.

- [ ] **Step 17: Commit, push and merge**

```bash
git add -A
git commit -F - <<'EOF'
chore: one branch, main

`dev` and the promotion machinery between it and `main` are removed. Every
pull request now targets `main`, which is both the default branch and the
released line, so there is no integration branch between a merge and live
play. scripts/promote.sh, scripts/tests/promote.sh and promote.yml are
deleted, check.sh stops running the suite that tested them, and every
document that named the two-branch model is rewritten.

The AGENTS.md sentence claiming the reference fleet sets
SVANBOT_UPDATE_BRANCH=dev was false on this box and is gone.

Generated-by: claude-code/deepseek-flash
Co-Authored-By: Claude Code <noreply@anthropic.com>
EOF
git push -u origin chore/trunk-on-main
gh pr create --base main --title "chore: one branch, main" --body "Removes \`dev\` and the promotion machinery. Design: \`docs/superpowers/specs/2026-09-28-trunk-on-main-and-the-strength-program-design.md\`.

Generated-by: claude-code/deepseek-flash"
gh pr merge --merge
```

---

### Task 5: Delete the `dev` branch and its ruleset

**Files:** none in the repository. This is a GitHub settings change.

**Interfaces:**
- Consumes: Task 4 — `promote.yml` must be gone from `main`, or a workflow that checks out `ref: dev` fails on every run after the branch disappears.
- Produces: nothing later tasks read.

- [ ] **Step 1: Confirm nothing still targets `dev`**

```bash
cd /home/administrator/SvanBot
gh pr list --state open --json number,baseRefName --jq '.[] | select(.baseRefName=="dev") | .number' | wc -l
git fetch origin --prune && git rev-list --count origin/dev --not origin/main
```

Expected: `0` and `0`. If either is non-zero, stop — a pull request or a commit would be lost.

- [ ] **Step 2: Confirm nothing on the remote has changed since the fetch**

```bash
gh api repos/SvanLabs/SvanBot/branches/dev --jq '.commit.sha'
git rev-parse origin/dev
```

Expected: the same SHA.

- [ ] **Step 3: Probe whether the token can do this at all**

```bash
gh api repos/SvanLabs/SvanBot/rulesets --jq '.[] | {id, name}' 
gh auth status
```

The token is a GitHub App installation token and reports `"admin":false` on this repository. If step 4 returns `403` or `Resource not accessible by integration`, perform steps 4 and 5 in the GitHub UI — Settings → Rules → Rulesets → "dev — where work lands" → Delete; and Branches → `dev` → Delete — and continue.

- [ ] **Step 4: Delete the ruleset**

```bash
gh api -X DELETE repos/SvanLabs/SvanBot/rulesets/24084702 && echo "ruleset deleted"
```

- [ ] **Step 5: Delete the branch**

```bash
git push origin --delete dev
```

- [ ] **Step 6: Verify**

```bash
git fetch origin --prune && git branch -r
gh api repos/SvanLabs/SvanBot/rulesets --jq '.[] | .name'
```

Expected: remote branches are `main` plus any live pull-request branch; rulesets list only `main — the released line`.

- [ ] **Step 7: Delete the stale local branches**

Every one of these is content-identical to, or behind, `origin/main`, and the compile fix from Task 1 has already been landed on its own branch:

```bash
git branch -D fix/compact-search-off-write-lock fix/env-test-empty-key ponytail/dead-code test/log-capture dev
git branch -m main 2>/dev/null; git checkout main && git reset --hard origin/main
git branch -D fix/setup-first-run fix/wiring-reach 2>/dev/null
git branch -vv
```

Confirm `git status --short` is clean before proceeding — in particular that no modification to `crates/libs/store/src/packed.rs` survives, since it was landed in Task 1.

---

### Task 6: Record the decisions on the blocked issues

**Files:** none. This writes to the issue tracker.

**Interfaces:**
- Consumes: nothing.
- Produces: four issues that can be picked up by an agent without another decision.

These comments are public. Post them only when the operator has confirmed the wording, and keep the `Generated-by:` line in each body.

- [ ] **Step 1: Record the decision on #347 and close it**

```bash
gh issue comment 347 --body "$(cat <<'EOF'
Decision: neither seam is built now. Piece 1, the log-capture helper, landed in #452 and unblocked
every site that could already be made to fail. Piece 2 — a way for `sv10-bot` to force a store write
to fail — is deferred until a call site actually needs it: the smaller of the two designs is still
machinery for three call sites that have not asked for it.

Closing with the decision recorded rather than leaving it open: nothing is left to choose, so none of
the three readiness labels applies. Reopen this if a call site needs the seam.

Generated-by: claude-code/deepseek-flash
EOF
)"
gh issue close 347 --reason "not planned"
```

- [ ] **Step 2: Post the decision that unblocks #334**

```bash
gh issue comment 334 --body "$(cat <<'EOF'
Decision: measure first, as this issue's own triage requires, then implement both the shared
projection and the connection cap. The measurement is cheap and the issue already specifies it: open
a stream against a busy table with `SVANBOT_TV_PORT` set, with `TABLE_EVERY` in hand, and count the
projections. The cache removes the per-viewer multiplication; the cap bounds a public,
unauthenticated route. Both land regardless of the measurement's size, because the second is a
safety property rather than a cost one.

Generated-by: claude-code/deepseek-flash
EOF
)"
```

Then `gh issue edit 334 --remove-label blocked-on-decision --add-label agent-friendly`.

- [ ] **Step 3: Post the decision that unblocks #17**

```bash
gh issue comment 17 --body "$(cat <<'EOF'
Decision: publish derived aggregates and the schema, never raw opponent hands. This is narrower than
the issue's own framing, which proposed a sealed copy of the live database; the live database holds
other players' hands, and no release of it is authorised. The scrub list must be confirmed against
the intended contents before anything is published, and the release is dated.

Generated-by: claude-code/deepseek-flash
EOF
)"
```

Then `gh issue edit 17 --remove-label blocked-on-decision --add-label agent-friendly`.

- [ ] **Step 4: Give #319 the readiness label it is missing**

`AGENTS.md` says every open issue carries exactly one readiness label and #319 carries none.

```bash
gh issue edit 319 --add-label agent-friendly
```

Then comment the hypothesis the investigation produced, so whoever picks it up starts from the evidence:

```bash
gh issue comment 319 --body "$(cat <<'EOF'
Investigation note. Three mechanical explanations were verified in the pricing path, all of which
produce this panel's exact shape without any river bug:

- `crates/libs/policy/src/policy/mod.rs:199` bans the raise outright on the river once one aggressive
  action has happened and hero equity is under 0.5.
- `crates/libs/policy/src/policy/responses.rs` builds every postflop raise target from
  `params.bet_sizes` alone, which `crates/libs/policy/src/policy/params.rs:123` sets to
  `[0.33, 0.55, 0.8, 1.2]`; a non-jam raise above 1.2x pot is not expressible.
- `crates/libs/policy/src/policy/mod.rs:360` computes the check branch from two outcomes only, with
  a hard-coded 0.66 bet, and has no raise term.

The analyst's deep re-solve prices a finer action set than live play can express, so some of the
reported mistakes may be the menu rather than the search. The cheap test is to log, for each recorded
river mistake, whether the chosen size is representable in the live menu.

Generated-by: claude-code/deepseek-flash
EOF
)"
```

- [ ] **Step 5: Close #18 as not planned**

```bash
gh issue close 18 --reason "not planned" --comment "$(cat <<'EOF'
Closing as not planned. The ticket history of another repository does not make this bot stronger,
faster, or easier to run, and importing it adds permanent noise to an issue tracker that is meant to
be taken by agents. It needs read access to a private tree and a wording review of every ticket as
well, which is a standing cost for archaeology.

Generated-by: claude-code/deepseek-flash
EOF
)"
```

- [ ] **Step 6: Close #15**

Its fix landed as #456, which made `scripts/setup.sh` build through `scripts/release.sh` instead of
writing `target/release` directly.

```bash
gh issue close 15 --reason completed --comment "$(cat <<'EOF'
Closed: `scripts/setup.sh` now builds through `scripts/release.sh`, so setup no longer writes
`target/release` and no longer strands the next release (merged as #456).

Generated-by: claude-code/deepseek-flash
EOF
)"
```

- [ ] **Step 7: Verify the board state**

Every close in this task runs before this step, because the count this asserts is the state the
closes produce.

```bash
issues=$(gh issue list --state open --json number,labels)
echo "$issues" | jq '.[] | "\(.number)\t\(.labels|map(.name)|join(","))"'
echo "$issues" | jq -e '
  ([.[].number] | sort) == [17, 319, 334, 363]
  and all(.[]; (.labels | map(.name) | index("agent-friendly")) != null)
  and all(.[]; (.labels | map(.name) | index("blocked-on-decision")) == null)
' >/dev/null \
  && echo "board ok: four open, every one agent-friendly, none blocked-on-decision" \
  || { echo "board WRONG: the listing above is not four open issues all labelled agent-friendly"; exit 1; }
```

Expected: **four** open issues — #363, #334, #319, #17 — every one labelled `agent-friendly`, and
none labelled `blocked-on-decision`. #315 closed when #457 merged; #18, #347 and #15 closed in this
task.

---

### Task 7: Verify the install

**Files:** none.

**Interfaces:**
- Consumes: Tasks 1-5.
- Produces: the evidence that Phase 0's success criterion is met.

- [ ] **Step 1: Run the repository's own verifier**

Invoke the `verify-install` skill. It answers which commit the fleet is playing, whether it matches the checkout, and which builds can be rolled back to.

- [ ] **Step 2: Confirm the three commits agree, and that `main` is the only line**

```bash
cd /home/administrator/SvanBot
git fetch origin --prune
cat artifacts/release-progress.json | python3 -c 'import json,sys; d=json.load(sys.stdin); print("installed:", d["commit"], d["state"])'
git rev-parse --short origin/main
git branch -r
```

Expected: the installed commit and `origin/main` are the same commit, and the remote branch list contains no `dev`.

- [ ] **Step 3: Confirm nothing reached `main` except through a pull request**

```bash
git log --oneline --first-parent origin/main -12
gh pr list --state merged --limit 12 --json number,title,baseRefName --jq '.[] | "\(.number)\t\(.baseRefName)\t\(.title)"'
```

Expected: every commit on `main`'s first-parent line corresponds to a merged pull request whose base is `main` (the promotion merges that predate this change are the exception and are expected).

---

## Phase 1

### Task 8: Fix the promotion gate's inert interim boundary

`crates/apps/bot/src/promotion.rs:87` computes an unshifted z, so promotion requires
`mean_bb / se_bb >= 3` **and** `lower_95() >= MIN_EDGE_BB`. The second implies the first whenever
`se_bb <= 0.009615` bb/hand, which is the case for every candidate at this variance level — so the
`z >= 3` clause never fires and the interim rule is identical to the final rule.

**Files:**
- Modify: `crates/apps/bot/src/promotion.rs:87`
- Test: `crates/apps/bot/src/promotion.rs` (`mod tests`, existing)

**Interfaces:**
- Consumes: `PairedResult { hands, mean_bb, se_bb, differing }` from `sv10_core::sim`.
- Produces: no signature changes. `verdict(&PairedResult, usize) -> Verdict` keeps its shape; only the boundary it applies changes.

- [ ] **Step 1: Cut the branch, then write the failing test**

Cut the branch *before* editing, so no change is ever made on `main` in this checkout:

```bash
cd /home/administrator/SvanBot
git fetch origin --prune
git checkout -b fix/promotion-gate-shifted-z origin/main
git rev-parse --abbrev-ref HEAD
```

Expected: `fix/promotion-gate-shifted-z`.

Then add to `mod tests` in `crates/apps/bot/src/promotion.rs`, beside `interim_looks_stop_for_futility_or_overwhelming_evidence_only`:

```rust
    #[test]
    fn an_interim_promotion_needs_z_above_the_bar_not_z_above_zero() {
        // +3.5 bb/100 at SE 1.0: lower bound +1.54 clears the +1 bar, and the UNshifted z is 3.5,
        // which promoted. Shifting by the bar gives (3.5 - 1.0) / 1.0 = 2.5, below EARLY_Z.
        // This is the case the old boundary let through and the reason it went unnoticed: every
        // other interim test in this file has SE large enough that the bar term already bound.
        assert_eq!(verdict(&res(3.5, 1.0), 2), Verdict::Continue);
    }
```

- [ ] **Step 2: Run it and watch it fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /home/administrator/SvanBot
python3 scripts/test.py promotion
```

Expected: FAIL — `assertion failed: left == Verdict::Promote, right == Verdict::Continue`. If it passes, `verdict` is not the function being exercised and the test is in the wrong place.

- [ ] **Step 3: Verify by hand that the arithmetic is what the test says**

```bash
python3 - <<'EOF'
mean, se, bar = 0.035, 0.01, 0.01
print("unshifted z      ", mean / se)
print("lower 95         ", mean - 1.96 * se, ">= bar:", mean - 1.96 * se >= bar)
print("shifted z        ", (mean - bar) / se)
print("se threshold     ", bar / (3.0 - 1.96))
EOF
```

Expected: `3.5`, `0.0154 … True`, `2.5`, `0.009615…`. The first line is the number that promoted; the third is the one that must not.

- [ ] **Step 4: Apply the fix**

Replace line 87:

```rust
    let z = if r.se_bb > 0.0 { r.mean_bb / r.se_bb } else { 0.0 };
```

with:

```rust
    // Shifted by the bar: the interim boundary must measure evidence for a *worthwhile* edge, not
    // for any positive one. Unshifted, `z >= EARLY_Z` is implied by `lower_95() >= MIN_EDGE_BB`
    // whenever `se_bb <= MIN_EDGE_BB / (EARLY_Z - 1.96)` (0.009615 bb/hand), which is every
    // candidate at this variance, so the clause never fired and the interim rule equalled the
    // final one.
    let z = if r.se_bb > 0.0 { (r.mean_bb - MIN_EDGE_BB) / r.se_bb } else { 0.0 };
```

- [ ] **Step 5: Run the whole promotion test module and check nothing else moved**

```bash
python3 scripts/test.py promotion
```

Expected: PASS, including the new test. Every pre-existing assertion must still hold — if one fails, the boundary moved for a case it should not have, and the fix is wrong.

- [ ] **Step 6: Confirm the behaviour change is intended and nothing else shifted**

```bash
python3 scripts/test.py            # the whole suite; the golden snapshot must not move
```

Expected: PASS with the golden file unchanged. `verdict` is not on the decision path, so a golden change means something else was edited.

- [ ] **Step 7: Commit**

```bash
git add crates/apps/bot/src/promotion.rs
git commit -F - <<'EOF'
promotion: the interim boundary measures the edge above the bar

`verdict` computed `z` as `mean_bb / se_bb`. Promotion requires that AND
`lower_95() >= MIN_EDGE_BB`, and the second implies the first whenever
`se_bb <= MIN_EDGE_BB / (EARLY_Z - 1.96)` — that is 0.009615 bb/hand, which
covers every candidate the learner sees. The `z >= EARLY_Z` clause therefore
never fired and the interim rule was identical to the final one, so the
one-sided error is the error of a constant 1.96 boundary applied at up to
twelve looks rather than the nominal figure the module documents.

Shifting by the bar makes the clause test what it was written to test. The
new case covers a candidate whose lower bound clears the bar while its
shifted z does not: it promoted before and continues now.

Generated-by: claude-code/deepseek-flash
Co-Authored-By: Claude Code <noreply@anthropic.com>
EOF
```

---

### Task 9: Correct the claimed error rate in the learner spec

`docs/SPEC-learner.md:230` states a rate the code did not deliver. `docs/SPEC-learner.md` is a live document, so it may only name paths that exist, and it must not gain a number this project has not computed.

**Files:**
- Modify: `docs/SPEC-learner.md:230-231`

**Interfaces:**
- Consumes: Task 8's change.
- Produces: nothing later tasks read.

- [ ] **Step 1: Read the current text**

```bash
cd /home/administrator/SvanBot
sed -n '228,232p' docs/SPEC-learner.md
```

Expected: the two lines beginning `Futility stops never raise the false-promotion rate.` and `one-sided error near 2.5%.`

- [ ] **Step 2: Replace them**

Replace those two lines with:

```markdown
   Futility stops never raise the false-promotion rate. The interim boundary is the 95% lower bound on
   the edge above the bar, so it fires only at the same evidence the final look needs; the overall
   one-sided error is that of a sequential design with the [`CONFIRM_CHUNKS`] looks, and it is not
   the single-look 2.5%.
```

Do **not** write a replacement percentage. The figure this investigation produced is a borrowed
computation; until this project computes and reproduces its own, the document states the design and
names the constant, which is checkable, rather than a rate that is not.

- [ ] **Step 3: Run the docs checks**

```bash
python3 scripts/docs-check.py && echo "docs-check ok"
grep -n "2.5%" docs/SPEC-learner.md || echo "no unbacked rate remains"
```

Expected: `0 problem(s)`, and no `2.5%` left in the file.

- [ ] **Step 4: Commit as part of Task 8's pull request**

```bash
git add docs/SPEC-learner.md
git commit -F - <<'EOF'
docs: the learner spec stops claiming an error rate the gate does not deliver

The z >= 3 interim clause was inert, so the one-sided error was never near
2.5%. The text now states the design and names the constant rather than
quoting a rate; this project has not computed its own figure and a borrowed
one is what put the false claim here in the first place.

Generated-by: claude-code/deepseek-flash
Co-Authored-By: Claude Code <noreply@anthropic.com>
EOF
git push -u origin fix/promotion-gate-shifted-z
gh pr create --base main --title "promotion: the interim boundary measures the edge above the bar" --body "Fixes the inert \`z >= EARLY_Z\` clause in \`crates/apps/bot/src/promotion.rs\` and corrects the rate \`docs/SPEC-learner.md\` claims for it.

Generated-by: claude-code/deepseek-flash"
gh pr merge --merge
```

---

### Task 10: Log the discarded confirmation evidence

When a confirmation rejects a candidate, `crates/apps/bot/src/learner/search/conclude.rs` records only the transition *key* in `confirm_rejected`. The measured `PairedResult` — up to 288,000 hands of evidence — is dropped. Whether a candidate is ever re-nominated is unknown; this task adds the log line that makes it a question with data behind it, and deliberately does **not** add the cache until the log says it recurs.

**Files:**
- Modify: `crates/apps/bot/src/learner/search/conclude.rs:55`

**Interfaces:**
- Consumes: `confirm: &PairedResult` (already a parameter), `transition_key(knob, old, new) -> String`, `search_ledger::load/save`.
- Produces: nothing. No signature changes; a log line only.

- [ ] **Step 1: Read the site**

```bash
cd /home/administrator/SvanBot
sed -n '50,58p' crates/apps/bot/src/learner/search/conclude.rs
```

Expected: the `Verdict::Reject(reason)` arm, then `funnel::note`, then `let mut ledger = search_ledger::load(...)` and `ledger.confirm_rejected.insert(transition_key(knob, old, new));`.

- [ ] **Step 2: Add the log line**

Insert immediately after the `confirm_rejected.insert(...)` line:

```rust
    // The result is discarded, not stored: `confirm_rejected` holds only the key, so a key that
    // comes back costs the full confirmation again. Record the measurement beside the key so the
    // question "does a rejected candidate ever return?" has data behind it before anything is
    // built to answer it.
    tracing::info!(
        "cycle {cycle}: confirmation rejected {knob} {old:.3}->{new:.3} over {} hands ({:+.2} bb/100, 95% {:+.2}..{:+.2})",
        confirm.hands,
        confirm.mean_bb * 100.0,
        confirm.lower_95() * 100.0,
        confirm.upper_95() * 100.0
    );
```

- [ ] **Step 3: Build and run the learner's tests**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
CARGO_TARGET_DIR=target/dev cargo check -p sv10-bot --message-format short 2>&1 | tail -5
python3 scripts/test.py learner
```

Expected: compiles, tests PASS. The format string uses only bindings already in scope.

- [ ] **Step 4: Confirm `clippy` is clean for the crate**

```bash
CARGO_TARGET_DIR=target/dev cargo clippy -p sv10-bot --all-targets -- -D warnings 2>&1 | tail -5
```

Expected: no warnings. A `tracing::info!` with a multi-line argument list is fine, but `clippy` rejects an unused format argument and this must not ship with one.

- [ ] **Step 5: Commit and open the pull request**

```bash
git checkout -b log/confirmation-evidence origin/main
git add crates/apps/bot/src/learner/search/conclude.rs
git commit -F - <<'EOF'
learner: log what a rejected confirmation measured

`conclude` stores only the transition key in `confirm_rejected`, so the
confirmation's own result is discarded and a candidate that returns pays the
full hand count again. Whether that happens is unknown. This logs the
measurement beside the key so the question can be answered before anything is
built to answer it.

Generated-by: claude-code/deepseek-flash
Co-Authored-By: Claude Code <noreply@anthropic.com>
EOF
git push -u origin log/confirmation-evidence
gh pr create --base main --title "learner: log what a rejected confirmation measured" --body "Records the discarded confirmation result. The cache is deliberately not built until the log shows a rejected candidate returning.

Generated-by: claude-code/deepseek-flash"
```

---

## Self-Review

Run after the plan is written, against the spec.

**1. Spec coverage.**

| Spec item | Task |
|---|---|
| Fix the compile break | 1 |
| Land #456, #457 | 2 |
| Land the spec on `main` | 3 |
| Delete `dev`'s machinery, rewrite docs, `check.yml` trigger, `update.sh` | 4 |
| Delete `dev` branch and ruleset (admin) | 5 |
| Decision comments, relabel #334/#17, label #319, close #18, #347 and #15 | 6 |
| `verify-install` | 7 |
| `promotion.rs:87` shifted z + test pinning it | 8 |
| Correct `docs/SPEC-learner.md:230` | 9 |
| Log the discarded confirmation evidence | 10 |
| CI `cache-workspace-crates` and the duplicate `target/dev` path | **not in this plan** — Phase 2 of the spec |
| Phase 3-5 | not in this plan, by decision 6 |

**2. Placeholder scan.** No `TODO`, `TBD`, or "implement later". Task 4 Step 12 is the one site where the exact replacement depends on a fixture's branch name, and it gives the command that reads it rather than guessing.

**3. Type consistency.** `verdict(&PairedResult, usize) -> Verdict` is unchanged. `PairedResult` fields used in Task 10 (`hands`, `mean_bb`) and in Task 8's test (`hands`, `mean_bb`, `se_bb`, `differing`) all exist on the struct as read at `crates/libs/policy/src/sim.rs:174`. `transition_key`, `search_ledger::load`/`save`, `Confirm` are all already in `conclude.rs`'s imports.

**4. Review Focus.** Items 1 and 2 are Tasks 4 steps 3 and 11. Item 3 is Tasks 2/3/4 pushing only to feature branches plus Task 7 step 3. Item 4 is Task 8 step 1. Item 5 is Tasks 2 step 1, 4 step 1, 5 step 1, 7 step 2.
