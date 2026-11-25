# A bot's policy is its own

Until now the five bots played one champion and one profile, so they were one player five times
over, and a mistake in it played on all five. From here a bot may play a policy of its own: a kind
(the logic), a style (the character) and a champion (the numbers it learns for itself). A policy
plays live only after a recorded margin check shows it is not worse than the reference policy,
today's play, by more than the margin, and only while a live guard holds. A bot with no assignment
plays the reference policy exactly as before. The design is
[`docs/superpowers/specs/2026-11-25-five-brains-design.md`](../superpowers/specs/2026-11-25-five-brains-design.md).

## The decisions

- **A policy is a kind, a style and a champion,** and each bot may have its own. Kinds are compiled
  in `strategy`. A style pins some numbers and names a profile. A champion is the bot's numbers:
  the fleet's, until Phase 2 gives each bot its own.
- **A bounded cost.** A policy plays only while it is not shown worse than the reference policy by
  more than the margin, 20 bb/100 to start (the live harm guard's own). That is read bout by bout
  over the Arena's rows through a confidence sequence, and kept honest live against the control.
  Styles are designed, not found: a style that had to beat the champion to stay would fade back
  into it.
- **New rows do not gate a release.** Stablemate rows, and later the exploiters', are recorded
  apart. The ship bar reads the rows it reads today and no others.
- **The fleet-file rule widens.** Fleet files held parameter sets and never a read of an opponent
  (ADR 0004). They now hold parameter sets and policy names, which are no more a read than a
  number is.
- **It reverses V11-09's one fleet champion, in stages.** Phase 2 gives each bot its own champion
  and freezes the reference at the fleet champion of that day. Until then the fleet's search and
  promotion are unchanged.

## Considered options

- **Profiles only.** One engine, with bots differing by the compiled rules in front of it. The
  cheapest, but rules cannot change how the engine values actions or solves, and different logic
  was asked for.
- **A separate policy module for each bot.** The most independent, but the solver and the pricer
  are duplicated and every copy needs its own proof.
- **v10's lineages** (its ADR 0002), where a bot drifts apart only when its own search clears the
  gate. They were built and never switched on, and they hand nobody a style.
- **A live control of bots left on the reference policy, against a share of each bot's own orbits
  played on it.** The bots left are the control while some are; Phase 2 gives every bot its own
  standing reference arm, since no bot is left.

## Consequences

- Sims are the long pole. A margin check reads at least 30 bouts in both leagues for two policies,
  and 6-max costs the most.
- The ship bar's canary stage compares the canary with the rest of the fleet, which is sound only
  while they play the reference policy. It needs re-basing once bots are assigned: a Phase 2 ticket
  and the operator's decision.
- Five searches (Phase 2) mean five chances of a false promotion a round, each costing one bot and
  not five. The confirmation sim on fresh deals stays.
- A new number in the parameter set is not free: it breaks old `champion.json` files and digests
  and gives the fleet search a new knob. Phase 1 avoids it by pinning the opening width in a style.

## Status

accepted (2026-11-25, V11-59). It reverses V11-09's one fleet champion in stages and widens the
fleet-file rule of ADR 0004.
