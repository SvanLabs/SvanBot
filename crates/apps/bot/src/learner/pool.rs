//! The search's one-knob proposals around the champion.

use crate::knobs;
use sv10_core::policy::Params;

/// One-knob perturbations of the champion, each with a human-readable label. Each line names only
/// the step it takes; where the step stops is [`crate::knobs`], which the dashboard draws the same
/// bar from (#322), so a bound cannot be widened for the search and stay narrow on the panel.
///
/// The bounds only limit what the gate gets to test; a bound the champion sits on is an untested
/// direction (LESSONS 29), so five were widened after sv10-ev-34 was found pinned on them (0160),
/// and realize_weight again to its natural floor 0 after three promotions walked it to 0.1.
pub fn challengers(p: &Params, cycle: u64) -> Vec<(String, f64, f64, Params)> {
    let mut out = Vec::new();
    // `old` is the knob's own reading of the champion, so a line cannot step from a value the
    // catalogue would not have reported; `bound` stops the step at the bound it is walking towards.
    let mut add = |knob: &str, old: f64, new: f64, f: &dyn Fn(&mut Params, f64)| {
        let new = knobs::bound(knob, old, new);
        if (new - old).abs() > 1e-9 {
            let mut c = p.clone();
            f(&mut c, new);
            out.push((knob.to_string(), old, new, c));
        }
    };
    let mut step_knob = |knob: &str, delta: f64, f: &dyn Fn(&mut Params, f64)| {
        if let Some(old) = knobs::get(knob, p) {
            add(knob, old, old + delta, f);
        }
    };
    let step = if cycle.is_multiple_of(2) { 1.0 } else { 0.5 };
    step_knob("fold_scale", -0.1 * step, &|c, v| c.fold_scale = v);
    step_knob("fold_scale", 0.1 * step, &|c, v| c.fold_scale = v);
    step_knob("initiative", -0.03 * step, &|c, v| c.initiative = v);
    step_knob("initiative", 0.03 * step, &|c, v| c.initiative = v);
    step_knob("open_bb", 0.5 * step, &|c, v| c.open_bb = v);
    step_knob("open_bb", -0.5 * step, &|c, v| c.open_bb = v);
    // Both directions on every knob (#760): a one-sided line left the other side to the global
    // scale knobs, so a champion walked one way had no way back that the gate could test.
    step_knob("three_bet_ip", 0.4 * step, &|c, v| c.three_bet_ip = v);
    step_knob("three_bet_ip", -0.4 * step, &|c, v| c.three_bet_ip = v);
    step_knob("three_bet_oop", 0.4 * step, &|c, v| c.three_bet_oop = v);
    step_knob("three_bet_oop", -0.4 * step, &|c, v| c.three_bet_oop = v);
    step_knob("raise_fold_bonus", -0.04 * step, &|c, v| c.raise_fold_bonus = v);
    step_knob("raise_fold_bonus", 0.04 * step, &|c, v| c.raise_fold_bonus = v);
    step_knob("realize_weight", 0.15 * step, &|c, v| c.realize_weight = v);
    step_knob("realize_weight", -0.15 * step, &|c, v| c.realize_weight = v);
    step_knob("call_margin", 0.01 * step, &|c, v| c.call_margin = v);
    step_knob("call_margin", -0.01 * step, &|c, v| c.call_margin = v);
    step_knob("jam_pot_ratio", 0.4 * step, &|c, v| c.jam_pot_ratio = v);
    step_knob("jam_pot_ratio", -0.4 * step, &|c, v| c.jam_pot_ratio = v);
    step_knob("raise_risk", 0.3 * step, &|c, v| c.raise_risk = v);
    step_knob("raise_risk", -0.3 * step, &|c, v| c.raise_risk = v);
    step_knob("four_bet", 0.2 * step, &|c, v| c.four_bet = v);
    step_knob("four_bet", -0.2 * step, &|c, v| c.four_bet = v);
    step_knob("limper_bb", 0.5 * step, &|c, v| c.limper_bb = v);
    step_knob("limper_bb", -0.5 * step, &|c, v| c.limper_bb = v);
    // Two fields the search never moved (#760): the raise guard's floor scale and how much of the
    // image our own play gives is priced in. Neither changes anything until the gate promotes it.
    step_knob("raise_gate", 0.15 * step, &|c, v| c.raise_gate = v);
    step_knob("raise_gate", -0.15 * step, &|c, v| c.raise_gate = v);
    step_knob("hero_image", 0.25 * step, &|c, v| c.hero_image = v);
    step_knob("hero_image", -0.25 * step, &|c, v| c.hero_image = v);
    // The flat 0.5 on the players still to act behind a preflop raise, now a knob (#746).
    step_knob("preflop_discount", 0.1 * step, &|c, v| c.preflop_discount = v);
    step_knob("preflop_discount", -0.1 * step, &|c, v| c.preflop_discount = v);
    // Stack-depth-aware preflop sizing (0171).
    step_knob("short_open_bb", -0.25 * step, &|c, v| c.short_open_bb = v);
    step_knob("short_open_bb", 0.25 * step, &|c, v| c.short_open_bb = v);
    step_knob("preflop_jam_bb", -5.0 * step, &|c, v| c.preflop_jam_bb = v);
    step_knob("preflop_jam_bb", 5.0 * step, &|c, v| c.preflop_jam_bb = v);
    step_knob("preflop_fold_scale", -0.1 * step, &|c, v| c.preflop_fold_scale = v);
    step_knob("preflop_fold_scale", 0.1 * step, &|c, v| c.preflop_fold_scale = v);
    step_knob("passive_fold_bonus", 0.05 * step, &|c, v| c.passive_fold_bonus = v);
    step_knob("passive_fold_bonus", -0.05 * step, &|c, v| c.passive_fold_bonus = v);
    step_knob("three_bet_call_margin", 0.02 * step, &|c, v| c.three_bet_call_margin = v);
    step_knob("three_bet_call_margin", -0.02 * step, &|c, v| c.three_bet_call_margin = v);
    step_knob("preflop_raise_risk", 0.3 * step, &|c, v| c.preflop_raise_risk = v);
    step_knob("preflop_raise_risk", -0.3 * step, &|c, v| c.preflop_raise_risk = v);
    step_knob("profile_response_weight", 0.5 * step, &|c, v| c.profile_response_weight = v);
    step_knob("profile_response_weight", -0.5 * step, &|c, v| c.profile_response_weight = v);
    step_knob("check_lookahead", 0.5 * step, &|c, v| c.check_lookahead = v);
    step_knob("check_lookahead", -0.5 * step, &|c, v| c.check_lookahead = v);
    // The default champion keeps legacy pricing. Promotion in either direction requires fresh-deal
    // confirmation, so a promoted on champion can later test the legacy path again.
    step_knob("tiered_all_in_fold_pricing", if p.tiered_all_in_fold_pricing { -1.0 } else { 1.0 }, &|c, v| {
        c.tiered_all_in_fold_pricing = v >= 0.5
    });
    // Postflop bet sizes as pot fractions: the four the policy started with, and a seven-size set
    // (0170) the 4x live budget and exact heads-up equity can afford. The scale knob keeps whichever
    // set the champion plays.
    const FOUR: [f64; 4] = [0.33, 0.55, 0.8, 1.2];
    const SEVEN: [f64; 7] = [0.33, 0.45, 0.55, 0.8, 1.0, 1.2, 1.5];
    let base: &[f64] = if p.bet_sizes.len() == SEVEN.len() { &SEVEN } else { &FOUR };
    let cur = knobs::get("bet_size_scale", p).unwrap_or(1.0);
    let scale = move |c: &mut Params, v: f64| c.bet_sizes = base.iter().map(|b| b * v).collect();
    add("bet_size_scale", cur, cur * (1.0 + 0.15 * step), &scale);
    add("bet_size_scale", cur, cur * (1.0 - 0.15 * step), &scale);
    let other: &[f64] = if base.len() == SEVEN.len() { &FOUR } else { &SEVEN };
    // Not a knob: this proposes a whole size list rather than a number, so it has no bar to draw and
    // no bounds to clamp. `bound` passes an unknown key through unchanged.
    add("bet_size_set", base.len() as f64, other.len() as f64, &|c: &mut Params, _| {
        c.bet_sizes = other.iter().map(|b| b * cur).map(|v| (v * 1e9).round() / 1e9).collect()
    });
    // One interior size on its own (#760): the 0.55-pot bet (index 1 of four sizes, 2 of seven) moves by 0.1 pot
    // while the others stay, so the set can take a shape the 4/7 toggle and the global scale cannot. Not a knob:
    // it proposes a whole size list. A step that would touch a neighbour is not proposed.
    let mid = if p.bet_sizes.len() == SEVEN.len() { 2 } else { 1 };
    if p.bet_sizes.len() > mid + 1 {
        for dir in [1.0, -1.0] {
            let v = ((p.bet_sizes[mid] + 0.1 * step * dir) * 1e9).round() / 1e9;
            if v > p.bet_sizes[mid - 1] + 0.03 && v < p.bet_sizes[mid + 1] - 0.03 {
                let mut sizes = p.bet_sizes.clone();
                sizes[mid] = v;
                add("bet_size_mid", p.bet_sizes[mid], v, &move |c: &mut Params, _| c.bet_sizes = sizes.clone());
            }
        }
    }
    // Mixing temperature (0131): the one strategy knob the pool never tried. The confirmation
    // gate admits it only on a positive 95% lower bound.
    add("temperature", p.temperature, p.temperature * (1.0 + 0.3 * step), &|c, v| c.temperature = v);
    add("temperature", p.temperature, p.temperature * (1.0 - 0.3 * step), &|c, v| c.temperature = v);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiered_all_in_pricing_is_a_single_knob_step_in_both_directions() {
        for enabled in [false, true] {
            let champion = Params { tiered_all_in_fold_pricing: enabled, ..Params::default() };
            let proposals: Vec<_> =
                challengers(&champion, 0).into_iter().filter(|(key, _, _, _)| key == "tiered_all_in_fold_pricing").collect();
            assert_eq!(proposals.len(), 1, "each champion must get one opposite-value candidate");
            let (_, old, new, challenger) = &proposals[0];
            assert_eq!((*old, *new), (f64::from(enabled as u8), f64::from((!enabled) as u8)));
            let expected = Params { tiered_all_in_fold_pricing: !enabled, ..champion };
            assert_eq!(serde_json::to_value(challenger).unwrap(), serde_json::to_value(expected).unwrap(), "only this knob changes");
        }
    }

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

    #[test]
    fn one_interior_bet_size_moves_alone_in_either_set() {
        for sizes in [vec![0.33, 0.55, 0.8, 1.2], vec![0.33, 0.45, 0.55, 0.8, 1.0, 1.2, 1.5]] {
            let champion = Params { bet_sizes: sizes.clone(), ..Params::default() };
            let mid = if sizes.len() == 7 { 2 } else { 1 };
            let moved: Vec<_> = challengers(&champion, 0).into_iter().filter(|(k, _, _, _)| k == "bet_size_mid").collect();
            // The seven-size set has 0.45 right below 0.55: its step down would land on the neighbour and is not proposed.
            let want = if sizes.len() == 7 { 1 } else { 2 };
            assert_eq!(moved.len(), want, "{moved:?}");
            for (_, old, new, c) in &moved {
                assert_eq!(*old, sizes[mid]);
                assert!((new - old).abs() > 0.05);
                assert_eq!(c.bet_sizes.len(), sizes.len());
                for (i, (a, b)) in c.bet_sizes.iter().zip(&sizes).enumerate() {
                    assert!(i == mid || a == b, "only the interior size moves: {:?}", c.bet_sizes);
                }
                assert!(c.bet_sizes.windows(2).all(|w| w[0] < w[1]), "sizes stay sorted: {:?}", c.bet_sizes);
            }
        }
        let crowded = Params { bet_sizes: vec![0.33, 0.35, 0.37, 1.2], ..Params::default() };
        assert!(challengers(&crowded, 0).iter().all(|(k, _, _, _)| k != "bet_size_mid"), "no step that would touch a neighbour");
    }

    #[test]
    fn every_knob_with_room_is_stepped_in_both_directions() {
        // #760: a one-sided line left a champion walked one way with no tested way back.
        let p = Params::default();
        let pool = challengers(&p, 0);
        for k in knobs::KNOBS.iter().filter(|k| k.key != "tiered_all_in_fold_pricing") {
            let now = k.get(&p);
            let moves =
                |up: bool| pool.iter().any(|(key, _, new, _)| key == k.key && if up { *new > now + 1e-9 } else { *new < now - 1e-9 });
            assert!(moves(true) || now >= k.max - 1e-9, "{} has no step up from {now}", k.key);
            assert!(moves(false) || now <= k.min + 1e-9, "{} has no step down from {now}", k.key);
        }
    }
}
