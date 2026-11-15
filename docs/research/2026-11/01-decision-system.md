# 01 — Decision system: state of the art (2025–2026) vs SvanBot

Researched 2026-11-15 for v11.0.0. External claims link primary sources; repo facts come from
`docs/ARCHITECTURE.md`, ADRs 0001–0002, `README.md` and `crates/libs/policy/`.

## What SvanBot does today

- **Single-decision exploitative EV pricing, no game tree.** `policy::decide_with`
  (`crates/libs/policy/src/policy/mod.rs:108`) rebuilds opponent ranges (`oprange`), deals shared
  Monte Carlo or exact deals, prices fold, check/call and a few raise sizes against response
  models (stats blended 30/70 with a small CPU MLP, plus live fits), self-calibrates, and picks
  the best EV.
- **Later streets are handled by one number.** A tunable `realize_weight` scales the equity we are
  assumed to realize (`params.rs:40`), and an optional one-street check-value term covers the next
  street (`params.rs:84`). Nothing searches past the current decision.
- **No equilibrium component.** There is no blueprint, no CFR, no balance and no safety bound. The
  policy is a best response to its own models of the opponents.
- **Compute.** The reference box is an i7-4770K with 32 GB and no GPU (`README.md:148`). Live
  decisions take 95 ms at the median and 325 ms at p95, against an 8 s cap
  (`DECISION_CAP`, ADR 0001) and the server's 45 s clock (`SPEC-protocol.md:134`), six-max. Five bots share
  the CPU with the learner and the analyst.

## State of the art

### Depth-limited real-time search

