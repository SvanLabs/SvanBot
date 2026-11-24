# Changelog

v11 starts this changelog afresh. v10's history is at the tag `archive/v10/final`.

## 11.1.7: unreleased

- An agent that stops mid-ticket hands it over (V11-53): it pushes, posts the note from
  `bun tools/handoff.ts <ticket>`, and labels the ticket `agent-handoff`, so that Codex, opencode
  or another Claude session takes it over from the branch. `AGENTS.md`, "Handing off".

- `scripts/update.sh <version>` rolls a release out to the whole fleet in one run (V11-50):
  - installs it if needed;
  - swaps the canary, then each other bot one at a time;
  - skips a bot already on the build, so a stopped run can be run again;
  - stops at the first bot that does not take it;
  - restarts the dashboard.

  `--canary-only` stops after the canary.
- The disk budget counts every `target/` in the clone, worktrees' and nested crates' included,
  not only the root one, and names each tree when the clone is over it (V11-48).

## 11.1.6: released 2026-11-24

- The learner searches for a challenger unattended (V11-49). `svanbot learn` does one step a
  run from a low-priority timer, `deploy/svanbot-learn.timer`:
  - Each number is moved a step down and up, in rings of shrinking steps, one paired sim a
    step, on a seed named by the champion and the challenger, never run twice.
  - A challenger is proposed only when a first sim and a confirmation on fresh deals both put
    it above the champion. It still goes through the canary's 3,000-hand harm guard.
  - The search waits, simming nothing, while a proposal waits or a probe runs. It never
    replaces a waiting proposal.
  - A proposal names the champion it was measured against, and the canary drops one whose
    champion has since been replaced.
  - The canary mirrors its running probe in `fleet/probe.json`, and the search keeps its log
    in `fleet/search.json`. Both hold parameter sets and sim numbers only.
- `scripts/start.sh` and `scripts/stop.sh` bring every configured bot and the dashboard up and
  down, under the updater's lock (V11-46).
- Arena history from before champion digests is kept, and the ship bar reports its other
  clauses beside an unknown champion (V11-47).

## 11.1.5: released 2026-11-24

- The dashboard is remade to a 2026 standard (V11-45): a near-black ground under a slow aurora,
  glass panels with a hairline gradient edge, one violet-to-cyan accent, and the platform's own
  sans with tabular figures. A floating header carries the bots as chips; each page opens with a
  large title and its state; hero tiles carry an icon and flash when their number moves; charts
  fill to zero; each table glows on its rail, with seat plates ringed by what the bot reads of the
  player. Ctrl/⌘ K (or `/`) opens a command palette that jumps to any bot, page, view or look,
  and the dashboard's own links move without a reload. Motion stops when the OS asks for less.
- The dashboard closes a connection that sends nothing without answering it. A browser opens one
  before it needs it, and the "bad request" it used to get after five seconds was shown as the
  page.

## 11.1.4: released 2026-11-24

- The dashboard's control room is a board (V11-44): panels in three columns that the reader
  arranges, hides and folds, shown a view at a time (Live, Opponents, Results, Learning, System,
  All), as v10's were. Each bot's table shows what the bot reads of each opponent at it, in an
  arena, felt or midnight look. New panels: the season board from the venue's public leaderboard,
  a bot's season standing, opponent intel, highlights, the activity log, notes, and the Arena's
  latest runs. The page redraws only what changed, so a note being typed or an open fallback
  survives the stream. `SVANBOT_SEASON=off` keeps the dashboard off the network.

## 11.1.3: released 2026-11-23

- The dashboard reads like v10's control room (V11-44): a tab per bot, the workspace bar, hero
  tiles, the signal rail, every bot's table live, and the fleet race.

## 11.1.2: released 2026-11-23

- The dashboard is a dark poker console, with each bot's table drawn on the felt (V11-44).

## 11.1.1: released 2026-11-23

