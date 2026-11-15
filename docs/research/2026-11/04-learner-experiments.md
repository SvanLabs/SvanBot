# 04 — Learner, champion gating and live experiments

Generated-by: claude-code/claude-opus-5-5

Read-only research for v11.0.0. Repo facts: SPEC-learner, ADR 0002, GLOSSARY, LESSONS, `experiment{,/*}.rs`, the #1024 worktree note, issues #1021/#1027.

## 1. What we have

- **Offline gate (works).** One-knob challengers, successive halving on paired deals, then a fresh-deal
  Haybittle–Peto confirmation (12 looks, futility from look 4, early stop at z ≥ 3, 95% LB ≥ +1 bb/100,
  Bonferroni for top-k). Variance reduction: all-in EV (`expected_net(600)`) plus a zero-mean turn/river
  chance correction, which narrows intervals by 14–30%. 13 promotions in 41 h, then none; 4 knobs moved.
- **League (built, idle).** ADR 0002 lineages, gene transfer and tournament refresh exist, but no
  `params.slot.*` key is set, so none of them has ever run.
- **Live experiments (built, never run).** `experiment.rs` alternates two "pair" bots in 120-hand blocks
  between champion and challenger. It sets a safety stop at 2,000 hands per arm with UB < 0, and a verdict at
  10,000 hands per arm. A `live-supported` verdict only reorders the learner's confirmation queue.

### Why no bot actually uses the experiment system

Four structural blockers; any one suffices:

1. **Rank precondition.** The mode turns on only after three readings showing the fleet holding #1–#4 of
   the season leaderboard (`mode.rs`, `TOP = 4`). The fleet ranks 80–99, and `experiment.mode.v1` reads
   `status: champion`, pair empty. A tool for climbing needs the bots already at the top.
2. **Lineages disable it.** `latch()` plays ordinary whenever any bot has `slot_params` (ADR 0002): switching the league on switches this off.
3. **The target dies before its sample arrives.** Target ids are `champion@evidence-epoch:transition`.
   The epoch is 500 fleet hands (~1.3 h at ~375 hands/h); the pair gets ~75 hands/h per arm, so a target sees ~100 hands per arm before retiring. Safety needs 2,000, the verdict 10,000 (~130 h). Promotions retire it too (`select()`).
4. **No statistical power, even if it ran.** Unpaired, on `ev_net` (all-in luck only). At a per-hand SD of ~10 bb, 10k hands per arm gives SE ≈ 14 bb/100 on the difference, against +1..+5 bb/100 targets: "inconclusive" by construction.

Keep: the per-hand latch, `HandTag` provenance, treatment hands out of production fits, "live never promotes".

## 2. State of the art

