# Opponent-modelling upgrades: a survey

Generated-by: claude-code/claude-opus-5-5

Research for [#1023](https://github.com/SvanLabs/SvanBot/issues/1023), part of #1020. The question:
which opponent-modelling upgrades could plausibly beat the current model, and what does each need
from the data the bot already has?

Scope and method. This note is read-only on the code: it read `crates/libs/model`,
`crates/libs/policy` and the fit modules under `crates/apps/bot/src` at commit `98c452e`
(2026-11-15). Nothing was built, run or measured, and no store or venue was read. The external
sources are peer-reviewed papers where they exist, with preprints marked. "Untested" means that
no source measured the claim in six-max no-limit hold'em and this repository has not measured it
either.

Labels used below: **Fact** (read in code or source), **Inference** (reasoned from facts), and
**Recommendation**.

## 1. The model today

**Response vectors (per-opponent tallies).** Every opponent has a `PlayerStats` of weighted
opportunity/hit counters: VPIP, PFR, opens, limps, 3-bet, 4-bet and the folds to each, c-bet, fold
to c-bet, bet-first, fold-vs-bet and raise-vs-bet per street, fold by bet-size bucket, WTSD, river
bluff share, positional opens and VPIP, showdown wins and think times
(`crates/libs/model/src/model/mod.rs:84-139`). Each observation first decays that player's tallies
by `0.5^(1/half_life)` (`mod.rs:423-425`), with a half-life of 1,000 hands
(`crates/apps/bot/src/lib.rs:80`). The population aggregate is never decayed. `Profile` shrinks
each rate independently toward the population rate with fixed pseudo-counts: 12 preflop, 10
postflop, 5 to 8 for the rarer stats (`mod.rs:402-403`, `model/profile.rs:22-45`). The population
rate is itself shrunk toward hand-set defaults with 150 pseudo-counts (`profile.rs:7-10`).
`fold_to_bet` turns the street rate and size buckets into a fold probability along a fixed
pot-odds curve (`mod.rs:340-349`).

**Ranges.** `oprange::estimate_ranges` replays the hand and weights every combo by how likely that
profile was to take each observed action with it (`crates/libs/model/src/oprange/mod.rs:1-3`,
`:329`). The likelihood shape is `RangeParams`: soft thresholds, bluff share by size, traps and
floors, plus a think-time exponent (`oprange/mod.rs:29-104`). `calibrate` fits these parameters
by maximum likelihood on shown combos at river showdowns and keeps the fit only on a held-out gain
(`crates/libs/model/src/calibrate.rs:1-7`, `:31`).

**Sizing tells.** For each opponent, a river tell `k` tilts the value and bluff shares of their
river betting range by `(size/0.66)^-k`. It is fitted on a grid, shrunk with a 300-sample prior,
needs at least 8 river-bet showdowns per player, and is installed only on a held-out gain at 95%
(`crates/libs/model/src/sizetell.rs:1-24`, `crates/apps/bot/src/playersize.rs:1-38`).

**Response network and residuals.** A 39→48→24→3 MLP predicts fold/call/raise from 16
situational inputs and 23 profile statistics (`crates/libs/model/src/features.rs:135-180`,
`crates/apps/bot/src/neural.rs:18`). It trains on the oldest 85% of live hands, under the
chronology contract `bounded-history-before-hand-v2` (`neural.rs:31`, `:139`). In pricing, the
net's fold probability is blended 70/30 with the stat fold estimate, and its raise share 60/40
(`crates/libs/policy/src/policy/responses.rs:175`, `:226`). Each opponent's net output is
multiplied by their shrunk observed/expected ratios per class, prior 30
(`crates/libs/model/src/residual.rs:1-8`, `crates/apps/bot/src/nnresidual.rs:98-117`).

**Fold calibration and live fits.** These are logit shifts on top of the response pricing.
Per-street and preflop shifts apply to everyone (`crates/apps/bot/src/foldcal.rs:1-24`), along
with the all-in call equity shifts (`crates/apps/bot/src/livefits.rs:1-30`). There is also a
per-opponent heads-up postflop fold offset, one Newton step with prior 20, capped at ±1.5 logits
(`crates/apps/bot/src/playerfold.rs:1-21`, `:117-118`). All of these are applied in `price_one`
(`responses.rs:205-211`) and installed through `playerfits` and `livefits` only while they win on
held-out newer hands.

**Fact: the paired simulation cannot credit a better model.** The learner's simulated opponents
are `ProfileClone`s (`crates/libs/policy/src/agents.rs:206-284`). Each clone plays the same stat
`Profile` the model reads, with each rate jittered independently by its sampling error
(`agents.rs:238-263`). Its bet size is random noise, unrelated to hand strength (`agents.rs:264-265`,
`:458`), and it never changes over a run. `neural.rs:118-122` states the consequence: a paired sim
"is structurally unable to credit the net for reading *real* opponents better than statistics."
This shapes every "how to test" entry below.

### Known defects a candidate must not lean on

These defects are being fixed on other tickets.

- `fold_to_cbet` also counts folds to a raise over the c-bet. The test at
  `crates/libs/model/src/model/extract.rs:114` checks who bet first, not what the folder faced.
  The stat feeds the net as input 27 (`features.rs:164`).
- Imported histories price another seat's `all_in` at our own starting stack
  (`crates/libs/model/src/history.rs:138`, `:187`). That inflates the population tallies every
  profile shrinks toward.
- Hero's preflop raise width ignores the fitted `raise_width`. This skews the image opponents form
  of us, `hero_seen_view` (`crates/libs/model/src/model/image.rs:64`).
- The sizing-tell fit counts a shared showdown hand once per fleet seat. It loops over every fleet
  bot's stored hands without de-duplicating them (`playersize.rs:23-31`).

The code reading turned up two more issues, both inferences.

- **Future profile in the sizing fit.** `playersize::samples` profiles every historical showdown
  with the *current* stored `ModelStore` (`playersize.rs:19`, `:37`). It does not use the profile
  as it stood before that hand, which is the contract neural training enforces (`neural.rs:31`).
- **River-bluff selection bias.** `river_bluff` is tallied only when the river aggressor showed
  (`extract.rs:158-164`). Bets that took the pot are never counted, so the stat measures bluffs
  *that got called*.

## 2. What the literature says

Fact: no source found measures opponent modelling against six-max no-limit hold'em bots. The
evidence is heads-up limit, heads-up no-limit, Kuhn poker, or non-poker domains. Strength claims
transfer only as hypotheses.

- **Contrary evidence: you can win without modelling anyone.** Pluribus beat elite professionals
  at six-max NLHE with a fixed strategy that "does not adapt to the observed tendencies of the
  opponents". It won 48 mbb/game (SE 25) in the 5-humans-plus-1-AI format, and 32 (SE 15) in 1
  human plus 5 copies. The authors argue that exploitation "require[s] too many samples to be
  competitive … outside of small games"
  ([Brown & Sandholm, *Science* 2019](https://par.nsf.gov/servlets/purl/10119653)). Their
  opponents were humans who adapt; the OpenPoker pool is bots, which is the case where modelling
  has its best documented returns.
- **Bayesian modelling with informed priors.** Bayes' Bluff keeps a posterior over opponent
  strategies and responds to it. It separates game uncertainty from strategy uncertainty and
  handles the partial observability of folded hands by marginalising
  ([Southey et al., UAI 2005](https://arxiv.org/abs/1207.1411)). Ganzfried & Sun give an exact
  Bayesian best response under Dirichlet priors
  ([arXiv:1603.03491](https://arxiv.org/abs/1603.03491)). A multiplayer extension beats exact Nash
  equilibrium in three-player Kuhn poker
  ([Ganzfried, Wang & Chiswick, arXiv:2212.06027](https://arxiv.org/abs/2212.06027), preprint).
- **A learned population prior with per-opponent correction, in NLHE.** Ponsen et al. learn a
  prior over opponent behaviour and a relational regression tree that corrects it per opponent.
  Training does not need every player's hole cards
  ([AAAI 2008](https://mlanthology.org/aaai/2008/ponsen2008aaai-bayes),
  [PDF](https://lirias2repo.kuleuven.be/bitstream/handle/123456789/183259/ponsen_aaai08.pdf;sequence=1)).
  The paper is a two-page abstract and gives no strength numbers.
- **Showdown censoring.** Folds hide cards, so showdown data are missing not at random. Estimates
  built from shown hands converge to "behaviour given the hand was revealed"
  ([Guo, arXiv:2608.09954](https://arxiv.org/abs/2608.09954), 2026 preprint, not peer-reviewed;
  synthetic evaluation only).
- **Robust exploitation.** Acting on a point estimate of a model is "extremely vulnerable to model
  error". Data-biased robust responses weight exploitation by how much data backs each part of the
  model
  ([Johanson & Bowling, AISTATS 2009](https://proceedings.mlr.press/v5/johanson09a.html);
  framing quoted from Bard et al. below). Norris & Watson add expert-designed exploits on top of a
  heads-up NLHE equilibrium base, triggered by statistical anomalies over a recent window, and
  report gains against static opponents
  ([CIG 2013](https://www.cs.auckland.ac.nz/research/gameai/publications/Norris_Watson_CIG13.pdf)).
- **Implicit modelling.** Implicit modelling precomputes a portfolio of responses and estimates
  each one's utility online, using off-policy importance sampling and Exp4. A small portfolio
  "would have won" the 2011 ACPC heads-up limit exploitation event, beyond the 95% margin, with
  no response tailored to those opponents
  ([Bard, Johanson, Burch & Bowling, AAMAS 2013](https://www.ifaamas.org/Proceedings/aamas2013/docs/p255.pdf)).
- **Learned opponent representations.** Davidson et al. used a neural net on a player's history of
  a few hundred hands to predict their next action. That work steered the statistics Poki used
  ([ICAI 2000](https://webdocs.cs.ualberta.ca/~duane/publications/abstracts/2000icai.shtml)).
  ASHE evolves LSTM opponent models for heads-up NLHE
  ([Li, UT Austin PhD 2018](https://nn.cs.utexas.edu/downloads/papers/li.phd18.pdf)). Grover et
  al. learn policy embeddings from small amounts of interaction, outside poker
  ([ICML 2018](https://arxiv.org/abs/1806.06464)).
- **Evaluation.** AIVAT is an unbiased estimator that removes chance and known-strategy variance.
  It needs over 10× fewer hands in NLHE
  ([Burch et al., AAAI 2018](https://arxiv.org/abs/1612.06915)), and Pluribus's own numbers use it.
- **Open bots.** No well-documented open six-max NLHE bot with an evaluated opponent-modelling
  component was found. OpenHoldem names opponent modelling an open problem
  ([arXiv:2012.06168](https://arxiv.org/abs/2012.06168)). Open Pluribus-style CFR projects do not
  model opponents.

## 3. Candidates

Each candidate below is a hypothesis. None has a measured gain here.

### C1. Censoring-aware range, bluff and sizing fits

- **Mechanism.** Fit `RangeParams`, the per-opponent sizing tells and the bluff share on every hand
  with a river bet, not only on shown ones. The likelihood of a bet that took the pot uncontested
  is the range's mass on the line, marginalised over unseen combos. The likelihood of a shown hand
  is the combo's weight, as today. This is Bayes' Bluff's treatment of unobserved hands applied to
  the existing likelihood.
- **Evidence.** Southey 2005 marginalises unobserved hands and Ponsen 2008 learns without them.
  Guo 2026 (preprint) shows the bias exists. Gain in six-max NLHE is untested.
- **Data.** Full action lines with sizes are present (`HandSummary.history`, `mod.rs:214`,
  `ActionRecord.to`/`pot_before`, `crates/libs/engine/src/engine/mod.rs:79-104`), and so are shown
  cards (`mod.rs:218`). All of it is already stored. The samples also need the `playersize.rs:23-31`
  duplicate fix and chronological profiles in place of `playersize.rs:19`.
- **Parity risk.** Medium. The fit must use the exact `estimate_ranges` that live play uses and
  the profile as it stood before each hand. Today's sizing fit fails the second condition.
- **Paired-sim test.** Clones do not size by strength (`agents.rs:458`), so a sim cannot show the
  gain as it stands. First compare held-out log-likelihood on newer showdowns against the
  installed fit (the existing gate). Then add a clone parameter that ties bet size to hand strength
  in both directions. Run champion against challenger on identical fresh deals, with the challenger
  using the censoring-aware fit, against a pool where half the clones carry tells.

### C2. Hierarchical prior over opponent types

- **Mechanism.** Replace "shrink each rate independently toward one population mean"
  (`profile.rs:22-45`) with an empirical-Bayes mixture. Fit K opponent types on the joint vector
  of tallies. A new opponent's prior is then the posterior-weighted mixture of type means, so ten
  hands of preflop data also move their postflop rates.
- **Evidence.** Southey 2005 reports faster learning from informed priors. Ganzfried & Sun give
  exact best response under Dirichlet priors, and Ponsen 2008 uses a learned prior with correction.
  The size of the gain in six-max is untested. The gain is concentrated where `confidence =
  hands/(hands+40)` (`profile.rs:57`) is low, which is new opponents and rare stats.
- **Data.** Per-player `PlayerStats` are present (`mod.rs:357`). The fit must not lean on
  `fold_to_cbet` (`extract.rs:114`) or on the population inflated by imported all-ins
  (`history.rs:138`, `:187`) until those are fixed.
- **Parity risk.** Medium. Types fitted on data from after a hand leak the future into training
  features (the net reads profiles). Types must be refitted only on hands before the network's
  training cut, and versioned like a feature layout (`features.rs:24-35`).
- **Paired-sim test.** Use the `adapt.rs`-style replay first: predict each opponent's first 50 to
  300 hands under both priors and score held-out log-loss (`crates/libs/model/src/adapt.rs:1-10`).
  Then run a paired sim of the stat model against the mixture prior, against clones sampled from
  the fitted mixture with only 20 to 100 observed hands each. This is partly circular, so treat it
  as a no-harm check.

### C3. Change-point detection on opponent tallies

- **Mechanism.** Keep the 1,000-hand exponential decay (`mod.rs:423-425`) and add an online test
  per player, on per-hand stats, for an abrupt shift in style, such as a bot update mid-season
  (the motive given in `adapt.rs:2-3`). On detection, raise the decay temporarily so the stale
  tallies fade.
- **Evidence.** Norris & Watson's window over the last M hands addresses the same problem. The
  benefit against bots that update is untested here.
- **Data.** Per-hand per-seat tallies (`mod.rs:453`) and timestamps are present.
- **Parity risk.** Low. Detection is causal and uses the same code in replay and live play.
  Clones jitter with sampling error, so the false-alarm rate must be set on replay, not in the sim.
- **Paired-sim test.** This is the one modelling change a clone sim can credit directly. Swap a
  clone's profile midway through the run (for example from nit to LAG). Run a paired champion
  against the champion with change-point detection, on identical deals.

### C4. History-conditioned opponent embedding in the response net

- **Mechanism.** Replace or augment the 23 summary-stat inputs (`features.rs:154-176`) with an
  embedding learned from the opponent's last N decisions. The per-player residual table (3 ratios,
  `residual.rs`) is a crude version of this.
- **Evidence.** Davidson 2000, Li 2018 (ASHE, heads-up) and Grover 2018. Gain in six-max NLHE is
  untested. Cost on CPU only is unmeasured (`AGENTS.md` §1).
- **Data.** Named per-decision samples are present (`features.rs:255`), as are the stored hands.
- **Parity risk.** High. The embedding window must come strictly before the hand, both in training
  and live (`neural.rs:31`), and live inference must rebuild the same window after a restart. A
  new layout is a new feature set and goes through the full gates.
- **Paired-sim test.** A sim cannot credit it (`neural.rs:118-122`). Compare held-out log-loss on
  the newest 15% against net plus residual, then run the existing no-harm poker gate.

### C5. Implicit modelling: choose among the fleet's own parameter sets online

- **Mechanism.** Keep the explicit model, and also keep a small portfolio of `Params`, such as the
  lineage parents. At each table, estimate each member's utility from the hands played and shift
  play toward the best member with a bandit (Bard 2013's Exp4 with importance sampling).
- **Evidence.** Bard 2013, heads-up limit, beyond the 95% margin. Untested in six-max NLHE.
- **Data.** Off-policy estimates need each member's action probabilities at the recorded decision.
  Replay records hold the full decision inputs (`docs/ARCHITECTURE.md`, the "Analyst" row), but
  the policy is close to deterministic, so importance weights are degenerate. It would need
  re-solving under each member (expensive) or a minimum mixing weight.
- **Parity risk.** Medium. Selection must stay a live-only layer like the live fits, so the learner
  still evaluates the champion alone. Promotion rules must not see the mixture.
- **Paired-sim test.** A sim can credit it, because selection reacts to realised utility, not to
  the model. Run a paired sim of the champion against the portfolio selector, against a pool that
  mixes archetypes and clones, with AIVAT-style variance reduction if it is implemented.

### C6. Confidence-scaled application of per-opponent corrections

- **Mechanism.** Today each installed per-opponent offset, ratio and tell applies at full point
  value once its global held-out gate passes. Scale each one by that opponent's evidence in the
  specific spot, which is the data-biased robust idea at the level the bot actually exploits.
- **Evidence.** Johanson & Bowling 2009 and Norris & Watson 2013. Untested here.
- **Data.** The counts behind each fit are present (`playerfold.rs:29-44`, `residual.rs:13-17`).
- **Parity risk.** Low. It is a pure transform of fits that are already installed.
- **Paired-sim test.** Use clones whose true profile differs from their modelled one by a known
  amount, so their sampling error is real. Run the champion with full-strength corrections against
  confidence-scaled ones.

## 4. Recommendation

1. **C1, the censoring-aware fits.** It corrects a measurable bias in the evidence that three
   installed fits rest on (`RangeParams`, the sizing tells and `river_bluff`). It needs no new
   data, and it reuses the existing held-out gate. Do it after the `playersize` duplicate fix,
   and adopt chronological profiles at the same time.
2. **C3, change-point detection.** It is cheap, causal (low parity risk), and the only candidate a
   clone paired sim can credit without new sim machinery.

C2 is the strongest literature case, but it should wait until the population tallies stop being
inflated. C4 has the highest parity risk and the least evidence on CPU. C5 is promising, but the
importance-sampling problem with a near-deterministic policy has to be solved first.

## 5. Open questions that could change this

- How much of the OpenPoker pool are fixed-strategy bots and how many adapt? Pluribus's argument
  against exploitation assumes opponents that adapt.
- How many of each opponent's river bets end without a showdown? If almost all go to showdown, the
  C1 bias is small.
- How often do opponent bots change style mid-season? The `review opponent-adapt` output would
  bound the value of C3.
