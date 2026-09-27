# SPEC — learner

How the autonomous learner (`crates/apps/bot/src/bin/learner.rs`, its steps in `crates/apps/bot/src/learner/`) improves the live strategy, and the gates
that keep a change out of live play until it is shown to win. Source of truth is the code; update this
file with any change to the loop, its budgets or its gates.

## Invariants

- Search only nominates. A parameter set goes live only when untouched **fresh deals** (different
  seeds) put its 95% lower bound at or above **+1 bb/100** against the same frozen clone pool. Interim
  promotion also requires z >= 3; search evidence can never override a failing fresh result.
- The neural response model is used live only while its no-future-profile held-out log-loss beats the
  stat model's by more than 0.01 nats and that exact artifact passes fresh-deal paired poker
  evaluation (`sv10_bot::neural::poker_verdict`: the clone pool cannot credit a net for reading real
  opponents, so the poker gate rejects demonstrated harm — a 95% upper bound at or below zero, or a
  net that never changed a decision — and approves otherwise, 0136);
  the fitted range model only while its held-out showdown log-likelihood beats the defaults by more than 0.005.
- Evaluations are paired: champion and challenger play identical deals, seats and opponents, and
  all-in runout luck and turn/river card luck are removed, so the difference measures the parameters and little else.
- The learner never blocks live play: it runs under `nice -n 15` (`scripts/start.sh`) on `tuning.learner_threads` (every
  logical core: 8; the scheduler gives live play priority) and takes a new release only between steps.
- Nothing runs longer than two minutes (0334): an evidence refresh or a champion search is a stored
  run advanced in steps of about 90 s of work (see Steps); the sliced search measures and decides
  exactly what the one-piece search did.
- Evidence refreshes and champion searches are separately paced (`sv10_bot::pacing`), so unchanged
  evidence is not repeatedly refitted while early-season search uses the available CPU.

## Steps (0334)

Nothing on the live system runs longer than two minutes. The learner keeps its job in
`learner.run.v1` (`sv10_bot::learner::run::Run`) and takes one step at a time; between steps it
checks for a release and a compute-profile change, so a hot swap waits at most one step and the
next process resumes the stored run.

- **Refresh**: three steps (daily range model ~100 s when due, live fits ~12 s, per-opponent fits
  ~27 s plus the population snapshot); quick ones share a step.
- **Search**: `begin` trains or reuses the response model (~30 s) and picks the first stage; each
  step then plays slices of the net's poker gate, a halving round or a confirmation chunk until its
  ~90 s are planned full (`STEP_TARGET_SECS`), sized by the measured table runs per second.
- **Slices are exact**: a slice is a range of tables (`sim::paired_sums_arms`); table `t` has the
  same seating and deals as in a whole evaluation, and `PairedSums` pools slices to the whole
  result. Rounds are judged and chunks looked at only once complete, so the Haybittle–Peto looks,
  seeds and verdicts are unchanged (tested: 41 one-slice steps decide as one step does).
- **A slice is planned when it is played, and capped (0343)**: the rate a slice is sized from was
  measured *before* it, and load that arrives in between is invisible to a plan made earlier. So a
  slice plans `rate.slice_runs(left, cap)` at the moment it starts, capped at `SLICE_TARGET_SECS`
  (40 s) — a rate that has fallen by half costs 80 s, inside the 120 s budget, where the same fall
  against the original one-shot 90 s plan produced the 161 s step that opened 0343. The first slice
  of a *process* is capped at `RESUME_SLICE_SECS` (10 s) instead, because a rate persisted by an
  earlier process may have been measured under load that no longer holds; the cap widens once a
  slice has been played and the rate observed again. Capping is free: it changes only how the work is
  cut, never which work, so the decision is identical (tested: a round covers every (candidate, table)
  exactly once under budgets `[72, 40, 32, 10, 3, 1]`, and a capped search decides what an uncapped
  one did, entry for entry).
- **Guards**: each step re-reads the champion and the population snapshot and abandons the run
  if either digest changed. After the net's gate the step ends, so the rest of the search prices
  with the net now live. Each step logs its time; one over 120 s is a warning (`jobs::over_budget`).

## One cycle

