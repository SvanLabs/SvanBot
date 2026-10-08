# Top-five control: current and recent-season baseline

Research for [Establish why the fleet does not hold all five leading places](https://github.com/SvanLabs/SvanBot/issues/955), part of [Sustain SvanBot ownership of leaderboard places 1–5](https://github.com/SvanLabs/SvanBot/issues/954).

## Finding

At **2026-10-08 22:46:59 UTC**, the fleet owns places **1, 2, 4, 5 and 6**. Quietflute is third. The weakest fleet bot, SurSvan, trails the strongest outsider by **65,021 chips**; Svanism trails by **46,301**, and SuraGunnar by **2,191**. This establishes the current score deficit, not its cause. The official frozen season 13 result confirms the fleet previously finished in all five leading places. Its weakest finisher then led Quietflute by **529,302 chips**. [Current qualified score leaderboard](https://api.openpoker.ai/api/season/leaderboard?sort_by=score&min_hands=10&limit=20), [frozen season 13 leaderboard](https://api.openpoker.ai/api/season/ac9f1eb2-935d-439b-93df-55237eefe1cc/leaderboard).

All five bots have positive recorded net in the fixed 24-hour and 72-hour windows. Their 24-hour completed-hand rates are similar; SurSvan has the highest rate. A simple lack-of-volume explanation for the weakest bot is therefore unsupported in this window. The sample does not identify a policy defect, prove improvement, or justify exposing the leaders to a change. Variance, different opponent populations, cumulative earlier results and outsider performance remain competing explanations.

## Sources, cutoff and population

The official current-season response identifies season 14, starting **2026-10-04 11:46:56.032131 UTC**, ending **2026-10-18 11:46:56.032131 UTC**. The leaderboard request ran from **22:46:59.183984** to **22:46:59.449386 UTC**; cutoff is **22:46:59.449410 UTC**. The local main-store read finished at **22:47:00.398294 UTC**. These are nearby independent observations, not a transactional snapshot of the venue and local store. The season window is **107.001 wall-clock hours**, not each bot's measured seated time. [Official season](https://api.openpoker.ai/api/season/current).

Only the five configured public fleet bots are included in local hand statistics. Rows are selected by completed-hand timestamp with an inclusive window start and exclusive cutoff, using the per-bot/time index. A positive recorded per-hand big blind is required for rates. Every selected row records a big blind of **20**. Net and EV populations exclude missing values separately; missing outcomes are never zero-filled. [Stored hand definitions](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/libs/store/src/store/hands.rs), [per-hand blind semantics](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/libs/store/src/store/hands/blinds.rs).

The local installed-release record names **9e1f228**, installed on 2026-10-05; the update check names main **7e7fd17** with 45 commits pending. The research checkout is 7e7fd17. Metrics span earlier binary/parameter eras; they are not an isolated treatment. The inspected stored EV implementation is identical at installed 9e1f228 and this checkout. The current store contains a shared parameter record and a Svanar slot override; its latest stored promotion changed call margin on October 5. Neither rank nor the cross-bot table below estimates that promotion's effect. Local sources: release-progress.json, update-check.json and the narrowly selected parameter/promotion kv keys in the live artifacts directory. [Lineage design](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/docs/adr/0002-five-lineages-one-gate.md).

## Frozen leaderboard

| Rank | Bot | Score | Venue hands | Rebuys | Account chips | Table chips |
|---|---|---:|---:|---:|---:|---:|
| 1 | SvanBotV10 | 757887 | 8274 | 0 | 725663 | 32224 |
| 2 | Svanar | 676656 | 8092 | 0 | 590350 | 86306 |
| 3 | Quietflute | 647562 | 8491 | 0 | 369651 | 277911 |
| 4 | SuraGunnar | 645371 | 8363 | 0 | 464883 | 180488 |
| 5 | Svanism | 601261 | 8204 | 0 | 568758 | 32503 |
| 6 | SurSvan | 582541 | 8251 | 4 | 527093 | 55448 |
| 7 | allInAmazonia | 474760 | 8815 | 0 | 456285 | 18475 |

The documented score is account chips plus table chips. Default starting funding is 5,000 chips; each rebuy contributes 1,500 and the default score penalty is zero. SurSvan's four rebuys therefore add **6,000 funded chips**. Its score minus documented funding is **571,541**, versus **571,910** locally recorded net: the **369-chip difference is unreconciled**, not evidence of extra winnings or losses. The equivalent local-net-minus-funding-adjusted-score differences are +631 for SvanBotV10, +3,235 for Svanar, 0 for SuraGunnar, and +196 for Svanism. In-progress pots, asynchronously refreshed stacks, missing records and settlement timing prevent exact reconciliation without an account/hand ledger. Banking between account and table does not by itself change the documented score. [Official scoring and rebuy rules](https://docs.openpoker.ai/llms-full.txt).

## Results at the fixed cutoff

Returns are observed from the actual table populations, not paired experiments. Each interval is the descriptive normal approximation, mean ± 1.96 × sample-standard-deviation / square-root(hand count), on each hand's return in big blinds. Heavy tails and repeated opponents weaken the independent-hand approximation; these are not calibrated causal confidence intervals or promotion evidence. The three windows overlap and cannot be counted as independent confirmations.

| Window | Bot | Stored hands | Hands/wall hour | Recorded net chips | Net chips/wall hour | Raw bb/100 (95% interval) | Stored adjusted bb/100 (95% interval) |
|---|---|---:|---:|---:|---:|---|---|
| season14 | SvanBotV10 | 8,270 | 77.29 | +753,518 | 7,042 | 455.6 (269.5 to 641.7) | 456.0 (281.7 to 630.4) |
| season14 | Svanar | 8,083 | 75.54 | +674,891 | 6,307 | 417.5 (302.6 to 532.3) | 439.1 (330.6 to 547.5) |
| season14 | SuraGunnar | 8,360 | 78.13 | +640,371 | 5,985 | 383.0 (210.9 to 555.1) | 430.6 (296.3 to 565.0) |
| season14 | Svanism | 8,199 | 76.63 | +596,457 | 5,574 | 363.7 (265.8 to 461.7) | 318.5 (233.7 to 403.3) |
| season14 | SurSvan | 8,246 | 77.06 | +571,910 | 5,345 | 346.8 (239.4 to 454.2) | 346.5 (247.1 to 445.8) |
| 72h | SvanBotV10 | 5,518 | 76.64 | +519,283 | 7,212 | 470.5 (216.9 to 724.2) | 464.6 (226.8 to 702.4) |
| 72h | Svanar | 5,333 | 74.07 | +381,006 | 5,292 | 357.2 (216.3 to 498.1) | 388.4 (254.6 to 522.1) |
| 72h | SuraGunnar | 5,690 | 79.03 | +415,402 | 5,769 | 365.0 (126.6 to 603.4) | 421.1 (240.7 to 601.6) |
| 72h | Svanism | 5,516 | 76.61 | +391,918 | 5,443 | 355.3 (235.8 to 474.7) | 312.3 (210.4 to 414.1) |
| 72h | SurSvan | 5,589 | 77.62 | +383,350 | 5,324 | 343.0 (222.7 to 463.2) | 355.5 (246.8 to 464.2) |
| 24h | SvanBotV10 | 1,821 | 75.88 | +197,382 | 8,224 | 542.0 (-64.0 to 1148.0) | 532.6 (-55.7 to 1120.9) |
| 24h | Svanar | 1,739 | 72.46 | +97,698 | 4,071 | 280.9 (41.8 to 520.0) | 292.6 (64.0 to 521.1) |
| 24h | SuraGunnar | 1,836 | 76.50 | +35,232 | 1,468 | 95.9 (-422.1 to 614.0) | 281.3 (-79.6 to 642.1) |
| 24h | Svanism | 1,803 | 75.12 | +74,723 | 3,113 | 207.2 (34.8 to 379.6) | 157.0 (2.5 to 311.5) |
| 24h | SurSvan | 1,842 | 76.75 | +110,014 | 4,584 | 298.6 (94.3 to 502.9) | 292.2 (106.7 to 477.7) |

The 24-hour flagship interval and both SuraGunnar intervals cross zero despite positive net. Between-bot intervals broadly overlap. No significant ranking of policy strength is established. The accounting identity of chips/hour as hands/hour times average chips/hand is useful for description, but does not separate table opportunity from policy skill.

**Stored adjusted return has limited verification semantics.** The EV filler stores raw net for ordinary hands and also for unverifiable all-ins; a non-null ev_net does not prove an all-in was reconstructed. The table reports existing stored adjusted return without substituting net for missing EV. The number of rows where stored EV differs from net is only a lower bound on affected/verified all-in observations, not an all-in verification rate. This bounded study does not reconstruct every all-in or remove non-all-in luck. [Installed EV filler and fallback](https://github.com/SvanLabs/SvanBot/blob/9e1f228f5621ed54a7f2537224ad2a1fbf9ab2c6/crates/apps/bot/src/luck.rs), [all-in estimator](https://github.com/SvanLabs/SvanBot/blob/9e1f228f5621ed54a7f2537224ad2a1fbf9ab2c6/crates/libs/model/src/allin.rs).

## Outcome and export coverage

| Bot | Local completed / venue hands, season 14 | Known local net | Non-null stored EV | EV differs from net | Retained season-14 exports | Export IDs absent from local window |
|---|---:|---:|---:|---:|---:|---:|
| SvanBotV10 | 8,270/8,274 | 8,270 | 8,269 | 452 | 8,224 | 4 |
| Svanar | 8,083/8,092 | 8,083 | 8,083 | 420 | 188 | 2 |
| SuraGunnar | 8,360/8,363 | 8,360 | 8,360 | 464 | 471 | 0 |
| Svanism | 8,199/8,204 | 8,199 | 8,199 | 450 | 729 | 1 |
| SurSvan | 8,246/8,251 | 8,245 | 8,245 | 438 | 188 | 0 |

The local/venue count ratios are **99.89–99.96%**, but the two counters need not define the same in-progress boundary. This is a count comparison, not proof that every venue hand or outcome was retained. SurSvan has one missing local net/EV; SvanBotV10 has one net with EV not yet filled. Other selected local net and EV values are present.

In the 72-hour and 24-hour windows, the retained export store has **no child-bot exports**. The flagship has 5,468 exports in 72 hours and 1,770 in 24 hours, versus 5,518 and 1,821 local completed rows. Export start time and main-store end time differ, so window-boundary ID differences alone are not necessarily missing hands. The child-export deficit makes independent profit reconciliation impossible from these retained sources. It does not imply the child bots stopped playing. [Export schema, timestamp and profit-column semantics](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/apps/bot/src/history/db.rs).

## Opponent mix and useful progress

Position counts below use the recorded button and participating-seat order; other combines the remaining positions. Heads-up button/small-blind overlap is distinguished by the reproduction script. No selected 24-hour hand required that separate label. Opponent shares count dealt-in opponent-seat observations, not their actions or transferable skill. No raw hands or opponent profiles are published.

| Bot | BTN/SB/BB/other hands, 24h | Distinct opponents | Largest opponent share | Top-ten share | Hands with Quietflute | Gaps ≥5 min / total span | Table-seeking events |
|---|---:|---:|---:|---:|---:|---|---:|
| SvanBotV10 | 304/305/305/907 | 68 | 15.3% | 54.5% | 0 | 0 / 0.0 min | 19 |
| Svanar | 295/295/288/861 | 62 | 10.1% | 55.7% | 0 | 2 / 15.0 min | 9 |
| SuraGunnar | 308/310/310/908 | 60 | 20.1% | 59.9% | 0 | 3 / 17.3 min | 0 |
| Svanism | 304/304/300/895 | 83 | 6.6% | 43.9% | 239 | 9 / 61.1 min | 34 |
| SurSvan | 312/313/316/901 | 75 | 10.5% | 48.0% | 0 | 1 / 5.1 min | 7 |

The opponent populations differ materially: SuraGunnar's most frequent opponent accounts for 20.1% of opponent-seat observations in 24 hours. Quietflute appears in 239 Svanism hands in that window and zero for the other four; across the season it appears in 398 Svanism and 420 SurSvan hands. These are observations of exposure, not a head-to-head chip-flow estimate. They cannot explain Quietflute's winnings at other tables. [Recorded position semantics](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/libs/engine/src/situation.rs), [table-quality inputs](https://github.com/SvanLabs/SvanBot/blob/9e1f228f5621ed54a7f2537224ad2a1fbf9ab2c6/crates/apps/bot/src/client/quality.rs).

Every ≥5-minute intercompletion gap in the last 24 hours begins and ends at the **same table**. Svanism's nine gaps total 61.1 minutes, but this is not measured downtime: long hands, opponent delays and incomplete local observations remain possible. Its 34 table-seeking events establish activity, not their cost or benefit. No missing-state, no-completed-hand watchdog, or decision-timeout/panic warning was found in the covered recent event sample. Two already-seated errors and one leave-pending error occurred; their presence alone does not show a long stall. The event sample is capped at the newest **30,000** rows before cutoff and begins **2026-10-07 21:32:32 UTC**, fully covering 24 hours but not 72 hours.

The older confirmed seated/idle episode and missing-state fallback are already tracked. Their cited September 29–30 evidence predates season 14, so it cannot establish this season's deficit. Use their existing decision paths: [Investigate: a bot sits seated and idle for 15 minutes after its table closes](https://github.com/SvanLabs/SvanBot/issues/933), [Triage: a turn with no parsed situation is folded whatever the cards](https://github.com/SvanLabs/SvanBot/issues/920).

## Retained rank history and remaining uncertainty

The inspected persisted sources contain frozen past-season leaderboards, a few season-boundary snapshots, the latest experiment-mode state, and findings-scan payloads. The scan payload schema contains calibration/classes/findings rather than leaderboard observations. The dashboard's leaderboard comparison state is an in-memory cache; it does not provide a persisted 24-hour/72-hour outsider score series. No such complete series was found in the bounded sources inspected. This is not a claim that none exists anywhere. [Leaderboard cache](https://github.com/SvanLabs/SvanBot/blob/9e1f228f5621ed54a7f2537224ad2a1fbf9ab2c6/crates/apps/bot/src/api/insights/leaderboard.rs), [findings snapshots](https://github.com/SvanLabs/SvanBot/blob/7e7fd17afbd8c7358ab7f3fc50f619d1967c9fb4/crates/libs/store/src/store/scans.rs).

The frozen season-13 ending is SvanBotV10 1,964,043; Svanism 1,813,869; SurSvan 1,577,024; Svanar 1,568,234; SuraGunnar 1,533,769; Quietflute 1,004,467. Locally retained October 3 and October 4 checks also show all five leading positions before that ending. These isolated successes establish neither sweep duration nor season-wide occupancy. No retained observations establish current-season time owning all five positions, recovery duration, outsider 24-hour/72-hour velocity, or across-season consistency. [Official frozen ending](https://api.openpoker.ai/api/season/ac9f1eb2-935d-439b-93df-55237eefe1cc/leaderboard).

Before selecting an intervention, the decision needs comparable outsider score history; a way to distinguish slow hands from actual unavailable playing time; outcome/EV verification status rather than non-nullness alone; and candidate evidence matched to the installed policy and actual opponent population. This baseline supports investigating those competing explanations. It does not select a strategy change, quantify tenfold headroom, or replace the unchanged paired fresh-deal promotion gate. The existing measurement decision already distinguishes lower-bound edge evidence from raw-chip season impact. [[grilling] Define better in numbers: metric, baseline and headroom before building](https://github.com/SvanLabs/SvanBot/issues/707).

## Reproduction and preservation

The local research directory beside this checkout contains public-snapshots.json, aggregate-baseline.json and query-baseline.py, all ignored runtime artifacts. The script performs unauthenticated GETs, then opens live SQLite files with **mode=ro**, enables **query_only**, caps each connection at 15 seconds of query work, and closes read transactions before analysis. It never opens credentials, mutates live state, trains, builds or copies databases. Live WAL is included; immutable mode is intentionally not used for these actively written stores.

Main-hand query, run once per configured bot, is SELECT hand_id, ended_at, net, ev_net, hero_seat, summary, table_id FROM hands INDEXED BY hands_bot_time WHERE bot=? AND ended_at>=? AND ended_at<? ORDER BY ended_at, with the official season start and cutoff above. SQL summaries stay text in this schema; JSON is decoded only for the selected season's roughly 41,000 rows. Window filters and per-hand blind normalization are applied in memory. History uses raw_time and the same bounded season start/cutoff on started_at, reading only bot, hand_id, started_at and profit. Events use the fixed maximum ID **194130**, lower bound **164130**, cutoff and LIMIT 30000. The gap and seeking refinement reads only each bot's indexed 24-hour timestamps/table IDs and the same bounded event range.

Artifact SHA-256 values at publication preparation:

- aggregate-baseline.json: `8a62ea00081239ce93d955bcfb92baf191add48ae3a29cc8bdc961cb1652ce2f`
- public-snapshots.json: `feccf0c88573a920d268461edf6c9369172c61c419d6d058ba213dbdbb18b9b5`

A later rerun refreshes the public responses and may see backfilled local values; it cannot reproduce the old upstream snapshot exactly. This note preserves the fixed aggregate tables. Raw local hands, table identifiers and detailed opponent records are not included.

Generated-by: codex/gpt-6
