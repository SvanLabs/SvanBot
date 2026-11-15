# Deploy: the units on the box

The box runs v11 as systemd **user** units, with no version in their names. Every path below is
under the home of the user the fleet runs as.

| File | What it is |
|------|------------|
| `svanbot@.service` | One bot at the venue. One instance per bot: `svanbot@<name>`. |
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
