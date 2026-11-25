# Per-bot policy, Phase 1: the tracer. Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans, inline. The operator's rule is no subagents (`CLAUDE.md`). Each task runs the repo's `tdd` loop: one failing test, the least code to pass it, repeat. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Two non-canary bots play `tight` and `loose` live, each proven against the reference policy by a recorded margin check and watched by a live guard, while every bot with no assignment plays exactly as it does today.

**Architecture:** A bot's policy becomes a value (`Policy`: a kind, a style, a parameter set) that decides in `strategy`. `learn` holds the margin rule and the policy-playing deciders and stablemate bouts. `runtime` holds the assignment, check and policy-run files in the fleet directory. `apps` holds `svanbot play …` and the live wiring. Opening width is the one new mechanic, pinned in a style, not added to `Params`.

**Tech Stack:** Rust (`svanbot-strategy`, `svanbot-learn`, `svanbot-runtime`, `svanbot`), SQLite through the existing `rusqlite`, Bash and Bun for the guard script and its test. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-25-five-brains-design.md`, design v2, approved 2026-11-25. This plan is the spec's Phase 1. Phase 0 (measurement) is `2026-11-25-per-bot-policy-phase-0-measurement.md`; only Task 11's checks wait on it.

**Where this plan departs from the design** (each is the plan's call, for the operator to overrule):
- The design's S5 says the status and each decision trace carry the policy's names. This plan does not: the bot logs its policy at start and on every change, and `svanbot play status` lists the assignments. A `Status` field touches five struct literals, a trace field touches the store's schema, and `main.rs` has 41 lines to spare. The field comes with the dashboard ticket of Phase 3.
- The margin pools only the standard rows (the pricer, v10.0.2 and the five archetypes). Stablemate rows are played, recorded and printed beside it, but a style is meant to do better against some opponents and worse against others, so the duels do not decide admission.
- The guard's "incident noted" is a `CRIT` line on stderr, which the cron wrapper appends to `cron.log`; the store's incidents belong to fair play.

## Global Constraints

- **Where and how to work.** A worktree under `.claude/worktrees/v11-<nn>-<slug>`, branched from `origin/main`; `~/NewSvanBot` stays on `main` and is never built in or edited. One ticket claimed at a time (`gh issue edit <n> --add-label agent-claimed`), as `svanlabs[bot]`: `GH_TOKEN=$(~/.config/svanbot/app-token.sh)`.
- **Build and test.** One build at a time. `T` below means `CARGO_TARGET_DIR=target/dev nice -n 15 ionice -c3 cargo nextest run --build-jobs 4 --test-threads 4`. Packages: `svanbot-strategy`, `svanbot-learn`, `svanbot-runtime`, `svanbot`.
- **Commits and PRs.** Subject `<area>: <what> (V11-<nn>, #<issue>)`. Every message ends with a `Generated-by: <tool>/<model>` trailer (the commit-msg hook refuses one without it). PRs go `--base main`; the body ends with the same line. `mise run commit` runs on every commit; run `mise run full` before a push.
- **Purity.** `core`, `sim`, `strategy`, `learn` take no clock, file, network or entropy; seeds and work limits come in as inputs; sums in a fixed order. No third-party crate or package.
- **500 lines.** Rust files stay under 500. Already close: `main.rs` 459, `fleet.rs` 437, `store.rs` 496, `status.rs` 374. New code goes in new modules; `main.rs` gains no more than about ten lines.
- **Legal and in-process.** Every kind returns its action through the same nearest-legal step. Decisions stay in-process.
- **Fleet files** hold parameter sets and policy names, never a read of an opponent (ADR 0004, widened by ADR 0005). Two stablemates never share a table; a stablemate row is the sim only.
- **Nothing changes unless assigned.** A bot with no assignment plays today's play bit for bit. The ship bar never reads a stablemate row. New rows are named inside `[A-Za-z0-9 -]`.
- **Values pinned by the spec.** The reference policy is kind `solving`, style `standard`, the fleet champion. `standard` pins `open_scale` 1.0, `tight` 0.75, `loose` 1.25. The margin is 20 bb/100 (20,000 mbb/100), the live harm guard's own; the guard is 3,000 hands. Opening shares are 15% UTG up to 80% heads up. A margin reading needs 30 bouts and is read at the 5% level through `learn::probe::Sequence`.
- **The Arena compare** (bit for bit). Tasks 2, 4, 6 and 7 change decision code and must show no movement. Build `svanbot` at the merge base and on the branch, and run both with a scratch fleet, so the live fleet's `arena.json` is never written: `SVANBOT_FLEET=$(mktemp -d) target/dev/debug/svanbot arena 2 7 20 <name>-hu.json` and `… arena 6-max 2 7 20 <name>-6.json`. Then `svanbot paired compare base-hu.json next-hu.json` and the same for 6-max: every line must read `+0 mbb/100, 95% CI ±0`, and the exit code is 0. Quote the lines in the PR.

## Review Focus

Each line has its test in the task named.

1. **An assignment file that is missing, unreadable, empty, or names a kind or style that no longer exists.** A live bot keeps playing the reference, says so once, and never crashes. (Task 8)
2. **An assignment to a bot name that is mistyped or has no store.** It is refused, never written; names match exactly. (Tasks 8, 9)
3. **A stale, foreign or failed check.** A check for another champion digest, another kind or style, or with `admitted: false` never lets a bot be assigned. A promotion after assignment does not unassign. (Task 9)
4. **A guard with too few hands, an empty control, or the styled bot counted in its own control.** It never reverts without evidence. (Task 10)
5. **Stablemate rows leaking into `arena.json`.** The ship bar's input is unchanged while policy runs exist. (Task 7)

## File map

| File | Task | Change |
|---|---|---|
| `GLOSSARY.md`, `docs/adr/0005-a-bots-policy-is-its-own.md`, `docs/README.md`, `docs/superpowers/{specs,plans}/…` | 1 | create, edit |
| `crates/strategy/src/preflop.rs`, `pricer.rs` | 2 | `share_at`, `opens_at`, `price_lines_at` |
| `crates/strategy/src/kind.rs`, `style.rs`, `lib.rs` | 3 | create |
| `crates/strategy/src/policy.rs` | 4 | `Policy`, kind-aware `decide` |
| `crates/learn/src/margin.rs`, `lib.rs` | 5 | create |
| `crates/learn/src/play.rs`, `lib.rs` | 6 | create |
| `crates/runtime/src/fleet.rs`, `fleet/policies.rs` | 7, 8, 9 | policy runs, assignments, checks |
| `crates/apps/svanbot/src/arena.rs`, `deciders.rs`, `play_arena.rs`, `main.rs` | 6, 7, 9 | policy-aware Arena, `svanbot play` |
| `crates/apps/svanbot/src/live_policy.rs`, `play.rs`, `play/judge.rs`, `play/guard.rs`, `lib.rs` | 8, 9, 10 | create |
| `crates/runtime/src/store/window.rs` | 10 | `corrected_since` |
| `scripts/guard.sh`, `tools/guard.test.ts`, `deploy/README.md` | 10 | create, edit |

### Task 1: Docs, ADR 0005 and the map (S0)

**Files:**
- Modify: `GLOSSARY.md`, `docs/README.md`
- Create: `docs/adr/0005-a-bots-policy-is-its-own.md`, `docs/superpowers/specs/2026-11-25-five-brains-design.md`, `docs/superpowers/plans/2026-11-25-per-bot-policy-phase-0-measurement.md`, `docs/superpowers/plans/2026-11-25-per-bot-policy-phase-1-tracer.md`

**Interfaces:**
- Produces: the glossary terms every later task names things by: **Policy kind** (`solving`, `pricing`), **Style** (`standard`, `tight`, `loose`), **Reference policy**, **Margin**, **Margin check**, **Assignment**, **Control**, **Stablemate row**.

- [ ] **Step 1: Claim the docs ticket and make the worktree.** `git fetch origin && git worktree add .claude/worktrees/v11-<nn>-policy-docs -b v11-<nn>-policy-docs origin/main`.
- [ ] **Step 2: Copy the approved design and both plans** into the paths above, unchanged, each keeping its `Generated-by:` line.
- [ ] **Step 3: Edit `GLOSSARY.md`** (`domain-modeling`: tight definitions, no implementation detail). Revise **Policy** (a kind, a style and a parameter set; a designed one is a brain) and **Opening range** (a style may scale its shares). Add **Policy kind**, **Style**, **Reference policy**, **Margin**, **Margin check**, **Control**, **Assignment** and **Stablemate row**, each with an `_Avoid_` line.
- [ ] **Step 4: Write ADR 0005**, `accepted (2026-11-25)`, in the form of ADRs 0001 to 0004 (context, decision, status, considered options, consequences). It records: a policy is a kind, a style and a champion, and each bot may have its own; the margin rule; stablemate rows do not gate a release; the fleet-file rule widens from "parameter sets only" to parameter sets and policy names; it reverses V11-09's one fleet champion in stages (Phase 2 freezes the reference). Considered and rejected: profiles only; a separate policy module per bot.
- [ ] **Step 5: Index it** in `docs/README.md`: an ADR row for 0005 and a "Designs and plans" section linking the three new files.
- [ ] **Step 6: Run `mise run commit`.** Expected: every check passes (no Rust changed; the secret scan and provenance checks run).
- [ ] **Step 7: Commit and open the PR** (`pr` skill; docs only, so no paired sim): `git add GLOSSARY.md docs && git commit -m "docs: a bot's policy is its own: glossary, ADR 0005, the design and its plans (V11-<nn>)" -m "Generated-by: <tool>/<model>"`.
- [ ] **Step 8: File the map and tickets. Do not run this step until the operator has seen the list below and said yes.** As `svanlabs[bot]`: create the map issue (title `V11-<nn>: Five brains: a policy of its own for each bot`, label `wayfinder:map`, body with Destination, Notes, Order, Decisions so far, Not yet specified, Out of scope, from the design), then one sub-issue per task below with labels `v11`, `wayfinder:task`, `feature` and its area, each body ending `Generated-by:`: S1 `area:strategy` (Task 2), S2 `area:strategy` (Tasks 3, 4), S3 `area:learn` (Task 5), S4 `area:learn` and `area:runtime` (Tasks 6, 7), S5 `area:runtime` (Task 8), S6 `area:tooling` (Task 9), S7 `area:runtime` (Task 10), S8 operations (Task 11). Link them with the REST calls in `docs/agents/issue-tracker.md`. **Done 2026-11-25:** the map is V11-59 (#1190) and the tickets are V11-60 to V11-74 (#1191 to #1205), blocked as the map's Order and each ticket say; Phase 0's five are among them.

### Task 2: Opening width (S1)

**Files:**
- Modify: `crates/strategy/src/preflop.rs`, `crates/strategy/src/pricer.rs`
- Test: `crates/strategy/tests/preflop.rs`, `crates/strategy/tests/pricer.rs`

**Interfaces:**
- Produces, in `preflop.rs`: `impl Position { pub fn share_at(self, scale: f64) -> f64 }`, which is `(self.share() * scale).min(1.0)`; `pub fn opens_at(class: usize, position: Position, scale: f64) -> bool`. `opens(class, position)` becomes `opens_at(class, position, 1.0)`.
- Produces, in `pricer.rs`: `pub fn price_lines_at(spot: &Spot, models: &[&ActionModel], budget: Budget, open_scale: f64) -> Decision`. `price_lines` calls it with `1.0`; the private `opening` takes the scale and calls `preflop::opens_at`.

- [ ] **Step 1: Write the failing tests.** In `tests/preflop.rs` (`use svanbot_strategy::preflop::{CLASSES, Position, opens, opens_at}`; positions are `behind` 1 to 5 with `in_position` both ways):
```rust
#[test] fn the_standard_width_is_the_opening_range_it_always_was()      // opens_at(c, p, 1.0) == opens(c, p) for every class and position
#[test] fn a_wider_width_never_closes_a_class_a_narrower_one_opens()    // opens_at at 0.75 implies at 1.0 implies at 1.25
#[test] fn a_share_never_passes_every_hand() {
    assert_eq!(Position { behind: 1, in_position: true }.share_at(1.25), 1.0);
    assert!((Position { behind: 5, in_position: false }.share_at(1.25) - 0.1875).abs() < 1e-12);
}
```
  In `tests/pricer.rs`, beside `an_unopened_pot_is_opened_or_folded_by_its_position`, using its `spot` and `facing` fixtures and `lines = vec![vec![]; 5]`:
```rust
#[test] fn a_hand_on_the_edge_of_the_range_opens_only_when_the_width_grows()
// find a class where !opens_at(c, utg, 1.0) && opens_at(c, utg, 1.25) (assert one exists), set the spot's hole to preflop::example(c):
// price_lines_at(.., 1.0).action == Action::Fold and == price_lines(..).action; price_lines_at(.., 1.25).action == Action::RaiseTo(50)
```
- [ ] **Step 2: Run** `T -p svanbot-strategy preflop pricer`. Expected: FAIL to compile (`opens_at`, `share_at`, `price_lines_at` not found).
- [ ] **Step 3: Implement** the three functions as the Interfaces say; no call site changes.
- [ ] **Step 4: Run** `T -p svanbot-strategy`. Expected: PASS, the existing tests included.
- [ ] **Step 5: The Arena compare** (Global Constraints). Expected: every line `+0 mbb/100, 95% CI ±0`.
- [ ] **Step 6: Commit** `strategy: the opening range has a width (V11-<nn>)`.

### Task 3: Kind and style (S2, first half)

**Files:**
- Create: `crates/strategy/src/kind.rs`, `crates/strategy/src/style.rs`, `crates/strategy/tests/style.rs`
- Modify: `crates/strategy/src/lib.rs` (add `pub mod kind; pub mod style;`)

**Interfaces:**
- Produces: `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum PolicyKind { Solving, Pricing }` with `pub const ALL: [Self; 2]`, `pub const fn name(self) -> &'static str` (`"solving"`, `"pricing"`) and `pub fn named(name: &str) -> Option<Self>` (exact, lowercase).
- Produces: `pub struct Style { pub name: &'static str, pub open_scale: f64, pub profile: Profile<'static> }` with a hand-written `Debug` that prints the name; `pub const STANDARD`, `TIGHT`, `LOOSE` (profile `ACTIVE` in all three, scales 1.0, 0.75, 1.25); `pub const ALL: [&'static Style; 3]` in that order; `pub fn named(name: &str) -> Option<&'static Style>`.

- [ ] **Step 1: Write the failing tests** in `tests/style.rs`:
```rust
#[test] fn a_kind_and_a_style_are_known_by_the_name_they_print()   // round trips; "solving"/"pricing" known; "Solving", "nope" are None
#[test] fn the_standard_style_is_todays_play()                     // STANDARD.open_scale == 1.0; its profile is empty iff ACTIVE is
#[test] fn the_working_styles_pin_the_values_of_the_design()       // TIGHT 0.75, LOOSE 1.25
#[test] fn every_style_pins_a_width_a_player_could_play()          // every ALL entry in 0.5..=1.5; names unique
```
- [ ] **Step 2: Run** `T -p svanbot-strategy style`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** the two modules per the Interfaces.
- [ ] **Step 4: Run** `T -p svanbot-strategy`. Expected: PASS.
- [ ] **Step 5: Commit** `strategy: a policy kind and a style (V11-<nn>)`.

### Task 4: The policy value (S2, second half)

**Files:**
- Modify: `crates/strategy/src/policy.rs`
- Test: `crates/strategy/tests/policy.rs`

**Interfaces:**
- Consumes: `price_lines_at` (Task 2), `PolicyKind`, `Style`, `STANDARD` (Task 3).
- Produces: `#[derive(Clone, Copy, Debug)] pub struct Policy { pub kind: PolicyKind, pub style: &'static Style, pub params: Params }` with
  `pub fn new(kind: PolicyKind, style: &'static Style, params: Params) -> Self`,
  `pub fn reference(params: Params) -> Self` (`Solving`, `STANDARD`),
  `pub fn name(&self) -> String` (`"solving/tight"`),
  `pub fn decide(&self, spot: &Spot, model: Option<&ActionModel>, personal: &[Personal], budget: Budget, iterations: u32, stats: &[SeatedStats]) -> Decision`.
- Internal: the body of `decide_profile` moves to a private `decide_with(spot, model, personal, params, budget, iterations, profile, kind, open_scale)`; `decide_profile` calls it with `Solving` and `1.0`, so no existing caller moves. `Pricing` skips `resolve`; both kinds price through `price_lines_at(.., open_scale)`. `Policy::decide` calls it with `self.style.profile.with_opponents(stats)` and the style's `open_scale`.

- [ ] **Step 1: Write the failing tests** in `tests/policy.rs`, using the file's existing spots (a heads-up flop that resolves, a three-way spot that prices, an unopened preflop spot):
```rust
#[test] fn the_reference_policy_decides_what_decide_profile_decides()
// for each spot, with and without a personal read: Policy::reference(Params::DEFAULT).decide(&spot, None, &personal, BUDGET, 20, &[])
//   == decide_profile(&spot, None, &personal, &Params::DEFAULT, BUDGET, 20, ACTIVE)   (Decision is PartialEq)
#[test] fn a_pricing_policy_never_resolves()          // the heads-up flop: branch Priced, drawn None
#[test] fn a_style_changes_how_wide_a_policy_opens()  // an edge class (Task 2's finder): reference Fold, loose RaiseTo(50); a class the reference opens and tight does not: tight Fold
#[test] fn a_policy_names_its_kind_and_style()        // "solving/tight", and Policy::reference(..).name() == "solving/standard"
```
- [ ] **Step 2: Run** `T -p svanbot-strategy policy`. Expected: FAIL to compile (`Policy` not found).
- [ ] **Step 3: Implement** per the Interfaces. Keep `policy.rs` under 500 lines; if the new code passes about 450, move `Policy` to `crates/strategy/src/policy/play.rs` and re-export it.
- [ ] **Step 4: Run** `T -p svanbot-strategy`, then the Arena compare. Expected: PASS; every compare line `+0 mbb/100, 95% CI ±0`.
- [ ] **Step 5: Commit** `strategy: a policy is a kind, a style and a parameter set (V11-<nn>)`.

### Task 5: The margin rule (S3)

**Files:**
- Create: `crates/learn/src/margin.rs`, `crates/learn/tests/margin.rs`
- Modify: `crates/learn/src/lib.rs` (add `pub mod margin;`)

**Interfaces:**
- Consumes: `learn::probe::Sequence`, `learn::arena::BOUT` (50).
- Produces:
```rust
pub const MARGIN: f64 = 20_000.0;   // mbb/100
pub const MIN_BOUTS: usize = 30;
pub enum Admission { Admitted, Refused, Unresolved }          // Clone, Copy, Debug, PartialEq, Eq
pub struct Reading { pub admission: Admission, pub mean: f64, pub radius: f64, pub bouts: usize }   // Clone, Copy, Debug, PartialEq
pub fn pooled(base: &[&[f64]], next: &[&[f64]]) -> Option<Vec<f64>>
pub fn read(differences: &[f64], margin: f64) -> Reading
```
  `pooled`: per bout, the mean over rows of `(next - base) / BOUT as f64 * 100_000.0`; `None` unless both have the same non-zero number of rows, every row has the same non-zero number of bouts in both. `read`: a `Sequence::new(200.0)` over the differences, `radius(0.05)`; fewer than `MIN_BOUTS` is `Unresolved`; else `mean + radius < -margin` is `Refused`; else `radius > margin` is `Unresolved`; else `Admitted`.

- [ ] **Step 1: Write the failing tests** in `tests/margin.rs`:
```rust
#[test] fn pooled_averages_each_bouts_difference_over_the_rows() {
    // rows base [[10.,20.],[0.,0.]], next [[15.,30.],[4.,4.]] -> Some(vec![9_000.0, 14_000.0])
}
#[test] fn pooled_refuses_runs_that_are_not_the_same_bouts()      // empty; unequal rows; unequal bouts: None
#[test] fn a_policy_inside_the_margin_is_admitted()               // read(&vec![-5_000.0; 60], MARGIN).admission == Admitted
#[test] fn a_policy_wholly_below_minus_the_margin_is_refused()    // vec![-40_000.0; 60] -> Refused
#[test] fn too_few_bouts_are_not_yet_read()                       // vec![-40_000.0; 10] -> Unresolved, bouts == 10
#[test] fn a_wide_interval_is_unresolved_not_admitted()           // 40 bouts alternating +150_000.0 and -150_000.0 -> Unresolved
#[test] fn a_wide_interval_wholly_below_minus_the_margin_is_still_refused()  // 40 bouts alternating -350_000.0 and -50_000.0 -> Refused
```
  Assert the admission only, never the radius value.
- [ ] **Step 2: Run** `T -p svanbot-learn margin`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** per the Interfaces.
- [ ] **Step 4: Run** `T -p svanbot-learn`. Expected: PASS.
- [ ] **Step 5: Commit** `learn: the margin rule (V11-<nn>)`.

### Task 6: The Arena plays a policy (S4, first half)

**Files:**
- Create: `crates/learn/src/play.rs`, `crates/learn/tests/play.rs`
- Modify: `crates/learn/src/lib.rs` (`pub mod play;`), `crates/apps/svanbot/src/deciders.rs`

**Interfaces:**
- Consumes: `Policy` (Task 4); `learn::reads::Reads`, `learn::arena::opponent_name`, `learn::paired::{Challenger, Match, duel_seen}`, `learn::ring::ring_seen`.
- Produces in `learn::play`:
```rust
pub const SAMPLES: u64 = 2_000;
pub const STABLEMATE: &str = "stablemate ";   // a stablemate row's name is STABLEMATE + the style's name
pub type Take = fn(&Reads, &[(usize, &str)], &ActionModel, f64) -> Vec<Personal>;
pub fn reading(policy: Policy, fixed: Option<u64>, seats: usize, take: Take) -> impl Fn(&Spot, u64, &Reads) -> Action;
pub fn player(policy: Policy, fixed: Option<u64>) -> impl Fn(&Spot, u64) -> Action;      // no reads: a stablemate
pub fn profile_stats(policy: &Policy, reads: &Reads, named: &[(usize, &str)]) -> Vec<SeatedStats>;   // empty when the style's profile is
pub fn rival_duel(game: &Match, deal: u64, challenger: &impl Challenger, rival: &impl Fn(&Spot, u64) -> Action) -> f64;
pub fn rival_ring_at(game: &Match, seats: usize, deal: u64, challenger: &impl Challenger, rival: &impl Fn(&Spot, u64) -> Action) -> f64;
```
  `reading` is the body of `deciders::reading_policy` with `policy.decide(.., &stats)` in place of `decide_profile(.., ACTIVE.with_opponents(&stats))` and `policy.params.exploit` for the read's budget. `player` is `deciders::challenger` the same way. `rival_duel` is `duel_seen(game, deal, challenger, &|_, spot, _, seed| rival(spot, seed), || {})`; `rival_ring_at` is `ring_seen(.., &|_, spot, _, seed| rival(spot, seed), |_| {})`, the code of the `pricer` row.
- `deciders.rs`: `reading_policy(params, fixed, seats, take)` becomes `learn::play::reading(Policy::reference(*params), …)`; `SAMPLES` re-exports `learn::play::SAMPLES`.

- [ ] **Step 1: Write the failing tests** in `tests/play.rs` (`GAME = Match { blinds: Blinds { small: 10, big: 20 }, stack: 2000, seed: 7 }`, `fixed = Some(2)`):
```rust
#[test] fn a_policy_playing_itself_round_a_table_breaks_even() {
    let play = player(Policy::reference(Params::DEFAULT), Some(2));
    for deal in 0..2 { assert!(rival_ring_at(&GAME, 3, deal, &play, &play).abs() < 1e-9); }
}
#[test] fn a_heads_up_stablemate_deal_plays_the_same_twice()    // rival_duel(..) called twice with the same arguments is bit-equal and finite
#[test] fn a_style_with_rules_collects_stats_per_seat()          // profile_stats(&Policy::reference(..), &Reads::default(), &[(1, "x")]).len() == 1
#[test] fn a_style_with_no_rules_collects_none()                 // a `static BARE: Style` with Profile::new(&[]): profile_stats(..) is empty
```
- [ ] **Step 2: Run** `T -p svanbot-learn play`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** `learn::play`, then rewrite `deciders::reading_policy` as the wrapper. `deciders::challenger` stays as it is.
- [ ] **Step 4: Run** `T -p svanbot-learn` and `T -p svanbot`, then the Arena compare. Expected: PASS; every compare line `+0 mbb/100, 95% CI ±0`.
- [ ] **Step 5: Commit** `learn: the sim's deciders play a policy, and a stablemate (V11-<nn>)`.

### Task 7: `svanbot play arena`, recorded apart (S4, second half)

**Files:**
- Create: `crates/runtime/src/fleet/policies.rs`, `crates/apps/svanbot/src/play_arena.rs`, `crates/apps/svanbot/tests/play.rs`
- Modify: `crates/runtime/src/fleet.rs`, `crates/apps/svanbot/src/arena.rs`, `crates/apps/svanbot/src/arena_ship.rs`, `crates/apps/svanbot/src/ship.rs`, `crates/apps/svanbot/src/main.rs`
- Test: `crates/runtime/tests/fleet.rs`

**Interfaces:**
- Consumes: `learn::play` (Task 6), `Policy`, `style::ALL`.
- Produces in `runtime::fleet`: `Fleet::policy_arena_runs(&self) -> Result<Vec<ArenaRun>, String>` and `Fleet::record_policy_arena(&self, run: &ArenaRun) -> Result<(), String>`, over `policy-runs.json`, with the same shape, replacement rule and `ARENA_HISTORY` as `arena.json`. In `fleet.rs`, `arena_runs` and `record_arena` become one-line calls to private `runs_in(file)` and `record_in(file, run)`.
- Produces in `arena.rs`: `play_bout(game, league, opponent, number, policy: &Policy, fixed)` (was `params: &Params`; the callers in `arena.rs`, `arena_ship.rs`, `ship.rs` pass `&Policy::reference(champion)`). Its challenger is `BoutReader::new(learn::play::reading(*policy, fixed, seats, Reads::deciding)).with_profile_stats(!policy.style.profile.is_empty())`, where it had `deciders::reading_policy(params, …)` and `!ACTIVE.is_empty()`. And `pub(crate) fn play_rows<R: Copy + Send + Sync>(rows: &[R], name: impl Fn(R) -> String, bouts: u64, play: impl Fn(R, u64) -> Vec<f64> + Sync) -> Vec<(Row, Vec<f64>)>`, extracted from `run` with its output unchanged.
- Produces: `svanbot play arena <kind> <style> [league] <bouts> [seed] [iterations] [out.json]`. Rows: `Opponent::all()` (the seven standard rows) then `stablemate standard`, `stablemate tight`, `stablemate loose`, each a `Policy::new(Solving, style, champion)` played with `learn::play::player`. The run's `champion` field is `"<policy name>@<params digest>"`. It is recorded with `record_policy_arena`, never `record_arena`, and also written to `out.json` when named.

- [ ] **Step 1: Write the failing tests.** In `crates/runtime/tests/fleet.rs`:
```rust
#[test] fn a_policy_run_is_kept_apart_from_the_arena_the_ship_bar_reads()
// record_policy_arena(&run): arena_runs() is empty and the arena.json file does not exist; policy_arena_runs() == [run]
#[test] fn a_policy_run_replaces_its_own_line_and_keeps_a_bounded_history()   // same league, commit, champion, seed replaces; ARENA_HISTORY per league
```
  In `crates/apps/svanbot/tests/play.rs` (spawn `CARGO_BIN_EXE_svanbot` with `.env_clear()`, as `paired_compare.rs` does):
```rust
#[test] fn play_with_no_subcommand_says_its_usage_and_exits_2()
#[test] fn an_unknown_kind_or_style_is_refused_before_anything_plays()   // exit 2; stderr names "solving", "pricing", "standard", "tight", "loose"
```
- [ ] **Step 2: Run** `T -p svanbot-runtime fleet` and `T -p svanbot play`. Expected: FAIL (methods not found; `play` is not a command).
- [ ] **Step 3: Implement** the runtime pair, then change `play_bout`'s parameter and its three callers, extract `play_rows`, add `play_arena.rs` with the `Rival` enum (`Standard(Opponent)` delegates to `play_bout`; `Stablemate(&'static Style)` plays `rival_duel` or `rival_ring_at`), and add `"play"` to `main.rs`'s dispatch beside `"learn"`, before the bot's start is recorded: a `play` run is not a bot's start and must not count in the updater's crash-loop record.
- [ ] **Step 4: Run** `T -p svanbot-runtime`, `T -p svanbot`, then the Arena compare. Expected: PASS; compare lines `+0 mbb/100, 95% CI ±0` (`svanbot arena` flows through `play_rows` now).
- [ ] **Step 5: Smoke the new command and time it.** `time SVANBOT_FLEET=$(mktemp -d) target/dev/debug/svanbot play arena solving tight 6-max 2 7 2 tight-6.json`. Expected: ten rows on the board, the three stablemate rows among them; the scratch fleet holds `policy-runs.json` and no `arena.json`. Note the wall time in the PR: Task 11 sizes its checks from it.
- [ ] **Step 6: Commit** `apps: svanbot play arena, with stablemate rows recorded apart (V11-<nn>)`.

### Task 8: Assignments and live play (S5)

**Files:**
- Modify: `crates/runtime/src/fleet/policies.rs`, `crates/apps/svanbot/src/lib.rs` (`pub mod live_policy;`), `crates/apps/svanbot/src/main.rs`
- Create: `crates/apps/svanbot/src/live_policy.rs`, `crates/apps/svanbot/tests/live_policy.rs`
- Test: `crates/runtime/tests/fleet.rs`

**Interfaces:**
- Produces in `runtime::fleet`: `#[derive(Clone, Copy, Debug)] pub struct Assignment { pub kind: PolicyKind, pub style: &'static Style }` (`PartialEq` by `kind` and `style.name`), and
  `Fleet::assignment(&self, bot: &str) -> Result<Option<Assignment>, String>` (`Ok(None)`: no file or no entry; `Err` names an unreadable file or an unknown kind or style),
  `Fleet::assignments(&self) -> Result<Vec<(String, Assignment, i64)>, String>` (bot, what, `since_ms`, by bot name),
  `Fleet::assign(&self, bot: &str, assignment: Assignment, since_ms: i64) -> Result<(), String>`,
  `Fleet::revert(&self, bot: &str) -> Result<bool, String>` (removes the file when the last entry goes). File `plays.json`: `{"bots":{"<name>":{"kind":"solving","style":"tight","since_ms":1}}}`. Names match exactly.
- Produces in `svanbot::live_policy`:
```rust
pub struct LivePolicy { /* fleet, bot, what it holds, whether the file is refused */ }
impl LivePolicy {
    pub fn open(fleet: Fleet, bot: &str) -> (Self, Vec<String>);     // notes to log
    pub fn refresh(&self) -> Option<String>;                         // a note only when what the bot plays by changed or the file became refused or readable
    pub fn policy(&self, params: Params) -> Policy;                  // the assignment, else the reference; the champion's numbers ride along
}
```
- `main.rs`: build `LivePolicy` beside `Probing`, log its notes; in the decision closure use `policies.policy(params)`, `learn::play::profile_stats(&policy, &reads, &named)` and `policy.decide(spot, model.as_ref(), &personal, budget, iterations, &stats)` in place of `decide_profile(.., ACTIVE.with_opponents(&stats))`; after `keep(..)` for an `Event::Hand`, call `refresh()` and log its note. Drop the now-unused `decide_profile` and `ACTIVE` imports.

- [ ] **Step 1: Write the failing tests.** In `tests/fleet.rs`:
```rust
#[test] fn no_file_and_no_entry_are_both_no_assignment()
#[test] fn an_assignment_is_kept_and_read_back_by_the_exact_name_only()   // "Svanar" found; "svanar" and "Other" None
#[test] fn reverting_removes_the_entry_and_the_file_with_the_last_one()
#[test] fn a_file_that_names_an_unknown_style_or_kind_is_refused_with_the_name()   // write plays.json by hand; the Err contains "nope"
#[test] fn assignments_are_listed_by_bot_with_the_hour_they_began()
```
  In `tests/live_policy.rs` (scratch fleet, `Params::DEFAULT`):
```rust
#[test] fn with_no_assignment_a_bot_plays_the_reference_policy()          // policy(..).name() == "solving/standard"
#[test] fn an_assignment_is_taken_up_between_hands_and_a_revert_is_too() // policy unchanged until refresh(); refresh() note mentions "solving/tight"; after revert, "reference"
#[test] fn a_refused_file_plays_the_reference_and_says_so_once()         // garbage, then an empty file: open() notes contain "refused"; refresh() is None next time; a fixed file refreshes with a note
#[test] fn another_bots_entry_is_not_mine()
#[test] fn the_champions_numbers_ride_along()                            // Params { exploit: 0.3 }.exploit survives into policy(..).params
```
- [ ] **Step 2: Run** `T -p svanbot-runtime fleet` and `T -p svanbot live_policy`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** the runtime methods in `fleet/policies.rs` and `LivePolicy`; wire `main.rs` as above. `main.rs` must end under 500 lines.
- [ ] **Step 4: Run** `T -p svanbot-runtime` and `T -p svanbot`. Expected: PASS.
- [ ] **Step 5: Check the unassigned bot is unchanged.** Run the Arena compare once more. Expected: `+0 mbb/100, 95% CI ±0`.
- [ ] **Step 6: Commit** `runtime, apps: a bot plays the policy it is assigned (V11-<nn>)`.

### Task 9: `svanbot play check`, `assign`, `revert`, `status` (S6)

**Files:**
- Modify: `crates/runtime/src/fleet/policies.rs`, `crates/apps/svanbot/src/play_arena.rs`, `crates/apps/svanbot/src/main.rs`, `crates/apps/svanbot/src/lib.rs` (`pub mod play;`)
- Create: `crates/apps/svanbot/src/play.rs`, `crates/apps/svanbot/src/play/judge.rs`
- Test: `crates/runtime/tests/fleet.rs`, `crates/apps/svanbot/tests/play.rs`

**Interfaces:**
- Produces in `runtime::fleet`: `pub struct Check { pub kind: PolicyKind, pub style: String, pub reference: Digest, pub seed: u64, pub iterations: Option<u64>, pub bouts: u64, pub admitted: bool, pub heads_up: String, pub six_max: String, pub checked_ms: i64 }` and `Fleet::checks(&self) -> Result<Vec<Check>, String>`, `Fleet::record_check(&self, check: &Check) -> Result<(), String>` over `checks.json`, newest first, one entry per kind and style.
- Produces in `svanbot::play`:
```rust
pub fn judge(reference: &ArenaRun, candidate: &ArenaRun, margin: f64) -> Result<(Reading, String), String>
pub fn assign(fleet: &Fleet, data_dir: &Path, bot: &str, kind: PolicyKind, style: &'static Style, now_ms: i64) -> Result<String, String>
pub fn revert(fleet: &Fleet, bot: &str) -> Result<String, String>
pub fn status(fleet: &Fleet) -> Result<String, String>
```
  `data_dir` is where the bots' stores are: the parent of the fleet directory (`fleet::beside_store` puts the fleet directory beside them).
  `judge` refuses runs that differ in league, seed or iterations, or whose standard rows differ in name or bout count; it pools only rows whose name does not start with `learn::play::STABLEMATE`, through `margin::pooled` then `margin::read`; the line reads `"{league}: {mean:+.0} mbb/100 ±{radius:.0} over {bouts} bouts: {admitted|refused|unresolved}"`. `assign` refuses, naming why: an unknown kind or style; a bot with no `<name>.db` in `data_dir`; no check on record for the kind and style; a check with `admitted: false`; a check whose `reference` is not the current champion's digest. Assigning `solving standard` is a `revert`.
- `svanbot play check <kind> <style> <bouts> [seed] [iterations]` (in `play_arena.rs`): for each of heads up and 6-max, play the reference policy and the candidate through `play_rows` with the same seed and rows, call `judge`, and record a `Check` (`admitted` only when both leagues read `Admitted`). It prints both lines and exits 0 when admitted, 1 when not.

- [ ] **Step 1: Write the failing tests.** In `tests/fleet.rs`: `a_check_is_kept_newest_first_one_a_policy_and_read_back_whole`. In `tests/play.rs` (library level, crafted `ArenaRun`s of 60 bouts per row, rows `pricer`, `nit`, `stablemate loose`):
```rust
#[test] fn a_policy_a_little_behind_the_reference_is_admitted()        // candidate bouts = reference bouts - 1.0 each: Admitted
#[test] fn a_stablemate_row_does_not_enter_the_pool()                  // a hopeless stablemate row, fine standard rows: still Admitted
#[test] fn runs_that_are_not_the_same_bouts_are_refused()              // different seed: Err mentions "seed"; different league; different iterations
#[test] fn a_bot_is_assigned_only_with_a_current_admitted_check()      // none: Err; admitted:false: Err; reference digest differs: Err mentions "stale"; admitted and current: Ok and fleet.assignment == Some
#[test] fn a_bot_with_no_store_is_refused_and_nothing_is_written()     // no <name>.db in data_dir
#[test] fn a_promotion_after_assignment_does_not_unassign()            // champion changes; assignment still read back; the next assign is refused as stale
#[test] fn assigning_the_reference_is_a_revert()
```
  CLI (spawn the binary with `SVANBOT_FLEET=<scratch>/fleet`, an empty `<scratch>/Svanar.db`): `assign_without_a_check_is_refused_with_exit_2`, `status_lists_assignments_and_checks_and_marks_a_stale_check`.
- [ ] **Step 2: Run** `T -p svanbot play` and `T -p svanbot-runtime fleet`. Expected: FAIL.
- [ ] **Step 3: Implement**; route `status`, `assign`, `revert` to `svanbot::play` and `arena`, `check` to `play_arena.rs` from the `"play"` dispatch. `main.rs` gains no more than a handful of lines.
- [ ] **Step 4: Run** `T -p svanbot-runtime` and `T -p svanbot`. Expected: PASS.
- [ ] **Step 5: Commit** `apps: svanbot play check, assign, revert and status (V11-<nn>)`.

### Task 10: The live guard (S7)

**Files:**
- Modify: `crates/runtime/src/store/window.rs`, `crates/apps/svanbot/src/play.rs`, `crates/apps/svanbot/src/play_arena.rs`, `deploy/README.md`
- Create: `crates/apps/svanbot/src/play/guard.rs`, `scripts/guard.sh`, `tools/guard.test.ts`
- Test: `crates/runtime/tests/window.rs`, `crates/apps/svanbot/tests/play.rs`

**Interfaces:**
- Produces: `Store::corrected_since(&self, since_ms: i64) -> Result<Vec<f64>, StoreError>`: `(net - luck) / big_blind` for each hand kept at or after `since_ms`, in the order kept.
- Produces in `svanbot::play::guard`:
```rust
pub enum Guarded { NotYet(String), Holds(String), Trips(String) }
pub fn judge(styled: &[f64], control: &[f64], hands: usize, margin_bb100: f64) -> Guarded
pub fn guard(fleet: &Fleet, data_dir: &Path, hands: usize, margin_bb100: f64) -> Result<Vec<(String, Guarded)>, String>
```
  `judge`: either side under `hands` is `NotYet`; else with `ours = verdict(styled)`, `theirs = verdict(control)`, `gap = ours.mbb_per_100 - theirs.mbb_per_100`, `ci = ours.ci95.hypot(theirs.ci95)`: `Trips` when `gap + ci < -(margin_bb100 * 1000.0)`, else `Holds`. `guard`: for each assignment, read the bot's `corrected_since(since_ms)` and every other store in `data_dir` that holds no assignment as the control (opened with `Store::open_reader`); `Trips` reverts the entry and prints a `CRIT` line naming the bot and the numbers to stderr. The CLI is `svanbot play guard`, with `hands` and the margin from `svanbot::learn::{GUARD_HANDS, GUARD_MARGIN}`.
- `scripts/guard.sh`: modelled on `scripts/auto-update.sh`: sets `PATH` for cron, takes a `flock` so one run at a time, runs `nice -n 19 ionice -c3 svanbot play guard`, and logs a dated line.

- [ ] **Step 1: Write the failing tests.** `tests/window.rs`: `corrected_results_are_net_less_luck_in_big_blinds_since_the_hour`. `tests/play.rs`:
```rust
#[test] fn the_guard_waits_for_the_hands_before_it_says_anything()             // 10 and 10 of 3_000: NotYet, never Trips
#[test] fn a_styled_bot_clearly_losing_to_the_control_trips()                 // 3_000 hands each, styled mean -0.4 bb, control 0.0, small spread: Trips
#[test] fn a_styled_bot_level_with_the_control_holds()
#[test] fn the_guard_never_counts_the_styled_bot_in_its_own_control()         // scratch stores via Store::open + append_played, hands = 20: control excludes assigned bots
#[test] fn a_trip_reverts_the_assignment_and_a_hold_leaves_it()
#[test] fn an_empty_control_is_not_yet_never_a_trip()
```
  `tools/guard.test.ts`: copy the harness of `tools/auto-update.test.ts` (scratch home, a `systemctl` that refuses, a call log); assert `guard.sh` calls `svanbot play guard` once, does nothing while another run holds the lock, and never reaches `systemctl`.
- [ ] **Step 2: Run** `T -p svanbot-runtime window`, `T -p svanbot play` and `bun test tools/guard.test.ts`. Expected: FAIL.
- [ ] **Step 3: Implement** `corrected_since` in `store/window.rs` (it is 88 lines; `store.rs` is at 496 and gets nothing), the guard, the CLI route and the script. Add the cron line to `deploy/README.md` beside the others: `*/30 * * * * $HOME/NewSvanBot/scripts/guard.sh >> $HOME/.local/state/svanbot/cron.log 2>&1`, with a short "Guarding a styled bot" section.
- [ ] **Step 4: Run** `T -p svanbot-runtime`, `T -p svanbot`, then `mise run full`. Expected: PASS everywhere, the tools tests included.
- [ ] **Step 5: Commit** `apps: a live guard for a styled bot (V11-<nn>)`.

### Task 11: Rollout (S8, operations)

No code. Do not start until the operator has said which two bots, and a release carrying Tasks 2 to 10 is installed and every bot runs it (`svanbot-release status`). SurSvan, the canary, keeps today's play. Phase 0's variance cut makes these checks short; without it they are slower, not wrong.

- [ ] **Step 1: Size a check from Task 7's smoke time.** Choose `<bouts>` so a check fits the box's spare hours at `nice -n 15`; the margin reading needs at least 30.
- [ ] **Step 2: Check `tight` and `loose`.** `svanbot play check solving tight <bouts> 7 20` and the same for `loose`. Expected: each prints a heads-up and a 6-max line and exits 0 (admitted). A style that exits 1 is not assigned; report its lines to the operator.
- [ ] **Step 3: Record that the rule refuses a weaker logic.** `svanbot play check pricing standard <bouts> 7 20`. Expected: exit 1, `admitted: false` in `svanbot play status`.
- [ ] **Step 4: Assign, one bot at a time, with the operator's two names.** `svanbot play assign <bot> solving tight`, wait for the bot's next hand end and confirm its log says `plays by solving/tight`; then the second bot with `loose`.
- [ ] **Step 5: Put the guard on cron** (the line in `deploy/README.md`) and run `svanbot play guard` by hand once. Expected: `NotYet` for each until 3,000 hands.
- [ ] **Step 6: When each bot has 3,000 hands under the guard without a trip,** post the numbers on the map: both check lines, the guard line, the stablemate rows from `policy-runs.json`. Phase 1 is done when the spec's list holds (design, "Phase 1 done when").
- [ ] **Kill switch.** `svanbot play revert <bot>`, or remove `plays.json`: every bot returns to the reference on its next hand.
