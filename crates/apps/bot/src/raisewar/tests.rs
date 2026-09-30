use super::*;
#[test]
fn inactive_stored_call_fits_never_install_a_shift() {
    let deep = DeepCallFit { active: false, shift: 0.15, ..Default::default() };
    let river = RiverJamFit { active: false, shift: 0.08, ..Default::default() };
    assert_eq!(installed_deep_call_shift(Some(&serde_json::to_string(&deep).unwrap())), 0.0);
    assert_eq!(installed_river_jam_shift(Some(&serde_json::to_string(&river).unwrap())), 0.0);
    assert_eq!(installed_deep_call_shift(Some("not json")), 0.0);
    assert_eq!(installed_river_jam_shift(None), 0.0);
    let deep = DeepCallFit { active: true, ..deep };
    let river = RiverJamFit { active: true, ..river };
    assert_eq!(installed_deep_call_shift(Some(&serde_json::to_string(&deep).unwrap())), 0.15);
    assert_eq!(installed_river_jam_shift(Some(&serde_json::to_string(&river).unwrap())), 0.08);
}

#[test]
fn short_all_in_raises_are_aggressive_even_without_reopening_action() {
    let short_raise = serde_json::json!({"kind": "AllIn", "to": 130, "bet_before": 0, "to_call_before": 100, "full_raise": false});
    assert!(is_raise(&short_raise), "a short all-in increases the bet despite not being a full raise");
    assert!(!is_raise(&serde_json::json!({"kind": "AllIn", "to": 100, "bet_before": 0, "to_call_before": 100, "full_raise": false})));
    assert!(!is_raise(&serde_json::json!({"kind": "AllIn", "to": 70, "bet_before": 0, "to_call_before": 100, "full_raise": false})));
    assert!(is_raise(&serde_json::json!({"kind": "AllIn", "to": 230, "bet_before": 100, "to_call_before": 100, "full_raise": false})));
}

#[test]
fn legacy_all_in_records_keep_their_existing_fallback() {
    assert!(is_raise(&serde_json::json!({"kind": "AllIn", "full_raise": true})));
    assert!(!is_raise(&serde_json::json!({"kind": "AllIn", "full_raise": false})));
    assert!(is_raise(&serde_json::json!({"kind": "AllIn"})));
    assert!(is_raise(&serde_json::json!({"kind": "AllIn", "to": 130, "bet_before": 0, "to_call_before": 100})));
}

fn played_summary(
    seats: &[(i64, [&str; 2])],
    names: &[&str],
    button: usize,
    actions: &[(usize, sv10_core::engine::Action)],
) -> serde_json::Value {
    use sv10_core::engine::Hand;
    let runout = parse_cards(&["6s", "8h", "9s", "Td", "6h"]).unwrap().try_into().unwrap();
    let states = seats.iter().map(|(stack, hole)| Hand::seat_state(*stack, parse_cards(hole).unwrap().try_into().unwrap())).collect();
    let mut played = Hand::with_cards(states, runout, button, 10, 20);
    for &(seat, action) in actions {
        assert_eq!(played.actor(), Some(seat));
        played.apply(action).unwrap();
    }
    assert!(played.actor().is_none(), "the fixture must finish through legal all-in actions");
    serde_json::json!({"players": names.iter().enumerate().collect::<Vec<_>>(),
        "board": ["6s", "8h", "9s", "Td", "6h"],
        "shown": seats.iter().enumerate().map(|(seat, (_, hole))| (seat, hole)).collect::<Vec<_>>(),
        "history": played.history})
}

#[test]
fn an_engine_short_shove_is_reported_apart_from_clean_calls() {
    use sv10_core::engine::Action::*;
    let h = played_summary(
        &[(230, ["7h", "5d"]), (1000, ["9d", "9c"])],
        &["Hero", "V"],
        0,
        &[(0, Call), (1, Check), (1, Check), (0, RaiseTo(100)), (1, RaiseTo(200)), (0, AllIn), (1, Call)],
    );
    assert_eq!((h["history"][5]["to"].as_i64(), h["history"][5]["full_raise"].as_bool()), (Some(210), Some(false)));
    let committed = commit_from_summary(&h, "Hero", 0, 1, "all_in", 0.86, (340, 100)).unwrap();
    assert!(!committed.call, "a legal short shove meets a calling range");
}

