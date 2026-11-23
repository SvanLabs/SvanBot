# SvanBot

SvanBot plays no-limit hold'em on openpoker.ai with several bots under one owner, learns from
every hand they play, and changes how it plays only when the evidence says the change wins.

## Language

### The venue and who sits at it

**Venue**:
openpoker.ai, the site where our bots play, and the only source of hand history.
_Avoid_: server, site, platform

**Owner**:
The one venue account that registers all of our bots.
_Avoid_: account, user, operator (the operator is the person who runs SvanBot)

**Bot**:
One player identity under the owner, with its own name, key and leaderboard rank.
_Avoid_: agent, instance, seat

**Seat**:
One numbered place at a table. A bot's identity is not its seat and a seat is not its identity: the
same bot sits in different seats, and a seat is held by different bots. A range in a decision trace
names its opponent by seat, because a seat is what a reader can find on the table.
_Avoid_: position, index, ordinal

**Roster**:
Every bot the owner has registered, under every name each one has played as.
_Avoid_: fleet names, bot list

**Alias**:
A name a bot played under before. It still means the same bot.
_Avoid_: old name, previous identity

**Fleet**:
The roster's bots that are playing live right now.
_Avoid_: swarm, cluster

**Stablemate**:
Another bot on the roster, seen from one bot's point of view.
_Avoid_: sibling, teammate, friend

**Opponent**:
Any player at the venue who is not on the roster.
_Avoid_: villain, enemy, rival

**Season**:
One leaderboard period at the venue. Ranks, hand counts and the hand-history export all reset with it.
_Avoid_: round, epoch

### Fair play

**Shared table**:
A table where two stablemates are seated at the same time. It must end before a hand is dealt.
_Avoid_: collision, overlap

**Shared hand**:
A hand that two stablemates were both dealt into. It is always a fair-play incident, never a
duplicate to drop.
_Avoid_: duplicate hand, double row

**Fair-play incident**:
Something that broke the venue's fair-play rules or came close to it. Each one is raised to the
operator and kept.
_Avoid_: warning, glitch

### Play

**Hand**:
One deal, from the blinds to the pot being awarded.
_Avoid_: game, round

**Session**:
One bot's unbroken stay at one table.
_Avoid_: sitting, visit

**Orbit**:
One full turn of the button around the table, so every seat posts each blind once.
_Avoid_: round, lap

