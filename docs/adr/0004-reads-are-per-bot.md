# Reads are per bot, and so is the store

The venue lets an owner run up to five bots, and it forbids same-owner bots from sitting together.
It does not say whether they may share what they learn about opponents. SvanBot takes the most
conservative reading of "related bots": **a bot's reads are its own**. What one bot learns about an
opponent never reaches a stablemate, so five bots play as five independent players.

## The rule

- **One store per bot.** Each bot keeps its hands and incidents in its own SQLite file
  (`~/.local/share/svanbot/<name>.db`). The store records its bot's name when it is made, in a
  `meta` row that triggers keep from changing, and `Store::open` refuses any other bot. Two units
  pointed at one file is a misconfiguration that would pool reads, and it fails at start.
- **A read comes from the bot's own hands alone.** A read is an opponent's departure from the
  prior (`GLOSSARY.md`). It is fitted from the hands in that bot's store, and from no other.
- **What may cross bots carries no opponent's identity.** The prior describes the whole population
  and may be fitted on every bot's history. So may the parameter set the learner promotes. A fit
  that crosses bots takes no opponent name, seat history or anything else keyed by opponent as an
  input, so nothing it learns is about one opponent.
- **No shared live state.** Bots run as separate processes (`svanbot@<name>`) with no channel
  between them. A bot learns what a stablemate saw only through the cross-bot fits above.

## Considered options

- **Pool reads across the fleet.** This gives more evidence per opponent and faster reads. But the
  venue counts same-owner bots as related, and pooled reads make five bots act on one shared
  picture of an opponent. That is too close to collusion to risk.
- **One shared store with a bot column.** Every reader would have to filter correctly forever. A file
  per bot makes the safe reading the only one that is possible.

## Consequences

- A bot new to an opponent starts from the prior, even when a stablemate has seen thousands of that
  opponent's hands.
- The season download (V11-13) fills each bot's own store, one bot at a time.
- The Arena and the learner may read every store, but only through fits that drop the opponent's
  identity.

## Status

accepted (2026-11-15, V11-01).