#[test]
fn a_later_engine_short_raise_excludes_an_earlier_multiway_call() {
    use sv10_core::engine::Action::*;
    let h = played_summary(
        &[(150, ["9d", "9c"]), (150, ["As", "Ad"]), (150, ["7h", "5d"])],
        &["B", "A", "Hero"],
        0,
        &[(0, Call), (1, Call), (2, Check), (1, RaiseTo(100)), (2, Call), (0, AllIn), (1, Call), (2, Call)],
    );
    assert_eq!((h["history"][5]["to"].as_i64(), h["history"][5]["full_raise"].as_bool()), (Some(130), Some(false)));
    assert!(
        crate::multiway::multiway_commit_from_summary(&h, "Hero", 0, 0, "call", 0.7, (160, 100)).is_none(),
        "the later short shove changes the price and still needs our response"
    );
    let (_, last) = crate::multiway::multiway_commit_from_summary(&h, "Hero", 0, 1, "call", 0.7, (420, 30)).unwrap();
    assert!(last.call);
    assert_eq!(last.raises_before, 2, "count the opening bet and the short raise");
}

use serde_json::json;

fn cards(s: &[&str]) -> Vec<Card> {
    parse_cards(s).unwrap()
}

/// A river call of an all-in: estimate, exact equity, pot 1,000 and a 500 call (price 1/3).
fn river_call(estimate: f64, exact: f64) -> Commit {
    Commit { street: 2, raises_before: 1, call: true, estimate, exact, pot: 1_000, to_call: 500, bet_to_pot: 1.0 }
}

#[test]
fn the_river_jam_fit_installs_a_shift_only_when_it_saves_chips_on_newer_calls() {
    // Over-estimated by 0.07: thin calls (estimate 0.36 at price 0.333) realize 0.29 and lose,
    // comfortable ones (0.60) realize 0.53 and win. The fitted shift folds the thin ones.
    let biased: Vec<Commit> = (0..800).map(|i| if i % 2 == 0 { river_call(0.36, 0.29) } else { river_call(0.60, 0.53) }).collect();
    let fit = fit_river_jam_call(&biased);
    assert!(fit.active, "{fit:?}");
    assert!((fit.shift - 0.07).abs() < 1e-9 && (fit.train_shift - 0.07).abs() < 1e-9, "{fit:?}");
    assert!(fit.held_out_saved > 0.0 && fit.held_out_lower > 0.0);
    assert!((installed_river_jam_shift(Some(&serde_json::to_string(&fit).unwrap())) - 0.07).abs() < 1e-9);

    // Calibrated calls: the thin ones break even, so folding them saves nothing.
    let calibrated: Vec<Commit> = (0..800).map(|i| if i % 2 == 0 { river_call(0.36, 0.36) } else { river_call(0.60, 0.60) }).collect();
    let fit = fit_river_jam_call(&calibrated);
    assert!(!fit.active && fit.shift == 0.0, "{fit:?}");

    // Too few held-out calls, other streets and our own shoves are ignored.
    assert!(!fit_river_jam_call(&biased[..200]).active);
    let mut turn = biased.clone();
    turn.iter_mut().for_each(|c| c.street = 1);
    assert_eq!(fit_river_jam_call(&turn).n, 0);
    assert_eq!(installed_river_jam_shift(None), 0.0);
    assert_eq!(installed_river_jam_shift(Some("not json")), 0.0);
}

#[test]
fn a_steady_overestimate_installs_on_held_out_calibration_even_when_chips_are_noisy() {
    // 2026-09-23 live data: river all-in calls over-estimated by ~0.08 on every refit, but a few
    // huge pots made "chips saved" too noisy to clear its bound, so the fix never installed.
    // Here every call is 0.08 high; one giant pot (a won call) swamps the chips criterion.
    let mut noisy: Vec<Commit> = (0..800)
        .map(|i| {
            let est = 0.30 + (i % 7) as f64 * 0.08;
            river_call(est, est - 0.08)
        })
        .collect();
    noisy[790] = Commit { pot: 2_000_000, to_call: 1_000_000, ..river_call(0.40, 0.32) };
    let fit = fit_river_jam_call(&noisy);
    assert!(fit.held_out_gap_lower > fit.train_shift / 2.0, "{fit:?}");
    assert!(fit.active && (fit.shift - 0.08).abs() < 1e-9, "{fit:?}");
    // No bias on held-out calls: nothing installs even though the older half looked biased.
    let mut drift: Vec<Commit> = (0..400)
        .map(|i| {
            let e = 0.3 + (i % 7) as f64 * 0.08;
            river_call(e, e - 0.08)
        })
        .collect();
    drift.extend((0..400).map(|i| {
        let e = 0.3 + (i % 7) as f64 * 0.08;
        river_call(e, e)
    }));
    let fit = fit_river_jam_call(&drift);
    assert!(!fit.active && fit.shift == 0.0, "{fit:?}");
}

