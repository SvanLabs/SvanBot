//! `review all` fleet report: tendencies, tripwire, head-to-head, biggest losses (0257).

use anyhow::Result;
use std::collections::HashMap;
use sv10_core::model::{Counter, HandSummary, ModelStore};
use sv10_store::store::Store;

fn r(c: &Counter) -> String {
    if c.opp == 0.0 { "  -  ".into() } else { format!("{:.2}({})", c.hit / c.opp, c.opp as i64) }
}

/// The fleet report behind `review all [LOSERS]` (and `review <bot>`): results, tendencies,
/// tripwire, head-to-head, biggest losses. Moved verbatim out of the binary (0257).
pub fn fleet_report(store: &Store, bb: f64, which: String, losers: usize, bots: &[String]) -> Result<()> {
    let mut hero_stats = ModelStore::default();
    let mut all_rows = Vec::new();
    let mut by_bot: HashMap<String, (i64, i64, f64)> = HashMap::new();
    // A renamed bot's hands count under its current name (SvanBotV7 is SvanBotV10, 0246).
    let current = crate::identity::current_names(store);
    for b in bots.iter().filter(|b| which == "all" || which == **b) {
        let rows = store.recent_hands(b, 100_000)?;
        for row in rows {
            let Ok(sum) = serde_json::from_str::<HandSummary>(&row.summary) else { continue };
            // A row without a net is a hand we only watched after sitting down mid-hand (0315): not
            // played, so not in the results.
            if let Some(net) = row.net {
                let name = current.get(b.as_str()).cloned().unwrap_or_else(|| b.to_string());
                let e = by_bot.entry(name).or_default();
                e.0 += 1;
                e.1 += net;
                e.2 += (net * net) as f64;
            }
            // Observe only our seat's behaviour by renaming it to a common label.
            let mut renamed = sum.clone();
            for p in renamed.players.iter_mut() {
                if p.1 == *b {
                    p.1 = "HERO".into();
                }
            }
            let others: Vec<String> = renamed.players.iter().filter(|p| p.1 != "HERO").map(|p| p.1.clone()).collect();
            let mut only_hero = ModelStore::default();
            only_hero.observe(&renamed, None);
            if let Some(h) = only_hero.players.remove("HERO") {
                hero_stats.players.entry("HERO".into()).or_default().merge(&h);
            }
            let _ = others;
            all_rows.push((b.to_string(), row, sum));
        }
    }
    println!("== results");
    let (mut th, mut tn) = (0, 0);
    // By bot name, so two runs print the same (the map's order is arbitrary).
    let mut rows: Vec<_> = by_bot.iter().collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    for (b, (h, n, sq)) in rows {
        let mean = *n as f64 / *h as f64;
        let hw = sv10_stats::moments::half_width(*h as f64, *n as f64, *sq, 1.96);
        println!("{:>11}: {:>5} hands {:>8} chips {:>7.1} bb/100 ± {:.1}", b, h, n, mean / bb * 100.0, hw / bb * 100.0);
        th += h;
        tn += n;
    }
    println!("{:>11}: {:>5} hands {:>8} chips {:>7.1} bb/100", "TOTAL", th, tn, tn as f64 / bb / th.max(1) as f64 * 100.0);
    if let Some(h) = hero_stats.players.get("HERO") {
        println!("== our tendencies (hits/opportunities)");
        println!(
            "vpip {} pfr {} open {} limp {} 3bet {} call_open {} fold_to_3bet {} 4bet {}",
            r(&h.vpip),
            r(&h.pfr),
            r(&h.open_raise),
            r(&h.limp),
            r(&h.three_bet),
            r(&h.call_open),
            r(&h.fold_to_3bet),
            r(&h.four_bet)
        );
        println!(
            "bet_first f/t/r {} {} {}   fold_vs_bet {} {} {}   raise_vs_bet {} {} {}",
            r(&h.bet_first[0]),
            r(&h.bet_first[1]),
            r(&h.bet_first[2]),
            r(&h.fold_vs_bet[0]),
            r(&h.fold_vs_bet[1]),
            r(&h.fold_vs_bet[2]),
            r(&h.raise_vs_bet[0]),
            r(&h.raise_vs_bet[1]),
            r(&h.raise_vs_bet[2])
        );
        println!("cbet {} fold_to_cbet {} wtsd {} river_bluff {}", r(&h.cbet), r(&h.fold_to_cbet), r(&h.wtsd), r(&h.river_bluff));
    }
    println!("== exploitation tripwire (villain bet-first rate with us in vs out; our fold rate vs MDF)");
    let pairs: Vec<(String, HandSummary)> = all_rows.iter().map(|(b, _, h)| (b.clone(), h.clone())).collect();
    for t in crate::analysis::tripwires(&pairs) {
        println!(
            "{:>7}: bet-first {:.2} ({}) vs {:.2} ({}) z={:+.1} | we fold {:.2} of {} vs MDF {:.2}{}",
            t.street,
            t.bet_into_us,
            t.bet_into_us_n,
            t.bet_elsewhere,
            t.bet_elsewhere_n,
            t.z,
            t.our_fold,
            t.faced,
            t.mdf_fold,
            if t.flagged { "  <-- opponents target our folds: call wider" } else { "" }
        );
    }
    let fleet: Vec<String> = bots.iter().map(|b| b.to_string()).collect();
    let mut h2h: Vec<(String, crate::headtohead::HeadToHead)> =
        crate::headtohead::compute(store, &fleet).into_iter().filter(|(_, h)| h.hands >= 150.0).collect();
    h2h.sort_by(|a, b| a.1.mean().partial_cmp(&b.1.mean()).unwrap_or(std::cmp::Ordering::Equal));
    let tested = h2h.len();
    println!(
        "== head-to-head (big blinds moved between us and them in champion hands, 150+; experiment-arm hands excluded, 0361; * = beats us at 95% across all {tested} tested, z {:.2})",
        sv10_stats::normal::family_z(tested)
    );
    for (name, h) in h2h.iter().take(12) {
        println!(
            "{:>20}: {:>5} hands {:>+8.1} bb/100, own 95% upper {:>+7.1}{}",
            name,
            h.hands as i64,
            h.mean() * 100.0,
            h.upper_95() * 100.0,
            if h.beats_us(150.0, tested) { " *" } else { "" }
        );
    }
    all_rows.sort_by_key(|(_, row, _)| row.net.unwrap_or(0));
    println!("== biggest losses");
    for (b, row, sum) in all_rows.iter().take(losers) {
        let names: HashMap<usize, String> = sum.players.iter().cloned().collect();
        println!(
            "-- {} {} hole {} board {} net {} winners {}",
            b,
            &row.hand_id[..8],
            row.hole,
            row.board,
            row.net.unwrap_or(0),
            row.winners
        );
        let decisions = store.decisions_for_hand(b, &row.hand_id)?;
        let mut di = decisions.iter();
        for rec in &sum.history {
            let name = names.get(&rec.seat).cloned().unwrap_or_default();
            let mine = name == *b;
            let mut line = format!(
                "   {:?} {:>14} {:?} to {} (pot {} call {})",
                rec.street, name, rec.kind, rec.to, rec.pot_before, rec.to_call_before
            );
            if mine && let Some(d) = di.next() {
                let det = &d["detail"];
                let cands: Vec<String> = det["candidates"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| {
                        format!(
                            "{}{} {:.0}",
                            c["action"].as_str().unwrap_or(""),
                            c["amount"].as_i64().map(|a| format!("@{a}")).unwrap_or_default(),
                            c["ev"].as_f64().unwrap_or(0.0)
                        )
                    })
                    .collect();
                // A refused decision recorded no equity (#424): printing it as 0.00 would read as
                // "hero never wins", the same lie the refusal removed from the policy.
                let eq = d["equity"].as_f64().map_or("--".to_string(), |e| format!("{e:.2}"));
                line.push_str(&format!("  <= eq {eq} [{}]", cands.join(", ")));
            }
            println!("{line}");
        }
        let shown: Vec<String> =
            sum.shown.iter().map(|(s, c)| format!("{}:{}{}", names.get(s).cloned().unwrap_or_default(), c[0], c[1])).collect();
        println!("   shown {}", shown.join(" "));
    }
    Ok(())
}
