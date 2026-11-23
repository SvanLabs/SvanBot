# Deploy: the units on the box

The box runs v11 as systemd **user** units, with no version in their names. Every path below is
under the home of the user the fleet runs as.

| File | What it is |
|------|------------|
| `svanbot@.service` | One bot at the venue. One instance per bot: `svanbot@<name>`. |
| `install.sh` | Stages and verifies a release bundle on a box with no toolchain. |
| `svanbot-dashboard.service` | The loopback-only fleet dashboard: `svanbot dashboard`. |
| `svanbot-clean.service`, `svanbot-clean.timer` | Daily `cargo sweep` of the dev clone's `target/dev`. |
| `journald-svanbot.conf` | The journal's 1 GiB cap. Root installs it: the operator's step. |

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

The unit adds `SVANBOT_STORE=~/.local/share/svanbot/<name>.db`, one store per bot. It runs
`~/.local/lib/svanbot/current/svanbot`, the slot the updater points `current` at (V11-10).

```sh
install -m 644 deploy/svanbot@.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now svanbot@<name>
journalctl --user -u svanbot@<name> -f
systemctl --user stop svanbot@<name>
```

A crash restarts the bot after 10 s, at most five times in five minutes, so a crash loop cannot
spend the venue's ten connections a minute per IP. Exit 1 (the venue refused the key) and exit 2
(setup missing) never restart: fix the env file, then `systemctl --user reset-failed` and start it
again.

A fair-play incident is logged at priority crit as `FAIR-PLAY INCIDENT` and kept in the bot's
store, in its append-only `incidents` table. To list every incident on the box:

```sh
journalctl --user -p crit -u 'svanbot@*'
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
is the window: it asks the bot to stop between hands, waits for it, stops it under systemd, points
`current` at the new build, starts it again, and rolls back if it does not come up or crash-loops
(three starts in five minutes). A bot that systemd has down (inactive or failed) has no hand to
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
svanbot propose --deals 2000 value_margin=0.35
svanbot propose --deals 2000                    # the search: each number a step down and up
journalctl --user -u svanbot@SurSvan | grep probe   # started, ended, promoted
```

The canary takes the proposal up between hands and plays the challenger in about half its orbits.
A promotion's harm guard is 3,000 hands. If it clears, the canary writes the champion file and
every other bot adopts it between hands, with no restart. If the challenger is clearly losing, the
probe ends harmful and the champion stays. To take a promotion back by hand, remove
`champion.json` (the default returns) or put the earlier file back; the bots follow between hands.
Run `svanbot propose` at low priority: a sim is CPU the live fleet shares.

### A release, start to end

```sh
gh release download v2026.11.16 --repo SvanLabs/SvanBot   # the bundle and its .sha256
deploy/install.sh svanbot-2026.11.16-x86-64-v3.tar.gz      # verifies and stages it
svanbot-release install ~/.local/lib/svanbot/staged
svanbot-release status                                       # the build is installed, no slot has moved
SVANBOT_CANARY=SurSvan svanbot-release update 2026.11.16   # the canary takes it first
```

### The first start, on a fresh box

A fresh box has no `current` and no bot running, and `svanbot-release` takes it from a downloaded
bundle to a running canary. Nothing else may be playing with these keys: a second connection with
a bot's key takes over its live session.

```sh
# 1. Each bot's env file, as in "A bot" above: key, name, roster, SVANBOT_CANARY=SurSvan.
# 2. The unit, enabled but not started: until `current` exists a start can only fail, and five
#    failures in five minutes leave the unit failed.
install -m 644 deploy/svanbot@.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable svanbot@SurSvan        # and each other bot, all without --now
# 3. The bundle: install.sh puts svanbot-release in ~/.local/bin and stages the release.
gh release download v11.0.0 --repo SvanLabs/SvanBot
deploy/install.sh svanbot-11.0.0-x86-64-v3.tar.gz
svanbot-release install ~/.local/lib/svanbot/staged
# 4. The canary's first update: it is down, so current is pointed at the build and the bot is
#    started, then watched for a crash loop like any other update.
SVANBOT_CANARY=SurSvan SVANBOT_NAME=SurSvan svanbot-release update 11.0.0
svanbot-release status
```

The rest of the fleet follows the same way, one bot at a time, once the canary's window is over:
`SVANBOT_NAME=<name> svanbot-release update --fleet 11.0.0`. Name the version: with none,
`update` takes the newest installed build by number, and an older dated build such as
`2026.11.16-1` outranks `11.0.0`.

If the canary does not come up, `update` says so, stops it again, and exits 1. A first build that
crash-loops has nothing to roll back to, so it stays in `current`, systemd stops retrying it after
five starts, and `update` exits 1 saying so. A unit left failed by its start limit refuses a start
until `systemctl --user reset-failed svanbot@<name>`.

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
install -m 644 deploy/svanbot-dashboard.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now svanbot-dashboard
ssh -N -L 8080:127.0.0.1:8080 <box>   # then open http://127.0.0.1:8080
```

The service reads three variables, with the defaults the unit sets:

| Variable | Default | What it is |
|----------|---------|------------|
| `SVANBOT_DATA` | `~/.local/share/svanbot` | Every bot's store and status file. |
| `SVANBOT_LIB` | `~/.local/lib/svanbot` | The install tree, whose slots the Release page shows. |
| `SVANBOT_DASHBOARD_ADDR` | `127.0.0.1:8080` | Where it listens. Keep it on loopback. |

The page is three hand-written files compiled into the `svanbot` binary, and the server is our
own, on the standard library: nothing else is installed, built or run on the box for it. The
pages are the fleet (`/`), one bot (`/bot/<name>`) and the release (`/release`): what is
installed, where `current` and `previous` point, and the build and champion each bot runs.

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

## The clean timer

```sh
MISE_ENV=box mise install    # once: builds the pinned cargo-sweep (mise.box.toml)
install -m 644 deploy/svanbot-clean.service deploy/svanbot-clean.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now svanbot-clean.timer
```

It removes build output nobody has touched in 14 days, at idle I/O priority. `tools/disk.ts`
reports `target/` against its 9 GiB budget after every `mise run full`.

## The journal cap (operator, root)

```sh
sudo install -D -m 644 deploy/journald-svanbot.conf /etc/systemd/journald.conf.d/svanbot.conf
sudo systemctl restart systemd-journald
```

Units of a user who is not lingering stop when that user logs out. For a fleet that must outlive
the session, the operator runs `sudo loginctl enable-linger <user>` once.
