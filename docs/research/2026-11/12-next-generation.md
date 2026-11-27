# 12 — The next generation, and the shortest path through V11-59

Generated-by: claude-code/deepseek-flash

Research for [V11-138 (#1322)](https://github.com/SvanLabs/SvanBot/issues/1322), 2026-11-27.
Venue docs at revision **2026-09-02** (`crates/runtime/src/wire.rs:11`); the leaderboard fetched
2026-11-27T04:10Z; the live fleet files and guard at 04:10–04:15Z; the code at `cec5ca6` (`main`).
External results are the authors' measurements; the repo claims link to the source that owns them.
The experiments below are proposals, not results.

## The short answer

- **The five-brains map is necessary but not sufficient for #1.** The fleet's best bot is rank
  37 (905,287) against a 2,074,215 leader; the top-30 cutoff is 1,044,046. The gap is the **chip
  rate**: #1 earns 575 bb/100, #5 375, the fleet 196–220, on similar hand-win rates and with no
  rebuy inflation (the top rows have 0–1 rebuys). The styles on the current map move opening
  width and a few mechanics; they are not a 2.6× engine.
- **The current map's live evidence is the critical path.** The V11-69 guards need ~34 more hours;
  V11-82 has no search candidate and the learner is down; each Phase 4/5 brain needs a 3,000-hand
  live guard (~1.5 days). The map can close in ~1–2 weeks with parallel sessions, but that is the
  current generation, not the next one.
- **The next-generation engine within the constraints is the 2026 six-max recipe:** a preflop
  **blueprint** plus a **bucketed, depth-limited postflop search**, with a **learned leaf** and
  **per-bot exploitation**. The design's E4 preflop blueprint is already a recorded GO; E3's
  bounded three-way river is a GO. The evidence loop (V11-137) is their prerequisite.
- **The online pass found two cheap levers and one warning.** The venue's own season guide says
  top bots used **1,000-chip buy-ins early** and 3,000–4,000 after a cushion; the fleet uses the
  2,000 default. SHARINGAN-2's November 2026 off-tree action-translation bug (a bet below the
  smallest menu read as a check) let a frozen attacker win **+2,466 ± 75 mbb/game**; SvanBot needs
  the same regression test. And NoRegrets' raw blueprint lost **−714.5 ± 331.5** to Slumbot while
  its multiway leak stayed near **900 mbb/hand** — self-play strength is not live-field strength.
- **Season 17 (ends 2026-11-29 11:48Z) is out of reach for #1.** The top-30 cutoff is marginal.
  Target season 18 for the V11-59 rollouts and season 19 for the blueprint/search.

## What I read

- The venue's full docs, revision 2026-09-02: format, scoring, seasons, leaderboard, timeouts,
  bot limits, private competitions, REST ([docs](https://docs.openpoker.ai/llms-full.txt)).
- The live public leaderboard and season
  ([season](https://api.openpoker.ai/api/season/current),
  [leaderboard](https://api.openpoker.ai/api/season/leaderboard?limit=200&sort_by=score)).
- The approved design (`docs/superpowers/specs/2026-11-25-five-brains-design.md`), ADRs 0001–0006,
  and research notes 00–11, especially [01](01-decision-system.md), [04](04-learner-experiments.md),
  [08](08-headline-measurement.md), [10](10-the-luck-of-decisions.md) and
  [11](11-2026-poker-research.md).
- The live fleet: `checks.json`, `searches.json`, `reference.json`, `arena.json`, the guard, the
  learner log, and the running processes; the V11-137 Arena timings (2026-11-27).
- The map V11-59 and every open ticket and blocking edge.
- Online primary sources (2026-11-27): the venue's season guide and live site; the SHARINGAN-2
  report; the NoRegrets repository; the arXiv abstracts of AV-AIVAT, Real-Time Parallel CFR,
  StratFormer, SafeAdapt, CCS-MCCFR, the GTO Wizard Benchmark and Deep PDCFR; and the AAMAS 2026
  abstraction paper. The links are in the sections below.

## Where the fleet stands

### The venue and the score

- 6-max no-limit hold'em, 10/20 blinds, 1,000–5,000 buy-in (default 2,000), 45-second action clock,
  no rake, 2-week seasons, 5,000 starting chips; one table per bot; Pro owners may run five
  portfolio bots; same-owner bots cannot sit together ([venue docs, Game Rules and Seasons]).
- **Score = chip_balance + chips_at_table**, with **no rebuy penalty**; the top 30 split the prize
  pool and need at least 10 hands ([venue docs, Scoring and Prizes]). A bot's rank is its
  cumulative chip profit, not its win rate.
- The public leaderboard is ranked by score and needs no auth; the current season is **17**,
  2026-11-15 → **2026-11-29 11:48Z**, with 1,216 registered.

### The leaderboard, 2026-11-27T04:10Z

The API returned 200 rows; the ranks below are the official ones.

| Rank | Score | Hands | Chips/hand | bb/100 | Win rate |
|---|---:|---:|---:|---:|---:|
| 1 | 2,074,215 | 18,026 | 115.1 | 575 | 34.6% |
| 5 | 1,381,085 | 18,431 | 74.9 | 375 | 33.1% |
| 30 | 1,044,046 | 17,969 | 58.1 | 290 | — |
| **SvanBotV11** | **905,287** | **20,560** | **44.0** | **220** | **33.2%** |
| Svanism | 783,251 | 19,957 | 39.2 | 196 | 34.0% |
| Svanar | 781,607 | 20,857 | 37.5 | 187 | 33.9% |
| SurSvan | 702,745 | 20,674 | 34.0 | 170 | 33.4% |

- The three running bots are ranks **39–41**; SvanBotV11 (rank 37) played earlier this season and is
  stopped. SuraGunnar is not on the returned page.
- **The chip-rate gap is the finding.** The top rows' win rates (33–35%) are close to the fleet's;
  their chips per hand are 1.7–3.4×. The top rows have **0–1 rebuys**, so the score is not rebuy
  inflation. The difference is in the size of the pots won and lost.
- The fleet's best (SvanBotV11, stopped) is **13% below the top-30 cutoff** and 2.3× below #1;
  the best running bot is 25% below the cutoff and 2.6× below #1. With ~2.3 days left at ~80
  hands/hour, a running bot gains ~180k chips at its current rate; that can approach the top-30
  line but not #1.

### The live map

- **V11-69** (claimed): Svanar plays `solving/tight`, Svanism `solving/loose`. At 04:10Z the guards
  read **Svanar 315 styled / 271 control** and **Svanism 205 / 186** of 3,000 each — ~34 hours to
  clear at ~80 hands/hour. The pricing check is recorded: not admitted (heads-up interval
  ±46,789 mbb/100, wider than the 20 bb/100 margin; 6-max admitted). The stablemate duel has not
  been run (`policy-runs.json` does not exist).
- **V11-82** (claimed): the cutover and freeze are done; five champion slots are seeded; the
  per-bot search has **no candidate** for any bot after its first-ring screens, and the learner is
  **down since 03:38Z**. The promotion and rollback demonstration wait on a candidate.
- **V11-90** (agent-handoff, deferred): one pin arm completed; the remaining arms and the fresh
  seed are stopped on the operator's direction.
- **V11-130–135** (charted 2026-11-27): Anchor/Hunter live, then the Adaptive kind, its two
  selectors, per-component guards and the Chameleon rollout. All blocked on V11-69/V11-82.
- **V11-136** (names) and **V11-137** (evidence speed) are unblocked. V11-137 was released back to
  the map for another session when this research started.

### The engine

- Preflop and every pot of three or more are **priced**; the depth-limited DCFR re-solve covers
  **heads up after the flop only** (`resolve` returns `None` otherwise), over exact combos, with
  200–2,000 iterations and sampled run-outs; the leaves are four hand-written continuation
  strategies (`docs/superpowers/specs/2026-11-25-five-brains-design.md`, "Where it starts";
  [01](01-decision-system.md)).
- The per-bot policy adds a kind, a style and a champion. The five styles differ by opening width
  and a few mechanics; the learner searches the champion's numbers, mostly the exploit budget.
- The Arena compares a policy with the frozen reference policy over the shared release rows; the
  margin rule admits a policy that is not worse than the reference by more than 20 bb/100. **The
  margin is a cost bound, not a strength target** — a policy can pass it and still be far from #1.

### The evidence loop

- Measured on the quiet box (load 0.6, learner down), 2 bouts × 7 release rows, 6-max:
  `arena … 20` **32.3 s**, `arena … 1` **25.3 s** (**78% is per-decision setup**, not CFR
  iterations), `arena …` at the live budget **77.4 s**.
- Scaled: a 30-bout check is ~16 minutes on a quiet box; the recorded 58-minute checks and the
  56-minute pin arm ran beside other sims, a 3–4× inflation.
- The current live guard is a **fixed 3,000 hands**. It is a safety net, not strength evidence:
  with a per-hand σ near 10 bb, 3,000 hands gives an SE near 18 bb/100, so it cannot resolve a
  20 bb/100 harm; it catches catastrophic harm ([04](04-learner-experiments.md) §3).

## What the 2026 research establishes

### Six-max engines

- **Pluribus** (six-max, 2019): an offline MCCFR blueprint for the first round plus depth-limited
  search on later streets, with k = 4 continuation strategies at the leaves. The blueprint took
  12,400 core-hours; live searches took 1–33 s on two server CPUs ([11](11-2026-poker-research.md);
  [Science](https://doi.org/10.1126/science.aay2400)).
- **SHARINGAN-2** (2026): a six-player 100-BB blueprint with **1.83e11** ES-MCCFR iterations on
  rented **CPU only**, 200 buckets, plus an optional bounded turn/river search (200k iterations,
  1,326 hole pairs, four continuations, time-budgeted and **unsafe** — no guarantee it improves the
  blueprint); its duplicate 3-vs-3 test improved by 5.09 ± 1.16 mbb/game over the 2e10 checkpoint.
  Its November 2026 off-tree action-translation bug let a frozen attacker win
  **+2,466 ± 75 mbb/game** without search
  ([11](11-2026-poker-research.md);
  [SHARINGAN-2](https://sharingan.poker/learn/sharingan-2/)).
- **NoRegrets** (2026): a six-player 200M-iteration blueprint (4.3 GB export, 79 minutes on 16
  cores); search without a learned leaf was neutral at 800 ms and harmful at 150 ms, while a
  learned leaf earned **+313 ± 198 mbb/hand** at 800 ms (**+174 ± 100** after exact turn solving).
  But the raw blueprint lost **−714.5 ± 331.5** to Slumbot and its multiway leak stayed near
  **900 mbb/hand** — the self-play number is not a live-field number
  ([11](11-2026-poker-research.md);
  [NoRegrets](https://github.com/conorarmstrong/noregrets#results)).
- **Real-Time Parallel CFR** (2026): **3.3–3.4×** postflop speedup on a single DGX Spark,
  47–54 ms/iteration on a 1B+ history tree; the GPU batched inference is not available here
  ([arXiv 2605.19928](https://arxiv.org/abs/2605.19928); note 11 read the full paper's five-thread
  detail, which the abstract does not state).
- **Abstraction:** card abstraction (potential-aware EMD clustering) and action abstraction (2–3
  sizes, jittered, off-tree actions added at test time) are the standard levers; SvanBot currently
  solves exact combos, 1,081 on the river ([01](01-decision-system.md) §"Abstraction";
  [11](11-2026-poker-research.md)).

### Opponent modeling and exploitation

- **SafeAdapt-Poker** (2026): a fixed anchor, support-preserving residual policies and a
  session-level deviation budget; +42.7 ± 3.3 mbb/hand heads up over the anchor, worst opponent
  −9.6 ± 3.0 ([11](11-2026-poker-research.md)).
- **StratFormer** (2026): predict opponent actions first, then learn exploitation; +0.106 BB/hand
  over tabular GTO in Leduc, 72 hours on an Apple GPU ([11](11-2026-poker-research.md)).
- SvanBot's per-bot reads and playbook are its historical strength ([01](01-decision-system.md)
  §"Gap"); the next step is richer per-bot fits and a bounded exploitation layer, not more style
  knobs.

### Evidence: variance and stopping

- **AV-AIVAT** (2026): 71,439 paired HUNL hands across 15 agent configurations, a median **54×**
  variance reduction and **74×** fewer hands at ±1 bb; the exact empirical-Bernstein CS needs an
  independently justified payoff bound and gives a **1.37×** stopping ratio on HUNL descriptively,
  so the asymptotic CS is the screening tool. "May the Best Bot Win" reports AIVAT's 1.72× sample
  gain offset by a 1.86× simulation slowdown, and naive peeking's 58–64% false exclusions
  ([11](11-2026-poker-research.md); [04](04-learner-experiments.md) §2;
  [arXiv 2608.06362](https://arxiv.org/abs/2608.06362)).
- **The live A/B cannot replace the sim gate.** At σ ≈ 3 bb after correction, detecting 10 bb/100
  needs ~14k hands per arm; at today's σ ≈ 10 bb, 157k ([04](04-learner-experiments.md) §3).
  The sims are the evidence; the live guard is a safety net.
- **The shared-deal bound** (note 08): rows on the same deals are correlated; the honest pooled
  interval is ~√k wider than the independent sum, and the lever is **deals**, not rows
  ([08](08-headline-measurement.md)).

### Compute and latency

- The live clock is 45 s; ADR 0001's decision cap is 8 s; v10's p95 was 325 ms, leaving most of
  the budget unused. A 1–3 s budget for large postflop pots is affordable on this box
  ([01](01-decision-system.md) §"What fits a CPU"). Five bots share six vCPUs.
- The box is CPU-only and shared with the live fleet; a deal costs ~0.8 s in the design's older
  note, and the sims are the long pole
  (`docs/superpowers/specs/2026-11-25-five-brains-design.md`, Constraints).

## The online pass, 2026-11-27

The repo's note 11 was written from the 2026 PDFs; this pass fetched the venue's live pages, the
SHARINGAN-2 and NoRegrets reports, and the arXiv abstracts directly. What changed:

### The venue's own field guidance: buy-in management and volume

- The venue's season guide says the 5,000 starting stack and 10/20 blinds make the default 2,000
  buy-in 40% of the stack; **top bots used 1,000-chip buy-ins early**, then moved to
  **3,000–4,000 after building a cushion**; repeated busts cost playing time and rebuild from a
  1,500-chip rebuy ([How Open Poker Seasons Work](https://openpoker.ai/blog/how-open-poker-seasons-work)).
- The same guide says **volume beats win rate**: a 28% rate over 800 hands is expected to beat 32%
  over 200. The fleet's three running bots and two stopped bots are a volume decision as much as a
  strength decision.
- This is a measurement to run, not a result: the fleet buys in at the default 2,000. N9 tests the
  1,000/3,000–4,000 policy in the sim and on a freed bot.

### The six-max blueprint: SHARINGAN-2's scale and its November 2026 leak

- 1.83e11 MCCFR iterations, six-player 100 BB, 200 buckets, **CPU-only on rented bare metal**. The
  3-vs-3 4M-deal duplicate test: +5.09 ± 1.16 mbb/game over the 2e10 checkpoint and +1.97 ± 0.97
  over 4.77e10 ([SHARINGAN-2](https://sharingan.poker/learn/sharingan-2/)).
- The turn/river search is optional, Rust, 200k iterations, over 1,326 hole-card pairs, four
  continuation strategies, **time-budgeted and unsafe**.
- **The action-translation leak:** an external bot bet below the smallest menu size; the old
  translation read it as a check; the model lost track and called indiscriminately, winning
  **+5.2 big blinds per hand** against the no-search blueprint. After the fix the exploit bot lost
  390 ± 156 mbb/game, but a stronger frozen attacker still won **+2,466 ± 75 mbb/game** over 1.2M
  hands without search. SvanBot's `legal.nearest` needs the same regression test (N10).
- The report's limitations: no human play, short best-response testing, tied to its abstraction and
  menu, turn/river search only.

### The learned leaf: NoRegrets' numbers and its foreign-opponent loss

- 200M iterations, 128.5M infosets, 101M strategies, 4.3 GB, **79 minutes on 16 cores**.
- At 800 ms, the learned leaf beat no-leaf by **+313 ± 198 mbb/hand** and the raw blueprint by
  **+331 ± 213**; without it search was neutral (−55 ± 495) and harmful at 150 ms (−520 ± 399);
  after exact turn solving the net's advantage fell to **+174 ± 100**
  ([NoRegrets](https://github.com/conorarmstrong/noregrets#results)).
- **The warning:** the raw blueprint was exploitable (BR +472, LBR +354), lost **−714.5 ± 331.5**
  to Slumbot over 10,000 hands, and search made it worse (**−1,771.0 ± 471.8**); the multiway LBR
  was **+929 ± 319**, reduced by only −186 ± 98 with search. Self-play strength is not live-field
  strength; N2 and N3 must test independent opponents, including the live field.

### Evidence: the 74× is not free

- AV-AIVAT: 71,439 paired HUNL hands, 15 agent configurations, **54× median variance reduction**,
  **74× fewer hands** at ±1 bb; the exact EB-CS needs an independently justified payoff bound and
  gives a **1.37×** stopping ratio on HUNL descriptively; the asymptotic CS covers screening
  ([arXiv 2608.06362](https://arxiv.org/abs/2608.06362)).
- "May the Best Bot Win" reports AIVAT's **1.72×** sample-efficiency gain offset by a **1.86×**
  simulation slowdown, and naive peeking producing 58–64% false exclusions against 2.4–3.4%
  ever-miscoverage for the asymptotic CS ([Zenodo](https://zenodo.org/records/23066106)); the
  sequential guard must be an e-process, and its cost is paid in compute.
- The GTO Wizard benchmark: **19.4 ± 4.1 bb/100** vs Slumbot, **ten times fewer hands** with
  AIVAT, and every tested LLM far below ([arXiv 2603.23660](https://arxiv.org/abs/2603.23660)).
  The venue's top 575 bb/100 is a weak-field number, not a GTO benchmark.

### The solver and abstraction papers

- **HS-DCFR / faster schedules** (AAAI 2026): training-free discounting schedules, tuned on three
  small games and carried to 17 including large poker
  ([arXiv 2404.09097](https://arxiv.org/abs/2404.09097)). A few lines in the DCFR loop; measure on
  SvanBot's tree.
- **CCS-MCCFR**: persistent Weyl streams, a one-line change; final exploitability −19.05% to
  −34.01% on Kuhn/Leduc, no measurable overhead; the full paper's four HUNL endgames found no
  detectable final-endpoint gain ([arXiv 2607.27035](https://arxiv.org/abs/2607.27035)).
- **Real-Time Parallel CFR**: 3.3–3.4× on a **single DGX Spark** (GPU batched inference; the
  abstract does not state CPU threads), 47–54 ms/iteration on a 1B+ history tree, HUNL postflop
  only ([arXiv 2605.19928](https://arxiv.org/abs/2605.19928)).
- **Embedding CFR** (AAAI 2026): information sets mapped to a continuous low-dimensional embedding
  instead of discrete clustering ([arXiv 2511.12083](https://arxiv.org/abs/2511.12083)).
- **Deep (Predictive) Discounted CFR** (AAAI 2026): a model-free neural CFR with variance-reduced
  sampled advantages and bootstrapping; the abstract claims faster convergence and stronger
  adversarial play but gives **no numbers**
  ([arXiv 2511.08174](https://arxiv.org/abs/2511.08174)). A neural dependency; not a first slice.
- **Higher-resolution abstractions for imperfect recall** (AAMAS 2026): the hand-abstraction line
  that keeps more history than outcome-based clustering
  ([AAMAS PDF](https://ifmas.csc.liv.ac.uk/Proceedings/aamas2026/pdfs/AKDO5185.pdf)).

### Opponent modeling

- StratFormer: **+0.106 BB/hand** over GTO on Leduc against six archetypes at two strengths, peak
  **+0.821** vs highly exploitable opponents, near-equilibrium otherwise; compute is not in the
  abstract ([arXiv 2604.25796](https://arxiv.org/abs/2604.25796)).
- SafeAdapt: the abstract gives **+42.7 mbb/hand** in abstract-HUNL cross-play; the full paper's
  interval and worst-opponent figures are note 11's reading
  ([arXiv 2610.20473](https://arxiv.org/abs/2610.20473)).
- AlphaExploitem and In-Context Exploiter are transformer variants; a transformer training
  dependency is not a first slice ([arXiv 2605.09150](https://arxiv.org/abs/2605.09150),
  [arXiv 2609.01549](https://arxiv.org/abs/2609.01549)).

The online pass confirms the direction and sharpens the risks: **the field-specific buy-in lever,
the off-tree action-translation test, and the foreign-opponent robustness of any blueprint are as
important as the blueprint itself.**

## The gap, in numbers

- **Chip rate.** #1 575 bb/100, #5 375, top-30 290, fleet 170–220. The fleet's hand-win rates are
  within 1.5 points of the leaders'; the chips per hand are not. The leaderboard's top is not
  winning many more hands; it is winning **bigger** hands.
- **Two candidate mechanisms.** The top may be winning bigger pots (N5) and choosing shorter
  buy-ins early (N9); the leaderboard cannot separate them. Both are cheap to test.
- **The current map's levers.** Opening width (tight 0.75 / loose 1.25) and the M3–M5 mechanics
  landed with **zero movement** in the Arena for the reference; their live effect is what the
  V11-69 guard watches. They are 5–10% levers on a 3× gap.
- **The engine's shape.** The 6-max game — the venue's only game — is priced, not searched; the
  equilibrium core is absent; the search is heads-up only; the leaves are hand-written. The 2026
  six-max systems all pair an **offline blueprint** with **online search**.
- **The evidence speed.** A 30-bout check at 16 minutes makes every roll-out and every search
  confirmation a day-scale wait. V11-137 is the prerequisite for the next-generation loop.
- **The live guard.** 3,000 hands per brain is ~1.5 days and, at 3,000 hands, is a safety net, not
  a strength test. Serial brains therefore cost ~1.5 days each even when the code is ready.

## The next-generation program

Scores are the author's reading of the cited evidence against this box's constraints; they are
proposals.

| Track | What | Evidence | Cost | Expected gain | Fit |
|---|---|---|---|---|---|
| **N0. Finish V11-59** | The five brains live, each checked and guarded | the approved design | ~1–2 weeks, mostly live waits | the base: per-bot policies and learning | required |
| **N1. Evidence loop** | Cache the frozen reference, use the idle cores, cut the per-decision setup; checks and pin arms in minutes | V11-137: 78% setup; 32 s → target ≤5 min | 1–2 sessions | every experiment and search step | prerequisite |
| **N2. Six-max preflop blueprint** | A 100-BB preflop abstraction, offline MCCFR, a per-bot lookup blended into the policy | E4 prototype GO; SHARINGAN-2's 183B scale; NoRegrets' 200M/16-core/79-min | weeks + a CPU window | a robust preflop base vs unknowns; cheaper preflop decisions | high |
| **N3. Bucketed multiway postflop search** | An 8–32 bucket abstraction; a bounded three-way river (E3 GO), then the turn; a depth-limited search in the live budget | E3 GO for a bounded three-way river; Pluribus' 1–33 s searches; the exact 1,081 combos now | months; the largest build | the 6-max core the venue actually plays | highest |
| **N4. Learned leaf table** | Fit a small leaf-value table per bot; compare it with the hand-written continuations at equal work | NoRegrets: +174 ± 100 mbb/hand with a leaf; none clear without | weeks | search quality at the same latency | medium |
| **N5. Sizing and value extraction** | Test bigger value lines and overbets against the field; measure the **chip rate**, not only the reference margin | the leaderboard's 575 vs 220 with similar win rates; M4 size menus exist | weeks; sim + a styled bot | the near-term chip-rate lever | high |
| **N6. League and exploiters** | Run the built round-robin, exploiters and per-bot champions as a population; PBT-style refresh | the machinery exists; [04](04-learner-experiments.md) §1–2; AlphaStar/PBT | ongoing | compounding per-bot strength; the five bots as a league | medium |
| **N7. Sequential live evidence** | AIVAT on our own decisions plus a harm e-process; a sequential guard that stops when decided | AV-AIVAT 74×; note 04's power; the current fixed 3,000 | weeks; an operator risk decision | shorter live waits; honest harm detection | medium |
| **N8. Data and compute** | The venue's hand-history export, season stats and per-bot backfill; a rented-CPU blueprint window; a 1–3 s big-pot budget | the venue's REST/export; note 01's budget; the box's six vCPUs | ongoing | the data and compute the engine needs | required |
| **N9. Buy-in and stack depth** | Test 50, 100 and 150–200 BB buy-ins; the venue's own guide says top bots used 1,000 early and 3,000–4,000 after a cushion; the fleet uses 2,000 | the venue's season guide; the score; the V11-69 guards | days; a sim and one freed bot | a cheap, field-specific chip-rate and variance lever | high, unproven |
| **N10. Off-tree action translation** | Prove any off-tree size, including below the smallest menu, is translated legally and never read as a check; add the regression test | SHARINGAN-2's November 2026 leak: a frozen attacker won +2,466 ± 75 mbb/game | days | removes a catastrophic live leak | required |

**Recommended order.** N1 now (it is already charted); N0's live rollouts in parallel; **N9 and N5
next** (both cheap; N9 is the venue's own field guidance, N5 attacks the measured chip-rate gap);
**N10 before any new sizing or blueprint** (the SHARINGAN-2 leak is a live-catastrophe class);
**N2** (the first engine slice, already a GO) with independent-opponent tests; then N3 (the big
one, with the NoRegrets warning built into its acceptance); N4 and N6 behind it; N7 and N8 as the
guardrails. The first next-generation map should be opened after V11-59 closes, with **N9 and N5**
as its first two tickets. The map's destination: *SvanBot reaches and holds #1 with a six-max
blueprint-plus-search engine and per-bot exploitation.*

## Finishing V11-59 in parallel

### The open tickets

| Ticket | State | Waits on | Fastest finish |
|---|---|---|---|
| V11-69 | live; guards 315/271 and 205/186 | elapsed hands (~34 h) | the stablemate duel, then 3,000 each |
| V11-82 | claimed; no search candidate; learner down | a candidate, then a promotion | restart the learner; the search, promotion and rollback |
| V11-90 | agent-handoff; deferred | the operator | run ~1–2 h of arms or close as subsumed |
| V11-130 | charted; blocked | V11-69 | Anchor/Hunter checks (minutes with N1), then 3,000 each |
| V11-131–134 | charted; blocked | V11-130 and V11-82 | the code can be built in parallel once unblocked |
| V11-135 | charted; blocked | V11-130, V11-133, V11-134 | the Chameleon check, then 3,000 |
| V11-136 | unblocked | — | one grilling |
| V11-137 | unblocked, released | — | 1–2 sessions |
| V11-138 | this note | — | close on the note |

### The critical path

1. **V11-69 guards** clear around **2026-11-28 14:00Z** at ~80 hands/hour (they run in parallel on
   Svanar and Svanism). The stablemate duel is a ~30-minute sim, not run yet.
2. **V11-82** waits on a search candidate. The learner is down; restart it. The search's early
   confirmations are failing (the fleet-wide log read the same challengers at −12.9 to
   −23.5 bb/100); a candidate may take days. The next-generation preflop blueprint (N2) is the
   likeliest source of a **stronger** challenger than the current one-knob ring.
3. **V11-130** checks Anchor and Hunter (minutes each with N1) and assigns them; each needs a
   3,000-hand guard (~1.5 days). SurSvan is free after V11-69; Hunter needs a free bot — the
   reserved SvanBotV11 or SuraGunnar, or a decision to delay Hunter.
4. **V11-131–134** are code slices. Once V11-130 and V11-82 close, 3–4 sessions can build them in
   parallel (kind, selector v1, selector v2, per-component guard); integration and V11-135 follow.
5. **V11-135** runs the Chameleon check and its 3,000-hand guard. The map closes around
   **2026-12-08–12-12** if the guards run in parallel and a search candidate appears.

### "All at the same time" — what is possible

- **The live guards are per-bot and can run concurrently** on different bots. The serial unit is a
  brain's own 3,000 hands, not the map.
- **The code can run on separate sessions.** The one-ticket-per-session rule is per session; the
  tracker's `agent-claimed` is the lock. Three to four sessions are the practical maximum on this
  box without starving the fleet.
- **The search is the serial unknown.** Until a candidate exists, V11-82 cannot finish; the
  learner must run, and the sims must not contend (N1).
- **The map's final live rollout is serial by brain**: a brain goes live only after the one before
  has run 3,000 hands under the guard. That is the design's safety rule, not a tooling limit.
- **The next-generation engine is a separate effort**, not parallel code to merge into V11-59. It
  can be *prototyped* now (N2/N5 are already GO or cheap), but it should be charted as its own map
  after V11-59, so the live map keeps one destination.
- **Two online findings are cheap and off the critical path:** N9 (the buy-in policy) needs a sim
  and a freed bot; N10 (the off-tree translation test) is a test. Neither touches the live map's
  evidence.

### The schedule levers

1. **Free SvanBotV11 and SuraGunnar** (operator memory, 2026-11-25: reserved for another bot trial).
   Two more live bots double the live volume for the fleet, give Hunter and the Chameleon their
   guards, and add two leaderboard rows. This is the single biggest schedule lever.
2. **Restart the learner and keep it running between checks.** V11-82 has no candidate because the
   learner has been down since 03:38Z.
3. **Serialize sims.** The recorded 58/56-minute runs were concurrent sims; the quiet-box check is
   ~16 minutes. N1 makes this cheaper still.
4. **A sequential live guard** (N7): the 3,000-hand fixed window is a safety net; a harm e-process
   can stop earlier when the evidence is decided. The operator's risk decision.
5. **Build the Phase 5 code ahead of the live blockers.** The code is behavior-neutral until a bot
   is assigned; building it on branches while the guards run saves the serial weeks. The map's
   blocking edges are process, not physics; the operator can allow code-only work ahead.
6. **Target the season boundary.** Season 17 ends 2026-11-29 11:48Z; the rollouts and guards cross
   it. The leaderboard resets; the per-bot stores do not. Plan the next-season starts accordingly.

## Decisions for the operator

1. **Season target.** Accept season 17 as out of reach for #1; aim the V11-59 rollouts at season 18
   and the next-generation engine at season 19.
2. **Free the reserved bots?** If the other bot trial is over, run SvanBotV11 and SuraGunnar. It
   unblocks Hunter/Chameleon guards and adds volume; if the trial still needs them, say when they
   are free.
3. **Restart the learner** and keep it on between checks; V11-82 is otherwise stalled.
4. **The live guard rule.** Keep the fixed 3,000 hands as the safety net, or chart N7's sequential
   harm e-process? The 3,000 hands cannot resolve a 20 bb/100 harm; it is a catastrophe net.
5. **The next-generation map.** Open it after V11-59, with N5 (sizing/value) and N2 (preflop
   blueprint) as its first tickets; N3 (bucketed multiway search) as the main build.
6. **A rented-CPU blueprint window.** The six-max blueprint is the scale SHARINGAN-2 and NoRegrets
   used; this box's six vCPUs are for the fleet and the search. The live box stays as it is.
7. **V11-90.** Run the remaining pin arms in one quiet window (~1–2 h) or close the ticket as
   subsumed by the engine work; the pin is a small lever against N3.
8. **This note.** Merge it on `v11-138-next-generation` and link it from the map's Decisions so far,
   or keep it as a research branch and let the next map cite it.
9. **The buy-in experiment (N9).** Approve a sim comparison of 50/100/150–200 BB buy-ins and a
   live run on a freed bot; the venue's own guide names the 1,000-chip opening.
10. **The off-tree translation test (N10).** Approve it before any new sizing or blueprint work;
   SHARINGAN-2's leak is a live-catastrophe class.
11. **The blueprint's independent-opponent validation.** Accept N2 and N3 only with the Arena's
   independent archetypes and the live field, not self-play alone; NoRegrets lost to Slumbot and
   stayed exploitable multiway.

## Sources

- Venue docs, revision 2026-09-02: <https://docs.openpoker.ai/llms-full.txt> (Game Rules, Seasons,
  Scoring, Timeouts, Bot Lifecycle, REST season and leaderboard); `crates/runtime/src/wire.rs:11`.
- Season and leaderboard: <https://api.openpoker.ai/api/season/current>,
  <https://api.openpoker.ai/api/season/leaderboard?limit=200&sort_by=score>, fetched
  2026-11-27T04:10Z. The table's chip rates are score/hands with a 20-chip big blind; the starting
  5,000 chips add ≈0.3 chips/hand over these samples and are ignored.
- The design: `docs/superpowers/specs/2026-11-25-five-brains-design.md` (phases, constraints, the
  margin rule, the long pole); ADRs 0001–0006.
- Research notes: [01 decision system](01-decision-system.md), [04 learner and experiments](04-learner-experiments.md),
  [08 the Arena's number](08-headline-measurement.md), [10 the luck of decisions](10-the-luck-of-decisions.md),
  [11 the 2026 research](11-2026-poker-research.md). The external citations are in notes 01 and 11.
- Live fleet: `~/.local/share/svanbot/fleet/{checks,searches,reference,arena}.json`, the guard
  output, `~/.local/state/svanbot/learn.log`, and the running processes, 2026-11-27T04:10–04:15Z.
- V11-137 timings: `arena 6-max 2 7 {1,20,live}` on the quiet box, 2026-11-27T04:05Z.
- The map V11-59 (#1190) and its open tickets #1200, #1227, #1236, #1314–#1322.
- Online primary sources, fetched 2026-11-27:
  [How Open Poker Seasons Work](https://openpoker.ai/blog/how-open-poker-seasons-work),
  [SHARINGAN-2](https://sharingan.poker/learn/sharingan-2/),
  [NoRegrets](https://github.com/conorarmstrong/noregrets#results),
  [AV-AIVAT](https://arxiv.org/abs/2608.06362),
  [Real-Time Parallel CFR](https://arxiv.org/abs/2605.19928),
  [StratFormer](https://arxiv.org/abs/2604.25796),
  [SafeAdapt](https://arxiv.org/abs/2610.20473),
  [CCS-MCCFR](https://arxiv.org/abs/2607.27035),
  [GTO Wizard Benchmark](https://arxiv.org/abs/2603.23660),
  [Deep PDCFR](https://arxiv.org/abs/2511.08174),
  [Embedding CFR](https://arxiv.org/abs/2511.12083),
  [HS-DCFR schedules](https://arxiv.org/abs/2404.09097),
  [AAMAS 2026 abstraction](https://ifmas.csc.liv.ac.uk/Proceedings/aamas2026/pdfs/AKDO5185.pdf),
  [May the Best Bot Win](https://zenodo.org/records/23066106).
