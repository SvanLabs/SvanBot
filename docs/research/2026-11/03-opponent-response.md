# 03 — Per-opponent response systems

Research for SvanBot v11.0.0. Read-only; nothing built or measured. Builds on
`docs/research/opponent-modelling-2026-11-15.md` (branch `research/opponent-modelling-1023`, C1–C6)
and #1025, which picks the first opponent-modelling item. This note covers the step *after* the model:
how the bot responds to what it reads. Labels: **Fact** (read in code or a source), **Inference**.

## 1. What SvanBot responds with today (Fact)

- **The response is a pure best response to a shrunk model.** `policy::decide_with` prices
  fold, call and raise sizes by EV against per-opponent ranges and response estimates. It takes the
  argmax and mixes only between near-equal options (`crates/libs/policy/src/policy/mod.rs:3`).
  There is no equilibrium or blueprint anchor.
- **Safety comes from four layers, and none of them is an exploitability bound:**
  - shrinkage toward the population (`confidence = hands/(hands+40)`, `model/profile.rs:57`);
  - held-out gates on every per-opponent read (`playerfits`);
  - caps, such as the fold offset of ±1.5 logits with prior 20 (`apps/bot/src/playerfold.rs:19,118`);
  - a 1,000-hand recency decay.
- **Exploitability is measured, not constrained.** `bench/exploit.rs` (#783) is a lower bound
  inside a 6-parameter archetype family.
- **Archetypes exist only in the simulator** (six, `agents.rs:67`). Live, each rate shrinks
  alone toward one population mean.
- **Adaptivity is handled only by decay.** The image opponents form of us (`hero_seen_view`) is
  built, but `Params::hero_image` is 0 (`params.rs:182`). Nothing tests whether an opponent adapts
  to *us*.
- **Table selection is a response layer** (soft/tough rating, head-to-head override, top-30
  seeking; `docs/GUIDE.md`). Reputation is keyed by agent id across renames (`reputation.rs`).

## 2. Literature

### Safe exploitation (RNR, DBR, safe best response)

- **RNR** computes a best response to an opponent model, under the constraint that the opponent
  plays that model only with probability p. Sweeping p traces the exploitation/exploitability
  Pareto frontier ([Johanson, Zinkevich & Bowling, NeurIPS 2007](https://proceedings.neurips.cc/paper/2007/hash/6e7b33fdea3adc80ebd648fffb665bb8-Abstract.html)).
- **DBR** replaces the single p with per-information-set confidence from observation counts, so
  thinly observed spots fall back to worst case
  ([Johanson & Bowling, AISTATS 2009](https://proceedings.mlr.press/v5/johanson09a.html)).
- **Safe exploitation** (Ganzfried & Sandholm, ACM TEAC 3(2), 2015) risks only value already
  won above the game value. Its per-iteration LP does not scale; Safe Exploitation Search moves it
  into real-time subgame refinement with an exploitability upper bound
  ([Liu et al., NeurIPS 2022](https://proceedings.neurips.cc/paper_files/paper/2022/file/b12a1d1014e952e676f5d6931d03241a-Paper-Conference.pdf),
  which also characterises the 2015 work).
- **Adaptation safety** asks only that a strategy be no more exploitable than the non-exploiting
  version. OX-Search does this by online search and stays robust to model error and deceptive
  opponents ([Ge et al., ICML 2024](https://proceedings.mlr.press/v235/ge24b.html)).
- **Depth-limited counter-strategies:** CDBR and CDRNR give fast responses to unseen opponents
  ([Milec, Kubíček & Lisý, AAMAS 2024](https://aamas.csc.liv.ac.uk/Proceedings/aamas2024/pdfs/p2393.pdf)).
  ABD uses a strategy portfolio past the depth limit, and reports more than twice the utility
  against opponents who err past the limit
  ([Milec, Kovařík & Lisý, arXiv 2501.10464, 2025](https://arxiv.org/abs/2501.10464), preprint).
- **ε-equilibrium bases:** safety relative to the worst-case value of the base you actually
  have ([Jeary & Turrini, arXiv 2307.12338](https://arxiv.org/abs/2307.12338)).
- **Certified exploits (CS-RNR):** anytime-valid confidence sequences release a deviation only
  once its interval clears the reference. An RNR solve then makes candidate counters, and each one
  is certified by a full best response against a budget before it is deployed. The paper reports
  6.2× the gain of a binary gate within budget, in Leduc
  ([Li & Huang, arXiv 2607.28520, 2026](https://arxiv.org/abs/2607.28520), preprint).
- **Showdown censoring:** information gathering priced under a safety floor
  ([Guo, arXiv 2608.09954, 2026](https://arxiv.org/abs/2608.09954), preprint).

**Caveat (Inference).** Every guarantee above is for two-player zero-sum games. Six-max has no
game value to be "safe" relative to, so the transferable part is the *mechanism*: weighting by
confidence, budgeted deviation, certifying before deploying. The theorem does not transfer.

### Clustering and population priors

- Informed priors speed learning ([Southey et al., UAI 2005](https://arxiv.org/abs/1207.1411));
  exact Bayesian best response under Dirichlet priors
  ([Ganzfried & Sun](https://arxiv.org/abs/1603.03491)); NLHE players cluster into about 7 types
  ([arXiv 1301.5943](https://arxiv.org/abs/1301.5943)).
- **Decision-theoretic clustering** groups opponents by *how they should be answered*, not by how
  they look ([Bard, Alberta PhD](https://www.cs.ualberta.ca/~games/poker/publications/bard.phd.pdf)).
  This is the right objective for a response layer.
- No 2024–26 six-max NLHE result on population priors was found.

### Few-shot online adaptation and Thompson-style exploitation

- **TAO** (offline, in-context opponent modelling;
  [Jing et al., ICLR 2024](https://proceedings.iclr.cc/paper_files/paper/2024/hash/a1d04870cf83a0f29819d66f1dfdbfcb-Abstract-Conference.html))
  and **OMIS** (in-context imitator, actor and critic plus decision-time search, with improvement
  guarantees against non-stationary opponents;
  [Jing et al., NeurIPS 2024](https://proceedings.neurips.cc/paper_files/paper/2024/hash/710445227fa8c1b6a9ceada902dd4741-Abstract.html))
  are the current few-shot state of the art. Neither is tested in NLHE, and both need GPU
  pretraining.
- **Posterior sampling** in imperfect-information games
  ([Zhou et al., ICLR 2020](https://bigml.cs.tsinghua.edu.cn/%7Ejun/pub/ps-rl.pdf)).
  Ganzfried & Sun find Thompson's response trails the full Bayesian best response. No source pairs
  Thompson exploitation with an explicit exploitability bound.
- **Implicit modelling** (Exp4 over a response portfolio) would have won the 2011 ACPC
  exploitation event ([Bard et al., AAMAS 2013](https://www.ifaamas.org/Proceedings/aamas2013/docs/p255.pdf)).

**Inference.** "Thompson with bounded exploitability" is best built as a bandit over a *portfolio
of pre-certified arms*. Each arm's exploitability is audited offline, so sampling cannot leave the
budget.

### Detecting adaptive opponents and other bots

- **BPR+** detects switches between stationary opponent strategies and adds new ones online
  ([Hernandez-Leal et al., AAMAS 2016](https://ifaamas.csc.liv.ac.uk/Proceedings/aamas2016/pdfs/p1315.pdf)).
  Deep BPR+ scales it up
  ([Zheng et al., NeurIPS 2018](https://proceedings.neurips.cc/paper/2018/hash/85422afb467e9456013a2a51d4dff702-Abstract.html)).
- Running-error switch detection ([arXiv 2406.06500, 2024](https://arxiv.org/html/2406.06500v1)).
  Against a non-stationary opponent exploration never stops.
- On OpenPoker every seat is a bot, so "bot detection" reduces to two questions. Is this seat
  fixed-strategy, switching (a version update) or learning (responding to us)? And is it the same
  agent under a new name (`reputation.rs` already keys by agent id)?

## 3. What bot-vs-bot platforms reward

- **ACPC:** total bankroll "mostly reflects performance against weaker opponents" and rewards
  exploitation. Instant runoff rewards defending against the strong
  ([IEEE Spectrum](https://spectrum.ieee.org/champions-declared-in-ai-poker-tournament);
  [ACPC 2017 results thread](https://groups.google.com/g/computer-poker/c/_f-c1ZJwK5w)).
- **OpenPoker** is a pure total-bankroll venue: `score = chip_balance + chips_at_table`, with no
  rebuy penalty by default and a free 1,500-chip rebuy (`docs/SPEC-scoring.md`, `docs/GUIDE.md`).
- Matchmaking is pure; re-queueing is the only table lever.

**Inference.** Losses are bounded per buy-in, gains against soft seats are not, and volume counts.
That makes the venue's payoff asymmetric toward exploitation. Exploitability matters mainly where
a *learning* opponent can find the leak and farm it over thousands of hands. So the safety budget
should be spent per opponent class (fixed versus adaptive), not uniformly.

## 4. Gap vs SvanBot

1. **No exploitation budget.** A read that passes its gate applies at full strength in every spot;
   no per-spot DBR weighting, no deployment certificate. The exploit meter gates nothing.
2. **No type layer.** New seats start from one population mean.
3. **No adaptivity classifier.** No "responds to us" or switch test beyond decay; hero image off.
4. **No safe anchor.** Without an equilibrium baseline, RNR/SES/OX-Search cannot be built as
   written. Safety must be measured against the champion itself (adaptation safety, Ge 2024).
5. **The sim cannot credit modelling** (`neural.rs:118-122`); options need live held-out gates or
   new clone behaviours.

## 5. Ranked options

**1. DBR-weighted reads plus an exploitation budget** (extends C6). Scale each per-opponent
correction by spot evidence n/(n+k); deploy a read set only if the exploit meter's loss versus the
champion-without-reads stays under X bb/100 (CS-RNR's certificate idea).
*Cost* low (1–2 wk; pure transforms, meter exists). *Risk* low (only shrinks reads; live-only, so
learner parity holds). *Gain* modest: downside protection in thin spots; makes the meter a gate.

**2. Adaptivity/switch classifier with per-class response.** Per seat: a change-point test on
per-hand tallies (BPR+-style, C3) and a *responds-to-us* test (does its residual against fleet bets
diverge from its residual against the pool, and trend?). Fixed bots: full reads, long half-life.
Switchers: decay reset. Learners: damped reads and size re-randomisation.
*Cost* low–medium (2–3 wk; `hero_seen_view` and tallies exist). *Risk* low (causal; false alarms
only cost exploitation; alarm rate set on replay). *Gain* medium, and sim-creditable (swap a clone
mid-run; add a clone that best-responds to our image).

**3. Archetype-mixture prior** (C2, clustered by best counter per Bard). K types on joint tallies,
K by held-out log-loss; a new seat's prior is its type posterior, optionally indexing a per-type
`Params` delta. *Cost* medium (3–5 wk; needs the population-inflation and `fold_to_cbet` fixes and
a versioned feature layout). *Risk* medium (future leakage into net features). *Gain* medium–high
only if turnover is high; measure the share of hands against seats under 100 hands first.

**4. Certified portfolio bandit** (C5 made safe). 3–5 `Params` arms, each pre-audited by the
exploit meter; Thompson-sample per opponent cluster on AIVAT-reduced outcomes with a champion
floor. *Cost* medium–high (4–6 wk; needs variance reduction). *Risk* medium (must stay outside
learner/promotion; interacts with the experiment pair). *Gain* medium; sim-creditable (Bard 2013).

**5. Subgame safe exploitation (SES/OX-Search/CDRNR).** *Cost* very high (needs a six-max
blueprint and solver). *Risk* high (95 ms p50 latency; two-player theory). *Gain* unknown. Defer.

**Order:** 1 then 2 in v11.0; 3 after the turnover measurement and data fixes; 4 after AIVAT.
