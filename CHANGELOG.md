# Changelog

v11 starts this changelog afresh. v10's history is at the tag `archive/v10/final`.

## 11.0.0 "The Deep Mass-ReWorkStructure": unreleased

A new codebase written from scratch (`docs/adr/0002-v11-from-scratch.md`), tracked as the map
V11-00.

- v10 archived. The code is at `archive/v10/final` and the 61 `archive/v10/*` branch tags. The store
  is a local archive that never goes to GitHub.
- Foundation: `GLOSSARY.md`, ADRs 0001–0002, the research notes behind the plan, the agent brief, and
  the Matt Pocock skills pinned to the repo.
- Workspace: six layers, `core` → `sim` → `strategy` → `learn` → `runtime` → `apps`, one crate per
  layer (ADR 0003). `clippy.toml` keeps I/O and the clock out of the pure layers, and
  `tools/layering.ts` checks the edges and the third-party crates.
- Gate: `mise run commit` runs fmt, Biome, the 500-line limit, the secret scan of staged changes,
  provenance, the layering check and clippy. `mise run full` adds nextest, the doc tests, `tsc`
  and the tool tests. Both build into `target/dev` at low priority. The hooks in `.githooks/` run
  the commit tier and check each commit message.
- Tools: `mise.toml` pins Rust 1.100.0, Bun 1.4.4, nextest 0.9.152 and Biome 2.6.7, and
  `tools/package.json` pins TypeScript 7.0.2 and `@types/bun` 1.4.4, each the latest stable
  release on 2026-11-15. `mise run setup` installs them all.
- CI: `.github/workflows/check.yml` runs the commit tier, the test tier and cargo-deny side by side
  on every pull request into `v11`, with the tools mise pins and our own crates cached. Every
  action is pinned to a commit.
- Disk: the dev profile keeps line tables only, and the full tier ends by reporting `target/`
  against a 9 GiB budget, failing over it.
- Cards: `svanbot-core` reads and writes the venue's two-character notation (`Ah`, `Ts`, `2c`),
  and a `CardSet` holds a board, a player's hole cards or what is left of the deck. With tests in
  the workspace, nextest now fails a run that finds none.
- Evaluator: the strength of the best five of five to seven cards, after OMPEval. Cards fold
  into a key by addition, so a board's key is made once and each player's two cards cost two adds
  and one table read. The tables are built in-process on first use. Checked against the published
  five-card counts (7,462 distinct strengths) and against a best-of-21 reference.
- Seeded generator: xoshiro256++ seeded through SplitMix64, written in `core`, with a stream per
  block index so the same seed gives the same draws on any machine and any thread count, and an
  unbiased `below(n)`. Checked against the reference implementation's published outputs.
- Rules of a hand: the blinds (heads-up the button posts the small blind), whose turn it is, the
  legal actions with raise-to ranges, short all-ins that reopen the betting only once they add up
  to a full raise, side pots, odd chips to the first winner left of the button, and uncalled chips
  returned. Checked by worked examples and 3,000 seeded random hands that must keep every chip.
- Equity: `svanbot-sim` enumerates every runout or samples seeded deals against weighted ranges,
  on 64 fixed blocks with a stream each. Shares count in whole sixtieths, so the same seed gives
  the same bits on any machine. Checked by counted outs, suit relabelling, and about 82% for aces
  against kings.
- Venue wire format: `svanbot-runtime` reads the venue's frames (turns with exactly the offered
  actions, hands, cards, seat lists, results, errors) into typed values, keeps frames it does not
  act on as `Other`, and writes actions that echo the hand and the turn token. Each case is the
  venue docs' own example (rev 2026-09-02).
- Pricer: the fallback decision in `svanbot-strategy`. It samples the bot's equity against each
  opponent's range (uniform until the action model arrives), raises for value well above its fair
  share, calls when the equity covers the price, and never folds a free check. Its work is a sample
  budget and a seed passed in, never the clock. `core::spot::Spot` is what it decides on.
- Session: `svanbot-runtime` turns venue frames into messages without a socket. It tracks the
  table and the hand, builds the spot, asks the decision it is handed, and plays the nearest
  offered action, never one the venue left out. Every seat list passes one seam, the fair-play
  guard: a stablemate at the table makes the bot leave and raise an incident, and a hand dealt
  with one is folded outright as a shared hand.
- The bot: `svanbot` connects to `wss://openpoker.ai/ws` with its key as a bearer token (rustls
  with ring), plays every turn by the pricer, reports fair-play incidents, and reconnects with a
  backoff. It reads its key, name and roster from the environment, never prints the key, and will
  not start without the roster.
- Action model: a probability for every move (fold, check, call, three bet sizes) by street,
  by whether a bet is faced and by strength bucket, with each opponent's range read from their
  line by Bayes. `learn` fits it on every stored hand, keeping a mucked hand as one that lost,
  and uses it only once it beats the baseline on held-out hands. Priors are fitted across
  opponents, and a drift monitor says when a model stops describing the play.
- Reads: one opponent's read tilts the population model by its evidence, inside the exploit
  budget. An opponent is classed as fixed, switching or learning, and a read is used only past
  the exploit gate.
- The re-solve: heads up after the flop, the rest of the street is solved by Discounted CFR over
  whole ranges, with one more round of betting past it played by continuation strategies. The
  policy re-solves where it can and prices everywhere else. Iterations are sized by the pot.
- The paired sim: `svanbot paired` plays two policies over the same deals from mirrored seats,
  with the luck of every card taken out, and gives the win rate with its 95% interval.
- The store and the season download: one append-only SQLite file per bot, which refuses any
  other bot. `svanbot download` pages a bot's own hand history into it, inside the venue's
  rate limits, and takes up where it stopped.
- The updater: CI builds and checksums a bundle on a `v*` tag. `svanbot-release` verifies and
  installs it, swaps the `current` and `previous` slots between hands, the canary first, and
  rolls back by itself when the new build starts three times in five minutes.
- The dashboard: each bot writes its status beside its store, and `svanbot dashboard` serves the
  fleet on loopback over one event stream, with one freshness indicator. The server is our own,
  on the standard library, and the page is hand-written and compiled into the binary: the fleet
  with one card a bot, one bot's page (its table, its ledger as a line chart with a table under
  it, what it plays by), and the release page (the slots, the builds installed, what each bot
  runs). It follows the OS light or dark setting.
- The Probe, live: every bot keeps a ledger of what each hand won (the stack's gain since the
  hand before, with the luck of the re-solve's own draws beside it) and plays by the fleet's
  champion parameter set. `svanbot propose` plays a challenger against the champion in a paired
  sim and, when the whole 95% interval is above 0, proposes it as a promotion. The canary
  (`SVANBOT_CANARY`, named by the operator) takes the proposal up between hands, plays one arm
  an orbit by a logged coin, and reads its verdict from a confidence sequence. A promotion runs
  a 3,000-hand harm guard on the canary first: cleared, the canary writes the fleet's champion
  and the other bots adopt it between hands; harmful, the champion stays. A restarted canary
  takes its probe up from its store.
- Agent workflow: `tools/hook.ts` holds an agent session to the rules a machine can check. It
  refuses a push to `main`, a forced push, a build outside `target/dev`, a release build and
  any start, stop or swap of a bot; it runs the layering check after an edit of a `Cargo.toml`;
  and it keeps a changed tree from being left failing the commit tier. `.claude/rules/` holds
  the rules of each part of the tree. `README.md` and `llms.txt` are written from the code.
