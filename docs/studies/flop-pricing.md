# Flop large-bet residual: retain the pricing policy

Generated-by: codex/gpt-6

Decision for [the continuation investigation](https://github.com/SvanLabs/SvanBot/issues/529):
retain the existing pricing policy. Reject installing the observed positive residual as an action
bonus. No parameter, correction cap, or promotion threshold changes. This closes the actionable
intervention decision; it does not establish that current continuation prices are calibrated.

The frozen 5,000 played-hand cohort ends 2026-09-29 19:46:50 UTC, SHA256
3ad69619396305171ac00de6febfb648bc520bff052f569603c3ad5e595b45cd. All 576 big-flop-bet calibration
rows match their actual chosen candidates and a unique hero raise in the recorded hand history.
The corrected residual is realized final-stack change minus the candidate's installed price.
Its mean is +11.953 bb, hand-cluster 95% interval [+6.586,+17.320]. Installed bias was zero.

| Split | Hands | Mean residual, bb | Hand-cluster 95% interval |
|---|---:|---:|---|
| Before 13:00 UTC | 296 | +7.627 | [+1.841,+13.414] |
| From 13:00 UTC | 280 | +16.526 | [+7.353,+25.699] |
| Ordinary | 444 | +12.471 | [+6.339,+18.604] |
| Control | 132 | +10.210 | [-.919,+21.338] |
| Reconstructed effective stack under 200bb | 386 | +11.420 | [+5.937,+16.903] |
| Reconstructed effective stack at least 200bb | 190 | +13.035 | [+1.151,+24.919] |

[Full strata](flop-pricing-strata.json) retain exact chip-size/pot ratios, six positions, starting
and reconstructed effective stacks, 108 active-opponent groups, final hero action and both arms.
Opponent identifiers are hashed; a multiway hand occurs in more than one opponent group.
Stacks are reconstructed from the recorded contribution ledger, not claimed as original replay
snapshots. Exact-size cells are often small. These are descriptive intervals without multiple
comparison or session-dependence correction, not independent causal estimates.

Continuation matters to interpretation: hands ending on the flop with a hero raise average
+3.940 bb residual (281), later river folds -28.142 (27), and later river raises +33.396 (61).
Those populations were selected by later actions and outcomes. Their differences cannot be turned
into an action bonus: a changed flop decision changes which continuation gets played.

A diagnostic paired intervention adds +11.953 bb to `flop:bet:big`, leaving every other frozen
champion parameter fixed. Unlike re-solving at shared candidate prices, its result is settled by
the poker engine over complete continuations on identical deals. The 16-profile opponent pool
comes from the frozen model snapshot. Source policy is unchanged by the later feature-layout work;
these runs use no neural model. Seed 91637, 32 tables × 1,500 hands per stack depth:

| Stack | Challenger minus champion, bb/100 | Paired 95% interval |
|---|---:|---|
| 100bb | -63.39 | [-72.98,-53.81] |
| 600bb | -75.28 | [-101.00,-49.56] |

[Exact paired parameters and model digest](flop-pricing-paired.json) record the comparison.
This rejects that intervention in the tested synthetic pool. It does not prove the observational
residual's cause, validate every shared price, or license extrapolation to the fleet's profit.
The retained 141 replay snapshots are selected expensive decisions; exact recorded profile/net
identity cannot be recovered for the remaining class observations from a later model snapshot. Do not
manufacture that identity or describe this study as 576 independently revalued original replays.
The diagnostic is clearly negative and is not submitted to fresh-deal sequential promotion.

Reproduce the aggregate report with `scripts/studies/poker.py`: supply the closed cohort copy,
its expected SHA256 and a distinct output file. It opens SQLite with read-only immutable/query-only
settings and validates packed lengths/CRC. The frozen inputs and local driver are retained in the
operator workspace; raw hands and opponent profiles are intentionally outside the Git repository.
Future intervention work needs representative original replay identity and independent continuation
prices before proposing another correction. This report keeps that limitation explicit while
resolving the current retain-versus-install decision.