#[test]
fn the_deep_call_fit_installs_only_on_a_held_out_calibration_win() {
    let deep = |est: f64, exact: f64, street: usize| Commit {
        street,
        raises_before: 2,
        call: true,
        estimate: est,
        exact,
        pot: 12_000,
        to_call: 6_000,
        bet_to_pot: 1.0,
    };
    // 20 bb blind: 12,000 is a 600 bb pot. Every deep call is 0.15 high on both halves.
    let biased: Vec<Commit> = (0..120)
        .map(|i| {
            let e = 0.4 + (i % 5) as f64 * 0.1;
            deep(e, e - 0.15, i % 3)
        })
        .collect();
    let fit = fit_deep_call(&biased, 20.0);
    assert!(fit.active && (fit.shift - 0.15).abs() < 1e-9, "{fit:?}");
    assert!((installed_deep_call_shift(Some(&serde_json::to_string(&fit).unwrap())) - 0.15).abs() < 1e-9);
    // The same calls in small pots are not deep: nothing to fit.
    let small: Vec<Commit> = biased.iter().map(|c| Commit { pot: 1_000, ..c.clone() }).collect();
    assert_eq!(fit_deep_call(&small, 20.0).n, 0);
    // Biased only in the older half: the held-out half does not confirm it.
    let mut drift: Vec<Commit> = biased[..60].to_vec();
    drift.extend((0..60).map(|i| {
        let e = 0.4 + (i % 5) as f64 * 0.1;
        deep(e, e, 1)
    }));
    let fit = fit_deep_call(&drift, 20.0);
    assert!(!fit.active && fit.shift == 0.0, "{fit:?}");
    // Too few held-out calls.
    assert!(!fit_deep_call(&biased[..60], 20.0).active);
    assert_eq!(installed_deep_call_shift(None), 0.0);
}

#[test]
fn call_fits_refit_hourly_between_search_cycles() {
    assert!(calls_refit_due(0.0, 3_600.0));
    assert!(!calls_refit_due(1_000.0, 4_000.0));
    assert!(calls_refit_due(1_000.0, 4_600.0));
}

#[test]
fn the_overbet_call_fit_uses_the_bet_size_band_only() {
    let call = |est: f64, exact: f64, ratio: f64| Commit {
        street: 1,
        raises_before: 1,
        call: true,
        estimate: est,
        exact,
        pot: 2_000,
        to_call: 900,
        bet_to_pot: ratio,
    };
    let biased: Vec<Commit> = (0..100)
        .map(|i| {
            let e = 0.45 + (i % 5) as f64 * 0.1;
            call(e, e - 0.3, 6.0)
        })
        .collect();
    let fit = fit_overbet_call(&biased);
    assert!(fit.active && (fit.shift - 0.3).abs() < 1e-9, "{fit:?}");
    // The same calls against normal-sized bets are outside the band.
    let normal: Vec<Commit> = biased.iter().map(|c| Commit { bet_to_pot: 0.8, ..c.clone() }).collect();
    assert_eq!(fit_overbet_call(&normal).n, 0);
    // 0.3 high in the older half, 0.05..0.15 in the newer: the full shift fails both tests,
    // but the held-out gap is still positive at 95%, so its lower bound is installed (0208).
    let mut shrinking: Vec<Commit> = biased[..50].to_vec();
    shrinking.extend((0..50).map(|i| {
        let e = 0.45 + (i % 5) as f64 * 0.1;
        call(e, e - if i % 2 == 0 { 0.05 } else { 0.15 }, 6.0)
    }));
    let fit = fit_overbet_call(&shrinking);
    assert!(fit.held_out_gap_lower > 0.0 && fit.held_out_gap_lower < fit.train_shift / 2.0, "{fit:?}");
    assert!(fit.active && (fit.shift - fit.held_out_gap_lower).abs() < 1e-12, "{fit:?}");
}

#[test]
fn the_overbet_slope_fit_tracks_a_gap_that_grows_with_the_bet() {
    use sv10_core::policy::overbet_size_x;
    let call = |est: f64, exact: f64, ratio: f64| Commit {
        street: 1,
        raises_before: 1,
        call: true,
        estimate: est,
        exact,
        pot: 2_000,
        to_call: 900,
        bet_to_pot: ratio,
    };
    // The over-estimate is 0.05 × x(r): 0.05 at 1.5x, ~0.27 at 120x.
    let sizes = [1.5, 2.5, 4.0, 10.0, 30.0, 120.0];
    let grows: Vec<Commit> = (0..120)
        .map(|i| {
            let r = sizes[i % sizes.len()];
            let e = 0.5 + (i % 4) as f64 * 0.08;
            call(e, e - 0.05 * overbet_size_x(r), r)
        })
        .collect();
    let fit = fit_overbet_slope(&grows);
    assert_eq!(fit.n, 120);
    assert!((fit.train_slope - 0.05).abs() < 1e-9 && (fit.held_out_slope - 0.05).abs() < 1e-9, "{fit:?}");
    assert!(fit.active && (fit.slope - 0.05).abs() < 1e-9, "{fit:?}");
    assert!(fit.held_out_gap_after.abs() < 1e-9 && fit.held_out_gap_before > 0.1, "{fit:?}");
    assert!((installed_overbet_slope(Some(&serde_json::to_string(&fit).unwrap())) - 0.05).abs() < 1e-12);
    // Calibrated calls: nothing installed.
    let calibrated: Vec<Commit> = grows.iter().map(|c| Commit { exact: c.estimate, ..c.clone() }).collect();
    let fit = fit_overbet_slope(&calibrated);
    assert!(!fit.active && fit.slope == 0.0, "{fit:?}");
    assert_eq!(installed_overbet_slope(Some(&serde_json::to_string(&fit).unwrap())), 0.0);
    // Too few held-out calls: no fit at all; normal-sized bets are outside the band.
    assert!(!fit_overbet_slope(&grows[..60]).active);
    let normal: Vec<Commit> = grows.iter().map(|c| Commit { bet_to_pot: 0.8, ..c.clone() }).collect();
    assert_eq!(fit_overbet_slope(&normal).n, 0);
    assert_eq!(installed_overbet_slope(None), 0.0);
    assert_eq!(installed_overbet_slope(Some("not json")), 0.0);
}

