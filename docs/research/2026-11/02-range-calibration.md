# 02 — Opponent range estimation and self-calibrating fits

Research for v11.0.0. The repo was only read. Nothing was built or edited.

## What SvanBot has today (read from the code)

- **Range reconstruction** (`crates/libs/model/src/oprange/mod.rs`). Each opponent starts from a 1326-combo `Range`. The replay walks the hand's actions and multiplies the range by a hand-shaped weight vector for each action (`postflop_likelihood_with`, plus preflop analogues). These weights are logistic "soft top-p" thresholds on the combo's strength percentile, plus floors, trap terms, bluff shares and bet-size polarization. 28 shape constants (`RangeParams`) control them. Opponent stats from `Profile` set the widths. Sizing tells and timing tells tilt the result (`size_tell_factor`, `think_tell_factor`). Heads-up spots enumerate deals exactly, so 1326 vectors are already cheap.
- **Range fit** (`crates/libs/model/src/calibrate.rs`, `crates/apps/bot/src/bin/calibrate.rs`, `crates/apps/bot/src/rangefit.rs`). It runs once a day. The objective is the mean log-likelihood of the *shown* combo under the replayed range, using hands that reached the river. It needs at least 1000 samples. The search is coordinate ascent over bounded params, always starting from `DEFAULT`, with 8 passes. Hands are split chronologically, 3/4 for training and the newest 1/4 held out. The gate is a paired held-out gain, clustered by hand, with a 95% lower bound above 0.005 nats. Two diagnostics exist: `stronger_share` (a PIT/rank statistic whose mean should be 0.5) per line, and held-out log-likelihood split by bet size.
- **Opponent stats** (`model/profile.rs`, `model/mod.rs`). Each rate is shrunk toward the population rate with fixed pseudo-counts: 150 for most stats, 15 for positional ones, and 12 in `adapt.rs`. Confidence is `hands/(hands+40)`. Recency decay is optional (`half_life_hands`), and the population prior never decays.
- **Other calibrations.** `foldcal` applies a logit shift per street, gated on log-loss with a 95% lower bound. `playerfits` adds per-opponent corrections that are kept only while they beat the shared model on newer hands. Self-calibration applies an EV bias per category (`policy/calibration.rs`). `stats::margins` gives residuals by predicted-EV bin.

In short, the gate discipline is good: proper scoring, held-out data, clustered SEs. The likelihood model and the priors are the weak parts.

## State of the art, by sub-topic

