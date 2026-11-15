# Hand audit for gameplay leaks: 2026-11-15

Generated-by: claude-code/claude-opus-5-5

This note answers [Audit recent fleet hands for gameplay leaks](https://github.com/SvanLabs/SvanBot/issues/1022),
part of [Stronger opponents, gameplay and learning](https://github.com/SvanLabs/SvanBot/issues/1020):
where does the fleet lose the most in recent hands, by spot, and which leaks are big enough to be
worth fixing?

**Answer.** No strategic spot loses enough to stand out from noise. Over 7,363 hands under the
current policy, every negative residual is |z| ≤ 2.5 across 183 tested classes. The leaks that look
large all sit in deep pots out of position on later streets: river jams, turn checks, flop checks
and turn calls. Each one is 15 to 110 hands, and its interval ends near zero.

The one leak that is certainly real has nothing to do with strategy. In 37 hands the bot decided to
act, but the server recorded a fold. In 33 of them the connection dropped and reconnected in between.
It is small, and its cost cannot be priced from the store, but it is a defect with an unambiguous
fix target. The fleet is winning everywhere else measured: by street, by position, and against every
opponent type with a usable sample.

## Data, cutoff and method

- **Source.** A read-only copy of the newest verified backup, `svanbot10-2026111503.db`. Its
  SHA-256 is `9aad628ca0eae48eb4cf9de23c4e5cfb0ac844b21739f5ae12d8de5846da25f3`. It was opened
  `mode=ro&immutable=1` with `query_only`. No live database, venue or bot API was touched, and
  nothing was built.
- **Cutoff.** The newest hand ended at `2026-11-15T03:06:40.999074891Z`. The newest decision is
  `03:06:49Z`, the newest calibration row `03:06:41Z` and the newest analyst audit `03:06:11Z`. The
  store holds 14,534 hands from 2026-11-13T12:00Z. Every one has a net, hole cards and a hero seat,
  so all 14,534 count as played. No hand ID repeats across bots (14,534 distinct), so no stablemate
  shares a stored hand. `hand_provenance` is empty, so there are no challenger or treatment arms to
  separate out.
- **Cohorts.**
  - *Primary* (the #905/#906 method): the latest 1,000 played hands per active bot, 5,000 in all,
    from 2026-11-14T12:50:01Z to the cutoff. Cohort SHA-256 (sorted `bot\thand_id` lines) is
    `d9ed3716cff287c9…`. Of these, 4,925 carry `sv10-ev-14` decisions and 75 have no hero decision.
  - *Fleet latest 1,000*: from 2026-11-15T00:26:18Z. It is a subset of the primary cohort, not an
    independent sample.
  - *Policy window*: every hand since `sv10-ev-14` took over at 2026-11-14T07:06:28Z, 7,363 hands.
    The primary cohort is 5,000 of these 7,363, so the two are **not independent**. The window adds
    power, not confirmation.
- **Hand-level breakdown.** Each hand is classified by:
  - the street it ended on;
  - six-max position, from `button` and the seats in `hands.summary`;
  - hero's terminal action, and whether it faced a bet;
  - hero's preflop line;
  - opponent type;
  - heads-up or multiway at the end.

  A class's *contribution* is Σ net over the class ÷ all cohort hands × 100. Its interval is a
  per-hand normal 95% interval on that indicator-weighted net. Nets are in each hand's own big
  blind, which is 20 chips in every hand.
- **Opponent type.** The principal opponent is the non-hero seat with the most chips in. It gets a
  tier from its VPIP and PFR over all stored hands, deduplicated by hand:
  - tight: VPIP < 20%;
  - regular: VPIP 20–45%;
  - loose-passive: VPIP ≥ 45% with PFR < 0.35 × VPIP;
  - loose-aggressive: VPIP ≥ 45% with a higher PFR;
  - unknown: fewer than 50 hands.

  These tiers are in-sample and descriptive. Population: 50 tight, 140 regular, 37 loose-passive,
  8 loose-aggressive and 10 unknown players. No names or profiles are published.
- **Decision-level spot pricing.** The calibration table stores, per non-fold decision:
  - the candidate's predicted chips relative to folding;
  - the realized final stack minus the stack before the action
    (`crates/apps/bot/src/client/handler.rs:199-209`, `crates/libs/venue/src/tracker/mod.rs:108`);
  - the scale, pot + call, in bb.

  *Residual* = realized − predicted, summed per hand and averaged over hand clusters. All-in
  runout luck is removed by adding `ev_net − net` to realized for adjusted all-ins. All-ins whose EV
  is unpriced are **excluded**: 221 calibration rows in 58 hands in the window, never replaced with
  the raw result. Contribution to bb/100 spreads each class's residual over all cohort hands.
  Positions in or out of position are taken from street action order.
- **Packed columns.** `decisions.detail`, `replays.record` and `audit_queue.record` are
  `sv10-pack` frames, BLOBs of DEFLATE against a stored dictionary
  (`crates/libs/store/src/packed.rs:1-17`, `:84-98`). They were not decoded. Only the real columns
  were used: `street`, `action`, `amount`, `to_call`, `pot`, `latency_ms`, `opponents`, `version`
  and `candidates`. So a candidate's fold probability and its alternatives are not visible here.
  `hands.summary` is plain JSON.
- **Coverage.**
  - In the policy window, 6,523 hands have a non-fold decision. Five of them have no calibration
    row, and 9 non-fold decisions in all lack one. These are the same turns the server never acted
    on, listed below.
  - Folds are never calibrated (`spot_category` returns `None` for folds,
    `crates/libs/policy/src/policy/mod.rs:50`). **Over-folding cannot be measured by this method.**
    That covers 3,520 fold decisions in the window.

## Results by bot and fleet

All-in EV leaves unpriced all-in hands out of its denominator and shows that count. It never fills
them in with the raw result.

| Population | Hands | Raw bb/100 (95%) | All-in EV bb/100 (95%) | EV hands / unpriced |
|---|---:|---:|---:|---:|
| SurSvan | 1,000 | +166.2 (−28.3, +360.7) | +88.8 (−78.3, +255.9) | 992 / 8 |
| SuraGunnar | 1,000 | +135.1 (−65.1, +335.3) | +153.8 (−12.1, +319.6) | 992 / 8 |
| SvanBotV10 | 1,000 | +299.6 (+70.0, +529.3) | +234.0 (+34.1, +433.9) | 992 / 8 |
| Svanar | 1,000 | +132.2 (−90.9, +355.3) | +207.2 (+18.6, +395.8) | 996 / 4 |
| Svanism | 1,000 | +318.6 (+105.4, +531.7) | +222.2 (+45.5, +399.0) | 988 / 12 |
| Primary, pooled | 5,000 | +210.3 (+115.3, +305.4) | +181.2 (+100.7, +261.7) | 4,960 / 40 |
| Fleet latest 1,000 | 1,000 | +251.3 (+60.4, +442.1) | +149.5 (−8.9, +307.8) | 990 / 10 |
| Policy window | 7,363 | +217.3 (+136.6, +298.0) | +174.5 (+105.9, +243.1) | 7,305 / 58 |

The primary cohort has 195 all-in runouts, 40 of them unpriced. The policy window has 306, 58
unpriced. The intervals assume independent hands. They correct for neither session nor opponent
dependence, nor for multiple comparisons. A profitable window describes recent results; it does not
prove a lasting edge.

## Hand-level breakdown (primary cohort, 5,000 hands)

No street, position or opponent class has a significantly negative contribution.

| Class | Hands | Contribution bb/100 (95%) |
|---|---:|---:|
| SB | 853 | −25.2 (−65.5, +15.0) |
| UTG | 731 | +22.8 (−14.0, +59.6) |
| BTN | 871 | +28.2 (−11.0, +67.5) |
| CO / HJ / BB | 846 / 805 / 883 | +56.3 / +57.0 / +70.2 (all lower bounds > 0) |
| Ended preflop | 900 | +14.4 (−21.9, +50.6) |
| Ended turn | 921 | +29.0 (−18.3, +76.2) |
| Ended flop / river | 1,111 / 2,068 | +59.6 / +107.4 (both lower bounds > 0) |
| vs loose-aggressive | 419 | −13.2 (−44.8, +18.5) |
| vs unknown (< 50 hands) | 28 | −6.1 (−15.1, +2.9) |
| vs tight | 572 | +6.1 (−25.5, +37.8) |
| vs loose-passive / regular | 1,198 / 2,783 | +76.4 / +147.0 (both lower bounds > 0) |
| Preflop limp | 137 | −9.7 (−20.8, +1.4) |
| Preflop 3-bet | 292 | −7.4 (−47.5, +32.6) |
| Multiway (5) at end | 17 | −5.3 (−14.2, +3.6) |

The policy window agrees in sign and order. SB moves to −0.2 (−33.3, +32.8) over 1,258 hands,
limps to +3.7, and 3-bets to +27.5. Most of the money lost by class is structural:

| Hero folds facing a bet on… | Hands | Contribution bb/100 |
|---|---:|---:|
| the flop | 852 | −75.5 |
| the turn | 573 | −82.7 |
| the river | 371 | −81.3 |

Folding loses what is already in the pot. That says nothing about whether the folds were wrong,
and the calibration cannot price folds.

## Decision-level candidate leaks (policy window; primary cohort in brackets)

The residuals are luck-adjusted. Each row is a priced class that loses to its own price.

| # | Spot | Hands | Residual bb/decision (95%) | Contribution bb/100 (95%) | z | Primary cohort |
|---|---|---:|---:|---:|---:|---|
| 1 | River all-in (hero jams), all | 23 | −78.8 (−137.3, −20.3) | **−24.6 (−45.1, −4.1)** | −2.35 | 15 hands, −13.7 (−32.4, +5.0) |
| 1a | … out of position, pot ≥ 50 bb | 15 | −120.9 (−198.5, −43.2) | −24.6 (−44.3, −4.9) | −2.45 | n/a |
| 2 | Turn check, OOP, pot ≥ 50 bb | 89 | −18.4 (−35.9, −1.0) | **−22.3 (−43.7, −0.8)** | −2.04 | 55 hands, −14.0 (−36.4, +8.4) |
| 3 | Turn call, OOP, pot ≥ 50 bb | 102 | −18.8 (−40.6, +3.1) | −26.0 (−56.5, +4.5) | −1.67 | 69 hands, **−41.2 (−75.0, −7.4)**, z −2.39 |
| 4 | Flop check, OOP, pot 15–50 bb | 344 | −3.5 (−6.5, −0.4) | **−16.3 (−30.6, −2.0)** | −2.23 | 260 hands, **−22.6 (−40.9, −4.2)**, z −2.41 |
| 5 | Flop check, OOP, pot ≥ 50 bb | 43 | −20.4 (−39.0, −1.8) | −11.9 (−23.2, −0.6) | −2.07 | n/a |
| 6 | River small bet, OOP | 259 | −3.7 (−8.4, +1.0) | −13.0 (−29.6, +3.5) | −1.54 | 184 hands, −13.3 (−33.9, +7.3) |
| Pool | All river decisions OOP | 1,312 | −2.5 (−5.5, +0.4) | −45.3 (−97.5, +6.9) | −1.70 | 908 hands, −53.9 (−115.6, +7.9) |
| Pool | All SB postflop decisions | 975 | −4.1 (−12.1, +4.0) | −53.9 (−160.8, +52.9) | −0.99 | 673 hands, −131.7 (−265.1, +1.7) |

Rows 1 and 1a overlap, and the pooled rows contain rows 1 to 6. Contributions are not additive.

The code that decides each spot, as of this commit:

- **River jams.** A jam is the `to == max_to` raise target. It is priced with realization fixed at
  1.0 (`crates/libs/policy/src/policy/mod.rs:236`). Its EV is
  `all_fold · fold_branch + (1 − all_fold) · called_value` (`mod.rs:288-289`). The fold
  probability comes from `ResponsePricing::for_raise_to` (`crates/libs/policy/src/policy/responses.rs`)
  and the shift from `measure::all_in_fold_branch` (`crates/libs/policy/src/policy/measure.rs`).
  Predicted value averaged +163 bb per jam. The realized gain on the river (no runout luck remains)
  was about +84 bb, an overestimate of about 79 bb, so fold equity or equity when called is
  overpriced. Which one cannot be split without the packed `detail` column (`fold_prob`).
- **Checks** (spots 2, 4, 5). The value is `eq · pot · realize(street, ip, eq, weight)`
  (`mod.rs:150-151`), blended with `check_lookahead` (`mod.rs:152-156`, `:343-390`). `realize`
  applies a fixed 0.9 multiplier out of position and a street weight of 0.75 on the flop and 0.4 on
  the turn (`responses.rs:352-356`). That multiplier does not depend on pot depth. The lookahead
  assumes a single 0.66-pot bet by one bettor (`mod.rs:363`).
- **Calls** (spot 3). The value is `eq_call · (pot + call) · r − call − margin · pot`, at
  `mod.rs:175`. The depth split of the `turn:call` correction starts at 500 bb
  (`crates/libs/policy/src/policy/calibration.rs:12`, `DEEP_CALL_MIN_POT_BB` in `mod.rs`). The loss
  here is in the 50–500 bb band, which reads the pooled correction.
- **River small bets** (spot 6). These are raise targets priced like row 1, with
  `realize + initiative` (`mod.rs:236`).

**None of these clears the noise.**

- With 183 tested classes, a Bonferroni-corrected two-sided 5% test needs |z| ≥ 3.6. Even a
  20-family correction needs |z| ≥ 3.0. The largest negative |z| is 2.47.
- The two windows overlap by 5,000 hands, so the primary cohort's agreement is not a replication.
- Several of these classes are selected by later play. Turn calls and checks in deep pots are
  populations that grew big *because* the hand continued.
- The residual is measured against a chosen candidate's own price, so winner's-curse selection
  pushes it downward.

Rows 4 and 1 are the most coherent with each other. Both say the same thing about out-of-position
equity in bigger pots, and both are consistent with a depth-blind 0.9 OOP factor. They are worth an
offline paired test, not a parameter change.

The classes that do clear the noise are **underpriced**:

| Class | Hands | Contribution bb/100 (95%) | z |
|---|---:|---:|---:|
| `flop:bet:big` | 666 | +105.5 (+65.0, +146.0) | 5.1 |
| `preflop:raise:big` | 4,049 | +102.8 (+44.4, +161.2) | 3.45 |

Here realized beats price. [`docs/studies/flop-pricing.md`](../studies/flop-pricing.md) already
tested installing the flop residual as a bonus and rejected it: −63 and −75 bb/100 in paired
simulation. So a positive residual is not a licence to bet more.

## The one certain leak: decided actions the server recorded as folds

In the policy window, the bot logged a non-fold decision in 37 of 7,363 hands, and the server's
history records hero folding at that turn. That is 0.50%, with a 95% interval of 0.34–0.66%. The
primary cohort has 28 of 5,000.

| Decided → recorded | Hands |
|---|---:|
| check → fold | 12, across flop, turn and river |
| raise → fold | 15 |
| call → fold | 10 |

Between the decision and the end of the hand there was:

| Event | Hands |
|---|---:|
| `connected as …` (a reconnect) | 33 |
| a resync only | 3 |
| `not_your_turn` rejection | 1 |

The window logged 90 reconnects, 21 connection resets and 5 broken pipes.

The cost is the gap between the action that was decided and the fold that was recorded, and that
cannot be priced from the stored columns. A free check that became a fold costs at least zero by
construction. It happened in 16 hands, which netted −155.5 bb.

All 37 forfeited hands together net −2.7 bb/100 (−5.7, +0.3), or −1.9 (−3.9, +0.2) in the primary
cohort. That is descriptive and does not bound the cost.

The action is sent through `conn.send` (`crates/apps/bot/src/client/decide.rs:228-238`), and
calibration is queued only if that send returns true. That accounts for the 9 uncalibrated
non-fold decisions. Recovery after a reconnect runs through `resync_missing_state`
(`crates/apps/bot/src/client/recover.rs:9`). These hands show that an action decided before a drop
is not delivered on the new connection in time. Why it fails (venue turn timeout, or a send
accepted into a dead writer) is not established here.

## The analyst's decision-loss classes

The analyst audit (`decision_audit`) in the window has 2,456 rows. Of these:

- 66 rows disagree with the live action;
- 2,390 rows store a gap of exactly 0;
- 0 rows have both a different action and a 0 gap.

The total gap is about 62 bb, or 0.85 bb/100. It is largest on turn raises (14.3 bb, 114 rows) and
river raises (9.2 bb, 118 rows).

The audit re-solves at the recorded prices (`crates/apps/bot/src/replay.rs:207-221`), so it cannot
detect mispricing. [#1015](https://github.com/SvanLabs/SvanBot/issues/1015) item 3 says a re-solve
with no equity is stored as a 0 bb verdict (`replay.rs:219-222`). In the stored columns such a
verdict cannot be told apart from agreement, so the 0.85 bb/100 is a subtotal over an unknown
measured denominator. It does not support any leak ranking.

## Recommendations

1. **Fix action delivery across reconnects.** This is first because it is a defect, not a
   judgement call. Write a regression test that drops the connection after a decision is made and
   before the server acknowledges it.
2. **Run an offline paired test of a depth-aware out-of-position realization.** The target is spots
   1, 2 and 4: `realize` at `responses.rs:352-356` and jam fold pricing at `mod.rs:236`/`:288`. Use
   the existing promotion bar (95% lower bound ≥ +1 bb/100). The observational residuals are too
   thin to justify a change on their own.
3. **Record fold decisions' foregone value.** Today the fold half of every spot is unmeasured
   (`mod.rs:50`). If over-folding is a leak, this audit cannot see it.
4. **Rerun this audit on a later cutoff.** Rows 1 to 4 need roughly 3–4× their current hands before
   a 0.2–0.3 bb/decision effect would clear a family-corrected threshold.

## What this cannot establish

- Exact per-candidate fold probabilities and alternatives, because they sit in the packed `detail`
  column and were not decoded.
- Whether a fold was a mistake.
- Causal effects of any change.
- Per-bot leak rankings. Each bot has only 2–7 river jams.
- Independence across the two windows, or between the hands of one session.

The scripts and the full aggregate JSON are kept in the session scratch area, outside the
repository. No raw hands, opponent names or profiles are published.