- A call is priced against every opponent still in the hand again (V11-42). Pricing it against
  the players behind as they fold, call or squeeze lost to v10.0.2 at 6-max: v11.1.0 − main was
  −18,640 ±16,727 mbb/100, and this build −8,227 ±16,449, over the same 2,000 deals. Raises keep
  their fold equity, and unopened pots their opening range.

## 11.1.0: released 2026-11-23

Win rate against the field, at every table size (V11-39).

- The pricer values each legal action in chips over the deals it samples. A raise is worth the pot
  when the table folds, plus the showdown against whoever calls. Each opponent folds as their
  action model folds their hand to a bet, more often the bigger the bet. A call is answered the
  same way by the players still to act. Raises come in the table's standard sizes.
- The preflop table: each of the 169 starting-hand classes' equity against one to five random
  hands, measured by our own sampler (`svanbot tables preflop`) and shipped in the binary. An
  unopened pot is opened with the opening range of its position, from 15% of hands at UTG to 80%
  on the heads-up button.
- The Arena plays 3-, 4- and 5-handed leagues beside heads up and 6-max, each on its own board.
  `svanbot paired compare` reads two runs on one seed and gives each row's paired difference.
- Chips are counted in 64 bits (`core::rules::Chips`).
- The release is built for x86-64-v2, the box's instruction level, and says so in `UPGRADE/cpu`.
  `install.sh` refuses a bundle built above the CPU it runs on.
- `Params` keeps `exploit` alone. `value_margin` and `raise_share` sized the old pricer's raises.
- The dashboard listens on port 5000 (V11-41).
- The README is rewritten, with `CONTRIBUTING.md`, `SECURITY.md`, a docs index, issue and PR
  templates, and a GitHub Pages site in `site/`, published by `pages.yml`. `tools/site.ts` checks
  every link in the commit tier.

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
- The headline: `tools/headline.ts` writes the README's one number from the Arena run pinned in
  `fleet/headline.json` — the pooled mbb/100 across every opponent, weighted by deals, with the
  pooled 95% interval that says whether it can be told from zero. Because the Arena plays one set
  of deals against every opponent, the interval is the shared-deal bound — the weighted mean of
  the rows' intervals, not a sum that would buy confidence from one sample seen six ways — so a
  100-deal run reads "not yet distinguishable from zero" and only clears at a few hundred shared
  deals a line (about 281 at the pinned effect size), which a release run has. A run clear of
  zero below says it is behind; a row with no measured interval leaves the
  pool unmeasured rather than exact; the headline counts deals once, and figures stop at three
  significant ones. The block names its
  commit, champion and seed, and says what the league is not. The commit tier fails a README that
  drifts from the pin, refuses a pin whose commit the repository does not hold, and refuses a
  cloned tree with no pin; a run is pinned on `v11` from the `arena` artifact a pull request's CI
  left, never from the pull request itself.
- A hand's trace and its frames each keep the cards, and `Hand::cards_disagree` compares them: the
  trace's copy is what lets the replayer work without a frame parser, the frames are the venue's own
  JSON, and a drift between the two would otherwise be invisible. Cards that arrive inside a
  reconnect's `resync_response` — its replayed events and its snapshot — count like any other, so a
  clean reconnect is not a false alarm and a reconnect that replays a different board still is one.
  A live hand that disagrees says so at crit priority and is still kept, since losing it would hide
  more than it fixes. A hand whose frames name no cards has nothing to compare and compares equal.
- The decision trace names a range by **seat**: the spot carries the seat each opponent's line came
  from, and the trace, the store and the page keep it, so a reader matches a range to a seat in the
  table view rather than to a position in a list. A spot built without seats (a sim's own table, or
  a caller that did not follow the hand) reads as an unknown seat, never as seat `0`.
- Agent workflow: `tools/hook.ts` holds an agent session to the rules a machine can check. It
  refuses a push to `main`, a forced push, a build outside `target/dev`, a release build and
  any start, stop or swap of a bot; it runs the layering check after an edit of a `Cargo.toml`;
  and it keeps a changed tree from being left failing the commit tier. `.claude/rules/` holds
  the rules of each part of the tree. `README.md` and `llms.txt` are written from the code.
