# Deploy: the fleet on the box

The box runs v11 from the scripts in `scripts/`, with no systemd (V11-52). Every path below is under
the home of the user the fleet runs as.

| File | What it is |
|------|------------|
| `scripts/bot.sh` | The supervisor, installed as `svanbot-bot`: it keeps one bot, the dashboard or the learner running. |
| `scripts/start.sh`, `scripts/stop.sh` | Start or stop the whole fleet: every configured bot, the dashboard and the learner. |
| `scripts/update.sh` | Roll a release out, canary first, then every other bot. |
| `scripts/auto-update.sh` | Cron runs it every half hour: a running bot on an older build than the newest release gets it, through `update.sh`. |
| `scripts/clean.sh` | `cargo sweep` of the dev clone's `target/dev`: build output untouched for 14 days. |
| `deploy/install.sh` | Stages and verifies a release bundle on a box with no toolchain. |

Each one keeps its pid, and its log, under `~/.local/state/svanbot/`:

```sh
svanbot-bot status <name>                 # running | restarting | stopped | failed
tail -f ~/.local/state/svanbot/<name>.log
```

> **While the v10 fleet plays, do not start `svanbot@<name>` with a fleet bot's key.** A new
> connection with a bot's key takes over that bot's live session. The first live run of v11 waits
> for the operator's approval, and so does the release.

## A bot

Each bot reads its setup from `~/.config/svanbot/<name>.env`, which stays outside the repo, mode
600:

```sh
install -d -m 700 ~/.config/svanbot
install -m 600 /dev/null ~/.config/svanbot/<name>.env
"${EDITOR:-vi}" ~/.config/svanbot/<name>.env
```

```sh
SVANBOT_API_KEY=<the bot's key>
SVANBOT_NAME=<name>
# Every name of every bot the owner has registered, aliases included. Fair play needs all of them.
SVANBOT_ROSTER=<name>,<name>,...
```

Add `SVANBOT_CANARY=SurSvan` to every bot's file: the bot it names runs the probes and takes each
promotion first, and the others wait for its harm guard (see "Probes and promotions" below).

The supervisor sets `SVANBOT_STORE=~/.local/share/svanbot/<name>.db`, one store per bot, unless the
env file names another. It runs `~/.local/lib/svanbot/current/svanbot`, the slot the updater points
`current` at (V11-10).

```sh
svanbot-bot start <name>      # in the background; refused if it already runs
svanbot-bot stop <name>       # TERM, waits for it to exit (KILL after 90 s)
svanbot-bot status <name>
tail -f ~/.local/state/svanbot/<name>.log
```

A crash restarts the bot after 10 s, at most five times in five minutes, so a crash loop cannot
spend the venue's ten connections a minute per IP. After the fifth, `status` says `failed` and it
stays down. Exit 1 (the venue refused the key) and exit 2 (setup missing) never restart. Fix the env
file, then `svanbot-bot start <name>` again. Each bot's address space is capped at 1 GiB, and a log
past 64 MiB is moved to `<name>.log.1` at the next start.

A fair-play incident is logged at priority crit as `FAIR-PLAY INCIDENT` and kept in the bot's
store, in its append-only `incidents` table. To list every incident on the box:

```sh
grep -h '^<2>' ~/.local/state/svanbot/*.log
```

## Start or stop the configured fleet

The operator or an agent runs these after installing a release. Before starting, check the warning
above about any bot keys the v10 fleet still uses.

```sh
scripts/start.sh                # each bot with a .env file, the dashboard and the learner
scripts/stop.sh                 # the learner, every bot with a pid file, then the dashboard
```

Starting leaves alone anything that already runs. Both scripts leave the release slots as they are,
and both refuse to run during a release update. Stop ends each program at once: use the release
updater for a swap that waits for the end of a hand.

After a reboot, cron starts the fleet. Add the two lines once, with the clone's own path:

```sh
( crontab -l 2>/dev/null
  echo "@reboot $HOME/NewSvanBot/scripts/start.sh >> $HOME/.local/state/svanbot/cron.log 2>&1"
  echo "@daily $HOME/NewSvanBot/scripts/clean.sh >> $HOME/.local/state/svanbot/cron.log 2>&1"
) | crontab -
```

## Updating the fleet

