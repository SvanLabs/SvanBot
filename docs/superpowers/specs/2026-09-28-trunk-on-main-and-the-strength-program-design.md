# Design — trunk on `main`, and a strength program that fits the evidence

> **Status:** design, approved in conversation on 2026-09-28, awaiting review of this document.
> **Scope:** the branch model, the release/speed path, the strength levers, and the open issue
> backlog. **Not** a specification of current behaviour — the `docs/SPEC-*.md` files hold that.

This document records what was decided, what was measured, and what was deliberately refused. It
lives under `docs/superpowers/`, which `scripts/docs-check.py` exempts as a historical record — so
its paths are not checked against the tree, because a design names things that do not exist yet.
That exemption is a courtesy, not a licence: where this document names a file a later phase deletes,
the deleting change updates it anyway, because a stale name is still wrong.

## 1. Why this exists

The operator asked for four things on 2026-09-28: make the bot as much stronger as possible, finish
every open issue, merge everything to `main` and use no other branch again, and put more compute
into live play now that the box has more cores. That request arrived with no measurement attached,
and the investigation that followed changed what should be built three times. This document is the
result of that investigation, not of the original request.

The single most useful thing it produced is negative: **the obvious strength lever — more compute
per decision — is not a lever.** Four independent lines of evidence say so, and they are in section
3. The program below is what is left once that is believed.

## 2. Decisions, locked

| # | Decision | Chosen |
|---|---|---|
| 1 | What "improve the bot" targets | Speed first, then strength. Not a bb/100 promise. |
| 2 | Branch model | Trunk on `main`. Delete `dev` and the promotion machinery. |
| 3 | Live compute | Initially "scale the sample budget with cores, p99 < 1 s"; **superseded** by decision 7. |
| 4 | Blocked issues | Resolve all four: #347 store seam, #334 TV cache + cap, #17 aggregates-only release, close #18. |
| 5 | Multiprocess | Kill write-lock stalls; keep one store. Do not shard, do not generalise split mode. |
| 6 | Deliverable shape | One program spec (this document) plus a full plan for the next phase only. |
| 7 | Program scope after evidence | The ultra pass in section 6: cut every item that does not survive the ladder. |

Decision 3 is recorded rather than deleted because it is worth knowing it was wrong, and why.

## 3. What the code actually is

Every claim here was verified against the source, not taken from a report. Line numbers are as of
`origin/main` at `61b2481`.

### 3.1 There is no game theory in this bot

```
grep -rln "cfr|regret|exploitab|best_response|nash|equilibrium" crates/   ->  no files
```

`sv10-policy` is a one-ply EV pricer over a candidate list. The analyst's "deep re-solve" is a
deeper *EV search*, not a solve. Nothing computes a best response or bounds exploitability. Every
strength mechanism is a fitted heuristic plus opponent statistics. This is the fact that makes the
rest of section 3 legible, and it is why section 6 refuses a solver for now.

### 3.2 The river errors have mechanical explanations

Four ceilings, all verified:

- **The raise is banned outright.** `crates/libs/policy/src/policy/mod.rs:199`:

  ```rust
  let raise_allowed = !(sit.street != Street::Preflop && street_raises >= 2 && eq < 0.55)
      && !(sit.street == Street::River && street_raises >= 1 && eq < 0.5);
  ```

  On the river, once anyone has raised and hero's equity is under 0.5, `raise_targets` is never
  called. Those hands are folded or called by construction.

- **The menu is four sizes.** `crates/libs/policy/src/policy/responses.rs` builds every postflop
  raise target from one loop over `params.bet_sizes`; `crates/libs/policy/src/policy/params.rs:123`
  sets `[0.33, 0.55, 0.8, 1.2]` and the champion runs scale `0.85`. Non-jam raises above 1.2x pot
  are not expressible.

- **The top of the menu collapses to all-in.** The same function clamps with
  `.map(|x| if x as f64 >= max_to as f64 * 0.7 { max_to } else { x })`, so any target within 70% of
  the maximum legal raise becomes a jam.

- **The check branch cannot raise.** `crates/libs/policy/src/policy/mod.rs:360` computes exactly two
  outcomes with a hard-coded `let size = 0.66`; there is no raise term, so checking can never be
  valued above calling. The champion also ships `check_lookahead: 0.0`, so it does not run live.

### 3.3 More samples is not the lever

Three independent confirmations:

