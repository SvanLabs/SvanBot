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