- **Pluribus (six-max, 2019).** This is still the reference design for more than two players
  ([Science paper](https://noambrown.com/papers/19-Science-Superhuman.pdf)):
  - Blueprint: MCCFR with early Linear CFR, 12,400 core-hours, under 512 GB, about $144. Played
    only on the first round; later rounds are re-solved by depth-limited search.
  - At the depth limit, each player chooses among k = 4 continuation strategies: the blueprint,
    and the blueprint biased toward fold, toward call and toward raise. This keeps the searcher from
    picking a brittle, unbalanced line.
  - Live play ran on two Haswell E5-2695 v3 CPUs with under 128 GB. Each search took 1–33 s, and
    hands averaged 20 s.
  - Theory: [depth-limited solving](https://arxiv.org/abs/1805.08195),
    [safe subgame solving](https://arxiv.org/abs/1705.02955).
- **ReBeL (heads-up).** Self-play RL plus search over public belief states (PBS). It provably
  converges to Nash in two-player zero-sum games ([arXiv 2007.13544](https://arxiv.org/abs/2007.13544)).
  From [the paper's PDF](https://arxiv.org/pdf/2007.13544):
  - The value network's input is about 2 × 1326 range probabilities plus pot and board.
  - Data generation used up to 128 machines with 8 GPUs each.
  - Beat Dong Kim by 165 ± 69 mbb/hand; no decision over 5 s.
- **Student of Games.** Growing-tree CFR search guided by learned value and policy networks.
  Sound; beats the strongest open heads-up NLHE agent
  ([arXiv 2112.03178](https://arxiv.org/abs/2112.03178), *Science Advances* 2023).
- **Exploitation inside search (2025).**
  [ABD, arXiv 2501.10464](https://arxiv.org/abs/2501.10464) puts a full opponent model *beyond* the
  depth limit, using strategy portfolios ("matrix-valued states"). Against opponents who make
  mistakes past the horizon it reports more than 2x the utility, while staying robust to rational
  opponents. This is the closest published match to SvanBot's goal of exploitation with a floor.
- **Real-time parallel CFR (2026).** About 50 ms per HUNL iteration with GPU leaf evaluation
  ([arXiv 2605.19928](https://arxiv.org/abs/2605.19928)); needs a GPU.

### CFR variants

- **DCFR.** Discounts positive and negative regrets and the average strategy separately. It is the
  robust default for poker-like games ([arXiv 1809.04040](https://arxiv.org/abs/1809.04040)).
  Linear CFR is a special case, and Pluribus used it.
- **PCFR+.** Predictive regret matching+. It is very fast on many games
  ([arXiv 2007.14358](https://arxiv.org/abs/2007.14358)).
- **PDCFR+ (IJCAI 2024).** Combines prediction with DCFR-style weighting. It is fastest on
  non-poker and small poker games, but **DCFR+ stayed fastest on large poker**
  ([arXiv 2404.13891](https://arxiv.org/abs/2404.13891); code at
  [rpSebastian/PDCFRPlus](https://github.com/rpSebastian/PDCFRPlus)).
- **HS-DCFR (AAAI 2026).** Training-free hyperparameter schedules for DCFR, often orders of
  magnitude better at a fixed run time. Schedules learned on three small games carry over to 17
  games, large poker included ([arXiv 2404.09097](https://arxiv.org/abs/2404.09097)). A few lines
  of change.
- **MCCFR** still solves blueprints; vector-form CFR solves small real-time subgames (Pluribus).

### Abstraction, networks and distillation

- **Card abstraction.** Potential-aware imperfect-recall clustering with earth mover's distance
  (EMD) is the standard method ([AAAI 2014](https://ojs.aaai.org/index.php/AAAI/article/view/8816)).
  Work from 2025 keeps part of the history that this method throws away:
  [KrwEmd, arXiv 2511.12089](https://arxiv.org/abs/2511.12089) and
  [arXiv 2510.15094](https://arxiv.org/abs/2510.15094).
- **Action abstraction.** ReBeL used at most three raise sizes, jittered ±0.1 pot, and adds
  off-tree actions to the subgame at test time ([PDF](https://arxiv.org/pdf/2007.13544)).
- **Neural CFR / distillation.** [Deep CFR](https://arxiv.org/abs/1811.00164) replaces tables
  with networks; ReBeL and SoG distil search into the nets that seed the next search.
- **Multiplayer DRL plus behaviour cloning (AAMAS 2026).** "Team of Rivals" (Havoc) uses
  behaviour-cloned specialists, with a value-based master that switches between them at each
  decision. It reports beating LLM-based and other baselines at much lower compute
  ([PDF](https://ifaamas.org/Proceedings/aamas2026/pdfs/LRNX6318.pdf)).

### LLM-free vs LLM-assisted

- **Every strong system is LLM-free:** Pluribus, ReBeL, SoG, and GTO Wizard AI, which beats
  Slumbot by 19.4 ± 4.1 bb/100 ([arXiv 2603.23660](https://arxiv.org/abs/2603.23660)).
- **Frontier LLMs are well short of strong agents.** The 2026 GTO Wizard Benchmark finds GPT-5.4,
  Claude Opus 4.6, Gemini 3.1 Pro and Grok 4 "far below" its agent
  ([arXiv 2603.23660](https://arxiv.org/abs/2603.23660)). The comparison is heads-up NLHE.
- **Fine-tuning helps, but the results are thin.** [PokerBench](https://arxiv.org/abs/2501.08328)
  shows fine-tuning lifts static accuracy, but a fine-tuned Llama still lost to GPT-4 in play.
  [SpinGPT](https://arxiv.org/abs/2509.22387), trained with supervised fine-tuning plus RL on
  solver hands, matches the solver 78% of the time. It reports +13.4 ± 12.9 bb/100 against Slumbot,
  which is not significant at 2σ.
- **Verdict:** keep LLMs off the decision path.

### What fits a CPU and a 45 s clock

- **Feasible on this box:** vector-form DCFR+ on a postflop subgame (2–3 sizes, rest of the
  street plus leaf values; Pluribus took 1–33 s on 2 server CPUs), abstraction-free turn and river
  subgames (SvanBot already enumerates heads-up runouts exactly), and an offline MCCFR preflop
  blueprint.
- **Not feasible without GPUs:** ReBeL-class PBS value nets (128 × 8 GPUs of data generation).
- **The real limit is CPU shared across bots, not the clock.** Five bots share about 8 threads.
  A 1–3 s budget per decision, applied only to large postflop pots, is affordable. The current p95
  is 325 ms, and ADR 0001 allows up to 8 s.

## Gap vs SvanBot

| Area | Best practice | SvanBot |
|---|---|---|
| Future streets | Depth-limited re-solve with continuation leaves | One scalar `realize_weight` plus a one-street check term |
| Balance and safety | Equilibrium core, with exploitation bounded (safe subgames, ABD) | Pure best response to models; no floor if a model is wrong |
| Multiway | Blueprint plus search with k continuations | Pricing with caller shares (`callers.rs`), no tree |
| Solver | DCFR+ / HS-DCFR / PCFR+ | None |
| Opponent model | Inside the search (ABD) or as a portfolio (Havoc) | Rich (stats, NN, live fits, tells): **SvanBot's strength** |
| Clock use | Seconds per hard spot | 95 ms median, so about 99% of the allowed time goes unused |

SvanBot lacks any lookahead over *strategies*: a fixed best response is exploitable by adaptive
bots, and its EVs carry the one-street realization error that `realize_weight` and
self-calibration patch.

## Ranked options

1. **Exploitative depth-limited postflop re-solve, heads-up pots first.**
   - *What:* run DCFR+ with an HS-DCFR schedule in Rust on a turn or river subgame (2–3 sizes plus
     all-in). Opponent ranges come from `oprange`. At the leaves, use Pluribus-style k = 4 biased
     continuations, with SvanBot's opponent model as one portfolio entry, as in ABD. Bound how far
     play can move from the equilibrium solution.
   - *Cost:* about 4–6 weeks. A new `sv10-solver` crate under `libs/`.
   - *Risk:* medium. Latency on a shared CPU, which needs a time-budgeted solver that can stop
     cleanly, and new golden snapshots.
   - *Gain:* the largest available. It replaces the realization heuristic where the pots are
     biggest, and it adds a safety floor.
2. **Offline six-max preflop blueprint (MCCFR with Linear or DCFR discounting) as a prior.**
   - *What:* the learner computes it at idle priority. Live play blends the blueprint with the EV
     policy, or uses it as the default when an opponent model has little data.
   - *Cost:* about 3 weeks, plus days to weeks of learner CPU.
   - *Risk:* low to medium (memory; abstraction quality).
   - *Gain:* medium. Robust preflop play against unknowns.
3. **Use the clock: an adaptive sample and solve budget for high-stakes spots.**
   - *What:* scale samples, and later solver iterations, with pot size, inside ADR 0001's gate.
   - *Cost:* about 1 week.
   - *Risk:* low. *Gain:* small to medium alone; the prerequisite for option 1.
4. **Policy distillation of solver output into `sv10-nn`.**
   - *What:* the analyst runs option 1 deeply offline, and a small MLP learns the solver's policy so
     that cheap spots get solver-like play without solving live.
   - *Cost:* about 3 weeks, after option 1.
   - *Risk:* medium (distribution shift).
   - *Gain:* medium. Lighter live CPU load.
5. **ReBeL or SoG learned PBS value net, or an LLM on the path.** Not recommended for v11.
   - *Why:* the value net needs GPU-scale self-play, and LLMs measurably trail solvers.
   - *Cost:* high. *Gain:* unproven on this hardware.