**Strength**:
How a player's best five cards rank at showdown. The higher strength wins, and equal strengths
split the pot.
_Avoid_: hand value, hand rank (a rank is a card's)

**Spot**:
A point where a bot must act. It is fixed by the street, the board, the positions, the stacks, the
pot and the action so far.
_Avoid_: situation, node, state

**Decision**:
The action a bot takes at a spot, with its size, and the reasons it was chosen.
_Avoid_: move, choice

**Action clock**:
The time the venue allows a bot to act at a spot before it acts for the bot.
_Avoid_: timeout, timer

**Resync**:
How a session survives a dropped connection: the bot asks the venue for every event after the last
one it applied, then takes the table as the venue's snapshot shows it. The action clock keeps
running meanwhile.
_Avoid_: catch-up; a reconnect is only the new socket

**Win rate**:
Chips won per hand, in milli-big-blinds per 100 hands (mbb/100).
_Avoid_: profit, score, bb/100 (when mbb/100 is meant)

### Reading opponents

**Range**:
The probability of each pair of hole cards an opponent could hold at a spot.
_Avoid_: hand range, weights, holdings

**Action model**:
How likely an opponent is to fold, check, call or bet each size at a spot, given their hole cards.
A range follows from it and the actions seen so far.
_Avoid_: showdown fit, tendency table

**Fit**:
To learn an action model from the hands in one bot's store, and to keep it only if it beats the one
in use on hands held out of the fit. A fitted model is kept beside the store and read back whole.
_Avoid_: train, calibrate

**Strength bucket**:
Where a pair of hole cards ranks among every pair still possible on the board at that moment, cut
into eight equal shares. The action model sees hole cards only through their bucket, and a
decision trace keeps each opponent's mass in each bucket rather than the whole range.
_Avoid_: tier, hand class, band, percentile

**Prior**:
What SvanBot believes about an opponent it has not yet seen enough of, taken from the whole
population.
_Avoid_: default, baseline profile

**Read**:
An opponent's own departure from the prior, learned from their hands alone. One bot's reads are its
own, never pooled across stablemates.
_Avoid_: profile, note, tag

**Evidence**:
The opponent's hands at spots like the current one. A read counts for only as much as its evidence.
_Avoid_: sample, data

**Drift**:
The actions opponents take no longer match what the action model predicts.
_Avoid_: decay, staleness

**Adaptivity class**:
Whether an opponent plays a fixed strategy, switches between strategies, or learns from how we play.
_Avoid_: opponent type, style

**Sparring archetype**:
A scripted opponent style that the sim plays against, standing in for a kind of real opponent. Five
so far: the **calling station** (calls any bet, bets only its best hands), the **nit** (folds all
but strong hands to a bet, raises those), the **maniac** (bets and raises with anything), the
**switching** one (a nit, then a maniac, turn about every 20 hands), and the **learning** one
(adapts to how often we bet).
_Avoid_: dummy, test bot, persona

### Deciding

**Policy**:
How a bot chooses a decision at any spot: the chance of each action, sizes included.
_Avoid_: strategy (for one bot's rules), brain

**Baseline**:
The policy a bot falls back on when it trusts no read: hard to exploit, not tuned to anyone.
_Avoid_: GTO, default strategy

**Exploit budget**:
How far the policy may move from the baseline toward what the reads suggest.
_Avoid_: aggression, exploit level

**Re-solve**:
Working out the policy for the current spot while the action clock runs, a few actions deep, with
the reads and ranges of the opponents in the hand.
_Avoid_: search, solve, lookahead

**Pricer**:
The fallback way to decide: price each legal action by its expected value against the ranges and
take the best. It covers every spot a re-solve does not.
_Avoid_: heuristic, EV engine

**Compute budget**:
The time a decision may spend at a spot. Bigger pots get more of it.
_Avoid_: think time, deadline

### Learning and promotion

**Parameter set**:
The tuned numbers a policy plays by, named by its digest.
_Avoid_: config, genes, weights

**Digest**:
The content hash that names a parameter set. Two equal parameter sets have one digest.
_Avoid_: version, id

**Champion**:
The parameter set the fleet plays by default.
_Avoid_: incumbent, best, main

**Challenger**:
A parameter set put forward to replace the champion.
_Avoid_: candidate, mutant, variant

**Learner**:
The part of SvanBot that turns stored hands into new action models and reads, and proposes
challengers.
_Avoid_: trainer, optimizer

**Paired sim**:
A sim in which two parameter sets play the same deals from mirrored seats, so the luck of the cards
cancels out.
_Avoid_: A/B sim, duplicate sim

**Luck correction**:
A hand's result with the luck of the deal subtracted, leaving what the decisions earned.
_Avoid_: variance reduction, adjusted result

**Canary**:
The one bot that plays live probes and takes every update and promotion first.
_Avoid_: test bot, guinea pig

**Probe**:
A live experiment on the canary that compares the champion with one challenger.
_Avoid_: experiment pair, trial, A/B test

**Arm**:
One of the two parameter sets in a probe.
_Avoid_: variant, side

**Orbit**:
Six counted hands at one table, once round the seats. A probe plays one arm for a whole orbit, and
the orbit is the unit its verdict counts.
_Avoid_: round, block, batch

**Proposal**:
A challenger the learner puts forward for the canary, as a probe or as a promotion.
_Avoid_: candidate, request, queue entry

**Ledger**:
A bot's own record of what each hand won, with the luck of its own draws set beside it.
_Avoid_: results, bankroll, tracker

**Verdict**:
A probe's conclusion: the challenger wins, loses, or cannot be told apart. A verdict holds at any
moment it is read, not only at a planned end.
_Avoid_: result, outcome

**Harm guard**:
The check that stops a probe or a rollout as soon as the challenger is clearly losing.
_Avoid_: circuit breaker, kill switch

**Promotion**:
A challenger becoming the champion: a paired sim puts it above the champion, the canary's harm
guard clears it, and then the rest of the fleet adopts it.
_Avoid_: rollout, upgrade, merge

**Ship bar**:
The evidence a release must show, and the approval it needs, before the fleet plays it.
_Avoid_: release criteria, gate (the gate is for changes, the ship bar for releases)

### Running it

**Store**:
SvanBot's own record of hands, decisions, fits and incidents.
_Avoid_: database, DB, artifacts

**Status**:
One bot's last known state, written beside its store every five seconds: connection, table, hands,
incidents (lifetime and this run) and last error. A status older than fifteen seconds is stale.
_Avoid_: heartbeat, metrics, telemetry

**Status writer**:
The bot's own writer of its status file: it writes on every change and refreshes on a timer. The
dashboard only reads statuses; nothing in a bot's play reads them.
_Avoid_: live feed, telemetry loop, beat

**Dashboard**:
The loopback-only page and server that read every bot's status and push the fleet live. It shows
the fleet; it never plays. Both are our own code, and the page is compiled into the `svanbot`
binary.
_Avoid_: UI, control panel, admin console

**Event stream**:
The dashboard's one server-sent stream: a fleet event whenever a status changes, and a tick every
five seconds so the page can tell a quiet fleet from a dead connection.
_Avoid_: websocket, feed, poll

**Export**:
The venue's hand-history download for one bot and one season.
_Avoid_: dump, scrape, sync

**Update**:
Replacing the build the fleet plays with a newer verified one, canary first, between hands.
_Avoid_: deploy, upgrade, hot-swap

**Slot**:
One of two installed builds, `current` and `previous`. An update fills one and a rollback switches
back to the other.
_Avoid_: version, install

**Rollback**:
Going back to the previous slot, or the previous champion, when the new one fails.
_Avoid_: revert, downgrade

**Gate**:
The checks a change must pass before it merges, run in tiers from quick to full.
_Avoid_: CI, pipeline, checks

**Arena**:
SvanBot's own benchmark league, where parameter sets and sparring archetypes play scored matches.
_Avoid_: tournament, ladder

**Bout**:
One stretch of the Arena: 50 deals, one sparring opponent, played in order so a learning one
learns across them. Bouts share nothing, so they run in any order.
_Avoid_: session, round, match

**League**:
Which table an Arena run is played at: heads up, or 6-max. The two measure different things (the
re-solve after the flop, the multiway path), so each keeps its own runs and its own board, and a
result is never pooled across them. The headline states the heads-up league.
_Avoid_: format, mode, division

**Ring**:
One deal played once from every seat of a full table, the challenger moving a seat each time
against the same opponents, so it holds every hand in every position and the luck of the cards
cancels as the mirror cancels it heads up. Each seat's decisions draw from that seat's own stream,
so a policy playing itself breaks even.
_Avoid_: rotation, orbit

**Scoreboard**:
The Arena's table of results: the opponent, the win rate over it, the 95% interval and the deals
behind it. A run is recorded with its commit and seed, so every line can be played again.
_Avoid_: leaderboard, standings

**v10.0.2**:
The Arena opponent that is v10's last release, ported into `sim::v10` with its default knobs and
held to the actions the real binary took (the parity test). It reads the whole table, as v10's
engine showed it a hand, so it plays only in the sim, and its row on both boards carries the
name `v10.0.2`: the bar a v11 release must clear.
_Avoid_: v10, the old bot, legacy

**Seen**:
The whole table at a seat's turn: the button, the board in the order it came, every seat's stack,
street bet and fold, and the hand's actions in order. Only the sim can show it, and only an
opponent that reads every seat, such as v10.0.2, is handed it; the challenger sees its spot.
_Avoid_: table state, view

**Headline**:
The README's one number: the pinned Arena run's pooled mbb/100 with the interval that says
whether it can be told from zero. The Arena plays one set of deals against every opponent, so the
headline counts deals once, not once per opponent, and its interval is the shared-deal bound — the
weighted mean of the rows' intervals — not a sum that would buy confidence from rows that are one
sample. It names the commit, champion and seed it came from, and the block is generated, never
typed. A run clear of zero says its number (below zero, that it is behind); a run that straddles
zero says it is not yet distinguishable from it.
_Avoid_: front page, summary, blurb

**Pin**:
Choosing the Arena run the headline states, by writing it to `fleet/headline.json`. A run is
pinned from the base branch and never from a pull request, whose commit may not merge, and the
check refuses a pin whose commit the repository does not hold.
_Avoid_: publish, set

**Dossier**:
Everything SvanBot knows about one opponent, in one place.
_Avoid_: profile page, file

**Decision trace**:
The record of one decision: the hole cards and board, the ranges, what each action was worth, and
why one won. The replayer reads it rather than playing the hand again, so the trace keeps its own
copy of the cards instead of parsing frames. Each range names the opponent by **seat**, the one
thing a reader can find at the table.
_Avoid_: debug log, explanation

**Branch**:
Which of the policy's two ways made a decision: the **re-solve** (`resolved`) or the **pricer**
(`priced`). The trace names its branch, and the store keeps it as a column.
_Avoid_: path, route, solved

**Drawn**:
The action a re-solve's mixed strategy picked: the mix, the values, and the index that won.
_Avoid_: sampled, rolled, picked

**Exploit gate**:
The test a fit must pass before a bot reads by it. A model that failed it is not in the process at
all, so the trace's `exploit` is `None` and the **baseline** read the spot.
_Avoid_: threshold, approval

**Public TV**:
The live table shown to an audience, with nothing private on it.
_Avoid_: stream, spectator view
