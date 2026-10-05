//! The catalogue is the search's bounds and the dashboard's track at the same time, so these fix
//! both halves: that it still stops the search exactly where the hand-written clamps did (#322 moved
//! them here and must not have moved a number), and that everything the search tunes is in it.

use super::*;
use crate::learner::pool::challengers;

#[test]
fn tiered_all_in_pricing_has_the_same_boolean_bar_as_the_search() {
    let knob = find("tiered_all_in_fold_pricing").expect("learner knob is visible in the dashboard catalogue");
    assert_eq!((knob.min, knob.max, knob.decimals), (0.0, 1.0, 0));
    for enabled in [false, true] {
        let p = Params { tiered_all_in_fold_pricing: enabled, ..Params::default() };
        assert_eq!(knob.get(&p), f64::from(enabled as u8));
    }
}

/// The bounds every clamp in `learner::pool` carried before they moved into the catalogue. Written
/// out rather than read from `KNOBS`, so this is a second copy on purpose: it is what makes the move
/// checkable at all. A deliberate widening (LESSONS 29 — a bound the champion sits on is an untested
/// direction) edits this line and says so in the change.
const POOL_BOUNDS_BEFORE: &[(&str, f64, f64)] = &[
    ("fold_scale", 0.4, 1.2),
    ("initiative", -0.06, 0.12),
    ("open_bb", 2.0, 4.5),
    ("three_bet_oop", f64::NEG_INFINITY, 5.0),
    ("three_bet_ip", f64::NEG_INFINITY, 4.5),
    ("raise_fold_bonus", -0.1, f64::INFINITY),
    ("realize_weight", 0.0, 1.6),
    ("call_margin", -0.10, 0.08),
    ("jam_pot_ratio", 1.0, 4.0),
    ("raise_risk", 0.0, 2.0),
    ("four_bet", f64::NEG_INFINITY, 3.0),
    ("limper_bb", f64::NEG_INFINITY, 2.5),
    ("short_open_bb", 1.8, 3.5),
    ("preflop_jam_bb", 12.0, 50.0),
    ("preflop_fold_scale", 0.5, 1.8),
    ("passive_fold_bonus", -0.25, 0.25),
    ("three_bet_call_margin", -0.14, 0.1),
    ("preflop_raise_risk", 0.0, 2.0),
    ("profile_response_weight", 0.0, 1.5),
    ("check_lookahead", 0.0, 1.0),
    ("bet_size_scale", 0.6, 1.8),
    ("temperature", 0.002, 0.05),
];

#[test]
fn the_catalogue_kept_every_bound_the_search_already_had() {
    for (key, min, max) in POOL_BOUNDS_BEFORE {
        let k = find(key).unwrap_or_else(|| panic!("{key} is searched but not in the catalogue"));
        // An infinity is a direction the pool never steps in, so the old code named no bound there
        // and the catalogue's value is new: it is the dashboard's end of the track, not a clamp.
        if min.is_finite() {
            assert_eq!(k.min, *min, "{key} floor moved");
        }
        if max.is_finite() {
            assert_eq!(k.max, *max, "{key} ceiling moved");
        }
        assert!(k.min < k.max, "{key} has an empty range");
    }
}

#[test]
fn every_knob_the_search_moves_is_one_the_profile_can_draw() {
    // The reason the catalogue exists: the panel's list was typed by hand, so a knob the search
    // gained never reached it. `bet_size_set` swaps a whole size list, which is not a number and has
    // no bar; it is the one proposal that is deliberately not a knob.
    let mut searched: Vec<String> = Vec::new();
    for cycle in 0..2 {
        for (knob, _, _, _) in challengers(&Params::default(), cycle) {
            if knob != "bet_size_set" && knob != "bet_size_mid" && !searched.contains(&knob) {
                searched.push(knob);
            }
        }
    }
    assert_eq!(searched.len(), KNOBS.len(), "searched {searched:?}");
    for knob in &searched {
        assert!(find(knob).is_some(), "{knob} is searched but the profile has no row for it");
    }
}