1. **Release check**: if `release.sh` installed a new binary, exit; the supervisor restarts on it.
   **Two-job pacing** (`learner.pacing`, checked every 10 s): the dashboard hand limit controls
   evidence refreshes at every point in the season. A refresh runs range calibration, live fits and
   per-opponent fits, snapshots the opponent models used to build population clones, then advances
   only its own hand watermark. Champion search has a separate watermark and schedule:
   - During the first 72 hours after `season.current.v1.started_at`, search runs whenever the
     dashboard cooldown ends, without waiting for more hands. The pacing study found that opponent
     rates move only about 0.2 percentage points per 500 hands and candidate-rank changes are smaller
     than seed noise, so early searches gain from additional independent samples rather than refitting
     nearly identical populations.
   - After 72 hours, search runs at the dashboard hand limit. It therefore never waits for more hands
     than an evidence refresh. Missing season state uses this conservative late-season policy.
   - A promotion requests one follow-up search after cooldown. The 6-hour maximum idle rule and an
     operator "Start next search early" request also start search; the operator request bypasses cooldown.
   - When both jobs are due, evidence refresh runs first and search follows on the next scheduler poll
     against the refreshed state.
   - While the learner is idle, the cheap live fits (fold calibration, deep-pot all-in calls) are
     refit hourly anyway, so a quiet table never leaves live play on stale calibration.
   `LEARNER_MIN_NEW_HANDS` defaults to 500 and the dashboard's `min_new_hands` setting (0–20,000)
   overrides it. `LEARNER_COOLDOWN_MINUTES` defaults to 60 and the dashboard's cooldown setting
   (0–1,440) overrides it. `LEARNER_MAX_BACKOFF` remains available but defaults to 1 (none).
   The dashboard reports the next job, accumulated and remaining hands, season day and scheduling
   reason. These settings change scheduling only; the fresh-deal +1 bb/100 promotion gate is unchanged.
   **Deep-pot all-in calls** (2026-09-23, `sv10_bot::raisewar::fit_deep_call`, key `deep_call.v1`): calls of an
   all-in in pots of at least 500 bb over-estimate equity while smaller pots are calibrated. Each cycle
   fits the mean over-estimate on the older half. The shift (`Params::deep_call_shift`,
   local, never promoted) installs only when the newer half confirms it at 95%, either on calibration
   (the over-estimate's lower bound above half the shift) or on chips saved. The river fit gained the same
   calibration test. Where both apply, a call uses the larger shift (`policy::call_equity`). The deep-pot and
   overbet (`fit_overbet_call`, key `overbet_call.v1`, bets of 1.5x pot or more) bands share one gate: when the
   full shift fails both tests but the held-out over-estimate is still positive at 95%, the band installs that
   lower bound instead of nothing (0208).
2. **Range re-fit** (at most daily): run `calibrate 20000`, a coordinate-ascent maximum-likelihood fit
   of `RangeParams` on the older 75% of showdown samples (min 1,000), validated on the newest 25%.
   `range_params.v1` records the fit and whether it is active.
   **Live fits** (`sv10_bot::livefits`, 0205): the fold calibration and the three all-in call shifts
   below go through one module. The learner calls `refit_all` each cycle and `refit_stale` at start and
   hourly while it waits (call fits always, fold calibration when missing or over an hour old). The fleet
   installs `LiveFits::load` at startup and whenever the 30 s watcher sees a stored fit change; the
   learner's champion and challengers play with `LiveFits::NONE`.
   **Per-opponent fits** (`sv10_bot::playerfits`, 0218): both per-opponent corrections below go through one
   module like the live fits: the learner calls `playerfits::refit` after the live fits (at start, every cycle,
   hourly while waiting, after a response-network approval); the fleet installs `PlayerFits::load` at startup
   and from the 30 s watcher (an unreadable store keeps the ones in play), and workers keep them across model
   reloads. A new per-opponent correction is one `PlayerFits` field plus its fit module.
   **Per-opponent fold calibration** (0214, `sv10_bot::playerfold`, key `player_fold.v1`): with the fold
   calibration (every cycle and hourly), each opponent who answered our heads-up postflop bets gets a logit
   offset on the street-calibrated fold estimate, `Σ(folded − p) / (Σ p(1 − p) + 20)` (one Newton step with a
   prior, capped at ±1.5). Installed only while offsets learned online from each opponent's earlier bets lower
   log-loss on the newer half at 95% (1,000+ bets). The fleet puts them in `ModelStore::fold_offsets`;
   `ResponsePricing` applies `Profile::fold_logit_offset` after the street shift
   for a single responder postflop only. `review player-fold` reruns the study.
   **Per-opponent river sizing tells** (0223, `sv10_core::sizetell` + `sv10_bot::playersize`, key
   `player_size.v1`): each opponent with 8+ river-bet showdowns gets a tell `k`, the grid value in −2..2 that
   maximises the likelihood of the hands they showed, shrunk by `n / (n + 300)`. The range model tilts that
   player's river betting range by `(size / 0.66)^−k` (value width and bluff share together, bounded ×¼..×4;
   exactly 1 at `k = 0`). Installed only while tells learned on the older 70% of river-bet showdowns raise the
   held-out likelihood of the newest 30% at 95% (300+ samples). `review sizing-tells` is the heterogeneity
   study, `review sizing-fit` reruns the fit.
   **Per-opponent response correction** (0210, `sv10_bot::nnresidual`, key `nn_residual.v1`): at learner start,
   hourly while waiting and after every response-network approval, live hands are replayed as training
   builds them (profiles known before each hand, warmed by past-season hands) and each opponent's
   observed/expected ratio per response class (fold, call, raise) facing a bet is kept against the live
   network, shrunk with 30 pseudo-observations (`residual::ResidualTable`). The ratios are installed only
   while the correction lowers held-out log-loss on the newest 15% of hands facing a bet at 95% (1,000+
   decisions) and only for the network they were fitted against (`net_trained_at`). The fleet puts them
   in `ModelStore::response_ratios` (never checkpointed; workers keep them across reloads), each
   `Profile::response_ratio` carries one, and `ResponsePricing` scales the network's prediction by it.
   The learner's simulations use unit ratios. `review nn-residual` reruns the study.
   **Fold calibration** (every cycle, and at learner start when the stored fit is missing or over an hour old; ~2 s, `sv10_bot::foldcal`): our heads-up postflop bets into no bet,
   the chosen candidate's fold estimate (un-shifted by the `fold_shift` recorded on the decision) against
   whether the opponent folded next. Per street, a logit shift is fitted on the older half and installed
   only if it improves log-loss on the newer half with a positive 95% lower bound; the installed shift is
   refitted on all samples (cap ±1.0). Stored in `fold_calibration.v1`; the fleet applies it as
   `Params::fold_logit_shift`, which is local and never promoted (the learner and golden run with 0).
   A street whose evidence misses the bound is left unshifted rather than partially applied (0156).
   **River all-in call shift** (every cycle, and at learner start when none is stored; `sv10_bot::raisewar`):
   our heads-up river calls of an all-in, estimated equity against exact equity versus the shown hand.
   The older half gives the mean over-estimate. It is installed only when folding the newer-half calls
   it flips would have saved chips with a positive 95% lower bound. The live policy subtracts it only
   from river call equity against an all-in; the recorded estimate stays raw (0159).
