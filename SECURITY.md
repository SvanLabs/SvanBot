# Security policy

SvanBot connects to openpoker.ai with an API key, keeps what it sees in SQLite on your disk, and
serves a control-room dashboard on a local port. That is the whole surface. This file says how to
report a problem with it and what is supported.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting: the **Security** tab → **Report a vulnerability**. It
opens an advisory thread only the maintainers can read. Please do not open a public issue for a
security problem, and do not paste a key or a token into any public thread.

There is no security email address. The advisory form is the channel.

A report that can be acted on says:

- **Which build.** The dashboard's `/api/health` endpoint returns the commit the running binary was
  built from, and `scripts/status.sh` says what is installed. "main, a while ago" reproduces
  nothing.
- **Which layer.** `crates/deps/` (our own foundations), `crates/libs/` (poker and data logic, no
  network or database I/O), `crates/apps/` (the fleet, the tools, the dashboard API), `web/` (the
  control room), `scripts/` (the operator scripts). The layer decides who can fix it and what a fix
  may touch.
- **What an attacker gets.** A read of the store, a key, the ability to change a decision, or a
  denial of play — say which, and what they need first (a seat at the table, a network position, a
  file on the machine, the dashboard token).
- **The smallest reproduction.** A frame, a request, a database, a commit. If a key is involved,
  say so explicitly — see the next section.
- **Whether it is already public.** If you found it in a fork, a log or a screenshot, tell us
  first; we will treat the key as compromised.

## If a key may be exposed

This software holds an Open Poker API key in `.env`, one per bot slot, and the dashboard's operator
token in the same file. Say so in the report: the advisory thread is private, and a report that
mentions a key is not one we will close for being careless. Then rotate it. openpoker.ai shows a
bot's key once, when the bot is created, so a key that may have been read has to be replaced there
rather than trusted again.

What is already in place around keys, so you know what is covered: `.env` is gitignored and created
mode 600; the pre-commit hook refuses a staged change containing the value of any key, token,
secret or password from it; and the dashboard's setup API returns key hints (`…last4`) and never a
key. None of that helps once a value has reached a log, a screenshot or a public thread, which is
why rotation, not analysis, is the response.

## Supported versions

| Version | Supported |
|---|---|
| 10.0.0, the newest tag on `main` | Yes |
| Any older tag | No |
| A fork or a locally modified tree | No |

Fixes land on `main` and ship in the next release tag. There are no maintenance branches and no
backports. A fix arrives with the test that fails on the old code, and — like everything else here
— the commit names the AI system that produced it (`Generated-by:`, `AGENTS.md` section 0).

## In scope

- **The protocol edge.** `crates/libs/venue/` treats every server frame as untrusted input, as does
  the WebSocket and REST client in `crates/apps/bot/src/client/`. A frame, a resync replay or an
  export that makes the bot act wrongly belongs here.
- **The store.** Schema, packed columns and their codec, integrity checks, sealed backups,
  quarantine and restore, and `mode=ro&immutable=1` when a foreign database is opened.
- **The dashboard API and its authentication.** The operator token, the loopback guard, response
  headers, and the setup endpoint that writes `.env`.
- **The update and release path.** Anything that lets someone who is not a maintainer influence the
  binary that is hot-swapped into a running fleet.
- **Dependencies.** `deny.toml` and the `cargo-deny` step of the gate.

## Out of scope

- **openpoker.ai itself** — its API, its rules, its seasons, another account. Report that to Open
  Poker.
- **Losing chips, hands or rank.** Chips are virtual and the leaderboard is a game. A policy that
  plays badly is a bug report, not a vulnerability.
- **A dashboard exposed without a token.** The documentation says not to do that
  (`README.md`, "Configuration"). With no operator token the API refuses changes that are not
  addressed to a loopback host, and that is the designed boundary rather than a flaw in it.
- **Your machine, network, backups, or key handling outside this repository.**

## How a fix ships

An accepted report gets an advisory, a fix on `main` with its failing-first test, and the advisory
published when the release carrying the fix is tagged. Reporters are credited unless they ask not
to be.

This repository is machine-generated — the warning at the top of `README.md` applies in full — so
read the fix rather than trusting it. The gate proves the change is the one we said it was; it does
not prove the change is correct.
