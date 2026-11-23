# Security

## Reporting a vulnerability

Report it privately, through GitHub's
[private vulnerability reporting](https://github.com/SvanLabs/SvanBot/security/advisories/new).
Never open a public issue for one. Please include what an attacker could do, the commit or release
it affects, and the steps that show it.

You will hear back within a week. A fix goes out as a new release through the updater, canary
first, and the advisory is published once every bot runs it.

## What counts

- Anything that could leak a bot's API key, the contents of its store, or an opponent's data.
- Anything that lets a party other than the operator start, stop, swap or steer a bot.
- A release bundle that installs without matching its published checksum.
- A way for two stablemates to share a table, or for one bot to read another's store.

## What the project does about it

- Keys and `.env` contents never enter the repo, and the commit tier scans every staged change
  for secrets.
- Every release bundle is built by CI from a tag, checksummed and attested. The updater verifies
  it before it installs, and `install.sh` checks it against its published checksum.
- The dashboard listens on loopback only, and is reached over an SSH tunnel.
- Every workflow action is pinned to a commit, and cargo-deny checks the dependencies on every
  pull request.
