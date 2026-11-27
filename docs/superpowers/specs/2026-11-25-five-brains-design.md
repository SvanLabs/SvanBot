# Five brains: a policy of its own for each bot

Generated-by: claude-code/claude-sonnet-5-5

Status: **approved by the operator, 2026-11-25.** It is the design of the map V11-59 (#1190). Its plans
are `docs/superpowers/plans/2026-11-25-per-bot-policy-phase-0-measurement.md` and
`docs/superpowers/plans/2026-11-25-per-bot-policy-phase-1-tracer.md`. Where a later ADR or the glossary
says otherwise, they win.

## The short version

- Every bot gets its own **policy**: a **kind** (the logic), a **style** (the character) and a
  **champion** (the numbers it learns for itself). The operator's word for it is "brain".
- Five brains, working names: **Anchor**, **Hunter**, **Grinder**, **Gambler**, **Chameleon**.
  Locked as the final names 2026-11-27 (V11-136, #1320): no rename in code or fleet files. The
  machine-readable style keys in play (`standard`, `tight`, `loose`, `anchor`, `hunter`) stay as
  they are; a brain's name is the prose label for a policy, not a fleet-file key.
- It is a program of six phases. Phase 1 is a thin tracer that proves the plumbing with two bots, one
  mechanic (opening width) and one new opponent (a stablemate). The later phases bring per-bot
  learning, a standing round-robin with stronger opponents, the engines, and adaptive style.
- Brains go live one at a time, each only after a recorded check against the reference policy and
  under a live guard. A bot with no brain assigned plays exactly as it does today.
- Sharper measurement starts first and runs beside Phase 1, because five brains multiply what the
  Arena must resolve.

## Intent

All five bots play one champion and one profile, so they are one player five times over. ADR 0004
already makes their *reads* independent: "five bots play as five independent players". This makes
their *play* independent, and gives each bot its own way to improve.

Why, as I read it (mine, not stated by the operator): independence in fact and not only in reads;
each bot specialising to the field it meets; more for the learner to search than one number; and a
failure in one logic no longer playing on all five bots.

## Where it starts

- The champion is a parameter set of one number, the exploit budget (`strategy::params`). The
  unattended search only nudges that number (`learn::search`).
- One policy plays everywhere: the profile's rules, then the re-solve, then the pricer
  (`strategy::policy::decide_profile`). The re-solve covers **heads up after the flop only**
  (`resolve` returns `None` otherwise). The preflop and every pot of three or more are priced, and
  the pricer takes its best action every time, with no mixing. One const profile serves every bot
  (`strategy::profile::ACTIVE`).
- The fleet shares one `champion.json`. The canary promotes it and the other four adopt it
  (`runtime::fleet`, `runtime::probing`).
- The only "styles" in the code are the five scripted Arena opponents, each deciding on one number,
  its equity (`sim::sparring`).
- v10 built per-bot lineages (its ADR 0002) and never switched them on.

## Goals, non-goals, anti-goals

**Goals**
- All five bots play their own policy, each learning its own champion.
- Five designed, named styles, each held to a bounded cost against the reference policy.
- Four kinds of stronger opponent, and a standing round-robin to prove the brains against them.
- Styles that adapt to the opponent and the table.
- Engines that differ in logic, not only in numbers.
- Every behaviour change test-first, with a paired sim quoted in its PR; all our own code.

**Non-goals**
- Changing the ship bar's rules for a release (its canary stage needs re-basing; see Traps).
- GPU work, or any third-party crate or package.
- Pooling anything about an opponent across bots.

**Anti-goals: a failure even if it technically works**
- Bots that look different but are worse than the reference by more than the margin.
- Any opponent read crossing bots (ADR 0004).
- A new row that silently blocks a release.
- Any change in a bot's play while no brain is assigned to it: today's Arena must match bit for bit.
- A brain that plays live on hope: none plays without a current, recorded check.
- A brain that is easier to beat than the reference: exploitability is measured, not assumed.

## Constraints (from the repo)

- `core`, `sim`, `strategy` and `learn` are pure: no clock, file, network or entropy; work limits and
  seeds come in as inputs; sums in a fixed order.
- No third-party crate or package. Decisions stay in-process (ADR 0001).
- Only legal actions reach the venue: every kind returns its action through the same nearest-legal
  step.
- Fleet files hold parameter sets and policy names, never a read of an opponent
  (`.claude/rules/runtime-and-apps.md`; ADR 0005 widens that rule from "parameter sets only").
  The learner never opens a store.
- Two stablemates never share a table; a stablemate row is the sim only.
- The box is CPU-only and shared with the live fleet: one build at a time, `nice -n 15 ionice -c3`,
  four jobs, in `target/dev`. A deal costs about 0.8 s (V11-00 map, on #1126), so sims are the long
  pole.
- The gate's tiers; Rust files under 500 lines; every commit and PR carries `Generated-by:`.

## Decisions

| Question | Chosen | By |
|---|---|---|
| What a style costs | Designed styles, kept at a bounded cost | operator |
| Opponents | All four kinds | operator |
| What differs between brains | Different logic, not only different numbers | operator |
| How big the first release is | Design for all five; go live in stages | operator |
| Build order | A thin slice through all three first | operator |
| How logic differs | Policy kinds plus profiles | operator (my recommendation) |
| The slice's mechanic | Opening-range width | operator (my recommendation) |
| The slice's opponent | A stablemate | operator (my recommendation) |
| Also in the design | Per-bot learning, a standing round-robin, adaptive style, advanced engines | operator |
| The five brains | Anchor, Hunter, Grinder, Gambler, Chameleon | operator (my sketch) |

## The five brains

| Brain | Kind (logic) | Style (character) | Needs built | Learns |
|---|---|---|---|---|
| **Anchor** | `Balanced` | The glossary's *baseline* made a brain: mixes its actions, blends borderline hands, exploit capped low, reads held. Hardest to exploit. | Mixing in the pricer and preflop (E1) | Its mixing spread and exploit cap |
| **Hunter** | `Exploiting` | Reads from the first evidence; a playbook per opponent type and class; the re-solve pinned to the read. | A playbook and a pinned re-solve (E2) | Its exploit stance and thresholds |
| **Grinder** | `Solving` | Tight-aggressive: narrow opens, value-heavy three-bets, small sizes, c-bets dry boards. | Opening width (M1), three-bet logic (M3), size menu (M4), texture c-bets (M5) | Widths, shares, sizes |
| **Gambler** | `Solving` | Loose-aggressive: wide opens, polarised three-bets, overbets, pressure. | The same mechanics, set to the other end | The same numbers, other ranges |
| **Chameleon** | `Adaptive` | Switches among the other four styles by opponent and table. | The four styles, a selector, a guard per component | Its selection priors |

Four kinds, five brains: `Solving` is today's engine and serves two. `tight` and `loose`, the styles
of Phase 1, are the first versions of Grinder and Gambler. Which bot plays which brain is the
operator's call at rollout; I suggest the canary, SurSvan, plays **Anchor**, the steadiest proxy for
the reference.

## The program, in six phases

Each phase is a set of small tickets, each test-first with a paired sim quoted where play changes, and
each leaves `main` shippable. Only Phase 1 is charted ticket by ticket now; the others are charted on
the map as the one before finishes.

| Phase | What | Needs |
|---|---|---|
| **0** Measurement | An AIVAT-style term in the paired sim and the ring; sequential reading everywhere; sim-to-live calibration | nothing; starts first |
| **1** Tracer | The policy seam, opening width, `tight` and `loose`, the margin rule, a stablemate row, assignments, a live guard | Phase 0's variance cut, for S8 only (or a longer sim) |
| **2** Per-bot learning | A slot, a search, a probe and a promotion for each bot; a standing reference arm; gene transfer; refresh; a frozen reference | Phase 1 |
| **3** Opponents | The round-robin; each bot's own field; new archetypes; the archive; the dashboard board | Phases 1, 2 |
| **4** Engines | `Balanced` (Anchor), `Exploiting` (Hunter), three-bet logic, size menus; exploiters; prototypes of a multiway re-solve and a blueprint | Phases 1, 3 |
| **5** Adaptive | `Adaptive` (Chameleon): selector, priors, a guard per component | Phases 2, 3, 4 |

Brains go live in the order they are ready: Grinder and Gambler after Phase 1, Anchor and Hunter after
Phase 4, Chameleon after Phase 5. Each only after its check is current and the stage before it has run
3,000 hands under the guard without tripping.

**Done when**

| Phase | Done when |
|---|---|
| 0 | The correction cuts the Arena rows' variance 2× or more and passes a zero-mean test, or the note says it did not and the operator decides; the margin check stops when decided (the round-robin and the search follow in Phases 3 and 2); the calibration slope is logged and printed by `svanbot calibration` |
| 1 | Two non-canary bots play `tight` and `loose`, each with a passing check on record and 3,000 hands under the guard; the stablemate duel is recorded; `Pricing` is on record as refused; today's Arena matches `main` bit for bit; the glossary and ADR 0005 are merged |
| 2 | All five bots have a slot, a search, a probe, a reference arm and a guard of their own; at least one bot has promoted its own champion through the whole path; the fleet-wide search is stopped and the reference frozen |
| 3 | The round-robin runs unattended and records the matrix, the archive and the regression rule; each bot has its gauntlet; the dashboard shows it |
| 4 | Anchor and Hunter each pass their check and play live; exploiters report each brain's exploitability; each prototype has a recorded go or no-go |
| 5 | Chameleon passes its check with all four components current and runs under per-component guards |
| Program | All five bots play their own brain, each current, each learning, each guarded |

### Phase 0: sharper measurement (prerequisite)

The pinned Arena run reads ±214,000 mbb/100 on 100 deals. The paired sim cancels the luck of the
cards: mirrored seats, shared run-outs, and the chance-node terms of AIVAT for every card that comes
(`sim::table::play_corrected`). It does not remove the luck of the decisions: our own mixed ones,
which the live probe already corrects (`learn::probe::corrected`), and the scripted opponents'.

- **AIVAT terms for decisions.** The decider returns the draw it made (`Drawn`), not only the action,
  so the sim can subtract each of our decisions' term, and each scripted opponent's, whose strategy
  is known. Our own draws exist only where the policy mixes, heads up after the flop, so at 6-max the
  opponents' rolls are the likelier gain: Phase 0 measures that before it builds it. Adopted only on a
  reproduced variance ratio of 2× or more on the Arena rows, and a zero-mean test (research note 04,
  section 5.9).
- **Sequential reading.** The margin check, the round-robin and the search read through
  `learn::probe::Sequence`, so a run ends when it is decided, not at a fixed count.
- **Sim-to-live calibration.** Every live verdict is logged beside its sim estimate. A slope whose
  interval excludes 1 says the Arena's opponents do not stand for the field (note 04, section 3).

### Phase 1: the tracer

1. **S0, docs.** Glossary, ADR 0005 (the policy's shape, the margin rule, rows that do not gate, the
   widened fleet-file rule) and the map (`domain-modeling`).
2. **S1, `strategy`: opening width.** A pinned number, `open_scale`, scales `Position::share()` (15% UTG
   up to 80% heads up), clamped to at most 1.0, threaded `policy` → `pricer::price_lines` → `opening` →
   `preflop::opens`. At 1.0 every decision is identical, the Arena's `pricer` row included.
3. **S2, `strategy`: kind, style, policy.** `PolicyKind::{Solving, Pricing}`; a style registry
   (`standard` 1.0, working values `tight` 0.75 and `loose` 1.25); a `Policy` value that decides.
   `Pricing` never re-solves; it exists so the seam is real and the rule can be seen refusing it.
4. **S3, `learn`: the margin rule.** Verdict predicates and the comparison of two policies' bouts.
5. **S4, `apps` and `learn`: the Arena plays a policy**, with stablemate rows in both leagues,
   recorded apart from the main board. A stablemate plays without reads.
6. **S5, `runtime`: assignments.** A fleet file of names, read between hands. No entry means today's
   play. The bot logs its policy at start and on every change, and `svanbot play status` lists the
   assignments; a status field and a trace field come with the dashboard ticket of Phase 3.
7. **S6, `apps`: `svanbot play check`, `assign`, `revert`.** `assign` refuses without a current check.
8. **S7, `runtime` and `apps`: the live guard.** Its own command. It reads results only (what each
   hand won, with its luck) through `Store::open_reader`, compares the bot with the control (the bots
   still on the reference policy) and reverts on harm.
9. **S8, operations.** Check `tight` and `loose`, assign them to two non-canary bots, watch the guard.

### Phase 2: per-bot learning

v10's ADR 0002 stages, on v11's seams:

1. **A slot per bot**: its own champion, starting as the reference's numbers with its style's pins
   applied, and falling back to the fleet's. No behaviour change while no slot exists.
2. **A search per bot.** `svanbot learn` round-robins over the bots that have a slot, one bounded step
   each, with a log keyed by that bot's champion. A sim plays that bot's kind and style, and the
   search moves only the numbers its style leaves tunable, within the style's limits.
3. **A probe and promotion per bot.** The probe machinery moves from "the canary" to "each bot, for
   its own proposals", one at a time. The harm guard runs on the bot being changed.
4. **A standing reference arm.** Each bot plays the reference policy on 10% of its orbits, so every
   bot has its own live control, from the same tables in the same hours. `probe::arm` already logs a
   chance other than one half, and the estimate weights by it. This replaces the control bots of
   Phase 1, which vanish as bots are assigned.
5. **Gene transfer.** A move that clears one lineage is offered to the others as a candidate, tested
   against their own champion by their own gate, and only inside their style's limits.
6. **Refresh.** A lineage dominated in the round-robin is reset to the best archived champion of *its
   own style*. v10 copied the winner; here that would erase the style.
7. **A frozen reference.** The fleet-wide search and promotion stop, and the reference is frozen at the
   fleet champion of the day. Each bot's own search replaces it. This keeps every check and every live
   control stable, and the operator can re-base the reference on purpose. A second ADR records it.

The numbers a style bounds now move into the parameter set. That is the migration the Traps warn of.

### Phase 3: opponents and the round-robin

The glossary already uses **League** for the table size, so the standing test is the **round-robin**,
and a brain's set of rows is its **gauntlet**.

- **Rows.** The scripted archetypes (shared), and new ones with sizing tells, a bluffer and a regular
  who mixes. Each other brain's current policy (stablemate rows). The bot's **own field**: a clone of
  the players it meets, fitted from its own store in its own process, where only identity-free fits may
  cross (ADR 0004). An **archive** of its last five champions. `pricer` and `v10.0.2` stay.
- **Regression rule.** A promotion may not lose to the last five archived champions at the 95% bound,
  and, once exploiters exist, may not raise exploitability past a bound the builder sets from the first
  measurements.
- **Cost.** Ten pairs heads up, the ring for 6-max, and the archive, all at low priority and read
  sequentially. The round-robin runs as its own supervised loop.
- **Dashboard.** The round-robin board, and each bot's brain, check and guard.

### Phase 4: engines

- **E1 `Balanced`.** The pricer draws among actions within a spread of the best, by a temperature in
  chips of the pot, and blends the opening boundary (a borderline class opens with a chance, not a
  cut). Exploit capped low, reads held, no profile.
- **E2 `Exploiting`.** A playbook per opponent type read from stats (station, maniac, nit) and per
  adaptivity `Class` (fixed, switching, learning, with the class's budget share), and the re-solve
  pinned to the read. The 30-hand evidence floor and the exploit gate stay the gate for any read in
  play.
- **Exploiters.** An exploiter is a policy of `Exploiting`'s kind, searched with the objective flipped:
  win rate against one brain's champion in the paired sim. The best one's win rate is that brain's
  **exploitability**, reported beside its win rate. Exploiter rows join the gauntlet here.
- **M3 three-bet logic, M4 size menus, M5 texture c-bets.** Knobs and rules for Grinder and Gambler.
- **E3 multiway re-solve and E4 blueprint: prototypes, not promises.** Each begins as a throwaway
  prototype (`prototype` skill) with a compute budget under the action clock and a go or no-go from
  the margin rule on the 6-max rows. A multiway re-solve over the eight strength buckets is the
  likelier win; a precomputed preflop blueprint with its own abstraction per bot is the costlier.

### Phase 5: adaptive style

- **Kind `Adaptive`** holds the four other styles as components and a selector.
- **Selector v1** is rules by opponent type and table: a station gets the Hunter's playbook, a maniac
  the Anchor's, a nit the Gambler's, an unknown the Anchor's. It chooses per orbit, never inside a
  hand.
- **Selector v2** is a bandit over components, with priors from the Arena's style by archetype matrix,
  updated by luck-corrected orbit results. A component whose margin check is stale is out; a component
  whose live interval is wholly below the harm margin is out.
- Each component is guarded as a brain is: a confidence sequence on its own orbits.

## The rules that hold it together

### The margin rule

A policy is held to the reference policy (kind `Solving`, style `standard`, the reference's numbers)
over the same Arena rows and deals, bout by bout (the pairing `svanbot paired compare` uses), read
through a confidence sequence.

- **Admitted** to live play when the interval is narrower than the margin and not wholly below minus
  the margin. The margin starts at 20 bb/100, the harm guard's own, so the first measurement is
  affordable, and tightens as the Arena sharpens.
- **Kept** while the live guard holds. Phase 1: the bot's luck-corrected result less the control's,
  over the same window, not wholly below minus 20 bb/100 after 3,000 hands. Phase 2 on: the same, on
  the bot's own reference arm.
- Until Phase 2 freezes the reference, a promotion of the fleet champion makes every recorded check
  stale. A stale check does not unassign; it blocks the next assignment until it is run again.

### What the ship bar reads, and the kill switch

The ship bar keeps testing the reference policy, as it does today, and no brain's rows. A release does
not change a bot's assignment. The kill switch is the fleet file of assignments: remove it, and every
bot plays the reference.

## Traps the builder should know

- **A new number in `Params` is not free.** `params_from` refuses a `champion.json` that lacks a
  number or whose digest differs, the digest hashes every number, and `search::next` moves every
  number in `named()`. So a new number breaks old files and old digests, orphans the search log,
  proposals and Arena history keyed by the old digest, and hands the fleet search a new knob. Phase 1
  avoids all of it by pinning `open_scale` in a style. Phase 2 must keep old files readable and the
  digest of an unchanged set stable; hashing only the numbers that differ from their defaults
  (`exploit` always) does it.
- **A new row would gate a release.** `Opponent::all()` feeds both the scoreboard and every heat of the
  ship bar, and `runtime::ship::bar` refuses any row but `v10.0.2` that is wholly below zero. New rows
  stay out of `all()`, go in their own run record, and the ship bar does not read them. A test proves
  its output is unchanged while such a record exists.
- **The canary stage reads a mixed fleet.** `release-check <tag> fleet` compares the canary with the
  rest of the fleet's export. It is sound only while those bots play the reference policy. Once bots
  are assigned it must be re-based, for example on the canary's own hands before and after the
  release, or by reference arm. That is a ticket of Phase 2, and a decision for the operator.
- **Five searches share one box.** `svanbot learn` steps once per half hour, so five lineages step a
  fifth as often unless the CPU grows or Phase 0 sharpens the sims. v10 accepted the other price: five
  false-promotion chances a round, each costing one bot and not five. The confirmation sim on fresh
  deals stays.
- **The `pricer` row must not move.** It is the same code as the `Pricing` kind, called with the
  default width.
- **CI's Arena job stays as it is.** It publishes the default heads-up board only; policy runs are
  box-only, and row names stay inside `[A-Za-z0-9 -]`, the charset the publication accepts (V11-36).
- **The learner never opens a store.** A bot's own field is built in that bot's process or by a
  per-bot command, never by `svanbot learn`.
- **`ACTIVE` is a const.** Live play (`main.rs`), the Arena (`deciders.rs`, `arena.rs`) and the
  profile-stats gate all read it; all must take the style's profile.
- **The sim's decider returns an `Action`.** The correction needs the draw, so the seam changes, and
  every scripted opponent needs its decision distribution exposed.
- **Opening width is a narrow lever.** It moves how often a bot opens an unopened pot; calls and
  re-raises are still priced. It suits the tracer (visible, cheap) and is not yet a whole style.

## Calls made in the design, for the operator to overrule

1. Phase 1 puts `tight` and `loose` on two non-canary bots; the three others stay on the reference
   policy as the control until Phase 2.
2. Anchor is the glossary's baseline made a brain. SurSvan plays Anchor when it is ready.
3. Four kinds: `Solving`, `Balanced`, `Exploiting`, `Adaptive`. `Pricing` exists only in Phase 1, to be
   refused.
4. The standing reference arm is 10% of a bot's orbits, from Phase 2.
5. The reference is frozen from Phase 2, and the fleet-wide search stops.
6. Refresh never copies across styles.
7. Multiway re-solve and the blueprint start as prototypes with a go or no-go.
8. The margin covers a bot's whole policy and starts at 20 bb/100.
9. In Phase 1 a style pins `open_scale`; it is not a number the learner tunes until Phase 2.
10. Stablemate rows are recorded apart, are not in `Opponent::all()`, and do not gate a release; a
    stablemate plays without reads.
11. Assignments live in a fleet file of names, read between hands, not in a bot's `.env`: the Arena
    needs them and must not read `.env` files.
12. "Round-robin" and "gauntlet" avoid the glossary's **League**; the reference *arm* reuses its **Arm**.
13. The ship bar tests the reference policy only.
14. Exploiters wait for the `Exploiting` kind, so they land in Phase 4, not Phase 3.
15. The five brain names stay as above (locked 2026-11-27); no style-key migration.

## Left to the builder

File and function names; exact signatures; the clamp on a share; the temperature and spread scales;
how many bouts a check needs; run-record formats; the status field name; the exploitability bound; how
the working style values are tuned after the first Arena run; the selector's contexts.

## Size

A program, not a ticket: a new map of roughly fifty tickets in six phases. Phase 1 is nine tickets and
is charted in full; Phases 2 to 5 are charted as each before it finishes, and the map's "Not yet
specified" says so. The long pole is sim time on a shared box, not code, which is why Phase 0 starts
first.
