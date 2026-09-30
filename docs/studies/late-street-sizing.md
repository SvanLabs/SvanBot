# Late-street sizing: retain the mixed policy

Generated-by: codex/gpt-6

Decision for [the mixed-policy sizing investigation](https://github.com/SvanLabs/SvanBot/issues/530):
retain the current temperature and size grid. The tested alternatives do not establish a gain that
clears the unchanged +1 bb/100 lower-bound promotion rule. No strategy or validation gate changes.
This is a decision about available interventions, not a conclusion that every continuation price
or candidate grid is optimal.

The same frozen 5,000 played hands end 2026-09-29 19:46:50 UTC, SHA256
3ad69619396305171ac00de6febfb648bc520bff052f569603c3ad5e595b45cd. Selected deep audits cover
1,867 decisions, 10.5% of the 17,761 decisions. Their late-raise gaps are 0.0849 bb on 160 turn
raises and 0.1280 bb on 130 river raises. Those are selected tactical disagreements at recorded
prices, not measured causal losses or independent validation of a shared price.

Extend the analysis to every recorded late-street hero raise with candidate prices: 1,296 raises,
including 873 turn and 423 river raises. Compute live maximum candidate EV minus actual chosen
candidate EV, using the prices recorded for that decision. This includes intentional softmax
choices; neither a lower-EV sampled action nor an altered deep best size is automatically a bug.

| Street / arm | Decisions | Mean live-price gap, bb | Hand-cluster 95% interval |
|---|---:|---:|---|
| Turn, all | 873 | .02684 | [.01484,.03884] |
| Turn, ordinary | 548 | .03210 | [.01379,.05041] |
| Turn, control | 167 | .02515 | [.00938,.04091] |
| Turn, treatment | 158 | .01037 | [.00160,.01914] |
| River, all | 423 | .05040 | [.02798,.07282] |
| River, ordinary | 273 | .03859 | [.01744,.05973] |
| River, control | 79 | .05487 | [.00132,.10841] |
| River, treatment | 71 | .09085 | [.00272,.17898] |

[Full aggregate evidence](late-street-sizing-strata.json) separates arms and chronological halves,
and keeps the selected deep-audit comparison distinct. Intervals cluster repeated decisions within
a hand, use a normal approximation and do not remove session dependence or multiple-comparison
selection. Original replay identity cannot be reconstructed for every decision from the 141
retained snapshots. This full recorded-price census is not presented as representative deep
re-solving or independent revaluation of all original states.

Independently test the actual mixed policies on complete engine-settled continuations. Freeze the
cohort's final champion parameters and 16-profile opponent snapshot; the cohort itself spans two
champion versions, so this does not claim to replay each historical decision's parameters. Change
only temperature from .007 to .001, or expand the four-size grid to seven sizes by adding its three
midpoints. These are global interventions, not isolated late-street changes. Both arms receive the
same cards; no neural artifact is loaded. Use 32 tables × 1,500 hands per comparison. A second
fixed seed checks replication; these fixed-size diagnostics do not substitute for the
learner's sequential fresh-deal confirmation.

| Alternative | Stack | Seed 91637 bb/100 [95%] | Fresh seed 91639 bb/100 [95%] |
|---|---|---|---|
| Lower temperature | 100bb | +2.81 [-1.01,+6.63] | +1.97 [-2.10,+6.05] |
| Lower temperature | 600bb | +2.18 [-9.69,+14.04] | +1.35 [-9.64,+12.34] |
| Denser grid | 100bb | -1.78 [-6.34,+2.79] | +1.09 [-3.52,+5.70] |
| Denser grid | 600bb | -8.28 [-22.08,+5.52] | -1.83 [-14.34,+10.69] |

[Exact parameters, seeds and model digest](late-street-sizing-paired.json) preserve all eight runs,
384,000 paired hands in total. Every interval includes zero and its lower bound remains below +1 bb/100. No
alternative qualifies for promotion. Keep the mixed policy: selected price gaps do not justify
reducing all raises, removing mixing, or installing a denser grid. New intervention work must first
supply representative original replay identity and independent prices; it must then pass the
existing fresh-deal sequential confirmation rather than selecting a favorable diagnostic run.

Reproduce the aggregate census with `scripts/studies/poker.py --study sizing`, supplying the closed
cohort copy, expected SHA256 and a distinct output file. Packed lengths/CRC and the read-only source
contract are shared with the flop study. Raw hands and opponent profiles remain in the operator's
workspace; Git retains derived evidence and the reproducible analysis utility.