**Bayesian range updating.** The standard is still Bayes on the 1326-combo belief: `r'(h) ∝ r(h)·π(a | h, s)`. Here π is a *normalized* behavioural policy, so the weights for every action available at a decision sum to 1 for each combo. That is the "range" half of a public belief state in ReBeL and Student of Games ([Schmid et al. 2023, Sci. Adv.](https://arxiv.org/abs/2112.03178); [DeepStack](https://arxiv.org/abs/1701.01724)). Opponent-strategy posteriors with Dirichlet priors go back to [Southey et al., "Bayes' Bluff"](https://arxiv.org/abs/1207.1411). A 2025 result matters for any "posterior over opponent strategies" design. [arXiv 2508.17671](https://arxiv.org/abs/2508.17671) shows that the common Bayesian-best-response update can be *inconsistent*: it need not converge to the true strategy. The paper gives a consistent MAP alternative under a Dirichlet prior, demonstrated on Kuhn poker. [Policy-based inference in card games](https://arxiv.org/abs/1905.10911) adds a warning: inference from a learned opponent model helps, but it costs exploitability when the model is wrong or the opponent is deceiving it.

**Hierarchical priors and partial pooling.** The textbook form is beta-binomial (or logit-normal) partial pooling. The population hyperparameters (mean μ and concentration κ) are estimated from the data rather than fixed, so a low-sample player shrinks by an amount the pool justifies ([PyMC partial-pooling example](https://pymc3-testing.readthedocs.io/en/rtd-docs/notebooks/hierarchical_partial_pooling.html); the [Bayesball](https://arxiv.org/abs/0802.4317) comparison of no, partial and complete pooling). Our fixed κ=150 is exactly the quantity empirical Bayes would estimate, per stat, by maximizing the beta-binomial marginal likelihood. Two extensions follow. Covariates (position, stakes, archetype cluster) can enter the prior mean. Correlated stats (VPIP, PFR, 3-bet) can share a multivariate logit-normal prior.

**Proper scoring and calibration metrics.** Log loss and Brier are strictly proper. ECE is not a proper score: it is binning-dependent and can rank models wrongly. [Ferrer & Ramos 2024](https://arxiv.org/abs/2408.02841) argue for a calibration loss taken from the decomposition of an expected proper score (refinement plus calibration) rather than ECE. [SmoothECE (Błasiok & Nakkiran, ICLR 2024)](https://arxiv.org/abs/2309.12236) gives a consistent, kernel-smoothed reliability diagram (`relplot`). For multiclass ranges, the [Silva Filho et al. survey (MLJ 2023)](https://arxiv.org/abs/2112.10327) covers one-vs-rest and top-label reliability. For range models specifically, the right reliability check is the **PIT/rank histogram** of the shown hand within the predicted strength distribution, which is what `stronger_share` computes. Its full histogram matters, not only its mean: a mean of 0.5 can hide a U shape, which means the range is too narrow.

**Online recalibration and drift.** The best-justified tools are anytime-valid. [Arnold, Henzi & Ziegel (AoAS 2023)](https://arxiv.org/abs/2109.11761) build e-values for probabilistic calibration based on rank histograms, valid under optional stopping. [PITMonitor (arXiv 2603.13156, 2026)](https://arxiv.org/abs/2603.13156) uses a mixture e-process on PIT values to detect *and locate* a calibration change. For recalibration under shift, [adaptive conformal inference](https://arxiv.org/abs/2106.00170) uses a one-line online update with long-run guarantees and no distributional assumptions. Newer variants add retrospective adjustment ([2511.04275](https://arxiv.org/abs/2511.04275)) and drift-aware, minimax-optimal regret ([2602.16537](https://arxiv.org/abs/2602.16537)). The same "online logit-shift with step size η" pattern fits `foldcal` directly.

**Censored data (no showdown).** Most hands never show cards, and how a hand ends depends on the combo, so missingness is *not at random*. The principled fix is to treat the combo as latent. Each hand contributes its marginal likelihood `Σ_h r₀(h) Π_t π(a_t | h, s_t)`, and a showdown contributes the joint term at the shown h. This can be fit by EM or by direct gradient. It requires π to be a proper action distribution. With a proper π, fold and non-showdown hands carry information (for example, the fold-to-river-bet rate constrains which combos bet). Without one, the fit sees only a selected subsample: the hands that reached showdown. A second, separate bias comes from **mucking**. If losing hands can muck at showdown, the shown combos skew strong. Showdown-win tallies are recorded, but I found no muck handling in the venue or client code, so this needs checking against the server's export.

**Fast representations.** 1326-combo vectors with exact enumeration remain the norm for live inference (DeepStack, ReBeL). For *fitting* a behavioural model, bucketing reduces the number of parameters and the variance: use 169 preflop classes preflop, and postflop use strength-percentile buckets (10–50), optionally potential-aware. The current state of the art in abstraction is EMD-based potential-aware clustering ([Ganzfried & Sandholm 2014](https://mlanthology.org/aaai/2014/ganzfried2014aaai-potential)) and history-aware [KrwEmd (2025)](https://arxiv.org/abs/2511.12089). SvanBot's percentile-indexed shape functions are already a continuous one-dimensional bucketing, which is cheap and good. What they lose is draw potential.

## Gap vs SvanBot

1. **The action weights are not a probability model.** `postflop_likelihood_with` returns unnormalized weights with additive floors. Bet, check and call weights for one combo do not sum to 1, and fold is never modelled as a likelihood. So (a) the fit can use only showdown hands, which is a censored, selected sample of about 1000 or more, refit daily; (b) the per-street rates in `Profile` and the range shapes are fitted by two separate procedures that never have to agree; and (c) bet sizes enter as a polarization knob, not as a size distribution.
2. **The priors are fixed pseudo-counts** (150, 15, 12, and 40 for confidence). Nothing estimates them, per stat, from how much players actually vary.
3. **The range shape is pool-only.** `RangeParams` is one global set. Per-opponent deviation exists only through stat rates and the sizing and timing tells. There is no hierarchical per-player shape.
4. **Recalibration happens in daily batches.** There is no sequential drift monitor. The PIT diagnostic exists, but only as its mean and only on demand (`CALIBRATE_LINES`).
5. **Selection by mucking is unverified.**
6. **Smaller items.** Every fit restarts from `DEFAULT`, with no warm start and no penalty toward the previous fit. The gate threshold (0.005 nats) is fixed. Both are fine, but a refit's variance is spent each day.

## Ranked options

**1. Empirical-Bayes prior strengths (fit κ per stat).** For each `Counter` stat, maximize the beta-binomial marginal likelihood of the players' tallies over κ (and optionally μ by position or table size) in the learner. Gate it with the existing `adapt.rs` log-loss study.
- *Cost:* low, about 1–2 days, pure `sv10-stats` plus `model/profile.rs`.
- *Risk:* low. It is a change in the decision path, so it needs goldens, but the change is monotone and well understood.
- *Gain:* medium on new or low-sample opponents, which is most of the opponents in a rotating pool.

**2. Sequential calibration monitor (e-process on the PIT and on fold log-loss).** Store the `stronger_share` PIT for every showdown and the fold residuals as they arrive. Run an Arnold/Henzi/Ziegel-style e-value or a PITMonitor mixture. On rejection, trigger the refit or roll back to defaults. Show the full PIT histogram and a SmoothECE diagram on the dashboard.
- *Cost:* low to medium, about 2–3 days, mostly in `sv10-bot`.
- *Risk:* very low. It is diagnostic and only gates fits.
- *Gain:* medium. Drift gets caught within hours rather than days, and U-shaped miscalibration, which the mean hides, becomes visible.

**3. Muck and selection audit.** Check whether openpoker's exports reveal every hand at showdown. If losers can muck, add an inverse-probability or "shown given lost" correction to `samples_from_hand`.
- *Cost:* low (an afternoon to verify).
- *Risk:* none to verify.
- *Gain:* possibly large if biased, since every fitted shape inherits the bias. Otherwise zero.

**4. A proper behavioural model with latent-combo marginal likelihood.** Replace the weight shapes with a normalized π(fold, check, call, bet-size bucket | strength bucket, context, profile). This could be a small softmax or GLM whose features are today's soft-threshold terms, so `RangeParams` survives as the parameterization. Fit it on *all* stored hands with EM or gradient, using showdowns as observed combos and every other hand marginalized over 1326 combos (bucketed for speed). The range update becomes exact Bayes, and fold, check and size behaviour come out of one model.
- *Cost:* high, about 2–4 weeks. It touches `oprange`, `calibrate`, goldens and replay versions.
- *Risk:* medium to high. It changes every opponent range in the decision path, and it must beat the incumbent on held-out showdowns under the existing gate before it is installed.
- *Gain:* high. It uses roughly 10–30 times more data, removes the censoring bias, and gives one coherent opponent model.

**5. Hierarchical per-opponent range shapes.** Allow a per-player offset on 2–4 key `RangeParams` (bluff share, `call_width`, `size_exp`). Give the offsets a Gaussian prior centred on the pool fit, with variance estimated from the pool. Install them through `playerfits`' held-out gate.
- *Cost:* medium, about 1 week.
- *Risk:* medium. It overfits regular opponents with few showdowns, but the gate contains that.
- *Gain:* medium against regulars. It is best done after option 4, which gives it far more signal per player.

The recommended order is 3, then 1 and 2 (cheap and safe), then 4 as the v11 centrepiece, then 5 on top of it.