3. **Neural response model** (`sv10_bot::neural::train_response_model`): every opponent decision in
   our live hands is a sample (38 features since 0135, 3 classes: fold, call/check, bet/raise). Server-export
   hands from `history.db` (newest 60,000) add training samples only. Validation is the newest 15% of
   live hands. MLP 38-48-24-3, 10 epochs Adam, seed `11 + cycle` (a stored net warm-starts only at the exact shape). Stored to `nn.response.v1` with its
   predictive `active` flag and `profiles-before-hand-v1` contract. Profiles are rebuilt sequentially
   and each hand is extracted before it is observed, so validation cannot contribute to its own
   features. A model trained less than 30 minutes ago (a follow-up cycle right after a promotion) is
   reused instead of retrained. The fleet exposes only an artifact that also records paired-poker
   approval, which the learner grants in the same cycle: once the clone pool is fitted, the champion
   plays `learner_tables × 4` tables twice on identical deals, once with the incumbent exposure and
   once with the candidate net, and `poker_verdict` reads the paired result. While an approved net is
   live, a fresh candidate waits in `nn.response.candidate.v1` (never read by the fleet) and is
   compared against that live net; it moves to `nn.response.v1` only on approval, so the fleet never
   plays without a response model during the gate or after a rejection (0142).
4. **Opponent pool**: the 16 most-observed opponents with 30+ hands (weighted by hand count). With
   fewer than 4 the cycle waits 5 minutes.
5. **Profile clones** (`agents::live_pool`, 0099): per opponent, a `ProfileClone` plays the player's
   full shrunk profile. Preflop: positional opens, limps, 3-bet/call, fold-to-3-bet/4-bet ranked within
   its own opening range, 4-bet responses. Postflop: c-bet and bet-first per street with the river bluff share,
   size-aware folds blended with fold-to-c-bet, raise-vs-bet, all ranked within its own range on the
   board. Every rate is redrawn each cycle (seed `7000 + cycle`), logit-normal with the sampling error
   of that player's counts, with random open (2.25–3.5 bb) and bet (40–95% pot) sizes. The policy
   still models the clone by the real player's profile, so it is never an oracle. The archetype
   `fit_clone` (4 rates only) is retired from the learner; `sim` keeps archetypes by default.
