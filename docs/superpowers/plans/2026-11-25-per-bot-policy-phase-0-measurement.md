# Per-bot policy, Phase 0: sharper measurement. Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans, inline. The operator's rule is no subagents (`CLAUDE.md`). Each task runs the repo's `tdd` loop: one failing test, the least code to pass it, repeat. Task 3 is a measurement, not a build: it ends in a recorded go or no-go. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make the Arena's paired sims resolve a 20 bb/100 difference in a fraction of today's deals, by taking the luck of decisions out of a deal's result the way the sim already takes out the luck of the cards, and make the margin check stop when it is decided.

**Architecture:** The sim's `play_observed` already subtracts the chance-node terms of AIVAT for every card that comes. This phase adds the decision-node terms: our own mixed decisions first (a hook on `Challenger` and a handle for the draws), then the scripted opponents', which is first a prototype with a go or no-go. A sequential `svanbot play check` and a calibration record of sim estimate against live edge finish the phase.

**Tech Stack:** Rust (`svanbot-learn`, `svanbot-runtime`, `svanbot`). No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-25-five-brains-design.md`, design v2, "Phase 0". Research note 04, sections 3 and 5.9, is the background.

**Not in this plan:** the search's sims keep their fixed count (making `svanbot learn` read sequentially is its own ticket), and the round-robin reads sequentially when it is built in Phase 3. The design lists both under "sequential reading"; this plan does the margin check, which is the one Phase 1 needs.

## Global Constraints

- The working rules of the Phase 1 plan's "Global Constraints" apply unchanged: a worktree under `.claude/worktrees/v11-<nn>-<slug>`, one ticket claimed at a time as `svanlabs[bot]`, `T` is `CARGO_TARGET_DIR=target/dev nice -n 15 ionice -c3 cargo nextest run --build-jobs 4 --test-threads 4`, commits end with `Generated-by: <tool>/<model>`, PRs go `--base main`, files stay under 500 lines, `core`, `sim`, `strategy` and `learn` stay pure, no new dependency.
- **A correction must have mean zero.** Whatever is subtracted from a deal's result is a function of the history up to a node times a zero-mean difference at it, so a deal is as fair as before and swings less. A term that cannot show this is not merged.
- **Adopted only on a reproduced variance ratio.** The corrected result is adopted when the heads-up pooled interval of the Arena's standard rows shrinks so that its variance falls by 2× or more, on two seeds (research note 04, section 5.9). At 6-max the policy mixes nowhere, so its own-decision term is zero and the ratio is near 1×: that is expected, and Task 3 is where 6-max is looked at.
- **The numbers move once.** Task 2 changes what an Arena run records, so a run before it and a run after it are not comparable bout for bout (`svanbot paired compare` across them would show the measurement, not the play). The PR says so, and every compare after it is between builds that both have it.
- Run Arena commands with a scratch fleet: `SVANBOT_FLEET=$(mktemp -d)`.

## Review Focus

1. **A challenger that reports a draw for a hand it did not decide, or reports one twice.** The term is subtracted once per decision that was drawn, and a hand that only the pricer decided subtracts nothing. (Task 1)
2. **A hand the sim abandons or a deal whose rotation reports draws out of order.** The draws of one rotation are never charged to the next. (Task 1)
3. **A reading that is wrongly decided early.** A check never admits or refuses on fewer than 30 bouts, and a heat that fails to play stops the run without recording a half heat. (Task 4)
4. **A calibration point with no sim estimate or an interval that is infinite.** It is not recorded, and a slope over fewer than two points is `None`, never a number. (Task 5)

## File map

| File | Task | Change |
|---|---|---|
| `crates/learn/src/paired.rs`, `ring.rs`, `arena/reader.rs`, `play.rs` | 1, 2 | `Draws`, `take_decided`, `with_draws` |
| `crates/apps/svanbot/src/arena.rs` | 2 | `play_bout` collects draws |
| `docs/research/2026-11/10-the-luck-of-decisions.md` | 2, 3 | create |
| `crates/runtime/src/ship/sample.rs`, `crates/apps/svanbot/src/play_arena.rs` | 4 | `sample_with`, sequential check |
| `crates/learn/src/calibration.rs`, `crates/runtime/src/fleet/calibration.rs`, `crates/runtime/src/probing.rs` | 5 | create, hook |

### Task 1: A hook for the decisions' draws

**Files:**
- Modify: `crates/learn/src/paired.rs`, `crates/learn/src/ring.rs`, `crates/learn/src/arena/reader.rs`, `crates/learn/src/play.rs`
- Test: `crates/learn/tests/paired.rs`, `crates/learn/tests/ring.rs`, `crates/learn/tests/play.rs`

**Interfaces:**
- Consumes: `learn::probe::{corrected, Decided}`; `learn::play::reading` (Phase 1, Task 6).
- Produces in `learn::paired`: `pub type Draws = std::rc::Rc<std::cell::RefCell<Vec<Decided>>>;` and a defaulted method on `Challenger`: `fn take_decided(&self) -> Vec<Decided> { Vec::new() }`, "the draws this hand's decisions were made by, taken and cleared".
- `duel_seen` and `ring_seen`: after each hand's `play_observed`, the hand's result is `corrected(won[seat], &challenger.take_decided())` in chips, then divided as before.
- Produces: `BoutReader::with_draws(self, draws: Draws) -> Self`, whose `take_decided` drains `draws`; and `learn::play::reading(policy, fixed, seats, take, draws: Option<Draws>)`, which pushes `Decided { strategy, values, taken }` for each decision that carries a `drawn`. Existing callers pass `None`.

- [ ] **Step 1: Write the failing tests.** In `tests/paired.rs` (`Blinds { small: 10, big: 20 }`, so a deal is `chips / 40`), a stub `Challenger` whose `decide` always folds and whose `take_decided` returns one `Decided { strategy: vec![0.5, 0.5], values: vec![10.0, 0.0], taken: 0 }` per call (a term of 5 chips a hand):
```rust
#[test] fn a_draw_a_challenger_reports_has_its_term_taken_off_each_hand()
// duel_seen with the reporting stub less duel_seen with the same stub reporting none == -10.0 / 40.0 exactly
```
  In `tests/ring.rs`, the same at three seats: `a_draw_is_taken_off_each_rotation` (a difference of `-15.0 / 60.0`). In `tests/play.rs`:
```rust
#[test] fn a_resolved_decision_notes_its_draw_and_a_priced_one_notes_none()
// a heads-up flop spot, draws Some(handle): handle holds one Decided whose strategy sums to 1.0; a preflop spot: handle stays empty
```
- [ ] **Step 2: Run** `T -p svanbot-learn paired ring play`. Expected: FAIL to compile (`take_decided`, `Draws`, the new parameter not found).
- [ ] **Step 3: Implement** per the Interfaces.
- [ ] **Step 4: Run** `T -p svanbot-learn`. Expected: PASS, every existing test included (nothing reports draws yet).
- [ ] **Step 5: Commit** `learn: the sim can subtract the luck of our own decisions (V11-<nn>)`.

### Task 2: Switch it on in the Arena and measure it

**Files:**
- Modify: `crates/apps/svanbot/src/arena.rs` (`play_bout`)
- Create: `docs/research/2026-11/10-the-luck-of-decisions.md` (and a row in `docs/research/2026-11/README.md` and `docs/README.md`)

**Interfaces:**
- Consumes: `BoutReader::with_draws`, `Draws`, `play::reading(.., Some(draws))` (Task 1).
- Produces: `play_bout` builds one `Draws` handle per bout, passes it to `reading` and to the reader. Every Arena run, and the ship bar's replay of a bout, now records corrected deals.

- [ ] **Step 1: Wire `play_bout`.** No new test: Task 1's tests cover the hook, and the next step covers the wiring end to end.
- [ ] **Step 2: Measure on two seeds.** From the merge base and from the branch, with a scratch fleet each: `target/dev/debug/svanbot arena 20 7 20 <name>-hu.json`, `… arena 6-max 20 7 20 <name>-6.json`, and the same with seed 11. Read each row's `95% CI` from the scoreboard. Expected: per-row means of the two builds agree within the wider interval (the term has mean zero), and the intervals narrow. Write each row's variance ratio, `(ci_before / ci_after)²`, and the pooled ratio per league into the research note, with the wall time.
- [ ] **Step 3: Run `T -p svanbot` and `mise run full`.** Expected: PASS. The ship bar's replay test (`crates/runtime/tests/ship.rs`) must still pass: a run and its replay are the same commit.
- [ ] **Step 4: Decide.** A pooled heads-up ratio of 2× or more on both seeds: open the PR with the wiring, the table of ratios, and the sentence that runs before and after it are not comparable. Below it: do not merge the wiring. Task 1's hook stays (it is inert), the note records the ratios, and Task 3 decides what can reach 2×.
- [ ] **Step 5: Commit** `apps: the Arena subtracts the luck of our own decisions (V11-<nn>)`.

### Task 3: The scripted opponents' decisions: a prototype

Throwaway until a go. Own decisions exist only where the policy mixes: the re-solve, heads up after the flop. At 6-max the pricer takes its best action every time, so the likelier gain there is the scripted opponents' rolls (the maniac's `roll < 0.65`, the learning one's `roll < steal`). This task measures how much, before anything is built.

**Files:**
- Create: `crates/learn/tests/aivat_prototype.rs` (marked `#[ignore]`, deleted unless there is a go)
- Modify: `docs/research/2026-11/10-the-luck-of-decisions.md`

