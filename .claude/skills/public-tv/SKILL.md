---
name: public-tv
description: Host the live table for an audience with no operator token — turn the public TV listener on, put it where it should be reachable, and prove it serves the table and nothing else. Use when someone asks to stream, embed or share the bots' table, or to check a public TV that is already up.
allowed-tools: Bash(curl -s 127.0.0.1:*), Bash(curl -s localhost:*), Bash(grep:*), Bash(ls:*), Read, Grep, Glob
---

# Host the public TV

The public TV is a second listener in the same process (`SVANBOT_TV_PORT`, default `0` = off) that
serves the table view to an audience with no operator token. Everything on it is unauthenticated —
that is the point of it, and it is why the next paragraph is the rule rather than the caution.

## The rule

**The listener is the whole security boundary, so binding it is publishing it.** There is no token to
check, no session to expire and no rate limit on this surface. What protects it instead is its shape:
the table view, its stream, `/api/health` and the built page, a 404 to every other `/api/` path, and
an allow-listed payload that drops the hole cards, the policy's decision, the think clock and the
per-seat opponent read (`crates/apps/bot/src/api/tv.rs`). None of that survives being wrapped around
the dashboard — so:

- turn the TV on with `SVANBOT_TV_PORT` and leave `SVANBOT_TV_HOST` on loopback;
- reach it from outside through a TLS reverse proxy (or a tunnel), which is also where the hostname,
  the certificate and any rate limit belong;
- never publish the dashboard's own port to give someone the table, and never bind the TV to `0.0.0.0`
  to try it out: a listener on a LAN is already public.

Publishing is the operator's decision, and the restart is theirs too — the permission classifier
blocks an agent from restarting the live fleet, and this is the change where that is exactly right.

## Turn it on

```sh
# .env, beside SVANBOT_WEB_PORT
SVANBOT_TV_PORT=8788
SVANBOT_TV_HOST=127.0.0.1     # the default; writing it down is a decision, not a formality
```

Then `scripts/stop.sh` and `scripts/start.sh`: the port is read at startup, and a hot-swap release
does not change it. The startup log names the address it bound, or says why it did not — a bind
failure is logged and never fatal, so a typo'd port shows as a missing listener rather than a dead
fleet. `SVANBOT_TV_PORT=0` turns it back off.

## What it serves

| Path | Answer |
|---|---|
| `/api/health` | `{"ok":true,"public":true,…}` |
| `/api/tv` | `{"public":true,"bots":[…]}` — every table, projected for a spectator |
| `/api/tv/events` | the same tables pushed as they change (`table` events only) |
| `/`, `/assets/…` | the built page — the same `web/dist` the dashboard serves |
| every other `/api/…` | **404** — not 401, not 200 |

## Verify it before an audience sees it

```sh
curl -s localhost:8788/api/health          # {"ok":true,"public":true,…}
curl -s localhost:8788/api/tv | head -c 300 # tables; no "hole", no "decision"
curl -s localhost:8788/api/state           # {"detail":"not on the public TV; …"} and 404
curl -s localhost:8788/ -o /dev/null -w '%{http_code}\n'   # 200: the page is built
```

Then repeat against the public URL once the proxy is in front of it. The proxy is part of the surface:
one that adds a login, or strips `Content-Security-Policy`, has changed what was published. A proxy
config that does nothing but forward — hostname, certificate, `reverse_proxy 127.0.0.1:8788` — is the
whole job; [`docs/OPERATIONS.md`](../../../docs/OPERATIONS.md) has the section this is the checklist
for.

## The trap this skill exists for

**A status code says nothing about which listener answered.** Both listeners end in a fallback service
that answers an unknown path with 200 and the built `index.html`, and the dashboard's auth layer
answers an `/api/…` route with 401 where the TV answers 404. So:

- a **401**, or a spectator who sees a login form, means the address is the *dashboard* — the TV has
  nothing to log into;
- a **200** from `/api/state` means the dashboard itself is published: take it back and put the TV on
  its own port;
- a **404 from `/`** while `/api/tv` answers means the page was never built — the API is up and the
  audience sees nothing. `web/dist` is what `scripts/release.sh` installs.

`"public": true` in `/api/health` is the only answer that identifies the listener. That is why it is
the first thing the client asks (`web/src/main.tsx` probes it before requesting a session) and the
first line of the checks above.