6. **Challengers** (`challengers`): one-knob perturbations of the champion (fold scale, initiative,
   open size, 3-bet sizes, raise-fold bonus, realization weight, call margin, jam ratio, raise risk,
   4-bet size, limper size, preflop fold scale, passive fold bonus, 3-bet call margin, preflop re-raise
   weight (0100), profile response weight (0101), check lookahead (0103), bet-size scale). The last three
   ship at 0: live-pool A/Bs measured no gain at the defaults tried, so only the learner can turn them on.
   Steps alternate full and half size by cycle parity; every knob is clamped to a bounded range.
   A bound the champion sits on is a direction the gate never tests, so pinned bounds are widened
   (0160, 2026-09-22): `preflop_fold_scale` up to 1.8 (it was 1.3, where the champion sat; the gate
   still rejected 1.2), `realize_weight` down to 0.1 (three promotions have since walked the champion
   to 0), `call_margin` to −0.10, `passive_fold_bonus` to −0.25 and `three_bet_call_margin` to −0.14.
7. **Successive halving** on common deals (seed `900000 + cycle·10000 + round·1000`):
   - Each round is one batch (`sim::paired_eval_many`): the champion plays each table once, and every
     (candidate, table) run shares the thread pool. Results are bit-identical to one `paired_eval` per
     candidate; the champion half of the work is no longer repeated per candidate.
    - Round 0 gives every candidate 1 table × `learner_hands` (liveness: a wide field, dropping
      only what never matters); each round doubles the tables, so round 1 carries the budget
      that can rank.
    - Results accumulate across rounds (`PairedResult::combine`), and across cycles through the
      rejection ledger (`sv10_bot::search_ledger`, 0285): each candidate's accumulator starts from
      its stored combined measurement, decided-dead transitions (a full screening budget of hands
      with 95% upper bound below +1 bb/100) are not proposed, and every measured transition is
      folded back at the end of the search phase. The ledger is scoped to the champion version and
      the evidence-refresh watermark, so a promotion or a refresh retires it; `review ledger`
      prints the barred and accumulating transitions. Confirmation runs on fresh deals and stays
      out of the ledger.
   - After each round a candidate is dropped when no simulated outcome changed (`differing == 0`) or
     its 95% upper bound is below +1 bb/100 (futility). The rest are sorted by mean and the better
     half (rounded up) continues.
   - It stops when one candidate has at least `learner_tables × learner_hands` hands, or the budget
     of 16 × that is spent.
8. **Promotion gate** (`sv10_bot::promotion`, 0097): the survivor is confirmed when its search mean
   is at least +1 bb/100 and some outcome changed. Its search interval is selection-biased (best of
   ~26), so it only selects. **Confirmation** on fresh deals decides alone. It runs up to 12 chunks of
   `4 × learner_tables × learner_hands` (72k hands each on this box, ~70 s; up to 864k hands, ~14 min;
   seed `55000000 + cycle·16 + chunk`, stride above the chunk count so cycles never share deals; 0149),
   accumulated with `PairedResult::combine`, in a Haybittle–Peto design:
   - After an interim chunk, stop for futility when the mean is ≤ 0 or the upper bound is below
     +1 bb/100, and from chunk 4 when even the full confirmation's standard error would leave the
     current mean's lower bound below +1 bb/100. Promote early only on z ≥ 3 with the lower bound
     at least +1 bb/100.
   - After the last chunk, promote when the 95% lower bound is at least +1 bb/100.
9. **Experiment targets** (0291): after the search, the learner publishes
   `learner.experiment-targets.v1` — the survivor it is about to confirm, then every ledger transition
   still undecided (ahead, 95% upper bound above +1 bb/100, not decided dead, never rejected by a
   completed confirmation), ranked by z of the mean above the bar. A completed confirmation rejection
   is recorded in the ledger's `confirm_rejected` and never offered again. When the fleet's
   experiment pair has returned `live-supported` for a target in the current scope, the next cycle
   skips the search and confirms that target on fresh deals: live evidence only prioritizes, the
   gate above alone promotes, and live results never combine with simulated ones.
   Futility stops never raise the false-promotion rate. The z ≥ 3 interim boundary keeps the overall
   one-sided error near 2.5%.
   Promotion writes `params.v1`, appends `sv10-ev-<n>` to `learner.lineage` and records the
   experiment. The fleet hot-reloads the parameters and stamps the version on each decision. A
   failed confirmation is recorded as a rejected experiment.

   Why: the old gate needed a positive lower bound on the search *and* on a single 24k-hand
   confirmation (SE ≈ 4 bb/100), so only edges far above the +1 bb/100 bar could pass; the same
   survivors came back cycle after cycle and were all discarded at the confirmation. The interim
   chunks above stop those early instead.