- [ ] **Step 1: Write the harness.** Play 400 corrected deals of the reference policy against the maniac, heads up and at 6-max (`learn::arena::{bout, ring_bout_at}`). For each deal also record how many of the opponent's decisions were rolls that went to a bet: add a prototype-only `Sparring::decide_noted` (in the test, or behind `#[cfg(test)]`), not a product API.
- [ ] **Step 2: Estimate the room.** Regress the deals' corrected results on that count, per league. The R² is the share of variance an opponent term could remove. Write the numbers into the note.
- [ ] **Step 3: Decide and record.** A go needs `R² ≥ 0.5` in a league (a 2× cut). On a go: write the follow-up plan (value estimate for the opponent's alternatives, the term, the zero-mean test, which archetypes expose a distribution) as a new ticket. On a no-go: delete the harness, record the numbers, and say Phase 0's 2× is not reachable by decision terms, so the margin check leans on Task 4's sequential stopping and on more deals.
- [ ] **Step 4: Commit** the note only: `docs: how much of the Arena's variance is the opponents' rolls (V11-<nn>)`.

### Task 4: A sequential check

**Files:**
- Modify: `crates/runtime/src/ship/sample.rs`, `crates/apps/svanbot/src/play_arena.rs`
- Test: `crates/runtime/tests/ship_sample.rs`, `crates/apps/svanbot/tests/play.rs`

**Interfaces:**
- Consumes: `learn::margin::{pooled, read, Admission, MIN_BOUTS}` and the `svanbot play check` of the Phase 1 plan, Task 9.
- Produces in `runtime::ship::sample`: `pub fn sample_with(runs: &mut [ArenaRun], play: &mut Player<'_>, out_of_time: &mut dyn FnMut() -> bool, record: &mut dyn FnMut(&ArenaRun), judge: &dyn Fn(&[ArenaRun]) -> Bar) -> Bar`, the body of `sample` with its judge passed in; `sample` calls it with `judge`.
- Produces: `svanbot play check <kind> <style> --until-decided [seed] [iterations]`. Its four runs (reference and candidate, heads up and 6-max) play a heat at a time, each heat one more bout of every row; after each heat both leagues are read through `margin::pooled` and `margin::read`. It stops `Refused` when either league is `Refused`, `Clears` when both are `Admitted`, and refuses at `ship::CAP_HOURS`. A check records only the final state.

- [ ] **Step 1: Write the failing tests.** In `tests/ship_sample.rs`:
```rust
#[test] fn sample_with_stops_at_the_heat_its_judge_decides()      // a judge that clears at 3 heats: 3 heats played, 3 records of each run, Bar::Clears
#[test] fn sample_with_refuses_when_the_cap_runs_out_undecided()  // a judge that never decides, out_of_time true after 2 heats: Bar::Refused
```
  In `tests/play.rs`, at the library seam, a judge built from crafted runs (60 bouts): `a_check_reads_nothing_on_fewer_than_thirty_bouts` (29 bouts of a hopeless candidate: `Bar::Open`), `a_check_refuses_the_moment_a_league_is_refused`, `a_check_clears_only_when_both_leagues_are_admitted`.
- [ ] **Step 2: Run** `T -p svanbot-runtime ship_sample` and `T -p svanbot play`. Expected: FAIL.
- [ ] **Step 3: Implement** `sample_with`, the check's judge (a function in `svanbot::play` beside `judge`), and the loop in `play_arena.rs`.
- [ ] **Step 4: Run** `T -p svanbot-runtime` and `T -p svanbot`. Expected: PASS, the ship bar's own sampler tests unchanged.
- [ ] **Step 5: Commit** `apps: a margin check that stops when it is decided (V11-<nn>)`.

### Task 5: Sim-to-live calibration

**Files:**
- Create: `crates/learn/src/calibration.rs`, `crates/learn/tests/calibration.rs`, `crates/runtime/src/fleet/calibration.rs`
- Modify: `crates/learn/src/lib.rs`, `crates/runtime/src/fleet.rs` (a `mod`), `crates/runtime/src/probing.rs`, `crates/apps/svanbot/src/main.rs` (one dispatch line)
- Test: `crates/runtime/tests/fleet.rs`, `crates/runtime/tests/probing.rs`

**Interfaces:**
- Produces in `learn::calibration`:
```rust
pub struct Point { pub sim: f64, pub live: f64, pub radius: f64 }   // bb/100; radius is the live interval's half-width
pub struct Slope { pub value: f64, pub radius: f64, pub points: usize }
pub fn slope(points: &[Point]) -> Option<Slope>
```
  Weighted least squares through the origin, each live edge weighted `1 / se²` with `se = radius / 1.96`: `value = Σ w·sim·live / Σ w·sim²`, `radius = 1.96 / sqrt(Σ w·sim²)`. `None` for fewer than two points, a non-finite radius, or `Σ w·sim² == 0`.
- Produces in `runtime::fleet`: `Fleet::record_calibration(&self, probe: u64, point: &Point, verdict: &str, ended_ms: i64) -> Result<(), String>` and `Fleet::calibration(&self) -> Result<Vec<Point>, String>`, over `calibration.json`, one entry per probe id, newest 50.
- `probing.rs`: when a probe ends and its proposal carried a `sim_estimate`, record the point with the live `(edge, radius)`.
- `svanbot calibration`: prints each point and the slope line `calibration: slope 0.8 ±0.5 over 6 probes` (or `fewer than two probes`).

- [ ] **Step 1: Write the failing tests.** In `tests/calibration.rs`:
```rust
#[test] fn a_live_edge_equal_to_the_sim_is_a_slope_of_one()
// [Point{sim:2,live:2,radius:1.96}, Point{sim:4,live:4,radius:1.96}] -> value 1.0, radius = 1.96 / 20f64.sqrt()
#[test] fn a_live_edge_half_the_sim_is_a_slope_of_one_half()        // live = 1 and 2 at sim 2 and 4: 0.5
#[test] fn fewer_than_two_points_or_an_infinite_radius_has_no_slope()
```
  In `tests/fleet.rs`: `a_calibration_point_is_kept_once_a_probe_and_the_newest_fifty`. In `tests/probing.rs`: `a_probe_that_ends_with_a_sim_estimate_leaves_a_calibration_point` and `a_probe_with_no_sim_estimate_leaves_none`.
- [ ] **Step 2: Run** `T -p svanbot-learn calibration` and `T -p svanbot-runtime fleet probing`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** per the Interfaces.
- [ ] **Step 4: Run** `T -p svanbot-learn`, `T -p svanbot-runtime`, `T -p svanbot`. Expected: PASS.
- [ ] **Step 5: Commit** `learn, runtime: sim estimate against live edge, logged (V11-<nn>)`.

**Phase 0 is done when** Task 2's ratios are recorded; Task 3 has a go or a no-go on record; the margin check stops when decided; and the calibration slope is logged and printed by `svanbot calibration`. Where a 2× cut is not reached, the note says so and the operator decides whether the checks run longer instead.