The fleet never builds. CI builds a release on a `v*` tag and publishes a checksummed bundle; the
box verifies and installs it. Nothing here is written on the box while the fleet plays except a
slot pointer, and that only between hands.

`svanbot-release` is the operator's tool, installed at `~/.local/bin/svanbot-release` from the
first bundle. Its four subcommands are the whole procedure:

```sh
svanbot-release status                  # what is installed, what is running, and where the slots point
svanbot-release install <staged>        # verify a staged release and put it in lib/<version>
svanbot-release update [<version>]      # swap current between hands, canary first, then roll back if it does not come up
svanbot-release rollback                # point current back at the build it replaced
```

`install` never touches a slot, so a release can be staged long before a window is found. `update`
is the window: it asks the bot to stop between hands, waits for it, stops it under its supervisor, points
`current` at the new build, starts it again, and rolls back if it does not come up or crash-loops
(three starts in five minutes). A bot that is down (`stopped` or `failed`) has no hand to
wait out, so `update` skips the stop: it points `current` and starts the bot, and if the bot does
not come up it is stopped again with `current` back where it was ([the first
start](#the-first-start-on-a-fresh-box)). The whole tree is one path, `SVANBOT_LIB` or
`~/.local/lib/svanbot`; which bot a window runs for is `SVANBOT_NAME`.

The **canary** (SurSvan) takes every update first. `SVANBOT_CANARY` names it, and `update` without
`--fleet` runs the canary's window: the canary is the bot it swaps. `--fleet` is the operator's word
that the canary's window is over, given once `svanbot release-check <tag> fleet` clears ([the ship
bar](#the-ship-bar)), so it needs no canary named, only the bot it swaps, `SVANBOT_NAME`. It swaps
one bot at a time.

```sh
# the canary first
SVANBOT_CANARY=SurSvan SVANBOT_NAME=SurSvan svanbot-release update 2026.11.16
# ... watch the canary for its window, then the fleet, one bot at a time
SVANBOT_NAME=<name> svanbot-release update --fleet
```

`scripts/update.sh` does the whole rollout in one run (V11-50):
1. It installs the release if it is not installed yet: `gh release download`, `install.sh`, then
   `svanbot-release install`.
2. It swaps the canary, the one the bots' env files name.
3. It swaps every other bot with `--fleet`, one at a time. A bot that already runs the build is
   skipped, so a run that stopped can simply be run again.
4. It stops at the first bot that does not take the build, a rollback included, and exits 1.
5. It restarts the dashboard if it runs, onto the new build.

`--canary-only` stops after the canary, for a canary window before the fleet. `--running-only`
leaves a bot that is not running alone, where a swap would start it: it is what `auto-update.sh`
uses.

### Always the newest release

The operator's rule (2026-11-25, V11-56): every bot runs the newest release, and
`scripts/auto-update.sh` keeps it so. Cron runs it every half hour:

```sh
( crontab -l 2>/dev/null
  echo "*/30 * * * * $HOME/NewSvanBot/scripts/auto-update.sh >> $HOME/.local/state/svanbot/cron.log 2>&1"
) | crontab -
```

Each run:
1. Reads the build each running bot runs. A bot that is down is left alone: it starts on `current`,
   which a rollout points at the newest build. With nothing running, there is nothing to do.
2. Asks GitHub for the newest release as the App: `GH_TOKEN` comes from
   `~/.config/svanbot/app-token.sh`, never from the operator's own `gh` login.
3. When any running bot is on an older build, runs `scripts/update.sh --running-only <version>`:
   the canary first, then the others one at a time, and a rollback for a bot that does not come up.

The canary's window in [the ship bar](#the-ship-bar) no longer holds the fleet back: the canary
swaps first and must hold, then the rest follow in the same run. The Arena stage, before a tag is
pushed, is unchanged.

A release that installed but did not take is left alone for six hours, and a newer one is tried at
once. One that could not be installed is tried again at the next run. The outcome is in
`~/.local/state/svanbot/cron.log`. To hold an older build on purpose, after a
`svanbot-release rollback` for one, pause it, and resume by removing the file:

```sh
touch ~/.local/state/svanbot/auto-update.pause
rm ~/.local/state/svanbot/auto-update.pause
```

```sh
scripts/update.sh 11.1.7                 # install if needed, canary, then the fleet
scripts/update.sh --canary-only 11.1.7   # the canary alone; run again without it later
```

One update runs at a time: a second `update` or `rollback` says an update is already in flight and
stops, because a window is short and a person is watching. If the canary's live evidence says the
new build is worse, the harm guard trips and `svanbot-release rollback` is the same move by hand.

### Probes and promotions

The fleet plays by one champion parameter set, `~/.local/share/svanbot/fleet/champion.json`
(`SVANBOT_FLEET` moves the directory). Until a promotion writes it, the champion is the built-in
default.

```sh
# The learner: a paired sim of a challenger against the champion. Above the champion, it is
# proposed as a promotion; `--probe` proposes it as evidence only, whatever the sim said.
svanbot propose --deals 2000 exploit=0.4
svanbot propose --deals 2000                    # the search: each number a step down and up
grep probe ~/.local/state/svanbot/SurSvan.log   # started, ended, promoted
```

The canary takes the proposal up between hands and plays the challenger in about half its orbits.
A promotion's harm guard is 3,000 hands. If it clears, the canary writes the champion file and
every other bot adopts it between hands, with no restart. If the challenger is clearly losing, the
probe ends harmful and the champion stays. To take a promotion back by hand, remove
`champion.json` (the default returns) or put the earlier file back; the bots follow between hands.
Run `svanbot propose` at low priority: a sim is CPU the live fleet shares.

### The unattended search

`svanbot learn` is the same search with nobody running it (V11-49). Each run does one step and
exits, and its supervisor starts the next run half an hour after the last one ended:

- **It waits** while a proposal waits or the canary runs a probe. That evidence is pending, so it
  sims nothing. The canary mirrors its running probe in `fleet/probe.json` for this.
- **It sims** the next step round the champion: each number a step down and up, in rings of
  shrinking steps (×0.8 and ×1.25, then ×0.9 and ×1.11, then ×0.95 and ×1.05). It runs one paired
  sim of 2,000 deals on a seed named by the champion and the challenger, so the same step is the
  same sim anywhere and is never run twice.
- **It proposes** a challenger only when a first sim and a confirmation on fresh deals both put
  the whole 95% interval above the champion. Many steps are tried, and one lucky sim must not
  become a promotion. The proposal goes to the canary's harm guard of 3,000 hands, as any
  promotion does, and only when the canary has nothing else: it never replaces a waiting proposal.
  It names the champion it was measured against, and the canary drops it unplayed if a promotion
  has replaced that champion since.
- **It says the search is spent** once every step round the champion is simmed and none is above
  it. It searches afresh round the next champion.

It reads and writes the fleet's files and nothing else: no store, no key, no venue. Every step and
what the last run did are in `fleet/search.json`; the dashboard shows them too.

```sh
svanbot-bot start learn                        # scripts/start.sh starts it too
tail -f ~/.local/state/svanbot/learn.log       # one line a run: simmed, proposed, waits or spent
svanbot-bot stop learn                         # stops the search; nothing it wrote is undone
```

Each step runs under the idle CPU scheduler at nice 19 and idle I/O, so a sim takes only CPU the
bots leave free. To start the search over round the same champion, remove `fleet/search.json`.

### A release, start to end

```sh
gh release download v11.1.0 --repo SvanLabs/SvanBot       # the bundle and its .sha256
deploy/install.sh svanbot-11.1.0-x86-64-v2.tar.gz          # checks the CPU, verifies, stages
svanbot-release install ~/.local/lib/svanbot/staged
svanbot-release status                                       # the build is installed, no slot has moved
SVANBOT_CANARY=SurSvan svanbot-release update 11.1.0       # the canary takes it first
```

A bundle is built for one instruction level, named in its file name and in `UPGRADE/cpu`: the
box's, x86-64-v2. `install.sh` refuses a bundle built above the CPU it is installing on, before
anything is staged, since a program that reaches an instruction its CPU lacks dies. Bundles up to
11.0.2 are named `x86-64-v3` but were built for the x86-64 baseline, so they run here too.

### The first start, on a fresh box

A fresh box has no `current` and no bot running, and `svanbot-release` takes it from a downloaded
bundle to a running canary. Nothing else may be playing with these keys: a second connection with
a bot's key takes over its live session.

```sh
# 1. Each bot's env file, as in "A bot" above: key, name, roster, SVANBOT_CANARY=SurSvan.
# 2. Nothing is started by hand: until `current` exists a start can only fail.
# 3. The bundle: install.sh puts svanbot-release in ~/.local/bin and stages the release.
gh release download v11.1.0 --repo SvanLabs/SvanBot
deploy/install.sh svanbot-11.1.0-x86-64-v2.tar.gz
svanbot-release install ~/.local/lib/svanbot/staged
# 4. The canary's first update: it is down, so current is pointed at the build and the bot is
#    started, then watched for a crash loop like any other update.
SVANBOT_CANARY=SurSvan SVANBOT_NAME=SurSvan svanbot-release update 11.1.0
svanbot-release status
```

The rest of the fleet follows the same way, one bot at a time, once the canary's window is over:
`SVANBOT_NAME=<name> svanbot-release update --fleet 11.0.0`. Name the version: with none,
`update` takes the newest installed build by number, and an older dated build such as
`2026.11.16-1` outranks `11.0.0`.

If the canary does not come up, `update` says so, stops it again, and exits 1. A first build that
crash-loops has nothing to roll back to, so it stays in `current`, its supervisor stops retrying
it after five starts, and `update` exits 1 saying so. `svanbot-bot start <name>` tries it again.

### The ship bar

A release reaches the fleet only over the ship bar (V11-22). `svanbot release-check` names every
clause a release fails and exits 1, or says it clears the bar and exits 0. It reads the fleet's
files and the stores and writes nothing. git names the tag's commit, so run it in a clone that has
the tag. It has two stages.

**Before the tag is pushed: the Arena stage.** Play the ship run from a build of the commit to be
tagged, with that commit named, then check it:

```sh
git tag v11.1.0                                    # local: not pushed until it clears
export SVANBOT_COMMIT=$(git rev-parse 'v11.1.0^{commit}')
nice -n 15 svanbot arena ship <seed>               # both leagues, until the bar is decided
svanbot release-check v11.1.0
git push origin v11.1.0                            # only once it clears
```

`svanbot arena ship` plays both leagues against every opponent a **heat** at a time, one more bout
of every row, and records both runs in `arena.json` after each heat. Each row is read through an
anytime-valid 95% confidence sequence, one observation a bout, from its 30th bout on. The run
stops at the first heat where the bar is decided. It passes once v10.0.2 is read below the policy
in **both** leagues, and is refused as soon as any row is read above it. A run still undecided
after 48 hours is refused too. A heat is about 11 minutes on this box, so the run is about
5½ hours at the least. It exits 0 when the bar passes, and 1 when it is refused.

`release-check` reads the newest run of each league from the tag's commit. It must have been
played under the fleet's champion, and three of its bouts, drawn from its seed, must play again
bit for bit as recorded. The policy must be read above v10.0.2 heads up and at 6-max, and no
sparring opponent or the pricer may be read above the policy, in either league. A run without a
commit (`local`) is never read.

**Before `update --fleet`: the canary stage.** Each bot marks in its store when it first started on
a release. The canary's window opens there and closes at its latest hand. Over that window the
canary must hold 20 000 hands, and the rest of the fleet another 20 000 by their export; the canary
must not lose to the fleet with the 95% interval of the difference wholly below zero. Short of
20 000 on either side, it is not yet measured, and refused. The fleet's side is its export, so
download every other bot's season first (see [the season download](#the-season-download)):

```sh
SVANBOT_CANARY=SurSvan ~/.local/lib/svanbot/11.0.0/svanbot release-check v11.0.0 fleet
```

The `fleet` stage runs the Arena stage again first, from the release's own build. The canary keeps
the first time it started on a release, so a rollback and a second update measure from the first.
## The dashboard

`svanbot dashboard` serves one page on loopback and pushes the fleet to it over one event stream.
Each bot writes `<name>.status.json` beside its store, mode 600, every five seconds; the dashboard
calls a status older than fifteen seconds stale, and keeps the last one on screen if the stream
drops. The page names opponents and shows their stacks, so it is never the public TV.

```sh
svanbot-bot start dashboard            # scripts/start.sh starts it too
ssh -N -L 8080:127.0.0.1:8080 <box>    # then open http://127.0.0.1:8080
```

It reads these variables, with the defaults its supervisor sets:

| Variable | Default | What it is |
|----------|---------|------------|
| `SVANBOT_DATA` | `~/.local/share/svanbot` | Every bot's store and status file. |
| `SVANBOT_LIB` | `~/.local/lib/svanbot` | The install tree, whose slots the Release page shows. |
| `SVANBOT_DASHBOARD_ADDR` | `127.0.0.1:8080` | Where it listens. Keep it on loopback. |
| `SVANBOT_SEASON` | on | `off` keeps the dashboard off the network: no season board. |

The page is hand-written files compiled into the `svanbot` binary, and the server is our own, on
the standard library: nothing else is installed, built or run on the box for it. The fleet (`/`)
and each bot (`/bot/<name>`) are control rooms laid out after v10's: a tab per bot, hero tiles, the
signal rail, then a board of panels in three columns. The tabs above the board show a slice of it
(Live, Opponents, Results, Learning, System, All). **Arrange** moves and hides panels, and each
panel folds. The table's look (arena, felt, midnight), the arrangement, the folds and the notes
are kept in the reader's browser only. The activity log is what changed while the page was open.
The release page (`/release`) shows what is installed, where `current` and `previous` point, and
the build and champion each bot runs.

The season board is the venue's public leaderboard (`GET /season/current` and
`GET /season/leaderboard`, no key), read once a minute on a thread of its own: the top 20 and
every bot of ours wherever it ranks. A failed read keeps the last board and says so.

A stopped bot keeps its last status file, so its card says `No recent status` rather than
disappearing. That is the intended reading: the dashboard shows what was last known, not only what
is live.

## The season download

`svanbot download` fills a bot's store with its season hand history, then exits. It reads the
season's export, the same download as the site's analytics page, with the bot's own key: one
request a season, about 75 seconds for 14 000 hands (the export answers nothing past offset
10 000, so it is never paged). It honours `Retry-After`. A lock beside the stores lets only one
bot download at a time. Every run reads the
season from its newest hand, and a hand the store already holds keeps its first copy, so it can
run while the bot plays, and a stopped run is simply run again.

```sh
for name in <name> <name> ...; do
  ( set -a; . ~/.config/svanbot/"$name".env; set +a; ~/.local/lib/svanbot/current/svanbot download )
done
```

v11 starts from a fresh store. The v10 stores are archived in `~/archive/svanbot-v10/` and are
never migrated.

## The clean job

```sh
MISE_ENV=box mise install    # once: builds the pinned cargo-sweep (mise.box.toml)
scripts/clean.sh             # cron runs it daily (see "Start or stop the configured fleet")
```

It removes build output nobody has touched in 14 days, at idle I/O priority. `tools/disk.ts`
reports every `target/` in the clone, worktrees' and nested crates' included, against one 9 GiB
budget after every `mise run full`, and names each tree when it is over.

## From systemd to the scripts (once, the operator)

Until V11-52 the fleet ran as systemd user units. The cutover is a fleet stop and a fleet start, so
run it when a minute of play can be lost, with nothing else connected. Run it from the main
folder, `~/NewSvanBot`, on `main`, and do the cutover first: until a bot runs under its
supervisor, the new `svanbot-release update` cannot swap it. Its start refuses a bot that another
process still plays.

```sh
deploy/install.sh <bundle>          # a bundle from V11-52 on installs svanbot-bot beside svanbot-release
# `disable` takes unit names, not patterns: one bot a line, from the env files.
for env in ~/.config/svanbot/*.env; do
  systemctl --user disable --now "svanbot@$(basename "$env" .env)"
done
systemctl --user disable --now svanbot-dashboard
systemctl --user disable --now svanbot-learn.timer svanbot-clean.timer   # where installed
rm -f ~/.config/systemd/user/svanbot*.service ~/.config/systemd/user/svanbot*.timer
rm -rf ~/.config/systemd/user/svanbot-dashboard.service.d
systemctl --user daemon-reload
scripts/start.sh                    # every bot, the dashboard and the learner, under the supervisor
svanbot-bot status SurSvan          # and each other bot: running
scripts/update.sh <version>         # then the release, canary first, through the supervisor
```

Then add the cron lines above. `/etc/systemd/journald.conf.d/svanbot.conf`, if root installed it,
can go too: no bot logs to the journal now.
