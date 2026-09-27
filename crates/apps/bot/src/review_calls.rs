//! `review raise-wars` / `timing-tells` / `sizing-tells`: opponent studies (0257).

use anyhow::Result;
use std::collections::HashMap;
use sv10_core::model::HandSummary;
use sv10_store::store::Store;

pub fn raise_wars(store: &Store) -> Result<()> {
    use crate::raisewar::{Commit, bin, commits_from_store};
    let (decisions, commits) = commits_from_store(store)?;
    println!("{decisions} postflop decisions; {} heads-up decisions that committed the hand to showdown", commits.len());
    println!("gap = live estimate minus exact equity vs the shown hand (positive: we over-estimated)");
    let row = |label: String, set: Vec<&Commit>| {
        let b = bin(&set);
        if b.n > 0 {
            println!("{label:34} n {:5}  estimate {:.3}  exact {:.3}  gap {:+.3} ± {:.3}", b.n, b.estimate, b.exact, b.gap, b.half_width);
        }
    };
    for (kind, call) in [("calls of an all-in", true), ("our raises/shoves that got called", false)] {
        println!("-- {kind}");
        row("all".into(), commits.iter().filter(|c| c.call == call).collect());
        for r in 0..4 {
            let label = if r == 3 { "3+ raises before".to_string() } else { format!("{r} raises before") };
            row(label, commits.iter().filter(|c| c.call == call && (c.raises_before == r || (r == 3 && c.raises_before > 3))).collect());
        }
        for (s, name) in ["flop", "turn", "river"].iter().enumerate() {
            row(name.to_string(), commits.iter().filter(|c| c.call == call && c.street == s).collect());
        }
        for (lo, hi) in [(0.0f64, 0.5f64), (0.5, 0.65), (0.65, 0.8), (0.8, 1.01)] {
            row(
                format!("estimate {lo:.2}-{:.2}", hi.min(1.0)),
                commits.iter().filter(|c| c.call == call && c.estimate >= lo && c.estimate < hi).collect(),
            );
        }
        // The villain's bet relative to the pot before it (2026-09-23): a 180x-pot turn shove was called with a set.
        for (lo, hi) in [(0.0f64, 0.75f64), (0.75, 1.5), (1.5, 4.0), (4.0, f64::INFINITY)] {
            let label = if hi.is_finite() { format!("villain bet {lo}-{hi}x pot") } else { format!("villain bet {lo}x+ pot") };
            row(label, commits.iter().filter(|c| c.call == call && c.bet_to_pot >= lo && c.bet_to_pot < hi).collect());
        }
        // Where the chips are (2026-09-23): three deep raise wars cost ~4,500 bb in one day while the
        // averages looked calibrated. Pot before our action, in big blinds.
        let bbv = store.latest_big_blind()?.unwrap_or(crate::live::DEFAULT_BIG_BLIND) as f64;
        for (lo, hi) in [(0.0f64, 100.0f64), (100.0, 500.0), (500.0, f64::INFINITY)] {
            let label = if hi.is_finite() { format!("pot {lo:.0}-{hi:.0} bb") } else { format!("pot {lo:.0}+ bb") };
            row(label, commits.iter().filter(|c| c.call == call && (c.pot as f64 / bbv) >= lo && (c.pot as f64 / bbv) < hi).collect());
            row(
                "  … with 2+ raises before".to_string(),
                commits
                    .iter()
                    .filter(|c| c.call == call && c.raises_before >= 2 && (c.pot as f64 / bbv) >= lo && (c.pot as f64 / bbv) < hi)
                    .collect(),
            );
        }
    }
    let bb = store.latest_big_blind()?.unwrap_or(crate::live::DEFAULT_BIG_BLIND) as f64;
    let fit = crate::raisewar::fit_river_jam_call(&commits);
    println!(
        "-- river all-in call fit (0159): n {}  older-half over-estimate {:+.3}  held-out over-estimate {:+.3} (95% lower {:+.3})  held-out saved {:+.1} bb/call (95% lower {:+.1})  {}",
        fit.n,
        fit.train_shift,
        fit.held_out_gap,
        fit.held_out_gap_lower,
        fit.held_out_saved / bb,
        fit.held_out_lower / bb,
        if fit.active { format!("install shift -{:.3}", fit.shift) } else { "not installed".into() }
    );
    let deep = crate::raisewar::fit_deep_call(&commits, bb);
    println!(
        "-- deep-pot all-in call fit (pot >= {} bb): n {}  older-half over-estimate {:+.3}  held-out {:+.3} (95% lower {:+.3})  saved {:+.1} bb/call (95% lower {:+.1})  {}",
        sv10_core::policy::DEEP_CALL_MIN_POT_BB,
        deep.n,
        deep.train_shift,
        deep.held_out_gap,
        deep.held_out_gap_lower,
        deep.held_out_saved / bb,
        deep.held_out_saved_lower / bb,
        if deep.active { format!("install shift -{:.3}", deep.shift) } else { "not installed".into() }
    );
    let over = crate::raisewar::fit_overbet_call(&commits);
    println!(
        "-- overbet all-in call fit (bet >= {}x pot): n {}  older-half over-estimate {:+.3}  held-out {:+.3} (95% lower {:+.3})  saved {:+.1} bb/call (95% lower {:+.1})  {}",
        sv10_core::policy::OVERBET_CALL_MIN_RATIO,
        over.n,
        over.train_shift,
        over.held_out_gap,
        over.held_out_gap_lower,
        over.held_out_saved / bb,
        over.held_out_saved_lower / bb,
        if over.active { format!("install shift -{:.3}", over.shift) } else { "not installed".into() }
    );
    // The size-scaled overbet fit (0233): the over-estimate grows with the shove's size.
    println!("-- calls of an overbet all-in by size (gap per size band; x = 1 + ln(r / 1.5))");
    for (lo, hi) in [(1.5f64, 2.5f64), (2.5, 4.0), (4.0, 10.0), (10.0, 40.0), (40.0, f64::INFINITY)] {
        let label = if hi.is_finite() { format!("villain bet {lo}-{hi}x pot") } else { format!("villain bet {lo}x+ pot") };
        row(label, commits.iter().filter(|c| c.call && c.bet_to_pot >= lo && c.bet_to_pot < hi).collect());
    }
    let slope = crate::raisewar::fit_overbet_slope(&commits);
    println!(
        "-- size-scaled overbet call fit: n {}  older-half slope {:.3}  held-out slope {:.3} (95% lower {:+.3})  held-out gap {:+.3} -> {:+.3}  saved {:+.1} bb/call (95% lower {:+.1})  {}",
        slope.n,
        slope.train_slope,
        slope.held_out_slope,
        slope.held_out_slope_lower,
        slope.held_out_gap_before,
        slope.held_out_gap_after,
        slope.held_out_saved / bb,
        slope.held_out_saved_lower / bb,
        if slope.active {
            format!(
                "install slope {:.3} (shift at 4x {:.3}, 40x {:.3}, 180x {:.3})",
                slope.slope,
                (slope.slope * sv10_core::policy::overbet_size_x(4.0)).min(sv10_core::policy::OVERBET_SLOPE_CAP),
                (slope.slope * sv10_core::policy::overbet_size_x(40.0)).min(sv10_core::policy::OVERBET_SLOPE_CAP),
                (slope.slope * sv10_core::policy::overbet_size_x(180.0)).min(sv10_core::policy::OVERBET_SLOPE_CAP)
            )
        } else {
            "not installed".into()
        }
    );
    // What the calls earned (0159): realized EV against the shown hand, by the margin the estimate
    // cleared the price by. A losing thin-margin band is chips a larger margin would keep.
    for (s, name) in ["flop", "turn", "river"].iter().enumerate() {
        println!("-- {name} calls of an all-in: realized EV by estimated margin over the price (bb per call)");
        for (lo, hi) in [(-1.0f64, 0.0f64), (0.0, 0.05), (0.05, 0.10), (0.10, 0.20), (0.20, 1.0)] {
            let set: Vec<f64> = commits
                .iter()
                .filter(|c| c.call && c.street == s)
                .filter_map(|c| Some((c.estimate - c.price()?, c.call_ev()? / bb)))
                .filter(|(m, _)| *m >= lo && *m < hi)
                .map(|(_, ev)| ev)
                .collect();
            if set.len() < 10 {
                continue;
            }
            let n = set.len() as f64;
            let mean = set.iter().sum::<f64>() / n;
            let se = (set.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0) / n).sqrt();
            println!(
                "margin {lo:+.2}..{hi:+.2}               n {:5}  EV {mean:+8.1} ± {:6.1} bb  total {:+9.0} bb",
                set.len(),
                1.96 * se,
                mean * n
            );
        }
    }
    Ok(())
}

