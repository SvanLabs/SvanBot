# Design — split flop and turn call calibration by pot depth

> **Status:** design, approved in conversation on 2026-10-18, awaiting review of this document.
> **Scope:** self-calibration's flop and turn call categories. **Not** a specification of current
> behaviour; `docs/GUIDE.md` §3.5 and `crates/apps/bot/src/tasks/calibration.rs` hold that.

This document records what was decided, the measurements behind it, and what was deliberately cut.
It lives under `docs/superpowers/`, which `scripts/docs-check.py` exempts as a historical record.

## 1. Why this exists

The operator asked on 2026-10-18 to make the bot stronger against other bots, and chose "plug
measured leaks" as the outcome. The investigation started from two leaks visible in
`review all` and `review rival`: big losses in deep-stack pots, and folding to turn and river bets
far above the minimum-defence frequency (turn 59%, river 72%, against 37%). Measuring them against
self-calibration's stored residuals changed what should be built twice. This document is the
result of that, not of the first reading.

## 2. Decisions, locked

| # | Decision | Chosen |
|---|---|---|
| 1 | What "stronger" targets | Plug measured leaks; each fix proven by the project's own gates. Not a bb/100 promise. |
| 2 | Approach | Depth-split self-calibration (over a range-model commitment term, or banking only). |
| 3 | Scope after held-out evidence | Split flop and turn calls only, at 500 bb pot + call; today's rule per class; river untouched. |
| 4 | Deployment | A separate operator decision on the finished change, as ruled in #960. |

## 3. What the data says

Measured on 2026-10-18 from `artifacts/svanbot10.db`: 824,329 calibration rows since 2026-09-14,
read-only. A residual is realized minus predicted chips relative to folding, in big blinds, on the
price before the calibration term (`client/decide.rs`: `predicted = candidate.ev - candidate.bias`).
`scale` is pot + call in big blinds.

### 3.1 The call error changes sign with pot depth on the flop and turn

Last 30 days, by `scale`:

| Pot + call | flop:call residual | turn:call residual | river:call residual |
|---|---|---|---|
| < 25 bb | +2.27 ± 0.37 | +0.77 ± 0.45 | −1.03 ± 0.40 |
| 25–60 bb | +3.55 ± 0.63 | +2.08 ± 1.16 | −3.27 ± 0.71 |
| 60–150 bb | +2.88 ± 1.81 | +2.66 ± 1.01 | −8.79 ± 1.32 |
| 150–400 bb | −7.75 ± 3.58 | −0.35 ± 2.27 | −19.27 ± 2.38 |
| 400–1,000 bb | −79 ± 32 | −66 ± 18 | −40 ± 15 |
| 1,000 bb + | −484 to −1,431 | −360 to −1,064 | −168 to −1,313 |

Flop and turn calls are **under**-priced in small pots and **over**-priced in deep ones; the sign
flips between 400 and 1,000 bb. River calls are over-priced at every depth, by a near-constant
6–9% of the pot.

### 3.2 One correction per category taxes the pots that did not earn it

Self-calibration fits one bb correction per category over every stored row
(`calibration_update`). Installed on 2026-10-18:

- `turn:call` carries **−1.37 bb** ("evidence, in full"). The deep rows pull the pooled mean
  negative, and the penalty lands on small turn pots, where calls realize *more* than predicted.
  This is the measurable part of the turn over-fold.
- `flop:call` carries **0** ("inside its 95% band"): the shallow surplus and the deep deficit cancel.
- `river:call` carries −15 bb (the downward cap) with a 9.5%-of-pot ceiling, which fits its
  proportional error in pots up to about 160 bb.

Split at 500 bb pot + call, all rows, today's rule per class:

