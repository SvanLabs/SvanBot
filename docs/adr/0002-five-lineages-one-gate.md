# Five bots, five lineages, one gate

The shared champion is the fleet's single point of strategy: one bad promotion plays on all five
bots at once, one search feeds all five, and with the search finding gains rarely now (of the last
40 recorded experiments 39 were rejected in search and 1 promoted; lineage `sv10-ev-53`) the fleet
has one slow path to get better. The operator asked for a design in which every bot evolves, on
short, medium and long time scales, using the machine's 12 cores. This records what we decided to
build and in what order. It changes no gate.

## Status

accepted (2026-10-04, #764, with the operator). Built in stages; each stage is its own change.

## Decision

Every fleet bot gets a **lineage**: its own parameter set and its own promotion history. All five
start as copies of the current champion, so nothing changes at the start, and a lineage diverges
only when its own search clears the existing promotion gate against its own parent (95 % lower
bound of at least +1 bb/100 on fresh deals, paired, all-in luck removed; `promotion.rs`, unchanged).
Evolution then works at three speeds:

**Short (every cycle, hours).** The one-knob search, successive halving and confirmation of the
learner, run per lineage. The five searches share one pool of about eleven idle-priority threads
(of 12 cores; the fleet plays first) by interleaving search steps round-robin, rather than five
separate pools: the search is already a persisted stage machine (`learner/search/stages.rs`), so
interleaving is a scheduling change. A lineage's cycle is slower than today's, the total search
throughput is higher (all cores instead of eight threads), and no turn on the live path waits for
it.

**Medium (days): gene transfer and tournament refresh.**
- *Gene transfer.* A transition (one knob, old to new) that clears the gate for one lineage is
  offered to the other four as a priority candidate in their next search. It is not copied: the
  adopter tests it against its own parent with the same gate, so a find that only fits one bot's
  table mix is rejected where it does not fit. This replaces rediscovering one improvement five
  times.
- *Tournament refresh.* Every few days the five parents play a paired round-robin on identical
  deals. A lineage dominated by another at the 95 % lower bound is replaced by a copy of the
  winner, and the replaced parameter set is kept as an archived branch (`archive/` rule). This is
  population-based training's exploit step, taken through the gate, and it stops a lineage from
  staying on a dead end.

**Long (weeks): structure.** What a lineage can change grows by reviewed changes, not by the search
editing itself: new knobs join the catalogue (`raise_gate`, `hero_image` and `preflop_discount`
did on 2026-10-04), the learned response model is retrained per lineage on that lineage's own
hands, and opponent-model fits stay shared (the data is shared) with per-lineage read depth as a
later option.

## What does not change

The gate and every number in it; the legal-action and echoed-token invariants; the 500-line and
one-layer-per-change rules; shared opponent data (models, tallies, reputation book); the
single-process decision path (ADR 0001). The protected trio and the experiment pair, which assume
one champion, are retired while more than one lineage exists (see consequences).

## Considered options

- **Keep champion plus pair** (the smallest change): rejected; one lineage, one slow path.
- **Widen the pair** (more parallel candidates, one lineage): rejected; still one lineage and the
  pair is two bots.
- **Five lineages with plain copying between them**: rejected; copying skips the gate and spreads a
  false positive to all five.
- **Time-sliced live A/B on one seat**: rejected as before (#715); a single seat has no board and
  worse statistics than the paired simulations that already gate.

## Consequences

- Five false-promotion chances per round instead of one (5 % each at the gate), each hurting one
  bot rather than five. Accepted: the loss is bounded to a seat and the gate is unchanged.
- Experiment mode (protected trio, experiment pair) cannot be defined with five different
  champions and is retired for as long as lineages differ; the live confirmation it gave is
  replaced by the tournament refresh's paired evidence.
- Live play reads the lineage of its own bot instead of one shared parameter set (`live.rs`), the
  promotion ledger (`search_ledger.rs`) and the lineage record become per-lineage, and the
  dashboard shows five lineages instead of one champion.
- Success is measured, not promised: promotions per week per lineage against today's, and the
  fleet's paired bb/100 against the single champion on the same deals. Ten times the compute does
  not give ten times the strength: a paired simulation of ten times the equity samples measured
  +0.27 bb/100 (95 % -1.90..+2.44) over 480,000 hands (#758), so the extra cores go to more
  lineages and more candidates, not to deeper search.

## Build order

1. Per-bot parameter sets in the live state, read from `params.slot.<bot>` and falling back to the
   shared champion: no behaviour change while no slot key exists.
2. Lineage-aware learner: per-lineage ledger, promotion and lineage record; interleaved search;
   all lineages start as copies.
3. Gene transfer.
4. Tournament refresh.
5. Dashboard lineage panel; retire experiment mode while lineages differ.