/// One river bet or raise by an opponent who showed down: its size and the shown hand's strength.
struct SizedBet {
    name: String,
    /// Chips added beyond a call, over the pot after calling.
    size: f64,
    /// Exact river equity of the shown hand against every other holding.
    strength: f64,
}

/// Least-squares slope of strength on ln(size), with its standard error.
fn size_slope(bets: &[&SizedBet]) -> Option<(f64, f64)> {
    let points: Vec<(f64, f64)> = bets.iter().map(|b| (b.size, b.strength)).collect();
    sv10_core::sizetell::log_size_slope(&points)
}

/// Opponent timing tells (0234): how much timed play is stored, typical think times by context,
/// and the shown strength behind slow, typical and fast postflop aggression (each action measured
/// against the actor's own typical time). `calibrate` fits the range model's `think_exp` from the
/// same showdowns and keeps it only under its held-out gate; this is the readable view.
pub fn timing_tells(store: &Store) -> Result<()> {
    use sv10_core::cards::Card;
    use sv10_core::engine::Street;
    use sv10_core::model::{ModelStore, aggressive, think_context};
    let fleet: std::collections::HashSet<String> = store.bot_names()?.into_iter().collect();
    let models: ModelStore = store.get_kv(crate::MODELS_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let (mut timed, mut untimed, mut unreadable) = (0usize, 0usize, 0usize);
    let mut by_context: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
    // (ln ratio to the actor's typical time, shown strength) for opponents' postflop aggression.
    let mut points: Vec<(f64, f64)> = Vec::new();
    for (_, _, summary) in store.hands_after(0)? {
        let Ok(h) = serde_json::from_str::<HandSummary>(&summary) else {
            unreadable += 1;
            continue;
        };
        for rec in &h.history {
            let name = h.players.iter().find(|p| p.0 == rec.seat).map(|p| p.1.as_str()).unwrap_or("");
            if fleet.contains(name) {
                continue;
            }
            let Some(ms) = rec.think_ms else {
                untimed += 1;
                continue;
            };
            timed += 1;
            by_context[think_context(rec)].push(ms);
            if rec.street == Street::Preflop || !aggressive(rec) {
                continue;
            }
            let (Ok(board), Some(cards)) =
                (<[Card; 5]>::try_from(h.board.as_slice()), h.shown.iter().find(|s| s.0 == rec.seat).map(|s| s.1))
            else {
                continue;
            };
            let ratio = sv10_core::oprange::think_ratio(rec, &models.profile(name));
            if ratio == 1.0 {
                continue;
            }
            let dead = board.iter().chain(cards.iter()).fold(0u64, |m, c| m | c.bit());
            let mut field = sv10_core::range::Range::full();
            field.remove_dead(dead);
            points.push((f64::from(ratio).ln(), sv10_core::equity::river_equity_exact(cards, &board, &field)));
        }
    }
    println!("{timed} timed opponent actions, {untimed} untimed (older hands, resyncs); {unreadable} unreadable summaries skipped");
    for (i, label) in ["after another action (server-paced)", "first on a new street"].iter().enumerate() {
        let mut v = by_context[i].clone();
        v.sort_unstable();
        if v.is_empty() {
            println!("  {label:38} no timed actions yet");
            continue;
        }
        let q = |f: f64| v[((v.len() - 1) as f64 * f) as usize];
        println!("  {label:38} n {:6}  median {:5} ms  p10 {:5}  p90 {:6}", v.len(), q(0.5), q(0.1), q(0.9));
    }
    println!("-- opponents' postflop bets and raises shown down, by think time against their own typical time");
    for (lo, hi, label) in
        [(f64::NEG_INFINITY, -0.4, "fast (< 0.67x typical)"), (-0.4, 0.4, "typical"), (0.4, f64::INFINITY, "slow (> 1.5x typical)")]
    {
        let set: Vec<f64> = points.iter().filter(|p| p.0 >= lo && p.0 < hi).map(|p| p.1).collect();
        if set.is_empty() {
            println!("  {label:24} n      0");
            continue;
        }
        let n = set.len() as f64;
        let mean = set.iter().sum::<f64>() / n;
        let se = (set.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0) / n).sqrt();
        println!("  {label:24} n {:6}  shown strength {:.3} ± {:.3}", set.len(), mean, 1.96 * se);
    }
    let rp = crate::fitted_range_params(store.get_kv(crate::RANGE_PARAMS_KEY)?.as_deref());
    match rp {
        Some(rp) => println!(
            "live range model: think_exp {:+.3} ({})",
            rp.think_exp,
            if rp.think_exp == 0.0 { "timing not used" } else { "slower aggression is read as stronger when positive" }
        ),
        None => println!("live range model: defaults (think_exp 0, timing not used)"),
    }
    Ok(())
}

