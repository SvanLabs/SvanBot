# Domain glossary

> **Read this when** a word seems to mean more than usual. These are the terms whose meaning here is
> narrower than in poker in general, and the dashboard and the docs use them exactly this way.
> The poker basics (bb/100, EV, range, VPIP) are in the glossary at the end of
> [`docs/GUIDE.md`](GUIDE.md). · [All docs](README.md)

## Leaderboard rank

An OpenPoker season position assigned to one bot from that bot's current chip score. A rank change
may combine that bot's chip movement with other bots' movement. It is never an aggregate fleet rank.

## Flagship bot

SvanBotV10 (named SvanBotV7 until 2026-09-23; one season record per renamed bot), the
highest-ranked member of the five-bot fleet by default. Statements such as "we reached #4" refer to
this bot unless another bot is named; in a season where another bot leads (Svanar was #1 on
2026-09-26), name the bot.

## Fleet

The five independently ranked bots—SvanBotV10, SuraGunnar, Svanism, SurSvan, and Svanar. Each plays
its lineage's promoted policy, and lineages can share the same parent. Fleet results aggregate
evidence about policy strength, but the leaderboard does not combine their scores.

## Top-five sweep

An observed official season leaderboard on which all five fleet bots qualify and occupy places
1 through 5. It describes one standing at one time; a sweep does not by itself establish sustained
control across seasons.

## Outsider margin

The lowest fleet season chip score minus the highest score outside the fleet, measured on the
same official leaderboard. A zero margin is a tie, whose assigned ranks still determine whether
the fleet holds a top-five sweep.

## Lineage

One bot's own parameter set and promotion history. Every fleet bot has one; all start as copies of
the champion and diverge only through promotions earned against their own parent.

## Gene transfer

Offering a transition that one lineage has proven to the other lineages as a priority candidate.
The receiving lineage tests it with its own gate; it is never copied.

## Tournament refresh

A periodic paired round-robin between the lineages' parents. A lineage that another dominates is
replaced by a copy of the winner.

## Protected trio

The three fleet bots holding the active public season's first three places when experiment mode
begins. They continue to play the champion policy for that activation.

## Experiment pair

The other two fleet bots when experiment mode begins: the bot in fourth place and the fifth fleet
bot, wherever it ranks. Their membership stays fixed for that activation.

## Rank slide

A flagship bot's movement to a worse leaderboard rank over a stated time window. Diagnose it by
separating the bot's own chip change from changes in the scores of bots around it.

## Live fit

A correction fitted from the fleet's own hands and installed in live play only: today the per-street
and preflop fold calibration and the river, deep-pot and overbet all-in call shifts. It is installed
only when it wins on held-out newer hands, is never promoted, and the learner evaluates without it.
Distinct from a promoted parameter (the learner's search) and from self-calibration (per-spot EV bias).

## Per-opponent read

A correction for one named opponent on top of the shared model, fitted by the learner and installed
through `playerfits` only while it predicts that opponent's newer hands better than the shared model
(held-out gain above its 95% band): fold calibration, response correction and river
sizing tell. Shown on the dashboard's Per-opponent reads panel. Distinct from a live fit,
which corrects everyone at once.

## Sizing tell

How one opponent's river bet size tracks the strength of the hand they bet, relative to the pool:
positive means bigger bets are stronger, negative means bigger bets are weaker. A tell `k` tilts that
opponent's river betting range by `(size / 0.66)^-k`; 0 is the pool curve.

## Decision margin

The spots whose predicted EV is within ±1 bb of the alternative (`margins::MARGIN`), where a correction
of a few big blinds changes the action. Self-calibration's upward corrections are bounded by the
residual measured there, not by the category mean, which big-EV spots dominate.

## Installed artifact

Anything live play loads from the store and replaces while running: promoted parameters, the neural
response model, the fitted range model, the compute profile's live budget, live fits and per-opponent
reads. All go through `installs`: a changed stored value is installed, a failed store read or malformed record keeps what
is installed.

## Autonomy watchdog

The fleet head's check that the loops keeping it learning still report: learner cycles, analyst
re-solves, hourly fold calibration and hourly backups (`watchdog`). It logs once when a loop
goes stale and once when it recovers; `review autonomy` prints the same view.

## Packed column

A database column holding cold JSON (decision details, replay and audit records, server hand exports
and their summaries) stored as an `sv10-pack` frame instead of text. Readers go through
`sv10_store::packed::Codec`, which accepts both forms. The fields SQL needs have real columns.
Hand summaries stay text.

## Data format

The highest storage format written next to the databases (`artifacts/data-format`): 1 = text only,
2 = packed columns. A build whose source reads a lower format is never installed over it: rollback
refuses it until `archive unpack` converts back.

## Update

The dashboard's one-click path from GitHub to a running build: fetch, gate, build, install, hot
swap between turns. The bots keep playing throughout, and a failed run leaves the installed build
and the checkout as they were. Rolling back to a saved build uses the same path.

## Host check

The read-only list of machine facts the fleet depends on (microcode, huge pages, memory, free space
on the SSD and the archive disk, SSD TRIM), each judged against the verified checklist, with the
operator's command when one is off (`GET /api/host`, System view).

## Enabled learning pipeline

A learning capability that continues gathering evidence, fitting candidates and validating them.
A pipeline can be enabled while its current candidate is not installed because the candidate did
not pass its evidence gate.

## Operational recovery

Automatic restoration of a failed fleet capability to useful progress while retaining valid
installed learning and recoverable evidence. Recovery succeeds when the capability resumes work,
not merely when a process exists again.

## Sampled sweep occupancy

The share of scheduled leaderboard observations in which all five fleet bots hold the official
qualified ranks 1 through 5. Missing or stale observations stay in the denominator and never count
as success; this is sampled evidence rather than proof of continuous ownership between observations.

## Sustained top-five ownership

Repeated all-five control of the official qualified leaderboard across complete seasons, assessed
by sampled sweep occupancy and each season's final standings under the operator's agreed criteria.
An isolated sweep or a single bot's rank does not establish sustained top-five ownership.
