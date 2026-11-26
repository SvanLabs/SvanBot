# 11 — Poker AI research for SvanBot's five brains

Generated-by: Codex/GPT-6

Research for [V11-88 (#1234)](https://github.com/SvanLabs/SvanBot/issues/1234), 2026-11-26.
Code checked at `e745cfb`. The experiments below are proposals. Published results are the
authors' measurements.

## The first experiment

Test an evidence-dependent cap on the heads-up flop and turn re-solve's continuation pin for
Hunter. Compare it with the current fixed pin and a zero-pin control, using the same range in
all three arms. Then isolate whether the cap keeps gains against fixed opponents while reducing
loss against the switching and learning archetypes. Two independent 2026 papers support testing
this trade-off. Neither gives a six-max safety guarantee for SvanBot. The change fits the existing
work budget because the re-solve can still run once per decision. [SafeAdapt-Poker §§3–5](https://arxiv.org/html/2610.20473v2),
[Confidence-Scheduled Restricted Responses §§4–7](https://arxiv.org/html/2607.28520v1).

For three players still in a pot, start with a bounded river decision model that lets the second
opponent respond to the first opponent's action. Compare it with the current pricer before
writing a multiway re-solve. For training, the cheapest candidate is a small action model built
from public action histories with a chronological prediction check and a separate paired sim.
For Anchor, test mixed pricing with measured action probabilities. These experiments fit the
[Five Brains plan](../../superpowers/specs/2026-11-25-five-brains-design.md).

## What the 2026 sources establish

### Opponent reads and adaptation

- **SafeAdapt-Poker** (October preprint, v2). A fixed anchor, support-preserving residual policies,
  and a session-level budget constrain deviations. The authors train separate models of local
  counterfactual risk and the value of saving budget for future decisions. In their abstract
  heads-up no-limit study, the full controller gained **42.7 ± 3.3 mbb/hand** relative to the anchor
  over the matched pool, with a worst-opponent delta of **−9.6 ± 3.0 mbb/hand**. The heuristic
  version gained 39.7 ± 3.6, with −13.8 ± 3.5 worst-case in that same pool. Exact best-response
  results come from Leduc. The held-out public hand-history experiment measures action
  prediction. The full controller's budget is on policy divergence, distinct from its payoff
  estimates. [SafeAdapt-Poker §§3–5 and Appendix D](https://arxiv.org/html/2610.20473v2).
- **Confidence-Scheduled Restricted Responses** (July preprint). A confidence sequence on pooled
  actions guides a restricted-response solve. A separate full-tree best response bounds the
  candidate's reference-relative loss in a tractable game. Main Leduc results report **+0.209**
  chips/hand steady-state gain for the scheduled policy against **+0.034** for a binary gate,
  with a largest measured certificate of 0.150 against a 0.15 budget. The fixed restricted
  response already earned +0.193 in that study. Follow-ups in larger Leduc and Liar's Dice show
  why confirmation time matters for rare or diffuse departures. The exact full-tree step is
  unavailable for SvanBot's six-max game. [Agents That Certify Their Own Exploits §§4–7](https://arxiv.org/html/2607.28520v1).
- **StratFormer** (April preprint). First predict opponent actions while playing a reference
  policy, then learn exploitation. Paired evaluation in two-player limit Leduc averaged
  **+0.106 BB/hand** over the tabular GTO policy across 12 scripted variants. The moderate
  over-caller lost **−0.171 BB/hand** relative to the reference. Training took about 72 hours on
  an Apple M-series GPU. The useful experiment is the two-stage curriculum and a mandatory
  calling-station row. [StratFormer §§4–6 and Table 2](https://arxiv.org/html/2604.25796v1).

### Training and playing styles

- **SPOKE** (October student-workshop paper). Teacher-relative preference penalties, an anchor
  to the teacher distribution, and legal action filtering were compared with several training
  objectives. Six-max evaluation is 100-BB *preflop action imitation* on 729 held-out grouped
  spots. It reports top-1 agreement of **0.775** versus **0.754** for supervised fine-tuning,
  with similar teacher KL. Value-based terms and exact exploitability were tested only in
  Kuhn and Leduc. Component-removal ablations were unmeasured. Its continuous aggression dial
  makes a candidate distribution easy to construct. Its effect on SvanBot's win rate needs
  its own paired sim. [Teaching AI Agent to Play Poker Strategically §§II–IV](https://arxiv.org/html/2610.27028v1).
- **Correlated Chance Sampling MCCFR** (July preprint). Persistent randomized Weyl streams for
  re-visited chance nodes reduced measured final exploitability **19.05–34.01%** across Kuhn
  and four Leduc configurations. Four fixed Libratus heads-up no-limit turn and river
  endgames, with two million iterations and 25 paired seeds each, found no detectable
  final-endpoint improvement. The paper distinguishes the persistent sampler's fixed-trajectory
  results from a reset variant's external-sampling guarantee. A trial belongs in a small
  tabular prototype with repeatable chance nodes. [CCS-MCCFR §§4–6 and Appendix G](https://arxiv.org/html/2607.27035v1).
- **Real-Time Parallel CFR** (May preprint). The authors report a **3.3–3.4×** postflop
  speedup on heads-up no-limit trees against their own single-thread baseline. The pipeline
  uses five CPU threads and a GPU on NVIDIA DGX Spark. Flop and turn evaluation use GPU
  leaf batches. River solving runs on the CPU. SvanBot still needs its own CPU and multiway timing.
  [Real-Time Parallel CFR §§4–6](https://arxiv.org/html/2605.19928v1).
- **PokerSkill** (May preprint). A structured rule library plus frontier API models improved
  heads-up results against GTO Wizard over default prompts. Its best model in the report still
  lost **57 ± 21 mbb/hand** (one standard error). That is evidence for a prompt system in
  one two-player benchmark. SvanBot already has structured cards, legal moves and an
  in-process decision path. [PokerSkill §§3–6 and Table 1](https://arxiv.org/html/2605.30094v1),
  [ADR 0001](../../adr/0001-decision-path-stays-in-process.md).

### Six-player blueprints and evaluation

- **SHARINGAN-2** (November author technical report). The six-player, 100-BB blueprint used
  **183 billion** external-sampling MCCFR iterations on rented CPU servers, with optional
  bounded turn and river search. In its reported four-million-deal duplicate **3-vs-3** test,
  the final blueprint beat its own early checkpoint by **5.09 ± 1.16 mbb/hand**, with AIVAT
  correction and a 95% interval. The report
  describes a severe bug from translating a legal small bet to a check in the old internal
  tree and shows that another attacker still beats the fixed blueprint without search.
  Three-vs-three self-cross-play measures a different setup from one SvanBot at a six-player
  table. The reported scale informs the budget for a narrow prototype. The off-tree failure
  supports testing legal size translation early. [SHARINGAN-2 technical report](https://sharingan.poker/learn/sharingan-2/).
- **NoRegrets** (author repository and experiment ledger). The July six-player 200M-iteration
  blueprint used a 4.3 GB export and 79 minutes on 16 cores. In paired self-cross-play, search
  without a learned leaf estimate showed no clear gain at 800 ms and lost at 150 ms. The author
  reports a positive gain for a learned leaf estimate, including **+174 ± 100 mbb/hand** versus
  the no-model baseline after exact turn solving was added to both arms. This is a useful reason
  to compare search with a cheap leaf table and without one at equal work. Results are specific
  to that engine and opponent pool. [NoRegrets results](https://github.com/conorarmstrong/noregrets#results).
- **Pluribus** (2019, historical reference). Six-player no-limit results support offline
  external-sampling MCCFR and depth-limited search with multiple continuation policies.
  They give no general Nash-convergence guarantee for six-player no-limit play. For the
  SvanBot prototype, track downside against fixed exploiters and independent opponents.
  [Brown and Sandholm, Science](https://doi.org/10.1126/science.aay2400).

### Measuring changes

- **GTO Wizard Benchmark** (March preprint). The preprint recalls a 2022 match in which GTO
  Wizard AI beat Slumbot by **19.4 ± 4.1 bb/100**. In the 200-BB heads-up benchmark the authors
  report roughly a threefold reduction in standard deviation using AIVAT. The benchmark has its own rules
  and access to correction inputs. [GTO Wizard Benchmark §§3–4](https://arxiv.org/html/2603.23660v1).
- **AV-AIVAT** (August preprint). Across 71,439 paired heads-up raw and corrected observations,
  reported median stopping improvement was **74×** for the asymptotic confidence sequence at
  nominal 95% confidence and ±1 BB precision. For the empirical-Bernstein sequence it was
  **1.37×** over 14 evaluable runs. The replay rounded the pooled observed maximum up to
  200 BB. The raw stream has that structural bound. For corrected outcomes, the paper
  supplies no corresponding analytic bound, so the result is descriptive. A finite-sample certificate needs a justified bound on those outcomes. Their online value estimator is predictable, learning from past games only.
  [AV-AIVAT §§4–7 and Appendix D](https://arxiv.org/html/2608.06362v1).
- **Heuristic Pathologies in AIVAT** (May preprint). Fitting the correction heuristic on the
  evaluated hands can manufacture small variance and steer the test statistic. The paper's
  inverse-variance-weighted example on Pluribus logs lowered estimated sample needs by **43%**
  relative to uniform weighting for that heuristic, at possible bias. Freeze corrections
  before evaluation and compare raw and corrected paired estimates. SvanBot's local V11-71/72
  tests missed the planned 2× variance cut, so the external numbers cannot set the local
  experiment's sample budget. [Heuristic Pathologies §§3–5](https://arxiv.org/html/2605.14261v1),
  [The luck of decisions](10-the-luck-of-decisions.md).

## Applying the findings

### 1. Hunter's continuation pin

`read::personal` already shrinks an opponent's frequencies toward the prior according to
evidence at the situation. The read pipeline can hold a read, reset it at a change, or damp it
for a learning opponent. `Policy::decide_with_profile` passes `params.exploit` to the re-solve
as a separate input. The re-solve uses it to pin a share of the opponent's *next-street*
continuation. The same model still supplies the current range, so zero pin is its own control.
The river has no further continuation to pin.
[Read model](../../../crates/strategy/src/read.rs),
[Read pipeline](../../../crates/learn/src/reads.rs),
[Policy](../../../crates/strategy/src/policy.rs),
[Re-solve](../../../crates/strategy/src/resolve.rs),
[Continuation](../../../crates/strategy/src/resolve/leaf.rs).

1. In sim, predeclare fixed, zero and evidence-capped arms. Keep profile overrides, sampled
   runouts, iterations, action menu and range model the same. Reuse permitted read evidence
   and adaptivity class. Where evidence for next-street facing and unfaced actions is
   available separately, use the lower supported share for a first prototype. Give all three
   arms the same selected action model for the range and continuation. Override the pin after
   model selection, keeping the configured read-mixing budget fixed.
2. Verify that held and unsupported reads earn no personal continuation pin, admitted reads
   can earn it, and a change point resets it. The existing admission rule gives switching
   and learning classes no personal pin. Measure eligibility and reacquisition by row. As
   a control, equal model and seed give identical river results when only pin changes.
   Test flop and turn separately and record how many decisions actually change.
3. Use fixed sparring archetypes, the calling station, an order-preserving switch sequence,
   the learning archetype, and the reference policy. Report matched per-row win-rate
   differences, time to recover after a switch, and worst tested losses. Run a fresh
   confirmatory seed before considering the existing margin and live guard. A cap on the
   continuation pin gives an empirical downside check only.

### 2. Fit the action model before spending evidence

The current fit is small and deterministic. `learn::fit` works with action histories, with
known or latent hole cards. `ActionModel` conditions on strength bucket, street, and whether
a bet is faced. Start by comparing the current model with a residual table shrunk toward its
current prediction. Add one context per experiment. Start with the number of live opponents,
then the prior action in betting order or faced size if the first context improves prediction.
Count opportunities for each action. Fit from hands already finished by that bot, split entire
hands chronologically before preparing features, and use disjoint sparring archetypes to
evaluate transfer. Later actions, showdown cards and payout belong in offline labels or
evaluation. A per-decision input is restricted to the spot and information already visible.
[Fit](../../../crates/learn/src/fit.rs),
[Action model](../../../crates/strategy/src/action.rs),
[Reads stay per bot](../../adr/0004-reads-are-per-bot.md).

Use held-out action log loss, calibration and per-context support to accept a fit. Then pass the
frozen fit through the paired sim to measure what happens when the policy acts on it. Keep
holdouts and confirmation independent of the champion search. The unattended search currently
changes one number, the exploit budget, and already confirms a challenger on fresh deals. Add
one dimension at a time after the per-bot champion path is ready. [Search](../../../crates/learn/src/search.rs),
[Five Brains phases 2–3](../../superpowers/specs/2026-11-25-five-brains-design.md).

### 3. Anchor, Grinder, Gambler and Chameleon

| Brain | CPU-only candidate | Comparison |
|---|---|---|
| Anchor | Start with a draw among legal pricer actions close to the best estimated value, using a pot-scaled spread and a supplied seed. Record the final probabilities after coalescing actions that resolve to the same legal size. | Current deterministic pricer versus the mixed pricer at equal work. Test replay, long-run action frequencies, worst-reference loss, and frequency of costly departures. |
| Grinder | Extend the existing opening-width seam one mechanic at a time. Test narrower opening width before a value-heavier three-bet or smaller size menu. | Frozen reference, then archived champions. Measure effect by position and pot size. |
| Gambler | Test wider opening width first, then a bounded increase in raise preference over an Anchor-like mixed distribution where multiple actions have support. | Shared seeded deals and matched tables with strong callers. Report how often the new style raises and how much it changes win rate. |
| Chameleon | Start with the planned per-orbit rules using a bot's own evidence. Route held reads and low-evidence tables to the designated reference component. | Planned selector versus each frozen component, including switches and delayed adaptation. Keep a guard per component before changing selector priors. |

The current kinds are `Solving` and `Pricing`. The policy already carries a style and receives a decision seed. Mixing is a distinct change to the pricer. Direct KL from a candidate that adds mass
on an action absent from the deterministic reference's support is infinite, so first use a
validated mixed reference if a later experiment needs a KL limit. Check units, legality and the
margin on the concrete legal actions actually played.
[Policy kinds](../../../crates/strategy/src/kind.rs),
[Styles](../../../crates/strategy/src/style.rs),
[Policy and draws](../../../crates/strategy/src/policy.rs),
[Five Brains phases 4–5](../../superpowers/specs/2026-11-25-five-brains-design.md).

### 4. Three players on the river

The pricer deals card-disjoint ranges together and integrates all caller subsets for each
sampled deal. Its raise responses currently multiply marginal continuation probabilities
conditional on those dealt cards and value continuations as calls. It has no re-raise branch
after our own raise. SvanBot already distinguishes eligible call pot from the full pot when
the caller knows contributions. `Spot` exposes the bot's own stack and commitments, live
opponent count, opponent lines and seats. The full ordered contribution and stack inputs
needed for an exact multiway decision are absent from that interface.
[Pricer](../../../crates/strategy/src/pricer.rs),
[Raise pricing](../../../crates/strategy/src/pricer/ev.rs),
[Spot](../../../crates/core/src/spot.rs).

1. Start in sim at river spots with exactly two live opponents, where acting order and
   contributions are known, there are no side pots, and each seat can cover the tested
   bet. Compare the existing caller-subset calculation with two ordered fold/call responses.
   The second response can depend on whether the first called and on the updated pot. Fix
   ranges, dealt cards and legal sizes between arms. Without conditional context effects,
   the new calculation should match the old one within floating-point tolerance.
2. Build synthetic fixtures for a forced fold, one caller, two callers, a tied showdown, and
   an overcall tendency. Then evaluate a single capped re-raise branch with the hero's legal
   response and actual chip accounting. Require the fallback where the prototype lacks ordered
   public state, when allocation of an all-in is ambiguous, or when the work cap is exhausted.
3. Record eligible-spot frequency, changed decisions, six-max paired win rate and tail latency.
   Compare one hero in a mixed field as well as against homogeneous sparring. Use bounded fixed
   work and time it outside the pure policy. Advance to an eight-strength-bucket re-solve only
   after the bounded calculation has been compared with the reference. On any extension to the
   turn, compare a small fitted leaf table against the existing hand-written continuations
   within the same work cap.

For the separate blueprint prototype, start with a frozen six-max 100-BB preflop abstraction,
explicit response to legal off-tree sizing, and deterministic seeds. Measure exploiters, visit
coverage, bytes and CPU time before widening the abstraction. A small enumerated game can verify
regret updates against exact values; include a matched independent-chance baseline if testing
correlated sampling. An abstraction that is stronger in internal self-play still needs fresh
six-max paired deals against independent opponents.

## Evidence for accepting a candidate

- Use mirrored seats in the heads-up sim and all six hero seats in the six-max ring.
  Record the seed, commit, candidate and champion digests, sample/iteration caps, rules, and
  opponent configuration. Rotate seats inside each deal and keep switching or learning opponents
  in deal order within a bout. For intervals, use the shared-deal or bout unit that reflects
  the dependence, and an appropriate confidence sequence when stopping early.
  [Paired sim](../../../crates/learn/src/paired.rs),
  [Arena](../../../crates/learn/src/arena.rs),
  [Reading the Arena](08-headline-measurement.md).
- Compare raw and corrected estimates on a frozen test set before enabling a new luck
  correction. For a candidate intended to reduce harm, publish the worst tested row and
  a matched table with an independent reference or a frozen exploiter. Decide using the
  approved margin rule, fresh confirmation and the live guard.
  [The luck of decisions](10-the-luck-of-decisions.md),
  [Five Brains margin rule](../../superpowers/specs/2026-11-25-five-brains-design.md#the-margin-rule).
- The Open Poker docs revision checked was **2026-09-02**, WebSocket V2. Public play has 2–6
  seats, 10/20 blinds, default 2,000-chip buy-in and a 45-second action clock. The venue's
  current `valid_actions` and turn identifiers control sends. Same-owner bots may meet in the
  local sim only. The production decision stays in process, pure below runtime, with only
  its bot's own reads. [Open Poker docs](https://docs.openpoker.ai/llms-full.txt),
  [ADR 0001](../../adr/0001-decision-path-stays-in-process.md),
  [ADR 0004](../../adr/0004-reads-are-per-bot.md).
