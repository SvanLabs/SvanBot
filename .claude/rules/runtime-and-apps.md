---
paths:
  - "crates/runtime/**"
  - "crates/apps/**"
  - "deploy/**"
---

# Runtime and apps

These are the two layers that touch the world: the venue, the store, files and the clock.

- **Read the venue's docs first:** <https://docs.openpoker.ai/llms-full.txt>. Check its revision
  line against the one `crates/runtime/src/wire.rs` names. Where the docs and v10 disagree, the
  docs win. A wire test uses the docs' own example frame.
- **Only legal actions reach the venue,** with the venue's hand and turn ids echoed, inside the
  action clock. The session plays the nearest offered action, never one the venue left out.
- **Fair play:** every seat list passes the one seam in `session`. Two stablemates never share a
  table, and a shared hand is an incident: kept in the store, logged at crit, never dropped.
- **A store belongs to one bot** (`docs/adr/0004-reads-are-per-bot.md`). Its tables are
  append-only, with one named exception: `decisions` is capped at `TRACED_HANDS` hands and the
  cap deletes the oldest, because an unbounded trace would grow until the disk ran out
  (`store/trace.rs`). Nothing else deletes or rewrites a row. What bots share goes through the
  `fleet` files and holds parameter sets only, never a read of an opponent.
- **Keys are never printed,** and nothing from a store, a status file or an `.env` goes into the
  repo, a log line quoted in an issue, or a PR.
- The clock and fresh entropy live here and are passed down as inputs. The decision stays
  in-process (`docs/adr/0001-decision-path-stays-in-process.md`).
- Agents run the fleet: start, stop and swap a bot with `scripts/` and `svanbot-bot`
  (`deploy/README.md`, V11-54). Never start a bot with a key another connection plays: the new
  connection takes over that bot's live session.