pub fn sizing_tells(store: &Store) -> Result<()> {
    use sv10_core::cards::Card;
    use sv10_core::engine::{ActionKind, Street};
    let fleet: std::collections::HashSet<String> = store.bot_names()?.into_iter().collect();
    let mut bets = Vec::new();
    let mut unreadable = 0usize;
    for (_, _, summary) in store.hands_after(0)? {
        let Ok(h) = serde_json::from_str::<HandSummary>(&summary) else {
            unreadable += 1;
            continue;
        };
        let Ok(board) = <[Card; 5]>::try_from(h.board.as_slice()) else { continue };
        for rec in h.history.iter().filter(|r| r.street == Street::River && matches!(r.kind, ActionKind::Raise | ActionKind::AllIn)) {
            let added = rec.to - rec.bet_before - rec.to_call_before;
            if added <= 0 {
                continue;
            }
            let Some(name) = h.players.iter().find(|p| p.0 == rec.seat).map(|p| p.1.clone()) else { continue };
            let Some(cards) = h.shown.iter().find(|s| s.0 == rec.seat).map(|s| s.1) else { continue };
            if fleet.contains(&name) {
                continue;
            }
            let dead = board.iter().chain(cards.iter()).fold(0u64, |m, c| m | c.bit());
            let mut field = sv10_core::range::Range::full();
            field.remove_dead(dead);
            let strength = sv10_core::equity::river_equity_exact(cards, &board, &field);
            bets.push(SizedBet { name, size: added as f64 / (rec.pot_before + rec.to_call_before).max(1) as f64, strength });
        }
    }
    println!("{} river bets/raises by opponents with the hand shown ({} unreadable summaries skipped)", bets.len(), unreadable);
    let buckets = [(0.0, 0.45, "< 45% pot"), (0.45, 0.85, "45-85%"), (0.85, 1.3, "85-130%"), (1.3, f64::INFINITY, "> 130% (overbet)")];
    println!("{:18} {:>6} {:>10} {:>12}", "size", "n", "strength", "weak (<0.5)");
    for (lo, hi, label) in buckets {
        let b: Vec<&SizedBet> = bets.iter().filter(|b| b.size >= lo && b.size < hi).collect();
        if b.is_empty() {
            continue;
        }
        let mean = b.iter().map(|b| b.strength).sum::<f64>() / b.len() as f64;
        let weak = b.iter().filter(|b| b.strength < 0.5).count() as f64 / b.len() as f64;
        println!("{label:18} {:6} {mean:10.3} {weak:12.3}", b.len());
    }
    let all: Vec<&SizedBet> = bets.iter().collect();
    let Some((pool, pool_se)) = size_slope(&all) else { return Ok(()) };
    println!("pool slope of strength on ln(size): {pool:+.4} ± {:.4}", 1.96 * pool_se);
    let mut by: HashMap<&str, Vec<&SizedBet>> = HashMap::new();
    for b in &bets {
        by.entry(b.name.as_str()).or_default().push(b);
    }
    const MIN: usize = 20;
    let mut rows: Vec<(&str, usize, f64, f64, f64)> = by
        .iter()
        .filter(|(_, v)| v.len() >= MIN)
        .filter_map(|(n, v)| size_slope(v).filter(|(_, se)| *se > 0.0).map(|(b, se)| (*n, v.len(), b, se, (b - pool) / se)))
        .collect();
    let (q, df, z) = sv10_core::sizetell::heterogeneity(&rows.iter().map(|r| r.4).collect::<Vec<_>>());
    println!(
        "{} opponents with {MIN}+ shown river bets; heterogeneity Q {q:.1} on {df} df (z {z:+.2}; > 1.64 means opponents differ beyond sampling noise); {} differ at |z| > 2",
        rows.len(),
        rows.iter().filter(|r| r.4.abs() > 2.0).count()
    );
    rows.sort_by(|a, b| b.4.abs().total_cmp(&a.4.abs()));
    println!("{:20} {:>5} {:>9} {:>8} {:>6}", "opponent", "n", "slope", "±95%", "z");
    for (n, k, b, se, z) in rows.iter().take(15) {
        println!("{n:20} {k:5} {b:+9.4} {:8.4} {z:+6.2}", 1.96 * se);
    }
    Ok(())
}
