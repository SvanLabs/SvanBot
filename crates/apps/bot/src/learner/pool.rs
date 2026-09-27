//! The search's one-knob proposals around the champion.

use sv10_core::policy::Params;

/// One-knob perturbations of the champion, each with a human-readable label. The bounds only limit
/// what the gate gets to test; a bound the champion sits on is an untested direction (LESSONS 29),
/// so five were widened after sv10-ev-34 was found pinned on them (0160), and realize_weight
/// again to its natural floor 0 after three promotions walked it to 0.1.
pub fn challengers(p: &Params, cycle: u64) -> Vec<(String, f64, f64, Params)> {
    let mut out = Vec::new();
    let mut add = |knob: &str, old: f64, new: f64, f: &dyn Fn(&mut Params, f64)| {
        if (new - old).abs() > 1e-9 {
            let mut c = p.clone();
            f(&mut c, new);
            out.push((knob.to_string(), old, new, c));
        }
    };
    let step = if cycle.is_multiple_of(2) { 1.0 } else { 0.5 };
    add("fold_scale", p.fold_scale, (p.fold_scale - 0.1 * step).max(0.4), &|c, v| c.fold_scale = v);
    add("fold_scale", p.fold_scale, (p.fold_scale + 0.1 * step).min(1.2), &|c, v| c.fold_scale = v);
    add("initiative", p.initiative, (p.initiative - 0.03 * step).max(-0.06), &|c, v| c.initiative = v);
    add("initiative", p.initiative, (p.initiative + 0.03 * step).min(0.12), &|c, v| c.initiative = v);
    add("open_bb", p.open_bb, (p.open_bb + 0.5 * step).min(4.5), &|c, v| c.open_bb = v);
    add("open_bb", p.open_bb, (p.open_bb - 0.25 * step).max(2.0), &|c, v| c.open_bb = v);
    add("three_bet_ip", p.three_bet_ip, (p.three_bet_ip + 0.4 * step).min(4.5), &|c, v| c.three_bet_ip = v);
    add("three_bet_oop", p.three_bet_oop, (p.three_bet_oop + 0.4 * step).min(5.0), &|c, v| c.three_bet_oop = v);
    add("raise_fold_bonus", p.raise_fold_bonus, (p.raise_fold_bonus - 0.04 * step).max(-0.1), &|c, v| c.raise_fold_bonus = v);
    add("realize_weight", p.realize_weight, (p.realize_weight + 0.15 * step).min(1.6), &|c, v| c.realize_weight = v);
    add("realize_weight", p.realize_weight, (p.realize_weight - 0.15 * step).max(0.0), &|c, v| c.realize_weight = v);
    add("call_margin", p.call_margin, (p.call_margin + 0.01 * step).min(0.08), &|c, v| c.call_margin = v);
    add("call_margin", p.call_margin, (p.call_margin - 0.01 * step).max(-0.10), &|c, v| c.call_margin = v);
    add("jam_pot_ratio", p.jam_pot_ratio, (p.jam_pot_ratio + 0.4 * step).min(4.0), &|c, v| c.jam_pot_ratio = v);
    add("jam_pot_ratio", p.jam_pot_ratio, (p.jam_pot_ratio - 0.4 * step).max(1.0), &|c, v| c.jam_pot_ratio = v);
    add("raise_risk", p.raise_risk, (p.raise_risk + 0.3 * step).min(2.0), &|c, v| c.raise_risk = v);
    add("raise_risk", p.raise_risk, (p.raise_risk - 0.3 * step).max(0.0), &|c, v| c.raise_risk = v);
    add("four_bet", p.four_bet, (p.four_bet + 0.2 * step).min(3.0), &|c, v| c.four_bet = v);
    add("limper_bb", p.limper_bb, (p.limper_bb + 0.5 * step).min(2.5), &|c, v| c.limper_bb = v);
    // Stack-depth-aware preflop sizing (0171).
    add("short_open_bb", p.short_open_bb, (p.short_open_bb - 0.25 * step).max(1.8), &|c, v| c.short_open_bb = v);
    add("short_open_bb", p.short_open_bb, (p.short_open_bb + 0.25 * step).min(3.5), &|c, v| c.short_open_bb = v);
    add("preflop_jam_bb", p.preflop_jam_bb, (p.preflop_jam_bb - 5.0 * step).max(12.0), &|c, v| c.preflop_jam_bb = v);
    add("preflop_jam_bb", p.preflop_jam_bb, (p.preflop_jam_bb + 5.0 * step).min(50.0), &|c, v| c.preflop_jam_bb = v);
    add("preflop_fold_scale", p.preflop_fold_scale, (p.preflop_fold_scale - 0.1 * step).max(0.5), &|c, v| c.preflop_fold_scale = v);
    add("preflop_fold_scale", p.preflop_fold_scale, (p.preflop_fold_scale + 0.1 * step).min(1.8), &|c, v| c.preflop_fold_scale = v);
    add("passive_fold_bonus", p.passive_fold_bonus, (p.passive_fold_bonus + 0.05 * step).min(0.25), &|c, v| c.passive_fold_bonus = v);
    add("passive_fold_bonus", p.passive_fold_bonus, (p.passive_fold_bonus - 0.05 * step).max(-0.25), &|c, v| c.passive_fold_bonus = v);
    add("three_bet_call_margin", p.three_bet_call_margin, (p.three_bet_call_margin + 0.02 * step).min(0.1), &|c, v| {
        c.three_bet_call_margin = v
    });
    add("three_bet_call_margin", p.three_bet_call_margin, (p.three_bet_call_margin - 0.02 * step).max(-0.14), &|c, v| {
        c.three_bet_call_margin = v
    });
    add("preflop_raise_risk", p.preflop_raise_risk, (p.preflop_raise_risk + 0.3 * step).min(2.0), &|c, v| c.preflop_raise_risk = v);
    add("preflop_raise_risk", p.preflop_raise_risk, (p.preflop_raise_risk - 0.3 * step).max(0.0), &|c, v| c.preflop_raise_risk = v);
    add("profile_response_weight", p.profile_response_weight, (p.profile_response_weight + 0.5 * step).min(1.5), &|c, v| {
        c.profile_response_weight = v
    });
    add("profile_response_weight", p.profile_response_weight, (p.profile_response_weight - 0.5 * step).max(0.0), &|c, v| {
        c.profile_response_weight = v
    });
    add("check_lookahead", p.check_lookahead, (p.check_lookahead + 0.5 * step).min(1.0), &|c, v| c.check_lookahead = v);
    add("check_lookahead", p.check_lookahead, (p.check_lookahead - 0.5 * step).max(0.0), &|c, v| c.check_lookahead = v);
    // Postflop bet sizes as pot fractions: the four the policy started with, and a seven-size set
    // (0170) the 4x live budget and exact heads-up equity can afford. The scale knob keeps whichever
    // set the champion plays.
    const FOUR: [f64; 4] = [0.33, 0.55, 0.8, 1.2];
    const SEVEN: [f64; 7] = [0.33, 0.45, 0.55, 0.8, 1.0, 1.2, 1.5];
    let base: &[f64] = if p.bet_sizes.len() == SEVEN.len() { &SEVEN } else { &FOUR };
    let cur = p.bet_sizes.iter().zip(base).find(|(_, b)| (**b - 0.55).abs() < 1e-9).map(|(s, _)| s / 0.55).unwrap_or(1.0);
    let scale = move |c: &mut Params, v: f64| c.bet_sizes = base.iter().map(|b| b * v).collect();
    add("bet_size_scale", cur, (cur * (1.0 + 0.15 * step)).min(1.8), &scale);
    add("bet_size_scale", cur, (cur * (1.0 - 0.15 * step)).max(0.6), &scale);
    let other: &[f64] = if base.len() == SEVEN.len() { &FOUR } else { &SEVEN };
    add("bet_size_set", base.len() as f64, other.len() as f64, &|c: &mut Params, _| {
        c.bet_sizes = other.iter().map(|b| b * cur).map(|v| (v * 1e9).round() / 1e9).collect()
    });
    // Mixing temperature (0131): the one strategy knob the pool never tried. The confirmation
    // gate admits it only on a positive 95% lower bound.
    add("temperature", p.temperature, (p.temperature * (1.0 + 0.3 * step)).min(0.05), &|c, v| c.temperature = v);
    add("temperature", p.temperature, (p.temperature * (1.0 - 0.3 * step)).max(0.002), &|c, v| c.temperature = v);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_pool_covers_temperature_both_directions() {
        let pool = challengers(&Params::default(), 2);
        let temps: Vec<f64> = pool.iter().filter(|(k, _, _, _)| k == "temperature").map(|(_, _, v, _)| *v).collect();
        assert_eq!(temps.len(), 2, "up and down perturbations");
        assert!(temps.iter().all(|v| (0.002..=0.05).contains(v)));
        assert!(temps.iter().any(|v| *v > Params::default().temperature));
        assert!(temps.iter().any(|v| *v < Params::default().temperature));
    }

    #[test]
    fn the_2026_09_22_champion_has_an_outward_step_on_every_pinned_knob() {
        // sv10-ev-34 sat on five search bounds, so the gate never saw the next step outward (0160).
        let champion = Params {
            preflop_fold_scale: 1.3,
            realize_weight: 0.4,
            call_margin: -0.04,
            passive_fold_bonus: -0.1,
            three_bet_call_margin: -0.06,
            ..Params::default()
        };
        let proposals = challengers(&champion, 0);
        let tries = |knob: &str, beyond: &dyn Fn(f64) -> bool| proposals.iter().any(|(k, _, new, _)| k == knob && beyond(*new));
        assert!(tries("preflop_fold_scale", &|v| v > 1.3 + 1e-9));
        assert!(tries("realize_weight", &|v| v < 0.4 - 1e-9));
        assert!(tries("call_margin", &|v| v < -0.04 - 1e-9));
        assert!(tries("passive_fold_bonus", &|v| v < -0.1 - 1e-9));
        assert!(tries("three_bet_call_margin", &|v| v < -0.06 - 1e-9));
        // sv10-ev-37 (2026-09-22 23:25, +5.11 bb/100) walked realize_weight down to the new 0.1
        // floor in three promotions; 0 (full equity realization) is the knob's natural floor.
        let ev37 = Params { realize_weight: 0.1, ..champion };
        assert!(challengers(&ev37, 0).iter().any(|(k, _, new, _)| k == "realize_weight" && *new < 0.1 - 1e-9));
    }

    #[test]
    fn the_learner_can_try_seven_bet_sizes_and_the_size_scale_keeps_the_set() {
        let four = Params::default();
        let seven = challengers(&four, 0)
            .into_iter()
            .find(|(k, _, _, _)| k == "bet_size_set")
            .map(|(_, _, _, c)| c)
            .expect("a four-size champion is offered the seven-size set (0170)");
        assert_eq!(seven.bet_sizes, vec![0.33, 0.45, 0.55, 0.8, 1.0, 1.2, 1.5]);
        // And a seven-size champion can go back to four.
        assert!(challengers(&seven, 0).iter().any(|(k, _, _, c)| k == "bet_size_set" && c.bet_sizes.len() == 4));
        // Scaling keeps whichever set the champion plays.
        for (k, _, _, c) in challengers(&seven, 0) {
            if k == "bet_size_scale" {
                assert_eq!(c.bet_sizes.len(), 7, "{:?}", c.bet_sizes);
            }
        }
    }
}
