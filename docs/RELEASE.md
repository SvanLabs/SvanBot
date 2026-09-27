# Release process

A release is a tagged commit on `main`, the built binaries attached to it, and a signed record of
which commit and which workflow produced them. This document is the process; `scripts/check.sh` is
the gate that decides whether a commit is releasable at all.

## What ships

| Artifact | Produced by | Verified by |
|---|---|---|
| `sv10-bot`, `learner`, `analyst` | the release workflow, cross-built | `gh attestation verify <file> --repo SvanLabs/SvanBot` |
| `THIRD-PARTY-NOTICES.md` | `python3 scripts/notices.py` | `python3 scripts/notices.py --check` (part of the gate) |
| SBOM (CycloneDX 1.5) | `python3 scripts/notices.py --sbom target/sbom.cdx.json` | attached to the release |
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
2. Update `CHANGELOG.md` and commit it. The release notes are generated from the merged pull
   requests, so the changelog entry is for readers who never open GitHub.
3. Tag: `git tag -a v10.0.0 -m "v10.0.0"` and push the tag. The tag triggers the release workflow.
4. The workflow builds the binaries, attests them, and creates the release **as a draft**.
5. Read the draft notes. `.github/release.yml` categorises merged pull requests into the note
   sections; anything that landed without a label falls outside every category, so check nothing
   important is missing before publishing.
6. Publish the draft. An immutable release pins its tag and refuses later changes to its assets, so
   this is the point after which a mistake means a new version rather than an edit.

## Verifying a download

Nobody should run a binary from this project on trust — the same reasoning as the warning in
`README.md` applies to the artifacts as much as to the source. From a machine with `gh`:

```
gh attestation verify sv10-bot --repo SvanLabs/SvanBot
```

That checks the binary against the workflow run that built it. To check the tag itself:

```
gh release verify v10.0.0 --repo SvanLabs/SvanBot
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
