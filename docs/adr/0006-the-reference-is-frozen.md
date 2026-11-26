# The fleet reference is frozen

From Phase 2's cutover the fleet's reference is the champion in play on the day, frozen on purpose,
and the fleet-wide search and promotion are retired. Each bot's own learning moves only its own
champion. The design is
[`docs/superpowers/specs/2026-11-25-five-brains-design.md`](../superpowers/specs/2026-11-25-five-brains-design.md),
Phase 2 item 7, and follows ADR 0005's promise to freeze the reference once bots have champions of
their own.

## Context

Until the cutover one champion in `champion.json` is every bot's champion, and `svanbot learn` and
the canary's probe move it. Every margin check is recorded against that champion, and the ship bar's
Arena stage holds a run to it. Phase 2 gives each bot a champion slot in `champions.json`, seeded
from the reference and then learned by that bot alone. A fleet reference that keeps moving would
make every recorded check stale, move every live reference arm, and make the canary ship stage read
a fleet that plays several policies. The reference has to stop where the per-bot learner starts.

## The decisions

- **Freeze once, on purpose.** `svanbot play freeze` writes the champion in play to
  `fleet/reference.json`: its numbers, its digest and its cutover time. `Fleet::champion` reads
  this file first; a bot without a slot plays the frozen reference. A second freeze is refused;
  moving it is a rebase.
- **The fleet-wide path stops.** While `reference.json` exists:
  - `svanbot learn` sims nothing and records that the reference is frozen; every later run,
    including the timer's next process after a restart, reads the file and does the same.
  - `Fleet::propose`, `Fleet::propose_if_free` and `Fleet::promote` refuse, and a proposal waiting
    at cutover is dropped.
  - A canary that has a fleet-wide probe in flight, or reopens one from its store, ends it as
    `frozen` and promotes nothing. Its own slot's champion is unaffected.
  - The per-bot search, proposals and promotions in Phase 2 items 2 and 3 are not this path: they
    move `champions.json` one bot at a time and carry on.
- **A rebase is the operator's move.** `svanbot play rebase <name>=<value> …` requires a frozen
  reference, writes the new parameter set and cutover time, and keeps the reference it replaced,
  newest first, in the same file. A rebase changes the baseline; it does not restart the
  fleet-wide search, which stays retired.
- **The ship check stays as it is.** `svanbot release-check` reads `Fleet::champion` and so reads
  the frozen reference. The Arena stage, the replays and the canary stage keep today's rules. The
  canary stage compares the canary with the rest of the fleet's export, which is sound only while
  those bots play the reference. Re-basing it, on the canary's own hands before and after the
  release or on each bot's standing reference arm, waits for the operator to approve the new
  baseline and its replayed evidence. Until then its mixed-fleet reading is a known limitation, not
  a reason to loosen the check.

## Considered options

- **A frozen flag in `champion.json`.** One file would hold both the numbers and whether they may
  move, so a promotion could silently unfreeze the reference. A separate file makes the freeze and
  its cutover time the record, and keeps the champion read unambiguous.
- **Delete `champion.json` at the freeze.** This hides the last fleet champion and breaks tools
  that still read it. The file stays, ignored while the reference exists.
- **Freeze automatically when the first slot is seeded.** A cutover is the operator's decision with
  a time and an approved baseline, not a side effect of seeding a slot.
- **Write `reference.json` by hand.** It would carry no verified digest and no recorded history.
  The file records both, and the CLI is the only intended writer.

## Consequences

- Every margin check recorded against the frozen digest stays current until an operator rebases;
  a rebase makes those checks stale on purpose.
- A bot without a slot, the canary included, keeps playing the frozen reference. A bot's own slot
  promotion leaves the reference and every stablemate's play untouched.
- `reference.json` holds parameter numbers, a digest and times, which are no more a read of an
  opponent than `champion.json` was (ADR 0004).
- Removing `reference.json` by hand unfreezes the fleet and would let the fleet-wide search run
  again. That is the escape hatch before per-bot learning is trusted, not a substitute for a
  rebase; after the cutover a rebase is the supported move.
- The canary ship stage's mixed-fleet caveat remains until the operator approves the re-based
  baseline and its replayed evidence.

## Status

accepted (2026-11-26, V11-78). It implements ADR 0005's Phase 2 freeze at the point where the
per-bot learner takes over.