10. Record the independent refresh and search row/time watermarks in `learner.pacing`, plus the
   search-only no-promotion streak and follow-up flag.

Every dropped, halved, rejected and promoted candidate is logged to `learner.log`, and the last 40
experiments are kept in `learner.experiments` for the dashboard.

## Paired evaluation (`sim::paired_eval`)

- Each table seats our policy plus 5 clones drawn by hand-count weight with a per-table seed. It is
  played twice with identical seeds: once with the champion, once with the challenger. Both run on
  the live `ModelStore` with learning on and the active neural model.
- 100-big-blind stacks reloaded every hand; decisions use `tuning.decision_samples` Monte Carlo
  samples.
- Per-hand difference (challenger − champion) in big blinds uses `Hand::expected_net(600)`: when
  betting closes before the river with two or more players left, the net is averaged over every
  runout of the unseen cards (or 600 random runouts). The expectation is unchanged and all-in
  variance is removed. Then `Hand::chance_correction(0)` is subtracted (0123): for the turn and river
  card dealt with betting open and hero still in, `pot before the street × (hero's showdown share
  with the actual card − mean share over every unseen card)`. It is zero-mean by construction
  (tested by enumerating every turn and river), so estimates stay unbiased while intervals narrow
  14–30% (≈1.4× fewer hands for the same power; measured on six seed/challenger pairs).
- `mean_bb`, `se_bb` (sample standard error), `differing` (hands whose result changed); bounds are
  mean ± 1.96·se.

## Budgets on this box

Pacing (0081): the fleet plays about 1,000 hands in 2 h 20 min. Before pacing, cycles ran back to back
at ~300% CPU, each seeing ~50 new hands. With pacing a cycle runs every 2–9 h plus follow-ups after
promotions, about 5% of the old CPU time.


`learner_threads` 8 (6 before 2026-09-17), `learner_tables` 12 (unchanged: still sized from logical cores − 2), `learner_hands` 1,500–3,000 (scaled by the measured
samples/s; 1,637–2,201 observed). Cycles (neural training ~25 s, successive halving over up to 27
candidates, confirmation) took 7–16 minutes with the fleet playing.

2026-09-27 (0334): cycles took a median 441 s and up to 2,241 s in one piece (net gate 93–143 s,
halving rounds 30–200 s, confirmation chunks ~110 s each, up to 12). The same work now runs as
steps of at most ~100 s.

## Known limits

- One knob changes per promotion, so interactions are found only through successive promotions.
- Paired confidence intervals assume independent hands; tables share clones and models, so the
  fresh-deal confirmation is the guard against optimistic intervals and the winner's curse.

## Versioned response-feature experiments

Production trains `PriorStreetCalls38` since 2026-09-22 (0135: held-out −2.77 mnats, 95% −3.95..−1.58, validating after 2026-09-21; −1.09, −1.86..−0.33, on 2026-09-18..21); inference picks the layout from each network's input width, so a stored 37-input net keeps playing until a 38-input one passes the predictive and paired-poker gates. `neural_ab --cutoff <RFC3339> --validation-end <RFC3339> --seed <N>` is a
read-only experiment: it splits live hands chronologically at the requested boundary, rejects hand-ID
overlap and fewer than 300 validation samples, and trains equal `[inputs,48,24,3]` networks with the
same seed and epoch budget. Profiles are rebuilt in timestamp order, so no future opponent statistics
enter an earlier sample. `PriorStreetCalls38` adds one capped feature: the named responder's calls
on completed earlier postflop streets. Current-street calls and other players never count.

The JSON report includes overall and stat-baseline log loss, paired loss-delta standard error and 95%
interval, repeated-call river fold/continue loss and calibration, plus street, confidence, facing-size,
heads-up/multiway, stack-depth, sparse/established-profile, and line-history slices. It does not write
`nn.response.v1`; prediction evidence alone cannot activate a layout or establish poker profitability.