1. `docs/LESSONS.md:1` records *"16x decision samples gained nothing"* among fixes that looked
   obviously right and measured negative.
2. The project's own audit dose-response: 4x samples moved river disagreement 5.88% -> 3.42%
   against 2.0x predicted by 1/sqrt(n), with the residual on the project's own 0.02 bb/decision
   noise floor.
3. The measured **systematic** biases — a 0.068 river all-in call shift, +0.291 on 4x+ overbets,
   0.154 +/- 0.080 on 500+ bb pots — are **100 to 1000x** the standard error of a 1.6M-sample
   estimate. The estimator is wrong by far more than the sampler is noisy.

And the live path's real ceiling is not even sampling: `SharedDeals::equity()` in
`crates/libs/equity/src/equity/deals.rs` is called six-plus times per decision, sequentially, and
that re-scan is ~75% of the decision wall. The thread sweep at 1.6M samples peaks at **eight**
threads (58.7 ms) and is *slower* at twelve (63.1 ms). More cores make a live decision worse.

### 3.4 The exactness path is heads-up only

`crates/libs/equity/src/equity/deals.rs:48`:

```rust
fn exact(hero: [Card; 2], board: &[Card], opponents: &[&Range], samples: usize, chunks: usize) -> Option<SharedDeals> {
    if opponents.len() != 1 || board.len() < 3 || board.len() > 5 {
        return None;
    }
```

Heads-up river, turn and flop are already exact, which is why the measured river p50 is 2.9 ms.
The gate keys on `samples`, and `samples` scales *down* with CPU speed, so a slower machine
silently falls from exact to sampled on the flop.

### 3.5 The promotion gate does not deliver its documented error rate

`crates/apps/bot/src/promotion.rs:87` computes an **unshifted** z:

```rust
let z = if r.se_bb > 0.0 { r.mean_bb / r.se_bb } else { 0.0 };
```

Promotion requires `z >= EARLY_Z && r.lower_95() >= MIN_EDGE_BB`. The second condition implies the
first whenever `se_bb <= 0.009615` bb/hand; the candidate in the record (`mean_bb -0.0024`,
`lower_95 -0.0142`) has `se = 0.00602`. **The `z >= 3` clause is inert at this variance level**, so
the interim rule is identical to the final rule.

`docs/SPEC-learner.md:230` therefore states something false:

> *Futility stops never raise the false-promotion rate. The z >= 3 interim boundary keeps the overall
> one-sided error near 2.5%.*

A group-sequential calculation puts the true one-sided error near **7.7%**, about three times
nominal. That figure is a computation, validated against Monte Carlo, and is **not** independently
reproduced here; the inertness of the clause is what was verified line by line.

This matters out of proportion to its size because **every later phase is graded by this
instrument.**

### 3.6 The store's write transactions are all deferred

`grep "TransactionBehavior" crates/libs/store/src/` returns nothing. All **11** write transactions
use `unchecked_transaction()`, which is `DEFERRED`. Under WAL the write lock is therefore acquired
lazily, at the first write statement, so a transaction can fail with `SQLITE_BUSY` /
`SQLITE_BUSY_SNAPSHOT` *after* doing work. `sqlite.org/rescode.html` gives the guarantee that
`BEGIN IMMEDIATE` buys: if it succeeds, nothing before `COMMIT` returns `SQLITE_BUSY`.

The store is otherwise well built: WAL, `synchronous=FULL`, a wait-recording busy handler
(`crates/libs/store/src/store/slow.rs`, ticket 0322) installed before the schema, and a four
connection read-only pool.

### 3.7 The release is already inside its budget

`scripts/release.sh:46` sets `BUDGET_SECS=120`. The last measured wall is **113 s**: lint 104 s,
release-build 86 s, test-build 44 s, run concurrently. `scripts/release.sh:136` already keeps the
over-budget log that `docs/LESSONS.md:46` asks for, and `scripts/build-lock.sh` already names a
held build lock. The 519 s that the speed issue leads with was a cargo build lock, not a build.

### 3.8 The working tree does not compile

`crates/libs/store/src/packed.rs:308` iterates `for (rowid, text) in &rows` where `rows` is already
`&[(i64, String)]`. `cargo check -p sv10-store` fails with `E0277: &&[(i64, String)] is not an
iterator`. The fix is one character. Until it is fixed the checkout cannot build and
`scripts/update.sh` cannot install from it. This was fixed as #458 before this document was
committed; it is recorded here because it is what Phase 0 started from.