#[test]
fn exact_equity_matches_known_matchups() {
    let h = cards(&["7h", "5d"]);
    let v = cards(&["9d", "9c"]);
    // Svanar 50be1280: flopped straight vs a set on 6s8h9s.
    let flop = exact_equity([h[0], h[1]], [v[0], v[1]], &cards(&["6s", "8h", "9s"]));
    assert!((flop - 0.648).abs() < 0.01, "straight vs set on the flop: {flop}");
    let river = exact_equity([h[0], h[1]], [v[0], v[1]], &cards(&["6s", "8h", "9s", "Td", "6h"]));
    assert_eq!(river, 0.0, "the paired river gave the set a full house");
    let aa = cards(&["Ah", "As"]);
    let kk = cards(&["Kh", "Ks"]);
    let tie = exact_equity([aa[0], aa[1]], [aa[0], aa[1]], &cards(&["2c", "3d", "4h", "5s", "9c"]));
    assert_eq!(tie, 0.5);
    let over = exact_equity([aa[0], aa[1]], [kk[0], kk[1]], &cards(&["2c", "7d", "Jc"]));
    assert!(over > 0.9 && over < 0.93, "AA vs KK on a dry flop: {over}");
}

fn hand() -> serde_json::Value {
    json!({"players": [[0, "V"], [3, "Hero"]], "board": ["6s", "8h", "9s", "Td", "6h"],
           "shown": [[0, ["9d", "9c"]], [3, ["7h", "5d"]]], "history": [
        {"seat": 3, "street": "Flop", "kind": "Raise", "full_raise": true},
        {"seat": 0, "street": "Flop", "kind": "Raise", "full_raise": true},
        {"seat": 3, "street": "Flop", "kind": "Raise", "full_raise": true},
        {"seat": 0, "street": "Flop", "kind": "Raise", "full_raise": true},
        {"seat": 3, "street": "Flop", "kind": "AllIn", "full_raise": true},
        {"seat": 0, "street": "Flop", "kind": "Call", "full_raise": false}]})
}

#[test]
fn our_shove_at_the_end_of_a_raise_war_is_the_committed_decision() {
    let c = commit_from_summary(&hand(), "Hero", 0, 2, "all_in", 0.86, (0, 0)).expect("the shove commits the hand");
    assert_eq!((c.street, c.raises_before, c.call), (0, 4, false));
    assert!((c.exact - 0.648).abs() < 0.01);
    assert!(commit_from_summary(&hand(), "Hero", 0, 1, "raise", 0.86, (0, 0)).is_none(), "the opponent raised again after it");
    assert!(commit_from_summary(&hand(), "Hero", 0, 2, "call", 0.86, (0, 0)).is_none(), "action must match the history");
    assert!(commit_from_summary(&hand(), "Other", 0, 0, "raise", 0.5, (0, 0)).is_none());
}

#[test]
fn calling_an_all_in_is_a_clean_call_sample() {
    let mut h = hand();
    let hist = h["history"].as_array_mut().unwrap();
    hist.truncate(4);
    hist.push(json!({"seat": 3, "street": "Flop", "kind": "Call", "full_raise": false}));
    let c = commit_from_summary(&h, "Hero", 0, 2, "call", 0.7, (1_000, 500)).unwrap();
    assert!(c.call);
    assert!((c.price().unwrap() - 1.0 / 3.0).abs() < 1e-12);
    assert!((c.call_ev().unwrap() - (c.exact * 1_500.0 - 500.0)).abs() < 1e-9);
    assert_eq!(c.raises_before, 4);
    let b = bin(&[&c]);
    assert!((b.gap - (0.7 - c.exact)).abs() < 1e-12);
}