#[test]
fn the_search_never_proposes_a_value_off_the_bar() {
    // Every knob at its own ceiling and floor, so each direction is tried from a champion that has
    // nowhere left to go: a step that escaped its bound would land off the track the panel draws.
    for at_max in [false, true] {
        let mut p = Params::default();
        for k in KNOBS {
            let v = if at_max { k.max } else { k.min };
            match k.key {
                "fold_scale" => p.fold_scale = v,
                "initiative" => p.initiative = v,
                "open_bb" => p.open_bb = v,
                "three_bet_ip" => p.three_bet_ip = v,
                "three_bet_oop" => p.three_bet_oop = v,
                "raise_fold_bonus" => p.raise_fold_bonus = v,
                "realize_weight" => p.realize_weight = v,
                "call_margin" => p.call_margin = v,
                "jam_pot_ratio" => p.jam_pot_ratio = v,
                "raise_risk" => p.raise_risk = v,
                "four_bet" => p.four_bet = v,
                "limper_bb" => p.limper_bb = v,
                "short_open_bb" => p.short_open_bb = v,
                "preflop_jam_bb" => p.preflop_jam_bb = v,
                "preflop_fold_scale" => p.preflop_fold_scale = v,
                "passive_fold_bonus" => p.passive_fold_bonus = v,
                "three_bet_call_margin" => p.three_bet_call_margin = v,
                "preflop_raise_risk" => p.preflop_raise_risk = v,
                "profile_response_weight" => p.profile_response_weight = v,
                "check_lookahead" => p.check_lookahead = v,
                "temperature" => p.temperature = v,
                "raise_gate" => p.raise_gate = v,
                "hero_image" => p.hero_image = v,
                "preflop_discount" => p.preflop_discount = v,
                "tiered_all_in_fold_pricing" => p.tiered_all_in_fold_pricing = v >= 0.5,
                "caller_mix" => p.caller_mix = v >= 0.5,
                "bet_size_scale" => p.bet_sizes = [0.33, 0.55, 0.8, 1.2].iter().map(|b| b * v).collect(),
                other => panic!("{other} has no assignment in this test"),
            }
        }
        for (knob, _, new, _) in challengers(&p, 0) {
            if let Some(k) = find(&knob) {
                assert!((k.min - 1e-9..=k.max + 1e-9).contains(&new), "{knob} proposed {new}, off {}..{}", k.min, k.max);
            }
        }
    }
}

#[test]
fn a_bound_only_stops_the_step_that_walks_towards_it() {
    // A champion outside a bound has an untested direction, not a value to be dragged back from
    // (LESSONS 29): the step away stops at the bound, and the step further out is left alone.
    assert_eq!(bound("call_margin", -0.09, -0.11), -0.1, "a step down stops at the floor");
    assert_eq!(bound("call_margin", 0.075, 0.085), 0.08, "a step up stops at the ceiling");
    assert_eq!(bound("call_margin", 0.5, 0.6), 0.08, "a champion above the range is still capped going up");
    assert_eq!(bound("call_margin", 0.5, 0.4), 0.4, "and is not pulled to the floor coming down");
    assert_eq!(bound("bet_size_set", 4.0, 7.0), 7.0, "a proposal that is not a knob passes through");
}

#[test]
fn the_bet_size_scale_reads_the_same_bet_in_both_size_sets() {
    // The 0.55-pot bet is index 1 of the four-size set and index 2 of the seven-size set (0170), so
    // the scale has to name the bet rather than the slot; a set with neither reads as unscaled.
    let four = Params { bet_sizes: vec![0.33, 0.55, 0.8, 1.2], ..Params::default() };
    let seven = Params { bet_sizes: vec![0.33, 0.45, 0.55, 0.8, 1.0, 1.2, 1.5], ..Params::default() };
    assert_eq!(get("bet_size_scale", &four), Some(1.0));
    assert_eq!(get("bet_size_scale", &seven), Some(1.0));
    let scaled = Params { bet_sizes: four.bet_sizes.iter().map(|b| b * 1.2).collect(), ..Params::default() };
    assert!((get("bet_size_scale", &scaled).unwrap_or_default() - 1.2).abs() < 1e-9);
    assert_eq!(get("bet_size_scale", &Params { bet_sizes: vec![], ..Params::default() }), Some(1.0));
    assert_eq!(get("not_a_knob", &four), None);
}

#[test]
fn every_row_the_profile_prints_is_readable() {
    // The panel prints `label`, `value.toFixed(decimals)` and `description`; a knob whose whole range
    // rounds to the same two decimals (temperature spans 0.002..0.05) needs more of them, or the bar
    // moves under a number that never changes.
    for k in KNOBS {
        assert!(!k.label.is_empty() && !k.description.is_empty(), "{} has no text", k.key);
        assert!(k.description.ends_with('.'), "{} description is not a sentence", k.key);
        assert_ne!(
            format!("{:.*}", usize::from(k.decimals), k.min),
            format!("{:.*}", usize::from(k.decimals), k.max),
            "{} endpoints print the same at {} decimals",
            k.key,
            k.decimals
        );
    }
}