### 3.9 The portfolio is five copies of one policy

All five bots report `version: "sv10-ev-40"` from the single global `params.v1` key. Five identical
policies buy no diversification and give the learner no parallel live evidence.

## 4. The program

Six phases. Each is its own pull request or small set of them, landing on `main` through a
short-lived branch that is deleted on merge.

### Phase 0 — ground truth and trunk

**Scope.** Fix the one-character compile break so the tree builds. Retarget and land #456 and #457
against `main` (they close #15 and #315). Delete the four local branches whose content is already in
`main`. Delete the `dev` branch, its ruleset, `scripts/promote.sh`, `scripts/tests/promote.sh` and
`.github/workflows/promote.yml` — **workflow first, branch second**, because the workflow checks out
`ref: dev` and would fail on every run after the branch is gone. Rewrite the branch sections of
`AGENTS.md` (including the false claim at line 33 that the reference fleet tracks `dev`),
`CONTRIBUTING.md`, `docs/README.md`, `docs/RELEASE.md`, `docs/OPERATIONS.md`, `README.md`,
`llms.txt`, `.env.example`, the push trigger in `.github/workflows/check.yml` and the two places in
`scripts/update.sh` that hardcode the branch-line message. Post the
four decision comments on the blocked issues, relabel #334 and #17 `agent-friendly`, give #319
the readiness label it is missing, close #18, close #347 with its decision recorded, and close #15.
Change the `main` ruleset to allow squash merges.

**Why first.** Every later pull request is created under this policy; it cannot be done afterwards.
Most of it is deletion.

**Done when.** The fleet's installed commit, the checkout, and `origin/main` are the same commit, as
reported by the `verify-install` skill; `dev` and its ruleset are gone; the gate is green.

**Blocker.** Deleting a ruleset and a branch needs repository admin, and the current token reports
`"admin":false`. If it cannot, that single step is a human action in the GitHub UI and everything
else proceeds.

### Phase 1 — measurement integrity

**Scope.** One line in `crates/apps/bot/src/promotion.rs:87`:

```rust
let z = (r.mean_bb - MIN_EDGE_BB) / r.se_bb;
```

Correct the false claim at `docs/SPEC-learner.md:230`. The number that replaces it must be one this
project computes and can reproduce — the 7.7% in 3.5 is a borrowed computation and must not be
written into a live document on that basis. Until it is recomputed, the honest text states the
design and the test, not a rate. Add one log line when a confirmation is rejected and its evidence
discarded, so that whether it recurs is a question with data behind it.

**Why here.** Every phase after this one is judged by this gate. A gate that overstates its own
error rate by three times is the wrong instrument to grade a strength change with, and this is the
highest value-per-character change in the program.

**Done when.** The gate's documented one-sided error matches its measured one, and a test pins the
boundary so the clause cannot go inert again unnoticed.

### Phase 2 — the boring speed wins

**Scope.** Two lines in `.github/workflows/check.yml`: set `cache-workspace-crates: "true"` on the
rust-cache step (it deliberately discards workspace-member artifacts, so every CI run recompiles all
~20 workspace crates), and delete the explicit `target/dev` cache path, which the action already
covers by caching `target`.

**Why here, and why only this.** The release is under budget (3.7) so the local path needs nothing.
The CI gate job is ~3 minutes per pull request and this targets the part of it that is workspace
recompilation. The saving is an **extrapolation, not a measurement** — the action's maintainer argues
against it on grounds that do not bind here (rare `Cargo.toml` edits, `CARGO_INCREMENTAL=0` already
set) — so it ships with a trial pull request and an explicit rollback.

**Done when.** One trial pull request shows the gate job's time before and after, and the cache is
not near the repository limit (`gh cache list`).

### Phase 3 — unshackle the river ceilings

**Scope.** Make the three ceilings in 3.2 explorable by the learner rather than fixed:

- the bet-size menu behind the existing `bet_size_set` knob, which already *"proposes a whole size
  list, not a number"*;
- the `raise_allowed` equity gate as a knob;
- the raise-less `check_lookahead` either given a raise branch or left off, decided by measurement.

Each ships **default off**, which is the project's own rule for a behaviour change that does not
clearly win (`docs/LESSONS.md:1`).

**Why here, and why not a solver.** The ceilings are the verified mechanical explanation for the
reported river errors, and they are what the literature says an under-raising player loses to. The
lazy move is to let the learner price the ceiling rather than to guess a better ceiling, because
this project has already paid for guessing: the c-bet response knob measured **-4.3 bb/100** and
preflop 3-/4-bet pricing **-34 bb/100** (`docs/LESSONS.md:1`). A tabular river solver is several
hundred lines of CFR and is deliberately **not** in this program; it becomes the fallback only if
the knobs show the *menu* is not the constraint.

**Done when.** Each knob has a paired-simulation result in its pull request, promoted or rejected on
the gate from Phase 1. A rejection is a complete and useful outcome.

### Phase 4 — the half-finished write-lock change

**Scope.** Give `compact_candidates` in `crates/libs/store/src/packed.rs` its reader caller, which is
the change that function was split out for. Its own doc comment records the failure it exists to
prevent: a scan of an already-packed column taking 8 s on a restored 760 MB store, *"under the write
lock stalled every hand insert"*.

**Why here.** It is the actual fix. `BEGIN IMMEDIATE` across the eleven deferred transactions is
**not** in this program: no measurement shows a mid-transaction failure, and speculative hardening
is what the ladder exists to refuse. It is recorded as an open question in section 8 instead.

**Done when.** The compaction scan runs off the write connection, and a test fails if a long scan is
ever issued on it again.

### Phase 5 — the remaining issues

| Issue | What happens |
|---|---|
| #334 | Measure first, exactly as its own triage comment requires: open a stream against a busy table with `SVANBOT_TV_PORT` set and count the projections. Then choose the cache, the cap, or both on the number. |
| #17 | Publish a dated release of derived aggregates and schema. **Never raw opponent hands** — narrower than the issue's own framing, which proposed a sealed copy of the live database. Confirm the scrub list before anything is published. |
| #347 | The `test-hooks` feature is **not** built. Piece 1 landed the log capture; the seam is deferred until a call site actually needs it. The issue is closed with that decision recorded and reopens if one does. |
| #18 | Closed in Phase 0. |

## 5. What was cut, and why

Recorded because each cut is a decision someone will otherwise re-propose.

| Cut | Reason |
|---|---|
| The whole "scale live samples with cores" design (decision 3) | Three independent sources say samples do not buy chips (3.3), and the live path is bound by a serial re-scan, not by simulation. |
| De-serialising `SharedDeals::equity()` | A 2.5-4x latency win on a path whose speed has no chip value (`docs/LESSONS.md:6`). Deferred into Phase 3 only if a knob shows depth is needed. |
| Making the two-opponent river exact | Sampling noise is 100-1000x below the measured systematic error. It removes a constraint that is not binding. |
| Gating exactness on term count rather than `samples` | A real bug, but it only bites on a machine slower than this one. Belongs with the install-anywhere work, not here. |
| `BEGIN IMMEDIATE` and a `SQLITE_BUSY_SNAPSHOT` retry loop | Speculative; no measurement shows the failure. Open question 8.1. |
| `sigma_D` on `PairedResult`; caching rejected confirmation evidence | YAGNI. A log line first; the cache when the log shows it recurs. |
| The `test-hooks` feature (#347) | Piece 1 already unblocked every testable site. |
| Restructuring `scripts/check.sh` to overlap clippy | ~30 s on a path already 7 s inside its budget. |
| Moving `SVANBOT_COMMIT` to a runtime read | The existing workaround holds; the research's own words were *"zero saving today"*. |
| A tabular river subgame solver | Several hundred lines, and the ledger warns that plausible strength changes here measure negative. Phase 3's knobs test the same hypothesis far more cheaply. Fallback only. |
| Linker change, sccache, `panic = "abort"`, `lto = "off"` | The box already links with rust-lld, measured fastest here. sccache cannot cache check-mode or `bin` crates, which is everything this repo ships. `lto = "off"` measured 4.1 s saved for 20-30% larger binaries with the runtime side unmeasured on a Monte Carlo binary. |

## 6. Research findings worth keeping

Six research passes were run. Their findings are cited here so the refusals above are checkable
rather than asserted.

**Dead ends, so nobody proposes them later.** Deep CFR, ReBeL and NFSP
([1811.00164](https://arxiv.org/abs/1811.00164), [2007.13544](https://arxiv.org/abs/2007.13544)) —
GPU-shaped and evaluated on limit or heads-up games. Unabstracted tabular CFR — no-limit hold'em is
on the order of 10^160 states, and stored unabstracted strategies cost 35 GB
([1705.02955](https://arxiv.org/abs/1705.02955)). Borrowing an ACPC card abstraction — the
exploitability lives in card abstraction, not betting abstraction
([1612.07547](https://ar5iv.labs.arxiv.org/html/1612.07547)). Action translation of off-tree bets —
2403 mbb/h against a bot versus 90 mbb/h within its own abstraction, same source. Chasing Slumbot or
the ACPC as a strength metric — heads-up and abstraction-based, and it cannot speak about 6-max.

**Worth keeping for later.** DCFR outperforms CFR+ on Libratus's own turn and river subgames by a
factor of two to three ([1809.04040](https://ar5iv.labs.arxiv.org/html/1809.04040)) — the right
choice if a solver is ever built. LBR is a cheap lower bound on exploitability
([1612.07547](https://ar5iv.labs.arxiv.org/html/1612.07547)), and the paper's own warning is why it
belongs beside the gate and never inside it: two bots can be statistically indistinguishable
head-to-head yet differ by ~1300 mb/g in exploitability. Pluribus's supplementary materials state
**"No GPUs were used at any point"** — a 28-core, 128 GB node — so CPU-only is not a compromise at
this scale.

**On the measurement.** The pairing, the shared champion, the luck removal, the fresh-seed stride
and the `differing == 0` test are all correct and none of them should change. The bar being +1 bb/100
rather than zero is doing the real work: at a true edge of zero the promotion probability is 0.000
even at the inflated alpha. It is **impossible to prove +1 bb/100** — the gate requires the 95%
lower bound to clear the bar, so a true edge of exactly +1 has zero power at any sample size. What
the design can resolve is +2 bb/100 and above, at roughly 194,000 fixed-sample hands or about
151,000 expected under the sequential design. +1.25 bb/100 at 80% power needs 3.1M hands, about
3.6x the current 864,000 ceiling.

## 7. Risks

- **Trunk on `main` removes the staging interval.** Today a change sits on `dev` before promotion;
  after this it reaches live play as fast as the Update runs. The fleet already tracks `main`, so
  the separation is not currently in force — but the change makes that permanent rather than
  accidental.
- **The `main` ruleset change and the branch deletion need admin** the token does not have.
- **Phase 3's knobs may all measure negative**, as the c-bet and 3-/4-bet precedents did. That is a
  real possible outcome and it is recorded here as such rather than as a failure.
- **Phase 2's CI saving is unmeasured**, and the action's maintainer argues against it. Trial pull
  request with a rollback.
- **The one-sided error figure in 3.5 is a computation, not a measurement.** The inertness of the
  `z >= 3` clause is verified; the 7.7% is not.

## 8. Open questions

1. **Is `BEGIN IMMEDIATE` worth it in this store?** The documentation gives a strong guarantee and
   the store already has a wait-recording busy handler, but nothing here has measured a
   mid-transaction `SQLITE_BUSY` failure. Answer with the handler's own recorded wait times before
   writing any retry loop.
2. **`GAP_MIN_DECISIONS = 500`** in `crates/apps/bot/src/findings/decision_loss.rs` is the exact
   constant that `docs/LESSONS.md:41` records as *deleting* rare classes rather than filtering them,
   and it is still 500. Lowering it widens the findings and the noise together. This needs a
   decision, not a default.
3. **The five-identical-bots portfolio** (3.9). Diversifying it is a genuine strength lever that
   does not require the bot to get stronger, but it is a new feature rather than a fix, so it is
   not in this program.
4. **`docs/SPEC-learner.md` is graded by the gate at `docs/LESSONS.md`-level trust and was found to
   state a false number.** Whether other live documents carry claims that nothing checks is a
   question worth asking once, separately.

## 9. Success criteria

This program is complete when:

- the fleet, the checkout and `origin/main` are one commit and no other branch exists;
- every open issue from 2026-09-28 is closed or carries a recorded decision;
- the promotion gate's documented error rate matches its measured one;
- each ceiling in 3.2 has either a paired-simulation result promoting it or a recorded rejection;
- and the compaction scan no longer runs on the write connection.

It is **not** a success criterion that the bot measures 100% stronger, because that is not a claim
this project can currently test. Section 6 gives the hand counts that show why.