**League / population training and champion-challenger.** PBT exploits and explores hyperparameters across
a population ([Jaderberg et al. 2017](https://arxiv.org/abs/1711.09846)). AlphaStar adds *main exploiters*,
which train only to beat the current mains, and *league exploiters*, which look for weaknesses across the
whole league. Exploiters reset periodically; mains use prioritized fictitious self-play ([DeepMind 2019](https://deepmind.google/discover/blog/alphastar-grandmaster-level-in-starcraft-ii-using-multi-agent-reinforcement-learning/)).
SvanBot's tournament refresh corresponds to PBT's exploit step. Missing: exploiter pressure, archive regression, and any population but one clone pool (which #1021/#1023 show cannot credit better reading).

**Variance reduction for poker.** AIVAT adds zero-mean control-variate corrections at chance nodes *and at
the decisions of any player whose strategy is known*. Any heuristic value estimate keeps it unbiased. In a man-machine NLHE match it cut SD by 85%, roughly 44× fewer games
([Burch et al., AAAI 2018](https://arxiv.org/abs/1612.06915)). The 2026 work reports variance cut by a median
of 54× on 71k paired heads-up hands, and pairs AIVAT with confidence sequences for anytime stopping, needing
74× fewer hands than raw outcomes at ±1 bb ([Li, Chen, Huang 2026, AV-AIVAT](https://arxiv.org/abs/2608.06362)). Exact finite-sample certificates need a payoff bound that limits the gain to ~1.4×.  [PokerSkill 2026](https://arxiv.org/abs/2605.30094) reports ~30×. For treatment-independent covariates, CUPED-style adjustment applies ([Lin & Crespo 2024](https://arxiv.org/abs/2410.09027)). Live play cannot duplicate deals, so it needs AIVAT plus covariates.

**Key fact for SvanBot:** our own strategy is *known* in live play. It is a softmax over logged candidate
EVs at temperature `params.temperature·pot`, truncated at 4·temp (`policy/mod.rs:304`). So the AIVAT term for our decisions, subtracting `EV(chosen) − Σ π(a)·EV(a)`, is computable exactly from the stored `candidates`, and it stays zero-mean however wrong the EVs are. The chance correction already exists in the sim and
can be reused.

**Sequential testing.** mSPRT gives always-valid p-values that let you monitor every hand and stop at any
time ([Johari, Pekelis, Walsh 2015/2022](https://arxiv.org/abs/1512.04922)). E-processes generalize this; e-values multiply
across experiments ([Ramdas et al. 2023](https://arxiv.org/abs/2210.01948)). Anytime-valid t-tests and regression with
covariates have been deployed at Netflix ([Lindon et al. 2024](https://arxiv.org/abs/2210.08589)).
Combination-test adaptive designs are equivalent to e-value tests
([Brannath & Fischer 2026](https://arxiv.org/abs/2606.00878)). A confidence sequence lets the sliced learner look every step for free.

**Bandits for live A/B.** With so few hands the live aim is *catching harm*, not best-arm identification.
Conservative exploration keeps performance above a baseline ([arXiv 2312.15458](https://arxiv.org/abs/2312.15458)). SPIBB
falls back to the baseline wherever data is thin ([Laroche et al.](https://arxiv.org/abs/1712.06924)). Top-two Thompson sampling ([Russo 2016](https://arxiv.org/abs/1602.08448)) helps only once effects are detectable.

**Off-policy evaluation from logged hands.** Doubly robust and weighted importance sampling estimators
work on logged trajectories with known propensities ([Dudík et al.](https://arxiv.org/abs/1103.4601)). Weak overlap blows up variance ([arXiv 2402.08201](https://arxiv.org/abs/2402.08201)). SvanBot
logs propensities implicitly, as softmax over stored EVs, and `replay` can re-solve a recorded decision
under challenger parameters. Per-decision WIS/DR is feasible for small moves inside the softmax support.
LESSONS 40 caveat: DR's Q-hat is our own EV model and inherits its mispricing.

**Safe rollout.** Canary, then fleet, auto-rollback when a harm e-process crosses 1/α.

## 3. What live evidence can do here (numbers)

To detect δ at 80% power, a two-arm test needs n ≈ 15.7·σ²/δ² hands per arm. One bot plays about
1,800 hands/day.

| Per-hand σ after correction | δ = 3 bb/100 | δ = 10 bb/100 |
|---|---|---|
| 10 bb (today's `ev_net`) | 1.74M per arm (~5 yr) | 157k (~6 months) |
| 3 bb (~11× variance cut, plausible AIVAT on own actions + chance) | 157k (~6 months) | 14k (~2 weeks) |

**Conclusion.** Live A/B on one bot **cannot replace the +1 bb/100 sim gate**. It *can* do three things:

- (a) catch harm of 10 bb/100 or more within days;
- (b) test changes the sim structurally cannot credit, such as the response net, per-opponent fits,
  `profile_response_weight` and read depth, where plausible effects are larger;
- (c) pooled over many probes, measure the **sim-to-live slope**. Regress live Δ on sim Δ to learn whether
  the clone-pool gate predicts real chips. That is #1021's question, answered continuously.

## 4. Gap vs SvanBot

| Area | SvanBot today | Gap |
|---|---|---|
| Live experiment | Needs leaderboard #1–#4, off with lineages, target dies in ~1.3 h | **Never runs** |
| Live variance reduction | `ev_net` (all-in EV) only | 10×+ variance left; own-action AIVAT is free from logged candidates |
| Sim variance reduction | Duplicate + all-in EV + turn/river chance (14–30%) | Every sim player's strategy is known, so full AIVAT applies and is unused |
| Sequential test | Haybittle–Peto, 12 fixed looks | No per-step looks, no e-value pooling across cycles |
| League | Lineages idle; one clone pool | No exploiter, no archive regression, no field-like pool (#1021) |
| OPE | Replay re-solves, no estimator | Unused screen over 14k+ live hands |
| Rollout | A promotion hits all five bots at once | No canary stage, no post-promotion live guard |
| Sim validity | Assumed (live preflop fold gap −0.89 logit) | No sim-to-live calibration |

## 5. Design sketch: an experiments system one live bot actually runs

**Name: Probe.** It is one canary bot, configured by the operator (`EXPERIMENT_BOT`), never chosen by rank.
It works with or without lineages: the control is that bot's own parent (its lineage, or the champion).

1. **Hypothesis record** (`experiment.probe.v1`): `{control_digest, treatment_digest, kind, sim_estimate,
   harm_margin, max_hands, started_at}`. The id is the *params digest pair*, not epoch-scoped, so it
   survives refits. A promotion that leaves the probed knob untouched rebases the control. Hands already
   collected are kept as a stratum labelled with the old control. Any other promotion ends the probe as
   `superseded`.
2. **Sources of treatments**, in priority order:
   - (i) a sim-blind change: a new response net, per-opponent fits on vs off, read depth;
   - (ii) the latest promotion, run as a *post-promotion guard* on the canary *before* fleet-wide adoption;
   - (iii) the top undecided ledger transition.

   The learner writes the queue; one probe at a time.
3. **Assignment.** Randomize per *orbit* (6 hands, so positions balance). The arm comes from a seeded hash
   of `(probe id, table, orbit index)` with p = 0.5, and the probability is logged. Fixed 120-hand alternation aliases with table turnover. The arm is latched per hand
   with the existing `latch`/`HandTag` code. Treatment hands stay out of production fits, as today.
4. **Outcome per hand** (`probe::corrected_value`): `ev_net`, then the sim's turn/river chance correction,
   then Σ over our decisions of `EV(chosen) − Σπ·EV`, using logged candidates and the logged temperature.
   Then a CUPED adjustment on deal-only covariates: hero's preflop all-in equity vs the field, position, and
   effective stack. The orbit is the unit for the SE, which handles carryover such as `hero_image`. Test the zero-mean property (LESSONS 28).
5. **Test.** A two-sided asymptotic confidence sequence on Δ (normal-mixture mSPRT, mixing variance tuned
   to 10 bb/100), updated each hand:
   - **Harm stop:** the CS upper bound falls below −`harm_margin`. `live-harmful`; for a promotion guard, **auto-rollback** and fleet adoption blocked.
   - **Sim disagreement:** the CS excludes `sim_estimate`. The probe is recorded `sim-live-gap`, and that
     result feeds #1021.
   - **Support:** the CS lower bound rises above 0. Recorded `live-supported`.
   - **Budget:** `max_hands` per arm (default 20k, about 3 weeks) runs out. Recorded `inconclusive`.
6. **Staged rollout of promotions:**
   - The sim gate promotes, and the canary adopts at once.
   - A guard probe runs on the canary: harm check only, 3k hands (~2 days; catches only gross harm, ~20 bb/100).
   - If there is no harm, all five bots adopt. A bad promotion hurts one bot, not five.
7. **Learner coupling** (the gate is unchanged):
   - Verdicts are priors only. `live-supported` jumps the confirmation queue, as today. `live-harmful`
     bars the transition for the champion's lifetime.
   - A `calibration` record pools every finished probe's (sim Δ, live Δ, SE) into a sim-to-live slope with
     a CI, shown on the dashboard. A slope CI excluding 1 is the signal to change the clone pool, not the bar.
8. **OPE pre-screen** (offline, in the learner): weighted per-decision importance sampling on the last N
   canary hands. Re-solve logged decisions under treatment params, keep those inside the 4·temp support, report effective sample size. A probe starts only if the OPE estimate is not
   clearly harmful. It never promotes.
9. **Sim-side upgrade in the same release:** apply full AIVAT inside `paired_eval`. Every clone's strategy
   is known there, so the correction can cover clone decisions as well as hero's. Adopt only on a reproduced variance ratio. A 2–5× cut would roughly
   double to quintuple confirmation throughput. It targets near-misses like `bet_size_scale` (+3.44, LB +0.57).
10. **League additions** (after lineages are switched on):
    - one *exploiter* lineage, gated on beating the champion rather than on +1 bb/100 itself, whose finds
      are offered back as gene-transfer candidates;
    - a regression check that a promoted parent is not worse than the last 5 archived champions at the 95% UB;
    - the #1021 field-like sparring pool as a second evaluation population.

**Not in scope:** live-only promotion, multi-arm bandits (no power), changing the +1 bb/100 bar.
