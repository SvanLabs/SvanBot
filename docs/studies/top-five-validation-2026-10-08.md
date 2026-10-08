# What the learning gates establish about live strength

Generated-by: codex/gpt-6

Research for [Verify what the current learning and simulation gates prove about live strength](https://github.com/SvanLabs/SvanBot/issues/957),
under [Sustain SvanBot ownership of leaderboard places 1–5](https://github.com/SvanLabs/SvanBot/issues/954).
The destination is sustained control across seasons while protecting leaders. Existing instruments
can establish input correctness, predictive improvement and simulated chip improvement in a stated
population. None establishes a guaranteed leaderboard position or a tenfold strength gain.

## Evidence boundary

Source inspected: current main **7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4**, compared with installed
baseline **9e1f228f5621ed54a7f2537224ad2a1fbf9ab2c6** using `git show` and source diffs.
[The current release brief](https://github.com/SvanLabs/SvanBot/issues/913#issuecomment-6050763232)
identifies that deployment gap. This investigation did not rebuild, release, train or independently
prove binary/source equivalence. Prior experiments below are reported evidence, not fresh reruns.

A bounded SQLite metadata read at **2026-10-08 22:46:06 UTC** opened the live store with `mode=ro`,
enabled `query_only`, set a two-second busy timeout and a four-second progress deadline, and read
named KV records only. Aggregate results appear below; raw hands, networks and opponent identities
were not exported. This is one observation, not a longitudinal process-health measurement.

The metadata inventory used `SELECT key, length(value), updated FROM kv WHERE key IN (...)`;
individual reads used `SELECT value, updated FROM kv WHERE key=?` for
`learner.search-funnel.v1`, `learner.tournament.v1`, `learner.transfers.v1`,
`nn.poker-evaluation.v1`, `nn.response.v1` and `nn.response.candidate.v1`. Network weights
were removed before retaining or reporting metadata. The funnel sums `bucket.outcomes` for
integer hours at least `floor(capture_unix_seconds / 3600) - 23`, matching the dashboard's
23 preceding hourly buckets plus the current partial hour; it is not an exact rolling 24-hour
cohort. The SQLite progress handler checked its deadline every 1,000 VM instructions.

## Three different gates

**Learned parameters.** Successive halving chooses candidates; its selected estimate cannot promote.
Up to three survivors receive new confirmation deals. The sequential gate requires a lower bound
clearing **+1 bb/100**, with candidate-count-adjusted bounds, up to 12 chunks, a shifted interim
z-score of at least 3, and futility stops. Gene-transfer offers enter the receiving lineage's own
search and must earn that gate. The stored promotion's interval fields are ordinary 95% intervals;
the verdict additionally uses the corrected bound. Do not interpret a search survivor or a displayed
positive interval alone as a promotion.
[Gate](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/promotion.rs),
[confirmation and fresh seeds](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/learner/search/stages.rs),
[gene transfer](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/learner/transfer.rs),
[promotion record](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/learner/search/conclude.rs).

**Response network.** New training on current main uses the oldest 85% of eligible live hands;
the newest 15% validate predictions. A candidate must beat the statistical baseline by more than
0.01 log-loss and satisfy supported shape and chronology requirements. Its separate complete-hand
poker check rejects zero changed outcomes or a 95% upper bound at or below zero. It permits an
uncertain negative point estimate: it is a demonstrated-harm check, **not** the learned-parameter
+1 gate or proof of profitable improvement. The simulator generates clone behavior from statistical
profiles, which limits its ability to reward a network for predicting actual opponents better.
[Neural eligibility and verdict](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/neural.rs#L54),
[training](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/neural.rs#L199),
[actual poker-check arms](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/learner/search.rs#L316).

**Live fits and per-opponent reads.** These have their own held-out objectives, not the parameter
promotion gate. Fold calibration fits the older half and requires positive newer-half log-loss gain
at its 95% lower bound. River/deep/overbet call fits test calibration and/or counterfactual savings
among observed calls; that is not a randomized test of future choices. Range fitting requires
held-out showdown likelihood gain above 0.005 nats at a hand-cluster 95% lower bound. Per-opponent
response ratios and fold offsets require positive predictive lower bounds and minimum sample counts;
river sizing tells use their own held-out likelihood check. These are useful evidence for their
specific populations, with selection and missing-outcome limits. Their likelihood improvement is
not interchangeable with a chip or rank improvement.
[Fold fitting](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/foldcal.rs#L140),
[call bands](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/raisewar.rs#L209),
[river calls](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/raisewar/river_jam.rs),
[range evidence](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/rangefit.rs),
[response residuals](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/nnresidual.rs),
[fold offsets](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/playerfold.rs),
[sizing tells](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/libs/model/src/sizetell.rs).

## Simulation population and actual exposure

Both inspected commits' paired engine seat **one hero plus five sampled opponents**. These are
six-max complete hands, despite older `bench6` comments describing `sim paired` as heads-up.
Five opponents are drawn with replacement from the supplied pool; the button rotates. The hero's
model learns between simulated hands. Fresh per-hand/seat decision streams keep arms comparable,
and the learner outcome removes measurable all-in and turn/river chance variance. This tests a
synthetic clone population, not five policy parents directly playing one another or the live venue.
[Installed paired seating](https://github.com/SvanLabs/SvanBot/blob/9e1f228f5621ed54a7f2537224ad2a1fbf9ab2c6/crates/libs/policy/src/sim/paired.rs#L101),
[current paired seating](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/libs/policy/src/sim/paired.rs#L101),
[settlement and chance correction](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/libs/policy/src/sim.rs#L111).

The learner uses at most 16 most-observed opponents with at least 30 observations, weights them by
historical hand count, and jitters profile rates. That can differ from the current tables, stakes,
new entrants and the particular competitor blocking fifth place. Its immutable stack objective
samples up to 256 valid six-seat layouts from a 4,096-row window, normalized to the simulator's
20-chip big blind. Invalid or other-size rows are excluded; no eligible rows explicitly falls back
to equal 100 bb stacks. Opponent identity and stack layout are sampled separately, so their live
joint relationship is not preserved. Search clears live fits; serialized population snapshots omit
per-opponent corrections, while the fitted range and eligible live network are loaded.
[Clone construction](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/libs/policy/src/agents.rs#L211),
[stack fixture](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/learner/stacks.rs),
[search inputs](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/learner/search.rs#L44),
[serialized model fields](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/libs/model/src/model/mod.rs#L355).

The installed `sim paired` CLI cannot load a response network; current main adds `SIM_NEURAL`,
which must be explicitly supplied. Both arms then use that network. `SIM_MODELS` chooses profile
clones; without it, the tool uses built-in archetypes. Its CLI uses equal stacks, not the learner's
recorded layouts. `bench6` is a separate frozen five-archetype, six-max diagnostic with positional
breakdowns; its hero has `nn: None` and learning disabled. It cannot exercise a neural-input repair,
and is a report rather than an exposure gate.
[Network-loader repair](https://github.com/SvanLabs/SvanBot/pull/940),
[current sim CLI](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/core/src/bin/sim.rs#L14),
[benchmark implementation](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/libs/policy/src/bench.rs#L175).

## Installed evidence and merged repairs

At the metadata observation, the live network remains **profiles-before-hand-v1**, active and
paired-approved, trained at 2026-10-08 20:59:07 UTC. Its stored losses are 0.666096 versus 0.756885,
with 1,857,500 training and 258,627 validation decisions. Those are old-contract scores, not clean
v2 validation. The latest stored neural poker check records **+13.74 bb/100**, lower 95% **+3.89**,
over **159,280** hands; its fixture records 3,856 eligible rows, 240 exclusions and 256 sampled
layouts. These KV values describe the installed learner's evidence; this session did not rerun it.

Current main's merged chronology repair requires bounded prior history, rejects unknown/overlapping
evidence, disallows old-contract warm starts and new approvals, and retains an already-approved v1
incumbent during migration. On a frozen copy, clean training passed predictive validation:
0.665357 versus 0.754367 on 246,304 held-out decisions. Complete-hand comparisons with actual
candidate/incumbent networks, profile clones and recorded layouts found **−1.15 [−12.60,+10.30]**
and **+0.96 [−9.62,+11.55] bb/100**, each over 48,000 hands. They establish neither a chip gain nor
demonstrated harm. Forced network removal was harmful on one seed, supporting incumbent retention.
The clean candidate is not established as installed by the live metadata above.
[Chronology repair and measurements](https://github.com/SvanLabs/SvanBot/pull/911),
[frozen study](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/docs/studies/neural-history-chronology.md),
[artifact identities and results](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/docs/studies/neural-history-paired.json).

The short-stack response-vector repair is merged on main, absent from the installed baseline.
Its published equal-stack runs with a stored network at 25 and 100 bb found **exactly zero change**
over 48,000 hands each: the affected over-stack response condition did not occur in that population.
A control moved outcomes. This verifies no measured effect there; it does not validate the
affected live population. The PR does not identify that loaded network as the clean v2 artifact.
Use the earlier stack-layout/network-identity study as a reproducible pattern, not as evidence for
this different code change.
[Parity repair and limits](https://github.com/SvanLabs/SvanBot/pull/941),
[remaining parity evidence](https://github.com/SvanLabs/SvanBot/issues/908).

Replay v4 is merged on main, absent from the installed baseline. It captures the previously omitted
hero image and requires complete inputs plus identical outputs for exactness. The 179 original
historical big-decision records cannot regain their unrecorded image: on repaired review they are
179 what-ifs and zero verified exact replays. Controlled recapture passed after the repair; that
does not restore historical inputs. A deep re-solve reuses recorded model prices, so a lower modeled
gap cannot independently validate those prices or prove chip improvement. New v4 evidence and
identified builds are required before grading a changed decision against original live inputs.
[Replay repair](https://github.com/SvanLabs/SvanBot/pull/912),
[replay study](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/docs/studies/replay-hero-image.md),
[capture, exactness and audit](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/replay.rs).

## Useful progress, transfers and tournament refresh

The last 24 hourly buckets observed contain **468 proposed**, **81 no-effect**, **47 below-bar**,
**331 halved-out**, **5 fresh-confirmation not-ahead**, **2 cannot-clear**, and **zero promotions**.
These are event counts with different event times, not a matched cohort whose percentages must add
to 100. A cycle number, queued candidate or fresh heartbeat alone does not demonstrate improvement.
The rollup spans lineages and records outcomes rather than all work in progress; it is not a
per-lineage throughput history. The transfer queue was empty, which does not prove transfers were
never offered or adopted.
[Funnel semantics](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/learner/funnel.rs).

The tournament KV contains one historical round, **159,280 hands per pair**, no replacements.
Its later state also retains a `skipped` message that all parents are equal; historical pair fields
persist when state is extended, so these fields must not be read as a single fresh round. The
accepted ADR allows replacing a dominated lineage with a copy of an already-promoted parent,
archiving the replaced parameters. Code judges parent policies separately against the clone pool
on paired deals, every three days when distinct parents exist. Replacement uses an ordinary 95%
lower bound **above zero**, fixed evaluation size, without the new-parameter sequential +1 gate
or a multiple-pair correction. Preserve this distinction when saying "one gate."
[Accepted lineage decision](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/docs/adr/0002-five-lineages-one-gate.md),
[tournament state, evaluation and replacement](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/learner/tournament.rs).

## Validation matrix

The following comparisons preserve existing gates; leader protection adds evidence discipline,
not a claim that every change has the same promotion rule.

| Proposed change | Instrument and population | What it establishes; blind spots | Required before/after evidence and leader constraint |
|---|---|---|---|
| New policy knob or gene transfer | Per-lineage paired six-max clones; frozen recorded stack objective; fresh sequential confirmation | Incremental simulated bb/100 against that parent; misses current table mix, model error and rank dynamics | Identify parent, code, models/network, stack digest and seeds; report changed outcomes and uncertainty; receiving lineage must clear existing +1 gate before exposure |
| Response network, feature layout or chronology | Chronological live prediction plus explicit incumbent/candidate network arms in complete hands | Predictive gain and absence of demonstrated clone-pool harm; neither alone proves live profit | Independent chronology, clean weights, sample counts, network identities, affected-stack coverage; retain approved incumbent until current-contract candidate earns existing approval |
| Response-pricing parity repair | Engine-derived vector parity plus old/new code on identical full-hand deals with an actual identified clean network | Correct inputs and effect where affected contexts occur; no-network or zero-occurrence runs do not exercise it | Include raw-call/stack-clipped cases, masks and layouts, meaningful affected-hand counts, observed asymmetric stacks, exact v4 replay when available; current equal-stack zero result cannot license a strength claim |
| Range fit, shared live fit, per-opponent read | Its chronological held-out likelihood/calibration/savings gate; complete-hand policy comparison when implementation changes | Improvement of the stated surrogate on selected observations; censored hands, repeated selection and live-fit interactions remain | Identify fit/network dependency, chronological cohort and denominator; preserve each activation rule and compare actual installed corrections; do not substitute parameter search that clears those fits |
| Tournament refresh | Parent policies compared in fixed paired clone-pool round-robin | Relative parent performance in that objective; no direct parent-vs-parent match or rank guarantee | Freeze identities and objective, retain pair uncertainty; existing lower95>0 dominance and archive rule govern copying; new transitions still use the separate +1 gate |
| Reliability or throughput | Failure regression, end-to-end before/after timing, recoverable progress and fleet opportunity counters | Restored useful work or freed compute; does not establish a poker edge | Prove real failure trigger and resumed progress, measure on same host when claiming speed; maintain legal-action and token invariants; behavior changes still need paired evidence |
| Table/stake allocation and sustained five-bot control | Time-indexed public standings plus participated hands, score deltas, stakes, outages and deployment identities | Actual exposure and descriptive results; ranks depend on competitors and volume | Predeclare season/time cohorts and all-five top-five occupancy, separate own score movement from rivals, keep missing-EV denominators honest; offline policy bb/100 is not a one-to-one rank forecast |

## Concrete blockers and next decisions

1. **Installed evidence differs from main.** The fleet still produces v1 training and old replay
   inputs. Decide and validate deployment separately; no research read silently changes live play.
   Reuse [Find and fix remaining defects, sweep 2](https://github.com/SvanLabs/SvanBot/issues/913)
   and the already-merged chronology/replay repairs.
2. **Affected parity population is not yet validated.** Keep
   [Match short-stack response vectors between training and live play](https://github.com/SvanLabs/SvanBot/issues/908)
   open for identified clean-network, asymmetric-stack and complete-input evidence. Published zeros
   cover a population in which the repair did not matter.
3. **History warm-up has no demonstrated enabling gain.** The clean-study snapshot lacked verified
   starts for every hand and selected no imported warm-up. Reuse
   [Measure: the warm-up window fix before history warm-up is enabled](https://github.com/SvanLabs/SvanBot/issues/935)
   for row diffs and paired checks; do not infer that an active pipeline means history is consumed.
4. **Surrogate and live-rank objectives need a bridge.** The profitable
   [fresh hand audit](https://github.com/SvanLabs/SvanBot/issues/906#issuecomment-6046832961)
   is descriptive, not causal. Choose prospective evidence for the relevant opponent/table/stack
   population and sustained all-five occupancy before ranking strategic proposals by expected value.
   Neither the daily funnel nor a re-solve's modeled gap establishes that bridge.

No code, thresholds or installed artifacts changed. This note resolves what the instruments can
and cannot establish; it does not select a new poker policy or promise future ranks.
