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
(three starts in five minutes). The whole tree is one path, `SVANBOT_LIB` or
`~/.local/lib/svanbot`; which bot a window runs for is `SVANBOT_NAME`.

The **canary** (SurSvan) takes every update first. `SVANBOT_CANARY` names it, and `update` without
`--fleet` runs the canary's window: the canary is the bot it swaps. `--fleet` is the operator's word
that the canary's window is over — nothing on the box knows it has been watched long enough — so it
needs no canary named, only the bot it swaps, `SVANBOT_NAME`. It swaps one bot at a time.

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
| `SVANBOT_WEB` | `~/.local/lib/svanbot/current/web` | The exported page. |
| `SVANBOT_DASHBOARD_ADDR` | `127.0.0.1:8080` | Where it listens. Keep it on loopback. |

The page is a static export, built by `bun run --cwd web build` into `web/out`; no Node process runs
on the box. The release must install that directory as `web` in the slot (V11-10); without it the
dashboard answers 503 and says the export is missing.

A stopped bot keeps its last status file, so its card says `No recent status` rather than
disappearing. That is the intended reading: the dashboard shows what was last known, not only what
is live.

## The season download

`svanbot download` fills a bot's store with its season hand history, then exits. It uses the
bot's own key, 200 hands a page, at most 30 requests a minute, and honours `Retry-After`. A lock
beside the stores lets only one bot download at a time. Stopped part-way, it resumes from the
offset the store recorded. A hand the store already holds keeps its first copy, so it can run
while the bot plays.

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
