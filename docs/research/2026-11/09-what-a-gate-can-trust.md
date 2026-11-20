# 09 — What a gate can trust: the pin, the artifact, and the replay

Generated-by: openclaude/deepseek-v4.1-flash-1017

Research for V11-22, the release gate that reads an Arena run. Note 08 settled *how to read* one
row and a pool of them; this note is about the step before that — whether the run a gate reads
ever happened. It comes from the build of V11-20 (PR #1105) and the probes below, run against the
merged tree at `bccad94`. It feeds V11-22 (the release gate) and V11-21 (the PR-to-run link).

## 1. What V11-20 actually guarantees

`tools/headline.ts` refuses a pin whose `commit` the repository does not hold:

```ts
const found = Bun.spawnSync(["git", "cat-file", "-e", `${commit}^{commit}`], { cwd: root });
return found.exitCode === 0;
```

That is a real check, and it stops the *honest* mistake: a hand-written `fleet/headline.json` with
an invented hash fails. But it is weaker than it reads. It asks "does this repository contain this
object?", not "did anyone run the Arena at this commit?". The V11-20 close-out said so plainly:

> the pin is traceable to a commit but not yet to a *played* run.

## 2. Three probes against the merged code

Each was run in a throwaway clone holding the real `tools/` and `README.md`, with one empty commit
to give it a history. None needs a change to our code.

**Probe A — a fabricated pin with a real commit is accepted.** A `fleet/headline.json` naming one
of the clone's own commits, champion `deadbeefdeadbeef` (not a digest any parameter set has), seed
17, and a single row of **+900,000 mbb/100 over 5,000 deals ±1,000** — a number no run produced and
no champion could support — passes both `--pin` and the check:

```
$ bun tools/headline.ts --pin fabricated.json
pinned 45576853fdcbe90325379e5020ec84b6f5da49ed and wrote its headline
$ bun tools/headline.ts
CHECK EXIT=0 — a number no run ever produced, accepted
```

**Probe B — the pin can name the commit that contains it.** Make a commit that adds the pin, then
point the pin's `commit` at that commit's own hash. `cat-file -e` finds it, because it is the
working commit. The check passes. A run cannot have happened *before* the file that describes it
was written, so this is self-referentially impossible and still accepted.

```
$ bun tools/headline.ts --pin fabricated.json   # commit = HEAD's own hash
pinned f149824d5f69604cdf93414cfa2b48228e387df0 and wrote its headline
$ bun tools/headline.ts
CHECK=0 — the pin names the commit that contains the pin itself
```

**Probe C — the pin can name a commit older than the Arena.** The repository's root commit
`b1cb58ac` predates `crates/learn/src/arena.rs`, so no `svanbot arena` existed to play it. It is in
the history, so `cat-file -e` holds it, and the check would accept a pin naming it.

## 3. Why this is not a defect in V11-20, and what it is

V11-20 was scoped to the *headline*: a number in the README that drifts from its pin, or a pin that
names nothing, fails. It does that. It was never scoped to prove the run — the ticket that owns
"the run happened" is V11-22's, and V11-20's note handed it over:

> a gate that reads a file anyone can write is not a gate.

`fleet/headline.json` is a file anyone can write. So is the artifact CI uploads. The pin is a
*pointer*; the thing a gate needs is the *evidence*, and the evidence has to be something a
hand-edit cannot forge cheaply.

## 4. What would actually tie a number to a run

Three properties, in increasing cost. A gate wants as many as it can afford.

1. **The run is replayable from its own description.** `{commit, champion, seed, bouts}` is enough
   to reproduce it: the Arena is deterministic in the seed (note 08 §3 — a deal's cards come from
   its number), and the commit fixes the code and the champion's parameters. So a gate can *replay*
   the run and compare. A fabricated number fails a replay; a hand-edited artifact fails it too.
   One condition: the replay must play each bout's deals **in order and once**. Two archetypes — the
   switching one (nit then maniac every 20 hands) and the learning one (adapts to how often we bet)
   — carry state across the deals of a bout, so the same deals in a different order are a different
   run, and the learning one is where the Arena budget actually goes (§4.2). Bouts share nothing,
   so they may replay in any order; within a bout, deal order is part of the description.
2. **The replay is affordable at release, not at every PR.** Measured here, not estimated:
   `svanbot arena 2 17` on this box, debug build, `nice -n 15 ionice -c3`, took **690 s of wall for
   1,190 s of CPU** (11.5 minutes, 1.7 cores). That is not "a few seconds": V11-20's note guessed
   27 s, which was never measured, and the run's own progress lines show the five stateless
   archetypes finishing early while the learning one ran on — consistent with the cost being its
   per-deal re-solve, though each bout was not timed separately. Either way the shape holds: one
   Arena run is minutes, so a gate can replay it once per release and cannot replay it per PR. The
   lever is unchanged — a gate that can afford the run can replay and compare, and need not trust
   the file at all — but V11-21's per-PR job records rather than replays.
3. **The artifact is keyed where it cannot be rewritten after the fact.** A CI artifact keyed by
   commit (`SVANBOT_COMMIT=$GITHUB_SHA`) is only as good as GitHub's retention and the run's own
   provenance; a file in the repo is not. If the gate replays, it does not need this — property 1
   makes the artifact redundant, and property 2 makes it affordable.

   One cheap half-step if the replay is ever too costly: sign the run with a `sha256` over its own
   text, the way `runtime::release::digest` signs an install bundle. That does not prove the run
   happened — anyone can hash a fabricated file — but it makes the artifact *tamper-evident*: a gate
   holding the digest out of band (a comment on the tag, a line in the release notes) rejects a
   later-edited artifact. It is strictly weaker than replaying and strictly stronger than nothing.
   Replay is still the answer, because it is the only one that needs no out-of-band channel.

The honest conclusion: **the release gate should replay, not read.** A gate that replays
`{commit, champion, seed, bouts}` and checks the pooled interval (note 08) against the recorded rows
cannot be fooled by a hand-written file, and costs one Arena run per release. That is V11-22's shape,
and it retires the `cat-file -e` weakness V11-20 shipped with — not by fixing the pointer check,
which is fine as a cheap early refusal, but by not depending on the pointer for the claim.

## 5. The replay was run, and it reproduced the pin

The premise of §4.1 is not a claim about the code; it is a measurement. `svanbot arena 2 17` was
run on the merged tree at `bccad94` and its rows compared to `fleet/headline.json`, which was played
at `f8c60d1`:

```
pin  : f8c60d1 308e7da4b29fd533 17
run  : deadbeef 308e7da4b29fd533 17      (SVANBOT_COMMIT was set to a placeholder)
identical rows: True | rows: 6 6
```

Every `mbb_per_100` and every `ci95` matched to the last bit, across a different commit and a
different process. So `{champion, seed}` really does pin a run down to its numbers, and a gate that
replays and compares exact rows is testing something real. It also means the *commit* is not what
makes a run reproducible — the champion digest and the seed are. That sharpens V11-22: the gate can
replay a pin without checking out its commit at all, if it has the same champion and seed; and it
must check the champion digest, because the seed alone does not name the parameters.

## 6. What this means for V11-21 and V11-22

- **V11-22 (the release gate):** read the run by replaying it, at the shared-deal bound of note 08
  §3, or it will pass a number the cards cannot justify — or a number nobody played. The pin check
  stays as a cheap first refusal; the *claim* rests on the replay. §5 shows a replay is exact and
  that `{champion, seed}` (not the commit) is what it needs. A release run at a few bouts costs
  minutes (§4.2), and note 08 §5 says the interval clears zero at about 281 shared deals a row,
  which a release run has and a 100-deal PR run does not. So the gate's run is long, and long runs
  are exactly what makes "replay it" the right test rather than "trust it".
- **V11-21 (the PR-to-run link):** a per-PR Arena job can stay read-only — it records, it does not
  gate — because the gate is at release, not at merge. The value of the PR job is the *diff*
  against the previous run (a fixed seed makes every PR replay the same cards), so a regression
  shows as a delta rather than as a number near zero.
- **The pin (V11-20):** unchanged in what it does. The README still names the run it came from, and
  the check still fails a drift or an unheld commit. What changes is that the release gate must not
  treat "the commit exists" as "the run happened".

## Sources

- `tools/headline.ts` — the `inHistory` check, and the `--pin` path.
- `crates/apps/svanbot/src/arena.rs` — `SVANBOT_COMMIT`, the seed, and `record_arena`.
- `crates/learn/src/arena.rs` — a run is named by commit, champion and seed, and is deterministic in
  the seed.
- `.github/workflows/check.yml` — `SVANBOT_COMMIT="$GITHUB_SHA" target/dev/debug/svanbot arena 2 17`,
  and the artifact it uploads.
- Probes A–C were run against the merged tree `bccad94` in a throwaway clone; no code was changed.