| Class | n | Residual | Today's rule gives |
|---|---|---|---|
| `flop:call` | 40,859 | +2.23 ± 0.47 | 0 (margin evidence −0.23 ± 0.60 supports no upward step) |
| `flop:call:deep` | 194 | −260 ± 95 | −15 bb (downward cap) |
| `turn:call` | 16,295 | +1.90 ± 0.68 | 0 (margin evidence +1.97 ± 1.01 does not clear 95%) |
| `turn:call:deep` | 342 | −305 ± 66 | −15 bb (downward cap) |

So the split removes the −1.37 bb tax from small turn pots and gives deep flop and turn calls a
penalty of their own.

### 3.3 The river over-fold is not a pricing leak

River calls realize less than predicted at every depth (3.1). Folding river bets often is what the
evidence supports; the 72% fold rate against a 37% minimum-defence frequency is not, on this
measure, chips left behind. Position cannot be read from calibration rows, so the out-of-position
pattern in `review rival` (Synapse −76 bb/100, tyrey −27 bb/100 when they act last) stays an
observation, not an attributed leak.

## 4. The change

### 4.1 One rule for the key

`sv10-policy` owns the rule, next to `spot_category`:

- `DEPTH_SPLIT_CATEGORIES = ["flop:call", "turn:call"]`;
- the depth threshold is the existing `DEEP_CALL_MIN_POT_BB` (500), applied to pot + call, so
  "deep" keeps one meaning in the code base;
- `calibration_key(category, scale_bb) -> String` returns `"<category>:deep"` for a split category
  at or above the threshold, the category unchanged otherwise.

Stored rows keep their category exactly as today. The class is derived from the stored `scale`
column, so every historical row splits at once: no schema change, no data migration, and the
change is undone by turning the split off.

### 4.2 Fit side

The store learns no poker. Its three summaries (`calibration_summary`, `calibration_pot_summary`,
`calibration_margin_summary`) and `calibration_samples_since` take a depth split argument — the
categories, the threshold and the suffix — and group by
`CASE WHEN scale >= ?threshold AND category IN (...) THEN category || ?suffix ELSE category END`.
`sv10-bot` passes policy's constants through one helper, so every caller groups the same way:
the live fit (`tasks/calibration.rs`), the findings scan (`findings.rs`), the published aggregates
(`derived.rs`) and `review margins` (`review_margins.rs`). `calibration_update` itself is
unchanged; it fits whatever keys it is given.

A test runs both executions over boundary rows (499.99 and 500 bb, a null scale, `river:call`,
`flop:raise:big`) and asserts the SQL key equals `calibration_key`. The rule exists once and is
executed twice; the test is what keeps the two in step (LESSONS 32).

### 4.3 Pricing side

`Params` gains `calibration_depth_split: bool` (`#[serde(default)]`, false). The policy prices a
candidate's calibration term from `calibration_key(category, scale)` when the flag is set, and from
the category as today when it is not. The candidate's recorded `category` stays the pooled one,
so stored rows, decision details and the analyst's categories do not change shape.

The flag is live-fitted state. `update_calibration_with` sets it together with `ev_bias` and
`ev_bias_pot_cap`, and both lists that define the live-fitted set carry it: `adopt_promoted` keeps
it local, `adopt_recorded_local` takes the record's. A replay record made before the change
deserializes with the flag false and prices exactly as it was decided, so `exact_inputs` and
same-build replay keep holding.

### 4.4 What it leaves alone

Non-call categories, `river:call`, preflop, the caps (−15 bb, +3 bb), the margin bound, the
per-pot penalty ceiling (0207), the live equity fits (`deep_call.v1`, the river jam and overbet
shifts) and every learner gate.

## 5. Evidence the change must carry

- **Paired simulation (hard invariant).** `sim paired` with `SIM_A` = today's installed
  `ev_bias`, `ev_bias_pot_cap` and flag false, and `SIM_B` = the split fit's corrections with the
  flag true, at 100 bb and at 1,000 bb stacks so deep pots occur. The archetype opponents do not
  reproduce live residuals, so this is a guard that nothing breaks, not a strength measurement. A
  B−A interval that excludes zero on the negative side stops the change.
