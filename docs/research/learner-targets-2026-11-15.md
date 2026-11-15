# What the learner can learn, and what to add next

Generated-by: claude-code/claude-opus-5-5

Answers [#1024](https://github.com/SvanLabs/SvanBot/issues/1024) on the map
[#1020](https://github.com/SvanLabs/SvanBot/issues/1020). This is read-only research. It changes no
code and no gate.

> What can the learner learn today, and which new learning targets would be the most valuable
> additions inside its existing search and gate?

## Answer

- **The search moves 29 catalogued knobs plus two size-list proposals**, one knob at a time, under
  the unchanged gate. In the store copy, 13 promotions landed in about 41 hours (sv10-ev-2 to
  sv10-ev-14, 2026-11-13 14:06 to 2026-11-14 07:06 UTC). The parameters show that only four knobs
  moved: `realize_weight`, `raise_risk`, `preflop_fold_scale` and `hero_image`. There has been no
  promotion since. The last 40 experiments are all rejections, and five of them were confirmation
  near-misses.
- **Everything else the learner fits sits outside the gate.** The live fits and the per-opponent
  reads are installed on held-out log-loss or chips-saved tests. The learner's simulations never
  see them.
- **Five knobs cannot be reached at today's stacks.** Recent six-seat hands have a median hero
  stack of 255 bb, and none has an effective stack at or below 50 bb. So `preflop_jam_bb`,
  `short_open_bb` and `short_jam_pot_ratio` change almost no simulated hand, and neither do
  `ess_fallback` and `tiered_all_in_fold_pricing`.
- **Top two additions:** (1) a separate turn and river bet-size scale, because postflop sizing is
  the closest near-miss in the copy and the simulated opponents do react to postflop size. (2) A
  preflop response model for the simulated opponents that reacts to bet size, because today they
  ignore our preflop size. Until that changes, the gate grades every preflop sizing knob against
  opponents that do not care about the size.

## Scope and sources

- **Store copy:** `svanbot10-2026111503.db`, a read-only copy of the newest verified backup, opened
  with `mode=ro&immutable=1`. **Cutoff:** the newest hand ended 2026-11-15T03:06:40Z and the newest
  kv write was 2026-11-15T03:06:51Z. The copy holds 14,534 hands, starting 2026-11-13T12:00:44Z,
  across five bots and 92 tables, in season 16. No `params.slot.*` key exists, so every bot played
  the one shared champion. Nothing in the copy predates 2026-11-13, so earlier lineage history is
  out of scope.
- **Code:** worktree at `98c452e`. The fleet ran `6c92369` (`learner.status.commit`). That commit
  is an ancestor of `98c452e`, and the two do not differ in any file cited here.
- **Store keys read:**
  - search: `params.v1`, `learner.lineage`, `learner.last-promotion.v1`, `learner.experiments`
    (the 40 newest), `learner.rejection-ledger.v1`, `learner.search-funnel.v1`, `learner.pacing`
    and `learner.experiment-targets.v1`
  - fits: `fold_calibration.v1`, `player_fold.v1`, `player_size.v1`, `nn_residual.v1`,
    `range_params.v1`, `river_jam_call.v1`, `deep_call.v1`, `overbet_call.v1`, `overbet_slope.v1`
    and `calibration.v1`
  - response net and experiments: `nn.response.v1`, `nn.poker-evaluation.v1` and
    `experiment.mode.v1`
  - the `events` and `hands` tables
- **Aggregates only.** No opponent names, hands or weights appear here.

Labels used below: **Observed** means read directly from code or store. **Inferred** means
reasoned from observations. **Recommended** is a proposal.

## 1. What the learner moves today

### 1a. The search and its gate (the only path to a promotion)

Each cycle, `learner::pool::challengers` (`crates/apps/bot/src/learner/pool.rs`) proposes one-knob
steps both ways from the champion. Steps alternate between full size and half size by cycle parity.
Each step is clamped to the bounds in `crates/apps/bot/src/knobs.rs`.

Candidates then pass through four stages:

1. **Screening.** Successive halving starts at 1,578–1,600 hands per candidate in round 0.
2. **The rejection ledger** (`crates/apps/bot/src/search_ledger.rs`). It bars any transition whose
   95% upper bound is below +1 bb/100. Its scope is the champion, the evidence epoch and the stack
   fixture.
3. **Fresh-deal confirmation** (`crates/apps/bot/src/promotion.rs`). Up to 12 chunks run, with a
   futility stop from chunk 4 and an early promotion at z ≥ 3. The 95% lower bound must reach
   +1 bb/100. Up to three survivors are confirmed per cycle, each against a Bonferroni z of 1.96,
   2.2414 or 2.3940 (`confirm_z`).
4. **The scenario.** The simulated opponents are profile clones of the 16 most-observed opponents
   (`crates/libs/policy/src/agents.rs`). Games start from recorded six-seat starting stacks
   (`crates/apps/bot/src/learner/stacks.rs`). Hero plays with the live response net, and with
   `LiveFits::NONE` and unit per-opponent ratios (`crates/apps/bot/src/learner/search.rs:53-60`).

**Observed: the knob catalogue, with recent evidence.** Ranges and steps come from the code. The
evidence comes from the ledger (scope sv10-ev-14, epoch 13,500) and from the last 40 experiments
(cycles 33–36, 2026-11-14 12:13 to 2026-11-15 00:39 UTC). Each figure is mean bb/100 with a 95%
interval, at the hands shown. "Confirm" rows give the mean and the lower bound recorded by
confirmation. "No effect" means no hand's outcome changed.

| Knob | Range | Full step | Champion | Best recent evidence |
|---|---|---|---|---|
| `fold_scale` | 0.4–1.2 | ±0.1 | 1.0 | 0.9: +6.3 [−17.3, +29.9] @ 11k |
| `preflop_fold_scale` | 0.5–1.8 | ±0.1 | **1.25** (promoted) | 1.30 confirm: −0.49, not ahead @ 48k |
| `realize_weight` | 0–1.6 | ±0.15 | **0.175** (promoted) | 0.025: +11.7 [−38, +62] @ 1.6k |
| `passive_fold_bonus` | −0.25–0.25 | ±0.05 | 0 | −0.05: +5.0 [−10.5, +20.5] @ 11k |
| `initiative` | −0.06–0.12 | ±0.03 | 0.03 | 0.06 confirm: +2.26, lower −2.34 @ 202k (cannot clear) |
| `open_bb` | 2.0–4.5 | ±0.5 | 2.5 | 2.0 confirm: −0.71, not ahead @ 48k |
| `short_open_bb` | 1.8–3.5 | ±0.25 | 2.5 | ±0.02, 3 differing hands of 1.6k |
| `preflop_jam_bb` | 12–50 | ±5 | 30 | no effect |
| `three_bet_ip` | 2.5–4.5 | ±0.4 | 3.0 | 2.6: +4.0 [−8.9, +16.9] @ 1.6k |
| `three_bet_oop` | 2.5–5.0 | ±0.4 | 3.8 | 4.0 confirm: +2.30, lower −0.61 @ 192k (cannot clear) |
| `four_bet` | 2.0–3.0 | ±0.2 | 2.3 | 2.1 confirm: +3.08, lower −0.09 @ 202k (cannot clear) |
| `three_bet_call_margin` | −0.14–0.10 | ±0.02 | 0 | +0.02: +6.4 [−15, +28] @ 4.7k |
| `call_margin` | −0.10–0.08 | ±0.01 | 0 | −0.01: +6.8 [−5.6, +19.1] @ 11k |
| `raise_risk` | 0–2 | ±0.3 | **0.55** (promoted) | last promotion: 0.85 → 0.55, +5.85, lower +3.10 @ 586,500 |
| `preflop_raise_risk` | 0–2 | ±0.3 | 0 | 0.3: −7.6 @ 4.7k |
| `limper_bb` | 0–2.5 | ±0.5 | 1.0 | 1.25 confirm: +0.58, lower −1.91 @ 192k |
| `jam_pot_ratio` | 1–4 | ±0.4 | 2.2 | 1 and 10 differing hands of 1.6k; no effect in c35 |
| `short_jam_pot_ratio` | 0–10 | off, or ±1 pot | 0 (off) | no effect (c33, c35) |
| `raise_fold_bonus` | −0.10–0.20 | ±0.04 | 0.06 | 0.10: no effect; 0.02: 4 differing hands |
| `bet_size_scale` | 0.6–1.8 | ×(1 ± 0.15) | 1.0 | **0.925 confirm: +3.44, lower +0.57 @ 461k (cannot clear)** |
| `profile_response_weight` | 0–1.5 | ±0.5 | 0 | 0.5: +2.1 [−10.4, +14.5] @ 11k |
| `check_lookahead` | 0–1 | ±0.5 | 0 | 0.5: +10.9 [−18.7, +40.5] @ 1.6k |
| `temperature` | 0.002–0.05 | ×(1 ± 0.3) | 0.01 | 0.013: +6.8 [−15.6, +29.2] @ 4.7k |
| `raise_gate` | 0–1.2 | ±0.15 | 1.0 | 0.85: no effect; 1.15: 3 differing hands |
| `hero_image` | 0–1 | ±0.25 | **0.25** (promoted) | 0.5: 27 differing hands of 1.6k |
| `preflop_discount` | 0–1 | ±0.1 | 0.5 | **0.4 confirmed twice: +0.96, lower −2.13 (c34); +1.54, lower −1.50 (c36)** |
| `tiered_all_in_fold_pricing` | 0/1 | flip | 0 | no effect @ 12.6k |
| `caller_mix` | 0/1 | flip | 0 | 1: +4.5 [−9.6, +18.5] @ 11k |
| `ess_fallback` | 0–1 | ±0.25 | 0 | no effect |
| `bet_size_set` (not a knob) | 4 ↔ 7 sizes | toggle | 4 | 7: −2.5 [−45, +40] @ 1.6k |
| `bet_size_mid` (not a knob) | interior size | ±0.1 pot | 0.55 | 0.65: +9.5 [−29, +48] @ 1.6k |

**Observed: promotion and rejection history in the copy.**

- `learner.lineage` lists sv10-ev-1 to sv10-ev-14.
- `events` holds 13 "promoted strategy parameters are now live" lines, from 2026-11-13 14:06 to
  2026-11-14 07:06.
- The funnel covers 2026-11-13 13:00 to 2026-11-15 00:59 UTC. It counts 1,927 proposed, 315 no
  effect, 187 below the bar, 1,240 halved out, 7 barred by the ledger, 66 confirmation rejections
  (26 cannot clear, 40 not ahead) and 13 promoted.
- Only the last promotion's knob is stored: `raise_risk` 0.85 → 0.55.
- **Inferred:** compared with `Params::default()`, the champion differs only in `realize_weight`
  (1.0 → 0.175), `raise_risk` (1.0 → 0.55), `preflop_fold_scale` (1.0 → 1.25) and `hero_image`
  (0 → 0.25). Assuming sv10-ev-1 was the default, the 13 promotions split roughly 6–7, 2, 3–4 and
  1–2 across those four knobs. The step sizes allow more than one split, so this is not exact.
- The ledger's `confirm_rejected` set for the current scope holds three transitions:
  `four_bet` 2.3 → 2.1, `initiative` 0.03 → 0.06 and `preflop_discount` 0.5 → 0.4.

**The other stages:**

- **Gene transfer** (`crates/apps/bot/src/learner/transfer.rs`) and **tournament refresh**
  (`crates/apps/bot/src/learner/tournament.rs`) need lineages. No slot keys exist, so neither ran:
  the copy has no `learner.transfers.v1` or `learner.tournament.v1` key.
- **Experiment mode** (`crates/apps/bot/src/experiment.rs`) is in `status: champion` with an empty
  pair. The fleet's ranks are 80–99, and the mode needs #1–#4. No `experiment.verdicts.v1` exists,
  so there is no live evidence on any target.
- **Response net.** A candidate trained at 00:20:47 was approved and moved live at 00:21:33.
  Separately, its paired check `nn.poker-evaluation.v1` reads −1.96 bb/100, lower −15.7, over
  50,496 hands. The approval rule was not examined here.

### 1b. Fits the learner refreshes outside the gate (live-only, never promoted)

**Observed, from code and store.** "Gate" means the test a fit must pass before it is installed.

| Fit | Key | Range or cap | Gate | State in the copy |
|---|---|---|---|---|
| Street fold calibration | `fold_calibration.v1` | ±1.0 logit | held-out log-loss lower bound > 0, 200+ held out | flop −0.39 (n 2,091), turn −0.58 (n 1,090), river not installed (lower −0.005) |
| Preflop fold calibration | same key | ±2.0 logit | same | **−0.89** (n 7,521): predicted 0.312 everyone folds, actual 0.190 |
| River all-in call shift | `river_jam_call.v1` | ≤ 0.15 | chips saved, lower bound > 0 | not installed (n 393) |
| Deep-pot and overbet call shifts | `deep_call.v1`, `overbet_call.v1`, `overbet_slope.v1` | slope ≤ 0.2 in the fit, 0.40 in the policy | calibration or chips saved | not installed (n 11 and n 31) |
| Per-opponent fold offset | `player_fold.v1` | ±1.5 logit, prior 20 | held-out log-loss at 95%, 1,000+ | installed: 231 opponents, gain +2.24 ± 2.04 mnats (n 1,979) |
| Per-opponent sizing tell | `player_size.v1` | k in −2..2, shrunk by n/(n+300) | held-out likelihood at 95%, 300+ | installed: 77 opponents, gain +4.98 ± 2.06 mnats (n 537) |
| Per-opponent response ratio | `nn_residual.v1` | prior 30 | held-out log-loss at 95%, 1,000+ | installed: 245 opponents, gain +4.05 ± 1.08 mnats (n 14,143) |
| Range model (28 fields) | `range_params.v1` | coordinate ascent | held-out showdown log-likelihood | installed (lower bound +0.369 nats) |
| Self-calibration, `ev_bias` | `calibration.v1` | per category, bounded by margin evidence | 95% band | 3 of 29 categories nonzero: preflop call +1.50 bb, preflop big raise +1.68 bb, flop call +0.44 bb |

## 2. What limits the search's reach

These findings are observed unless marked otherwise.

1. **The stack regime makes five knobs unreachable.** Take the newest 4,096 hands and keep the
   3,546 six-seat records. Not one has an effective stack (hero against the deepest opponent) at or
   below 50 bb, and only 2.0% are at or below 100 bb. The median hero stack is 255 bb. The search
   plays these recorded stacks (`stacks.rs`), so knobs that only act on short stacks or all-ins
   never change a simulated hand. The ledger then bars them, but its scope expires every
   500-hand evidence epoch, so they are proposed again. No effect accounts for 315 of 1,927
   proposals (16%).
2. **The simulated opponents ignore our preflop size.** Facing one raise, a clone 3-bets, calls or
   folds on fixed percentile thresholds from its profile (`agents.rs:354-368`). An opener facing a
   3-bet uses `fold_to_3bet` whatever the size (`agents.rs:369-381`). Postflop, clones fold by
   `fold_to_bet(street, size)`, so they do react to size there (`agents.rs:465-476`).
   **Inferred:** `open_bb`, `three_bet_ip`, `three_bet_oop`, `four_bet` and `limper_bb` are graded
   against callers who do not react to size. Three of the five near-misses are preflop sizing or
   pricing knobs (`four_bet`, `three_bet_oop`, `preflop_discount`).
3. **The search and the live fit pull preflop folds in opposite directions.** The search promoted
   `preflop_fold_scale` up to 1.25, which assumes more preflop folds. The live preflop calibration
   then removes 0.89 logit, because real opponents fold far less than predicted. Live play is the
   sum of the two. **Inferred:** the clone pool folds preflop more readily than the real
   population. The search cannot see this, because hero plays with `LiveFits::NONE` and the clones
   fold exactly as their profiles say.
4. **The confirmation bar is stricter when several candidates are confirmed.** Cycle 36 confirmed
   three survivors, so each faced z 2.394 rather than 1.96. The near-misses `four_bet` (+3.08) and
   `bet_size_scale` (+3.44) were stopped for futility, not for a negative mean.

## 3. Candidate new targets, ranked

"Reachable" means the existing search can propose and promote it through the unchanged ADR 0002
gate (`docs/adr/0002-five-lineages-one-gate.md`), given one new `Params` field, one entry in
`knobs.rs` and a line in `pool.rs`, which is the pattern `raise_gate`, `hero_image` and
`preflop_discount` followed.

| # | Candidate | What it changes | Reachable without a gate change? | Evidence it needs | Parity risk |
|---|---|---|---|---|---|
| 1 | **Turn and river bet-size scale** (`late_bet_size_scale`, multiplying the sizes on top of `bet_size_scale`) | Lets the late streets take sizes the flop does not; today one scale moves every street | **Yes.** One continuous knob, a ±15% step, a 0.6–1.8 range | A paired search and confirmation as usual. Baseline: `bet_size_scale` 0.925 reached +3.44, lower +0.57, over 461k hands. Check that a late-only step changes enough hands at 1.6k screening | **Medium.** Clones react to postflop size (`fold_to_bet` by size bucket, the c-bet blend), but `raise_vs_bet` ignores size and the size buckets are coarse (3). Uses the sizing tells live, which the sim does not |
| 2 | **Clone preflop reaction to bet size** (fit each clone's continue rate against open and 3-bet size from live hands) | The evaluation population, not hero. Makes the preflop sizing knobs gradeable | **Yes, the gate is unchanged.** The stack-fixture digest and the ledger's `evaluation` id change, so old evidence retires. A clone change also changes the golden/sim baselines | Live continue-vs-size curves (the preflop calibration's n of 7,521 gives the scale). A held-out log-loss test against today's size-blind clone, then a re-run of the five preflop knobs | **Low for live play** (it closes a gap). It moves the search's objective, so earlier preflop promotions should be re-tested |
| 3 | **Opponent-model shrinkage and half-life** (`W_PRE` 12, `W_POST` 10, the population weight 150, `OPPONENT_HALF_LIFE_HANDS` 1,000, the fixed positional scales) | How fast hero's read of each opponent leaves the population prior. The policy reads these every decision, and nothing refits them | **No, not through the search.** The clones are built from the same profiles (`live_pool`), so a shrinkage knob would change the opponents too. Fit it on held-out log-loss, as the range model is, and install it live | Re-run the adaptation study (`crates/libs/model/src/adapt.rs`) on the current window; a 95% held-out gain over 1,000+ decisions | **High** if pushed through the search; **low** as a held-out fit |
| 4 | **Open size by position** (early and late positions, small blind) | Splits `open_bb` | **Yes** | Only meaningful after #2; today `open_bb` 2.0 is not ahead and 3.0 reads +0.55 at 11k | **High until #2 lands** (size-blind callers) |
| 5 | **Fold scale by street** (flop, turn, river) | Splits `fold_scale` the way the live calibration splits its shift | **Yes** | `fold_scale` 0.9 reads +6.3 [−17, +30] at 11k. Needs the ledger to retire on a net change (see defects) | **High.** It overlaps the live per-street shift: the search tunes against profile-exact clone folds, while live play stacks the street shift on top |
| 6 | **Read depth**: how much hero trusts per-opponent deviations from the population (generalising `profile_response_weight`) | Hero-side shrinkage only | **Yes** | `profile_response_weight` 0.5 reads +2.1 [−10, +15] at 11k | **High, structural** (inferred): clones are drawn around the profile hero reads, so the sim cannot reward reading deeper than the profile |
| 7 | **Per-opponent strategy knobs** (for example a per-opponent `call_margin`) | Play against one named opponent | **No.** One opponent is about 1/16 of the pool, so a knob that acts only on them must win about 16 bb/100 against them to move the pooled bound by +1. That is a gate change in practice | Belongs in the per-opponent reads (held-out tests, `crates/apps/bot/src/playerfits.rs`), not the search | **High** (clone fidelity for one opponent) |

**Not a new target, but cheap: suspend the short-stack knobs while the stacks are this deep.**
Gate `preflop_jam_bb`, `short_open_bb` and `short_jam_pot_ratio` on the fixture holding enough
short effective stacks. The screening budget then goes to knobs that can move a hand. That is a
scheduling change, not a gate change.

## 4. Known defects that bear on these targets

- **Shared showdown hands counted once per seat in the sizing-tell fit**
  (`crates/apps/bot/src/playersize.rs:23-31`). This affects the installed tells and anything that
  reads them, including candidate 1's live behaviour. **Observed:** in this copy no two fleet bots
  recorded the same hand. Matching on table and end second, or on table, board and pot, finds 0
  pairs, and every `hand_id` is unique. So the window's fit (n 537 held out) is not inflated. This
  is a property of this window only.
- **The rejection ledger's scope ignores the live response net.** This affects every search
  target, and most of all the fold-pricing knobs (`fold_scale`, `raise_fold_bonus`,
  `passive_fold_bonus`, `profile_response_weight`, and candidates 5 and 6), because the net prices
  opponent responses for both arms. In the copy, the net changed at 00:21:33, inside cycle 36,
  under the same ledger scope (sv10-ev-14, epoch 13,500). Whether earlier entries in that scope
  were measured against the previous net cannot be established from the copy.
- **A killed step re-runs its funnel writes.** The funnel counts above are upper bounds. The
  promotion count is cross-checked against 13 promotion events and a lineage of 14, so it is not
  inflated.

## 5. Open questions that could change the ranking

- Do the clones fold more preflop than real players because of their thresholds, or because the
  hero's fold model misprices multiway folds? A sim-vs-live fold rate per spot would decide whether
  candidate 2 should rank first.
- Was the net approved at 00:21:33 despite a negative paired mean through a non-inferiority rule?
  If nets can swap often, the ledger-scope fix comes before any fold-pricing target.
- Promotion history before 2026-11-13 is not in this copy. An older store would show whether the
  same four knobs keep absorbing the promotions.
