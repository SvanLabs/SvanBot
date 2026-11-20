# 08 — Reading the Arena's number: what a pooled interval can and cannot say

Generated-by: openclaude/deepseek-v4.1-flash-1017

Research for V11-20, the README headline, kept from the build of PR #1105. The Arena
(`crates/learn/src/arena.rs`, `crates/apps/svanbot/src/arena.rs`) plays the champion heads-up
against the pricer and every sparring archetype and records a run in `fleet/arena.json`. This note
is about **how to read those runs honestly**: the arithmetic of pooling them, and the one
structural fact — the deals are shared — that a naive pool gets wrong. It feeds V11-20 (the
headline), V11-22 (the release gate that reads runs) and V11-23 (the league's shape).

## 1. What a row already is

A row is one opponent's `verdict` over its deals (`paired.rs:100`):

- `mbb_per_100 = mean(results) · MBB_PER_100`, where each result is the challenger's win in big
  blinds on one **duplicate-mirrored deal** — the deal played twice, once from each seat, so the
  luck of the cards cancels to first order.
- `ci95 = 1.96 · SE(mean) · MBB_PER_100`, with `SE(mean) = sample_sd / √n`.

So `(ci95/1.96)²` **is** the variance of that row's mean. It is not a per-deal standard
deviation; multiplying it by `n` would be the variance of a single deal. This distinction is the
first place a pool goes wrong.

## 2. The pool, done three ways

With `w_i = n_i / N` the share of the pooled sample a row carries, the pooled rate is
`Σ w_i · rate_i`. Its interval is where the three readings diverge:

| Reading | Half-width | Says of the pinned 600-deal-shape run |
|---|---|---|
| **Independent rows** (the first, wrong build) | `1.96 · √Σ w_i²·(ci_i/1.96)²` | ±97,636 — "clear of zero" |
| **Perfectly correlated rows** (the honest bound) | `Σ w_i·ci_i` | ±214,459 — **covers zero** |
| **Divides by n again** (the very first bug) | independent / √n | ±9,764 — absurdly sure |

The first build summed variances *and* divided by each row's deals again, printing ±9,764: it
credited the run with a sample size it did not have, squared. Reviewing it removed the extra `n`
and left the independent sum, ±97,636, which still says "clear of zero".

## 3. The fact that governs the answer: the rows share their deals

`arena.rs` plays every opponent over the **same** deal numbers —
`(number*BOUT..(number+1)*BOUT)` — and a deal's cards come from its number (`paired.rs`: a deal is
drawn from its own stream of the seed). So the six rows of a run are six measurements of **one set
of 100 deals**, played six ways, not 600 independent deals. Two opponents both winning deal 7 is
not two pieces of evidence; it is one deal's cards, seen through two policies.

That makes the rows **positively correlated**, and a positive correlation makes the sum of
variances too narrow. With `SE_i` each row's standard error and `z_d` the pooled result on deal
`d`, the pooled SE is `SD(z)/√N` and `SD(z) ≤ (1/k)Σ SE_i` by Cauchy–Schwarz, so

    1.96·SE_pooled ≤ Σ w_i·ci_i.

The right-hand side is exactly the perfectly-correlated figure, and it is the only bound the data
actually supports. For equal rows it is `√k` times the independent one — 2.2× on the committed run,
because the `nit` row is far tighter than the rest.

## 4. What this costs, and what it buys

- **It costs the headline its number today.** At 6 opponents × 100 shared deals the interval is
  ±214,000 mbb/100 around +128,000, and it covers zero. The block says so. This is not a defect in
  the tool; it is the true power of a 100-deal league, and it is the same reason CI's Arena job
  already says (V11-15) that two bouts a line "straddles zero by ±200,000 mbb/100".
- **It buys an honest gate.** V11-22's bar is "the interval above zero". A gate that read the
  independent pool would pass a run the shared deals cannot justify. Reading the correlated bound
  is conservative in the safe direction: a run that clears it clears any real correlation.
- **The lever is deals, not opponents.** `√k` comes from a wider pool only because more rows are
  more measurements of the *same* deals; to narrow the interval you play more deals. A run needs
  roughly `(214,459/9,764)² · 100 ≈ 4,800` shared deals before the pinned interval would clear zero
  at this effect size — about 96 bouts, well inside the box's reach for a release run (V11-22).

## 5. Smaller readings the build settled

- **Deals are counted once.** "over 100 deals", not "600", because the 600 is 100 deals replayed
  six ways. Summing rows would inflate the sample by the opponent count.
- **Three significant figures, at every scale.** ±201,399 over 100 deals cannot support the tens;
  the block prints 201,000. The first formatter honoured three figures only under 1,000.
- **Zero is a coin flip, both ways.** A run whose interval covers zero says "not yet
  distinguishable from zero"; a run clear of zero *below* says it is behind, not "indistinguishable"
  — an interval under zero is distinguishable from it.
- **An unmeasured row poisons the pool.** A row with `ci95 = 0` measured nothing; taking it as an
  exact zero would be the widest possible claim from the row that said the least. The pool's
  interval is then "not measured", not narrow.
- **The pin must be traceable.** The check refuses a pin whose `commit` the repository does not
  hold (`git cat-file -e`), so a hand-written `fleet/headline.json` with a matching README cannot
  pass. This is the same concern V11-22 raises from the other side — "a gate that reads a file
  anyone can write is not a gate" — and it does not fully close it: a commit that exists but was
  never *played* would still pass. Closing that needs the run itself recorded (an artifact keyed by
  commit), which is V11-21's and V11-22's to settle.

## Sources

- `crates/learn/src/paired.rs` — `verdict`, and the deal-from-its-number stream.
- `crates/learn/src/arena.rs`, `crates/apps/svanbot/src/arena.rs` — every opponent plays the same
  deal numbers; a run is named by commit, champion and seed.
- `.github/workflows/check.yml` — the Arena job's own note that a 100-deal line straddles zero.
- The pooled-interval bound is Cauchy–Schwarz on `SD(Σ)`, standard; no external citation needed.