- **Golden snapshot.** Default `Params` has an empty `ev_bias` and the flag false, so the snapshot
  is expected to stay unchanged; if it moves, every changed line is explained.
- **Offline fit.** The pull request shows `calibration_update` run on the live summaries before and
  after the split: each class's correction and `bound_by`, and the turn margin rows.
- **Live read after an approved deploy.** `review margins` per class: small-pot `turn:call` margin
  residual stays at or above zero (no new over-calling), and the deep classes' residuals.

## 6. What was cut, and why

Recorded because each cut is a decision someone will otherwise re-propose.

| Cut | Reason |
|---|---|
| A per-pot penalty for deep call classes behind a held-out gate | Prototyped on all rows (older 75% fit, newest 25% test). After the 95% haircut the penalties were 0.9–4.2% of the pot, no deep class passed either held-out test, and the 7 turn calls the penalty would have folded realized +209 bb. The deep-pot loss (63 marginal turn and river calls in 1,000 bb + pots, −30,123 bb over five weeks) is concentrated in a few giant pots and cannot be told from variance at about 350 rows per class. |
| Splitting `river:call` | Its error is proportional at every depth. Split, the shallow class's bb mean falls to about −12 bb and today's rule would weaken the penalty in 150–500 bb pots, against the evidence. |
| Applying pot-relative corrections generally (0079) | Same held-out result; the "controlled fleet split" 0079 waited for was retired with the experiment pair (ADR 0002). |
| A commitment term in the range model | The principled fix for deep pots, and the next candidate if the deep classes keep missing after this runs; larger, and range changes have measured negative before (LESSONS 1). |
| Banking deep stacks | Manages deep-pot exposure rather than pricing; a live configuration change, already listed as experiment 2 in #954, and the operator's decision. |

## 7. Tests

- `calibration_key` at, below and above the threshold; non-split categories pass through.
- SQL grouping equals `calibration_key` over boundary rows (4.2).
- Pricing: with the flag set, a deep turn call takes the `turn:call:deep` correction and a shallow
  one takes `turn:call`; with the flag clear, both take `turn:call` (the pre-change behaviour).
- `adopt_promoted` keeps the flag local; `adopt_recorded_local` takes the record's.
- A record serialized without the field replays with the flag false (`exact_inputs` holds).
- An all-in and side-pot pricing test with the flag set (LESSONS 15).

## 8. Documentation

`docs/GUIDE.md` §3.5 (depth classes for flop and turn calls), the decision-path paragraph in
`docs/ARCHITECTURE.md` where it names self-calibration, and `docs/CONTEXT.md` if *depth class*
needs a glossary line. `docs/CHANGELOG.md` gets one entry.

## 9. Risks

- **The gain is small and may not be measurable.** The removed turn tax changes decisions only where
  a turn call's uncorrected EV sits between zero and about 1.4 bb; the deep penalty folds only deep
  calls priced under 15 bb, under 3% of the pot. No bb/100 gain is claimed.
- **Small-pot turn calls become more attractive.** The evidence for it is measured where we already
  call (the residual is positive and the margin residual is +1.97 ± 1.01), which is the same
  extrapolation the +3 bb cap exists for. Nothing here raises a correction; it removes a penalty
  the class's own rows do not support.
- **Findings and published aggregates gain two categories.** `flop:call:deep` and `turn:call:deep`
  will likely be listed as mispriced (n ≥ 200, residual beyond 2 bb), which is true.

## 10. Success criteria

- The fit shows `turn:call` without the −1.37 bb penalty and the two deep classes with their own
  corrections; `river:call` is unchanged.
- The paired simulation, the golden snapshot and `scripts/check.sh full` pass.
- Every pre-change replay record still replays exact.
- After an approved deploy, `review margins` reports the classes separately, and the small-pot
  `turn:call` margin residual is not negative at 95%.
