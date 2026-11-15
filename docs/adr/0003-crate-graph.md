# Six layers, one crate per layer, checked by machine

v11's Rust code is six layers, lowest first:
- **core**: the rules of the game as the venue plays them, and the shared values
- **sim**: hands played out from a seed
- **strategy**: ranges, reads and the policy
- **learn**: fits, challengers, paired sims, the Arena and probe verdicts
- **runtime**: the venue client, the store, updates and the clock
- **apps**: the binaries

A crate may depend only on crates in its own layer or below it, and no app depends on another app.
The four layers below `runtime` are **pure**: they never touch the network, the store, files, the
terminal or the clock, and they give the same result for the same inputs. Both rules are checked
by machine on every change, not by a reviewer.

## The layout

| Layer | Crate | Holds | Pure |
|---|---|---|---|
| core | `crates/core` (`svanbot-core`) | cards and the `Ah` notation, the evaluator, the rules of a hand (blinds, legal actions, raise-to, side pots, showdown), spots, the seeded generator, digests | yes |
| sim | `crates/sim` (`svanbot-sim`) | dealing, equity by enumeration and sampling, the vectorised loops, sparring archetypes | yes |
| strategy | `crates/strategy` (`svanbot-strategy`) | the action model and ranges, reads, the adaptivity class, the exploit budget, the re-solve, the pricer, the compute budget | yes |
| learn | `crates/learn` (`svanbot-learn`) | fits of action models, priors and reads; challengers; paired sims; Arena scoring; probe verdicts and the harm guard | yes |
| runtime | `crates/runtime` (`svanbot-runtime`) | the venue client, the store, updates and slots, the clock, fresh entropy | no |
| apps | `crates/apps/<name>` | `svanbot` (the bot, the dashboard and the public TV), `svanbot-release` (the updater) | no |

A crate's layer is the directory under `crates/` that holds it. A layer that later splits into
several crates keeps them under its own directory (`crates/strategy/range`), and they may depend on
each other within the layer. `web/` (the dashboard, a static export the bot serves) and `tools/`
(the Bun tasks behind the gate) are not crates and never ship inside one.

## What "pure" asks of the code

- **Time-bounded work takes its limit as an input.** The re-solve runs under the action clock, but
  `strategy` never reads the clock. The caller passes in a budget: a count of iterations, or a
  stop check. `runtime` backs the stop check with the clock, and the sim backs it with a counter,
  so a sim decision is reproducible and a live one still meets the clock.
- **Randomness comes from `core`'s seeded generator,** seeded through the inputs. Fresh entropy
  is drawn in `runtime`.
- **No `HashMap` or `HashSet` in a pure layer.** Their iteration order changes from process to
  process. Use `BTreeMap`, `BTreeSet` or a `Vec` indexed by a dense id.
- **The venue's wire format stops at `runtime`.** That covers the JSON envelope, turn tokens,
  state hashes and rate limits, all from <https://docs.openpoker.ai/llms-full.txt>. `runtime`
  translates them to `core`'s types and back, and no layer below it knows them. The game rules
  are the exception, because they *are* `core`'s rules: 6-max, 10/20 blinds, raise-to sizes, side
  pots, no rake, and the two-character card notation.
- **A pure layer takes no third-party crate** unless it is on the allowlist in the root
  `Cargo.toml` (`[workspace.metadata.layering.pure-third-party]`), with its reason beside it. The
  allowlist starts empty. Dev-dependencies are free: tests may use whatever they need.

## How it is checked

- **`clippy.toml`** bans the clock, the network, files, the environment, processes, the terminal
  and `HashMap`/`HashSet` in every crate. `runtime` and each app turn the bans off once, at their
  crate root. That one attribute is how a crate declares that it does I/O.
- **`tools/layering.ts`** reads `cargo metadata` and reports:
  - an edge to a higher layer, or from one app to another (`upward`)
  - a crate outside `crates/<layer>/` (`unplaced`)
  - a third-party crate in a pure layer that is not on the allowlist (`third-party`)
  - library code in a pure layer that turns a clippy ban off (`io-opt-out`). Integration tests
    under `tests/` may turn one off, for example to read a golden file.

The gate's commit tier, `bun tools/gate.ts commit`, runs both, so a broken rule is caught before
it is committed (V11-03).

## Considered options

- **v10's three directories, `deps/`, `libs/` and `apps/`, with 18 crates.** Rejected. The
  directory said what kind of crate a crate was, not where it sat in the graph, so the allowed
  edges could not be read from a path. The rule was kept by the `boundaries` review agent, which
  this ADR replaces with the two checks above. v11 runs no subagents, and a rule a machine can
  check should not wait on a reviewer.
- **One crate per concept** (cards, evaluator, rules, equity, range, opponent, decision, and so
  on). Deferred, not rejected. Every crate split makes an interface public and adds a manifest to
  keep. In v10, CI time was going on rebuilding crates that were never cached, not on crate
  size. A layer splits only when `/measure` shows a build-time gain.
- **cargo-deny `bans` with `wrappers`** to keep I/O crates out of pure layers. Rejected as the
  primary check. It sees only third-party crates, not our own edges or `std`'s I/O. It also runs
  in CI only, under the gate decision for V11-03.

## Consequences

- An edge appears in a manifest only once code uses it, so the empty crates of this first commit
  declare none. The graph is the rule above, and the checks hold every manifest to it.
- The workspace lists the five library crates by name. `crates/apps/*` joins the list with the
  first binary, because cargo rejects a member glob that matches nothing.
- `unsafe_code` is denied workspace-wide. A layer that needs it, such as the vectorised equity
  loop in `sim`, allows it on the smallest item that needs it, with a `SAFETY:` comment. That
  case is decided in V11-05.

## Status

accepted (2026-11-15, V11-04). It replaces v10's `deps/`/`libs/`/`apps/` layout and its
`boundaries` agent.
