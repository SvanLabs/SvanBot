---
name: restore-store
description: Put SvanBot's runtime data back after a loss or a quarantine, and prove the restored databases are real before a person moves them in. Use when a box that should have history has an empty store, when a database was quarantined, or when a machine is being rebuilt.
allowed-tools: Bash(cargo run --release --bin archive:*), Bash(scripts/fetch-data.sh:*), Bash(sqlite3 -readonly:*), Bash(scripts/stop.sh:*), Bash(pgrep:*), Read, Grep, Glob
---

# Restore the store

Runtime data is not in this repository. `artifacts/` holds the databases, the archives and the hourly
backups, and it is untracked — so a fresh clone has an empty store, which is the normal first-boot
state and not a fault. This skill is for the other case: a box that should have history and does not,
or one whose database was damaged.

## The rule

**Nothing is restored over `artifacts/`.** `archive restore` writes into a directory you name and
never into `artifacts/`; `scripts/fetch-data.sh` refuses while the fleet runs and when a database
already exists. Both are deliberate. A restore with a live writer is a corrupt database with extra
steps, and `artifacts/quarantine/` is the only copy of what the store held when it was damaged — it is
never deleted to make a startup check pass (`docs/SPEC-data.md`).

## Pick a source

| Source | Command | When |
|---|---|---|
| The local archive | `cargo run --release --bin archive -- list` | the box still has `artifacts/archive` (or `SVANBOT_ARCHIVE_DIR`, usually a second disk) |
| A published data release | `SVANBOT_DATA_REPO=<owner/repo> scripts/fetch-data.sh` | the box is new, or the archive disk is gone |

`archive run` writes what is due — a weekly full, otherwise today's daily — and prunes to 14 daily,
8 weekly and 12 monthly, exiting 2 rather than filling the disk. `fetch-data.sh` needs `gh`
authenticated for the data repository, which is the operator's and not this one, so there is no
default to fall back on; `--all` also brings backups, snapshots, logs and screenshots.

## Procedure

1. **Stop the fleet**: `scripts/stop.sh`, then confirm nothing still holds the store
   (`pgrep -f sv10-bot`). The learner and the analyst write to it too.
2. **Verify before restoring**: `... archive list`, then `... archive verify <NAME> --deep`. A shallow
   verify hashes the file; `--deep` decompresses the content. Exit 1 means take the next archive, not
   that you repair this one.
3. **Restore into scratch space**: `... archive restore <NAME> --to /tmp/restore-<date>`. It rebuilds
   `svanbot10.db`, `history.db` and `repo.bundle` there, verified.
4. **Prove it before it replaces anything**:
   ```sh
   sqlite3 -readonly /tmp/restore-<date>/svanbot10.db \
     "pragma quick_check(5); select count(*), max(ended_at) from hands; select count(*) from decisions;"
   ```
   Compare those with what the dashboard showed before the loss. A restore that opens and is empty is
   a failed restore that looks like a success.
5. **Move it in by hand, fleet stopped**: copy `svanbot10.db` and `history.db` into `artifacts/`. The
   fleet runs `quick_check(5)` at startup and moves a damaged file to `artifacts/quarantine/`, so a
   file that is wrong is caught there rather than during play.
6. **Start and confirm**: `scripts/start.sh`, then `curl -s localhost:5000/api/health` and the hands
   count on the dashboard. The learner's models rebuild from the stored hands on their own if the
   `models.v1` checkpoint is older than the data (`docs/SPEC-data.md`).

## When it does not work

- **`archive verify` fails on every archive** — suspect the disk before the data. A failing disk turns
  a verified write into zeros; `/sys/fs/ext4/*/errors_count` reports it, and the archives' sidecar
  SHA-256 files are how you find out which copies went with it.
- **The store opens but the counts are far below what you expect** — check which archive you restored:
  a daily carries the day, a weekly full carries everything. `archive list` prints sizes and times.
- **`fetch-data.sh` refuses** — the fleet is running, or a database already exists. `FORCE=1` is for
  when you mean to overwrite it, and it is not a first thing to try.
