# The luck of decisions

## The maniac's randomized bets

V11-72 measured 400 deals per league on seed 7, with 20 re-solve iterations,
the default reference policy, and its decision-draw correction enabled. Heads up
mirrored each deal twice. At 6-max, the challenger rotated through every seat.
The challenger started a fresh bout reader every 50 deals, as in the Arena.

The prototype called the existing maniac decider and counted its legal raises
across all hands of each deal. The maniac raises only when its random roll falls
below 0.65 and the raise is legal. The response was the challenger's corrected
result in big blinds per hand. An ordinary least-squares fit with an intercept
measured the association between the raise count and that result.

| League | Deals | R² |
|---|---:|---:|
| Heads up | 400 | 0.022813 |
| 6-max | 400 | 0.036392 |

The prototype took 159 seconds on the shared measurement workspace. Both
estimates fell below the 0.50 threshold. The result is a no-go for this opponent
term. The raise count explained about 2.3% of the measured variance heads up and
3.6% at 6-max. This test leaves the Phase 0 2× target unsupported by the measured
decision terms. The sequential margin check can stop as soon as the evidence
decides it. Checks that stay unresolved need more deals.

The regression measures association in this prototype. The count also depends on
how many legal betting opportunities a deal offered.

## The reference policy's decision draws

V11-71 compared the Arena before and after enabling the decision-draw correction.
The baseline was the V11-70 merge tree (`8e921ed`). The experimental wiring
was `5fb469e`. Each run used default champion parameters, 20 bouts (1,000 deals
per row), 20 re-solve iterations, and a scratch fleet. Seeds 7 and 11 were
measured in both leagues. A ratio above 1 means the correction reduced variance.

For each row, the ratio is the square of the baseline 95% half-width divided by
the corrected half-width. For each pooled ratio, the seven standard rows were
averaged per matching bout, then the variance of those 20 bout means was compared.
This keeps the covariance across opponents that played the same seed.

| Opponent | Heads up, 7 | Heads up, 11 | 6-max, 7 | 6-max, 11 |
|---|---:|---:|---:|---:|
| pricer | 0.962× | 0.980× | 1.000× | 0.980× |
| v10.0.2 | 0.969× | 0.985× | 0.961× | 1.003× |
| calling station | 0.992× | 0.997× | 1.000× | 1.000× |
| nit | 1.000× | 1.000× | 1.000× | 1.000× |
| maniac | 0.973× | 0.987× | 1.000× | 1.000× |
| switching | 0.961× | 0.996× | 1.000× | 1.000× |
| learning | 0.984× | 0.980× | 1.000× | 1.000× |
| Pooled | 0.824× | 0.955× | 1.006× | 1.008× |

All before/after row means agreed within the wider corresponding interval.

The pooled heads-up ratio missed the 2× threshold, so the Arena wiring
stays experimental. The opt-in hook remains available. Runs made with the
experimental wiring use corrected results and cannot be compared bout for bout
with runs that use the original measurement.

| Run | Baseline wall time | Corrected wall time |
|---|---:|---:|
| Heads up, seed 7 | 12m43.133s | 18m37.147s |
| Heads up, seed 11 | 17m11.620s | 19m44.090s |
| 6-max, seed 7 | 19m31.797s | 18m54.894s |
| 6-max, seed 11 | 19m30.665s | 10m7.156s |

Wall times include contention from other runs and checks in the same workspace.
