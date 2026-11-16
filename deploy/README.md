# Deploy: the units on the box

The box runs v11 as systemd **user** units, with no version in their names. Every path below is
under the home of the user the fleet runs as.

| File | What it is |
|------|------------|
| `svanbot@.service` | One bot at the venue. One instance per bot: `svanbot@<name>`. |
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
