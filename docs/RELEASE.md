# Release process

> **Read this when** you are tagging a version. **Before this:** the gate is green on the exact
> commit (`scripts/check.sh full`). **Related:** [`CHANGELOG.md`](../CHANGELOG.md). · [All docs](README.md)

A release is a tagged commit on `main` that the gate passed, with a changelog entry and release
notes. This document is the process; `scripts/check.sh` is the gate that decides whether a commit
is releasable at all.

## Promoting `dev` to `main`

Work lands on `dev`. `main` is the released line: what everyone who installs SvanBot runs, and where
their Update fetches from. It moves only by promotion — a pull request from `dev` into `main`, merged
**with a merge commit**, never a squash, which would give `main` a commit `dev` does not have and
make the next promotion a conflict to resolve by hand instead of a merge that changes no file.
Because promotion waits on a green `dev`, `main` never carries a build that failed.

**It happens by itself.** `.github/workflows/promote.yml` runs `scripts/promote.sh` when `check`
finishes green on `dev`. The script keeps one promotion pull request open with auto-merge armed, and
GitHub merges it when the gate passes on it — a pull request's merge ref is recomputed on every push
to its head, so that one pull request always tests the head `dev` has now. A red `dev` promotes
nothing, because the workflow only fires on a green run, and a `main` that carries a commit of its
own — a non-merge commit `dev` does not have, which is what a squash promotion or a commit pushed
straight to `main` leaves behind — is refused by name rather than merged. A `main` that already holds
what `dev` holds promotes nothing and opens no pull request, and that test is the two **trees**, not
the two commit ids: a promotion is a merge commit, so it makes the ids permanently different however
identical what they hold. The promotion merge commits themselves are not the refusal's business:
they live on `main` alone by construction, so `main` stops being an ancestor of `dev` the moment it
has been promoted once, and a check phrased that way would refuse every promotion after the first.

The same thing by hand, which is also the retry after a failure:

```sh
scripts/promote.sh            # open or reuse the promotion pull request, arm auto-merge
scripts/promote.sh --check    # print what it would do and change nothing
```

The reference fleet tracks `dev` (`SVANBOT_UPDATE_BRANCH=dev`) rather than `main`, so every change is
played before it is promoted.

After it merges, every fleet following `main` installs it at its next Update (or `scripts/update.sh`).
Tags are cut on `main`.

## What ships

Pushing a `v*` tag runs `.github/workflows/release.yml`: it attaches the portable x86-64-v2/v3
bundle (`scripts/portable.sh`) with its SHA-256 and a build attestation, and pushes a container image
to `ghcr.io/svanlabs/svanbot`. Running the workflow by hand with an existing tag backfills one.

| Artifact | Produced by | Verified by |
|---|---|---|
| The tagged source | `git tag -a` on a commit the gate passed | the green CI run on that commit |
| Portable bundle (`.tar.gz`) | the release workflow, `scripts/portable.sh` | `gh attestation verify <file> --repo SvanLabs/SvanBot`, and its `.sha256` |
| Container image | the release workflow, `Dockerfile` | `gh attestation verify oci://ghcr.io/svanlabs/svanbot:<tag> --repo SvanLabs/SvanBot` |
| `THIRD-PARTY-NOTICES.md` | `python3 scripts/notices.py` | `python3 scripts/notices.py --check` (part of the gate) |
| SBOM (CycloneDX 1.5) | `python3 scripts/notices.py --sbom target/sbom.cdx.json` | attached to the release by hand, when wanted |
| Build identity | `scripts/release.sh`, which bakes the commit into `sv10_bot::BUILD_COMMIT` | `curl 127.0.0.1:5000/api/health` reports it |

The build identity is the one worth explaining. A running bot answers with the commit it was built
from, so a release binary is never a mystery about which source produced it — and a dashboard that
reports a commit no release ever had is a build you did not make.

## Before you tag

`scripts/check.sh full` must pass on the exact commit you are about to tag. It runs rustfmt,
clippy with `-D warnings`, `cargo-deny`, the notices check, the full workspace test suite and
`tsc`. `scripts/check.sh deep` runs the same suite with the property tests at 100,000 cases, and is
what to run when the release follows a change to the engine, the evaluator or the learner — those
are the parts where a small sample can pass by luck.

Two things the gate cannot check for you:

- **A behaviour change needs a paired simulation**, not a unit test. `CONTRIBUTING.md` §8 is the
  rule and `docs/OPERATIONS.md` describes the run.
- **A performance change needs a measurement**, on the same machine, against the previous commit.

## Cutting the release

1. Confirm the gate: `scripts/check.sh full`, on a clean checkout of the commit.
2. Move the `[Unreleased]` entries in `CHANGELOG.md` under a dated heading for the new version,
   and merge that as a pull request. The release notes are generated from the merged pull requests,
   so the changelog entry is for readers who never open GitHub.
3. Tag the merge commit and push the tag:
   `git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin vX.Y.Z`.
4. Create the release **as a draft** with generated notes:
   `gh release create vX.Y.Z --draft --generate-notes --title "SvanBot vX.Y.Z"`.
5. Read the draft notes. `.github/release.yml` categorises merged pull requests into the note
   sections; anything that landed without a label falls outside every category, so check nothing
   important is missing. Add the `Generated-by:` line naming the system that wrote the notes.
6. Publish the draft. From here a mistake means a new version rather than an edit.

## Verifying a release

Nobody should run code from this project on trust — the warning in `README.md` applies to a release
as much as to `main`. There are no binaries to verify yet, so verification is of the source: check
that the CI run on the tagged commit is green, then build it yourself.

```
gh run list --repo SvanLabs/SvanBot --commit "$(git rev-list -n1 vX.Y.Z)"
```

## Status

- **Not on crates.io.** Every crate sets `publish = false`. The libraries under `crates/libs/` are
  written to be publishable — no path dependencies outside the workspace, no private registry — so
  publishing them later is a change to `Cargo.toml`, not a refactor. It has not been done.
- **MSRV tracks stable.** `rust-toolchain.toml` pins a specific version and CI builds on it. The
  `rust-version` fields say 1.98, which is not a tested minimum, only what the workspace has always
  compiled on; treat "tracks the pinned stable" as the real answer.
- **No MSRV job yet.** A `cargo hack check --rust-version` job would turn that last point from a
  caveat into a fact. Until it exists, do not claim support for an older toolchain.
