use super::*;
use crate::situation::Situation;
use sv10_cards::cards::parse_cards;
use sv10_cards::eval::eval;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

fn c2(a: &str, b: &str) -> [Card; 2] {
    [Card::parse(a).unwrap(), Card::parse(b).unwrap()]
}
fn board(s: &[&str]) -> [Card; 5] {
    parse_cards(s).unwrap().try_into().unwrap()
}

#[test]
fn blinds_and_first_actor_six_max_and_heads_up() {
    let mut rng = SmallRng::seed_from_u64(1);
    let h = Hand::new(&[1000; 6], 0, 10, 20, &mut rng);
    assert_eq!(h.seats[1].bet, 10);
    assert_eq!(h.seats[2].bet, 20);
    assert_eq!(h.actor(), Some(3));
    assert_eq!(h.pot(), 30);
    let l = h.legal();
    assert_eq!(l.call_amount, 20);
    assert_eq!(l.min_raise_to, Some(40));
    assert_eq!(l.max_raise_to, Some(1000));

    let hu = Hand::new(&[1000, 1000], 1, 10, 20, &mut rng);
    assert_eq!(hu.seats[1].bet, 10, "button posts small blind heads-up");
    assert_eq!(hu.actor(), Some(1));
}

#[test]
fn everyone_folds_to_big_blind() {
    let mut rng = SmallRng::seed_from_u64(2);
    let mut h = Hand::new(&[1000; 6], 0, 10, 20, &mut rng);
    for _ in 0..5 {
        h.apply(Action::Fold).unwrap();
    }
    assert!(h.is_finished());
    assert_eq!(h.net(), vec![0, -10, 10, 0, 0, 0]);
    assert!(!h.showdown());
}

#[test]
fn preflop_big_blind_gets_option_then_flop_order() {
    let mut rng = SmallRng::seed_from_u64(3);
    let mut h = Hand::new(&[1000; 3], 0, 10, 20, &mut rng);
    // Button calls, SB completes, BB has option.
    h.apply(Action::Call).unwrap();
    h.apply(Action::Call).unwrap();
    assert_eq!(h.actor(), Some(2));
    assert!(h.legal().can_check);
    h.apply(Action::Check).unwrap();
    assert_eq!(h.street, Street::Flop);
    assert_eq!(h.board.len(), 3);
    assert_eq!(h.actor(), Some(1), "small blind acts first postflop");
}

#[test]
fn min_raise_tracks_last_full_raise_and_short_all_in_does_not_reopen() {
    let seats = vec![
        Hand::seat_state(1000, c2("Ah", "Ad")),
        Hand::seat_state(1000, c2("Kh", "Kd")),
        Hand::seat_state(1000, c2("Qh", "Qd")),
        Hand::seat_state(150, c2("2h", "3d")),
    ];
    let mut h = Hand::with_cards(seats, board(&["7c", "8s", "9d", "Js", "4c"]), 0, 10, 20);
    // seat 3 acts first (left of BB = seat 2... button 0, sb 1, bb 2, first 3)
    assert_eq!(h.actor(), Some(3));
    h.apply(Action::Call).unwrap(); // 3 limps 20
    h.apply(Action::RaiseTo(100)).unwrap(); // 0 raises to 100 (raise size 80)
    assert_eq!(h.actor(), Some(1));
    assert_eq!(h.legal().min_raise_to, Some(180));
    h.apply(Action::Call).unwrap(); // 1 calls
    h.apply(Action::Call).unwrap(); // 2 calls
    // 3 goes all-in to 150: raise of 50 < 80, not a full raise.
    assert_eq!(h.actor(), Some(3));
    h.apply(Action::AllIn).unwrap();
    assert!(!h.history.last().unwrap().full_raise);
    // Seat 0 already acted and faces only an incomplete raise: call or fold only.
    assert_eq!(h.actor(), Some(0));
    let l = h.legal();
    assert_eq!(l.call_amount, 50);
    assert_eq!(l.min_raise_to, None);
    h.apply(Action::Call).unwrap();
    h.apply(Action::Call).unwrap();
    h.apply(Action::Call).unwrap();
    assert_eq!(h.street, Street::Flop);
    assert_eq!(h.pot(), 600);
}

#[test]
fn betting_geometry_matrix_covers_short_stacks_and_actor_exclusion() {
    let mut rng = SmallRng::seed_from_u64(31);

    // An incomplete opening all-in does not prevent an unacted player from making a full raise.
    let mut short_open = Hand::new(&[1_000, 1_000, 1_000, 30], 0, 10, 20, &mut rng);
    assert_eq!(short_open.actor(), Some(3));
    short_open.apply(Action::AllIn).unwrap();
    assert_eq!(short_open.actor(), Some(0));
    assert_eq!(short_open.current_bet(), 30);
    assert_eq!(short_open.pot(), 60);
    let legal = short_open.legal();
    assert_eq!(legal.call_amount, 30);
    assert_eq!(legal.min_raise_to, Some(50));
    assert_eq!(legal.max_raise_to, Some(1_000));
    short_open.apply(Action::RaiseTo(50)).unwrap();
    assert!(short_open.history.last().unwrap().full_raise);
    assert_eq!(short_open.actor(), Some(1));
    assert_eq!(short_open.pot(), 110);

    // A call is capped by the actor's stack and cannot be represented as a raise.
    let capped = Hand::new(&[15, 1_000, 1_000], 0, 10, 20, &mut rng);
    assert_eq!(capped.actor(), Some(0));
    let legal = capped.legal();
    assert_eq!(legal.call_amount, 15);
    assert_eq!(legal.min_raise_to, None);
    assert_eq!(legal.max_raise_to, None);
    assert_eq!(capped.pot(), 30);

    // A short big blind still leaves a full big-blind bring-in for players who can cover it.
    let short_blind = Hand::new(&[1_000, 1_000, 10], 0, 10, 20, &mut rng);
    let names = vec!["button".into(), "small".into(), "big".into()];
    let short_blind_situation = Situation::from_hand(&short_blind, 0, &names);
    assert_eq!(short_blind.current_bet(), 20);
    assert_eq!(short_blind_situation.current_bet(), 20);
    assert_eq!(short_blind_situation.call_amount, 20);

    // Folded and all-in seats are skipped; the two players with chips continue heads-up postflop.
    let mut excluded = Hand::new(&[100, 15, 100], 0, 10, 20, &mut rng);
    assert_eq!(excluded.actor(), Some(0));
    excluded.apply(Action::Call).unwrap();
    assert_eq!(excluded.actor(), Some(1));
    assert_eq!(excluded.legal().call_amount, 5);
    excluded.apply(Action::Call).unwrap();
    assert!(excluded.seats[1].all_in());
    assert_eq!(excluded.actor(), Some(2));
    excluded.apply(Action::Check).unwrap();
    assert_eq!(excluded.street, Street::Flop);
    assert_eq!(excluded.actor(), Some(2));
    assert_eq!(excluded.pot(), 55);
    assert!(excluded.legal().can_check);
    excluded.apply(Action::Check).unwrap();
    assert_eq!(excluded.actor(), Some(0), "all-in seat must not be offered an action");

    let mut folded = Hand::new(&[1_000; 3], 0, 10, 20, &mut rng);
    folded.apply(Action::Call).unwrap();
    folded.apply(Action::Fold).unwrap();
    folded.apply(Action::Check).unwrap();
    assert_eq!(folded.street, Street::Flop);
    assert_eq!(folded.actor(), Some(2), "big blind acts first against the button heads-up postflop");
    assert_eq!(folded.pot(), 50);
}

#[test]
fn side_pots_pay_the_right_players() {
    // Seat 0 short all-in with the best hand, seats 1 and 2 deep with 2nd/3rd best.
    let seats = vec![Hand::seat_state(100, c2("Ah", "Ad")), Hand::seat_state(1000, c2("Kh", "Kd")), Hand::seat_state(1000, c2("Qh", "Qd"))];
    let mut h = Hand::with_cards(seats, board(&["2c", "7s", "9d", "Js", "4c"]), 0, 10, 20);
    h.apply(Action::AllIn).unwrap(); // seat 0 (button, first to act 3-handed) shoves 100
    h.apply(Action::RaiseTo(400)).unwrap(); // sb raises
    h.apply(Action::Call).unwrap(); // bb calls 400
    // Flop: sb bets, bb folds -> sb wins side pot uncontested, main pot to seat 0.
    h.apply(Action::RaiseTo(300)).unwrap();
    h.apply(Action::Fold).unwrap();
    assert!(h.is_finished());
    assert!(h.showdown());
    let net = h.net();
    assert_eq!(net[0], 200, "AA wins main pot of 300");
    assert_eq!(net[2], -400);
    assert_eq!(net[1], 200, "KK wins the 600 side pot and gets its uncalled 300 back");
    assert_eq!(net.iter().sum::<i64>(), 0);
}

#[test]
fn settlement_matrix_handles_three_tiers_main_tie_dead_money_odd_chip_and_refund() {
    let runout = board(&["2c", "3d", "4h", "9s", "Kc"]);
    let seats = vec![
        Hand::seat_state(1_000, c2("5c", "6c")),
        Hand::seat_state(1_000, c2("5d", "6d")),
        Hand::seat_state(1_000, c2("Ah", "Ad")),
        Hand::seat_state(1_000, c2("Qh", "Qd")),
        Hand::seat_state(1_000, c2("Jc", "Td")),
    ];
    let mut h = Hand::with_cards(seats, runout, 0, 10, 20);
    let invested = [101, 101, 201, 501, 301];
    for (seat, amount) in h.seats.iter_mut().zip(invested) {
        seat.invested = amount;
        seat.stack = seat.start_stack - amount;
    }
    h.seats[4].folded = true;

    let payouts = h.settle(&runout);
    assert_eq!(payouts, vec![252, 253, 300, 400, 0]);
    assert_eq!(payouts.iter().sum::<i64>(), invested.iter().sum::<i64>());
    assert_eq!(payouts[3] - 200, 200, "seat 3 gets 200 uncalled back plus the top side pot");
    let final_stacks: Vec<i64> = h.seats.iter().zip(&payouts).map(|(seat, payout)| seat.stack + payout).collect();
    assert_eq!(final_stacks.iter().sum::<i64>(), h.seats.iter().map(|seat| seat.start_stack).sum::<i64>());
    let net: Vec<i64> = final_stacks.iter().zip(&h.seats).map(|(stack, seat)| stack - seat.start_stack).collect();
    assert_eq!(net.iter().sum::<i64>(), 0);
}

/// A pot no seat can win is void, and a void pot is returned to the seats that paid into it: every
/// seat gets its own wagers back, so the hand moves no chips between players. Play cannot reach this
/// state (the hand finishes at one live seat, `advance_if_needed`), so it is built directly; an
/// ordered side pot with a seat that folded for free is still settled seat by seat.
#[test]
fn settlement_of_an_all_folded_pot_refunds_every_contributor() {
    let runout = board(&["2c", "3d", "4h", "9s", "Kc"]);
    let seats = vec![
        Hand::seat_state(1_000, c2("5c", "6c")),
        Hand::seat_state(1_000, c2("5d", "6d")),
        Hand::seat_state(1_000, c2("Ah", "Ad")),
        Hand::seat_state(1_000, c2("Qh", "Qd")),
    ];
    let mut h = Hand::with_cards(seats, runout, 0, 10, 20);
    let invested = [20, 200, 0, 60];
    for (seat, amount) in h.seats.iter_mut().zip(invested) {
        seat.invested = amount;
        seat.stack = seat.start_stack - amount;
        seat.folded = true;
    }

    let payouts = h.settle(&runout);
    assert_eq!(payouts, invested.to_vec());
    assert_eq!(payouts.iter().sum::<i64>(), invested.iter().sum::<i64>(), "a void pot creates or destroys no chips");
    let final_stacks: Vec<i64> = h.seats.iter().zip(&payouts).map(|(seat, payout)| seat.stack + payout).collect();
    let net: Vec<i64> = final_stacks.iter().zip(&h.seats).map(|(stack, seat)| stack - seat.start_stack).collect();
    assert_eq!(net, vec![0, 0, 0, 0]);
    // No seats at all: no contributions, so nothing to pay and nothing to panic on.
    assert!(split_pots(&[], &[], &[], 0).is_empty());
}

#[test]
fn split_pot_and_all_in_runout() {
    let seats = vec![Hand::seat_state(500, c2("Ah", "2d")), Hand::seat_state(500, c2("Ac", "3d"))];
    let mut h = Hand::with_cards(seats, board(&["Kc", "Ks", "Qd", "Qs", "Jc"]), 0, 10, 20);
    h.apply(Action::AllIn).unwrap();
    h.apply(Action::Call).unwrap();
    assert!(h.is_finished());
    assert_eq!(h.net(), vec![0, 0]);
    assert_eq!(h.board.len(), 5);
}

#[test]
fn expected_net_removes_all_in_luck_exactly() {
    // Heads-up flop all-in: AhKh vs QsQd on 2h 7h Qc. Enumerate turn and river by hand.
    let (hero, villain) = (c2("Ah", "Kh"), c2("Qs", "Qd"));
    let run = board(&["2h", "7h", "Qc", "3s", "4d"]);
    let seats = vec![Hand::seat_state(1000, hero), Hand::seat_state(1000, villain)];
    let mut h = Hand::with_cards(seats, run, 0, 10, 20);
    h.apply(Action::Call).unwrap();
    h.apply(Action::Check).unwrap();
    h.apply(Action::Check).unwrap();
    h.apply(Action::AllIn).unwrap();
    h.apply(Action::Call).unwrap();
    assert!(h.is_finished() && h.showdown());
    let known = [run[0], run[1], run[2]];
    let dead = known.iter().chain(hero.iter()).chain(villain.iter()).fold(0u64, |m, c| m | c.bit());
    let unseen: Vec<Card> = (0..52u8).map(Card).filter(|c| dead & c.bit() == 0).collect();
    let (mut hero_share, mut n) = (0.0, 0.0);
    for a in 0..unseen.len() {
        for b in a + 1..unseen.len() {
            let m = known.iter().fold(0u64, |m, c| m | c.bit()) | unseen[a].bit() | unseen[b].bit();
            let (hv, vv) = (eval(m | hero[0].bit() | hero[1].bit()), eval(m | villain[0].bit() | villain[1].bit()));
            hero_share += if hv > vv {
                1.0
            } else if hv == vv {
                0.5
            } else {
                0.0
            };
            n += 1.0;
        }
    }
    let want = hero_share / n * 2000.0 - 1000.0;
    let mut rng = SmallRng::seed_from_u64(5);
    let got = h.expected_net(5000, &mut rng);
    assert!((got[0] - want).abs() < 1.0, "expected {want}, got {}", got[0]);
    assert!((got[0] + got[1]).abs() < 1e-6);
    // A hand decided on the river keeps its realized net.
    let seats = vec![Hand::seat_state(1000, hero), Hand::seat_state(1000, villain)];
    let mut r = Hand::with_cards(seats, run, 0, 10, 20);
    while r.actor().is_some() {
        r.apply(if r.legal().can_check { Action::Check } else { Action::Call }).unwrap();
    }
    assert_eq!(r.expected_net(5000, &mut rng), r.net().iter().map(|&x| x as f64).collect::<Vec<_>>());
}

#[test]
fn a_zero_runout_budget_samples_one_runout_and_never_nan() {
    // Heads-up flop all-in, as above: the runout space is bigger than a sample, so this is the
    // sampling path. A budget of zero runouts cannot be averaged into an expectation, so it means
    // the cheapest sample there is, one runout, rather than no runouts and a division by zero (#6).
    // One random runout still samples the same expectation, and the answer is finite per seat.
    let (hero, villain) = (c2("Ah", "Kh"), c2("Qs", "Qd"));
    let seats = vec![Hand::seat_state(1000, hero), Hand::seat_state(1000, villain)];
    let mut h = Hand::with_cards(seats, board(&["2h", "7h", "Qc", "3s", "4d"]), 0, 10, 20);
    h.apply(Action::Call).unwrap();
    h.apply(Action::Check).unwrap();
    h.apply(Action::Check).unwrap();
    h.apply(Action::AllIn).unwrap();
    h.apply(Action::Call).unwrap();
    assert!(h.is_finished() && h.showdown());
    let zero = h.expected_net(0, &mut SmallRng::seed_from_u64(11));
    assert!(zero.iter().all(|v| v.is_finite()), "a seat came back NaN: {zero:?}");
    assert_eq!(zero, h.expected_net(1, &mut SmallRng::seed_from_u64(11)), "zero runouts means one runout");
    assert!(zero.iter().sum::<f64>().abs() < 1e-9, "the runout leaked chips: {zero:?}");
}

#[test]
fn random_hands_conserve_chips() {
    let mut rng = SmallRng::seed_from_u64(99);
    for i in 0..20_000 {
        let n = 2 + i % 5;
        let stacks: Vec<i64> = (0..n).map(|_| rng.random_range(15..3000)).collect();
        let mut h = Hand::new(&stacks, i % n, 10, 20, &mut rng);
        let mut steps = 0;
        while h.actor().is_some() {
            let l = h.legal();
            let r = rng.random_range(0..10);
            let a = match r {
                0 => Action::Fold,
                1..=4 => Action::Call,
                5..=7 => match (l.min_raise_to, l.max_raise_to) {
                    (Some(a), Some(b)) => Action::RaiseTo(rng.random_range(a..=b)),
                    _ => Action::Call,
                },
                _ => Action::AllIn,
            };
            h.apply(a).unwrap();
            steps += 1;
            assert!(steps < 200);
        }
        assert_eq!(h.net().iter().sum::<i64>(), 0);
        assert!(h.seats.iter().all(|s| s.stack >= 0));
    }
}

/// The chance correction is zero-mean over the deal: enumerate every turn and river card for a fixed
/// line (three players see every street, checking down after a flop bet and call) and the average
/// correction must vanish, while the realized card still moves the correction for a single deal.
#[test]
fn chance_correction_averages_to_zero_over_every_turn_and_river() {
    let holes = [c2("Ah", "Kd"), c2("Qs", "Qc"), c2("7h", "6h")];
    let flop = parse_cards(&["Jh", "Td", "2h"]).unwrap();
    let dead = holes.iter().flatten().chain(flop.iter()).fold(0u64, |m, c| m | c.bit());
    let unseen: Vec<Card> = (0..52u8).map(Card).filter(|c| dead & c.bit() == 0).collect();
    let (mut sum, mut count, mut nonzero) = (0.0f64, 0usize, 0usize);
    for &turn in &unseen {
        for &river in unseen.iter().filter(|&&r| r != turn) {
            let seats = holes.iter().map(|h| Hand::seat_state(2_000, *h)).collect();
            let mut h = Hand::with_cards(seats, [flop[0], flop[1], flop[2], turn, river], 0, 10, 20);
            for a in [Action::Call, Action::Call, Action::Check] {
                h.apply(a).unwrap();
            }
            // Flop: bet and two calls; turn and river: everyone checks.
            h.apply(Action::RaiseTo(60)).unwrap();
            h.apply(Action::Call).unwrap();
            h.apply(Action::Call).unwrap();
            while !h.is_finished() {
                h.apply(Action::Check).unwrap();
            }
            let c = h.chance_correction(0);
            nonzero += usize::from(c.abs() > 1e-9);
            sum += c;
            count += 1;
        }
    }
    assert!(nonzero > count / 4, "the correction must depend on the cards ({nonzero}/{count})");
    assert!((sum / count as f64).abs() < 1e-9, "mean correction {} over {count} deals", sum / count as f64);
}

#[test]
fn cumulative_short_all_ins_reopen_at_a_full_raise_for_each_actor() {
    for (last_all_in, expected_min) in [(179, None), (180, Some(260))] {
        let mut rng = SmallRng::seed_from_u64(701);
        let mut hand = Hand::new(&[1000, 130, last_all_in, 1000], 0, 10, 20, &mut rng);
        hand.apply(Action::RaiseTo(100)).unwrap();
        hand.apply(Action::Call).unwrap();
        hand.apply(Action::AllIn).unwrap();
        hand.apply(Action::AllIn).unwrap();
        assert_eq!(hand.actor(), Some(3));
        assert_eq!(hand.legal().min_raise_to, expected_min);
        if expected_min.is_some() {
            hand.apply(Action::RaiseTo(260)).unwrap();
            assert!(hand.history.last().unwrap().full_raise);
        }
    }
}

#[test]
fn cumulative_reopening_uses_each_players_last_call_level() {
    let mut rng = SmallRng::seed_from_u64(702);
    // First actor 3 raises100; 4 calls100; 0 shoves130; 1 calls130; 2 shoves180.
    let mut hand = Hand::new(&[130, 1000, 180, 1000, 1000], 0, 10, 20, &mut rng);
    for action in [Action::RaiseTo(100), Action::Call, Action::AllIn, Action::Call, Action::AllIn] {
        hand.apply(action).unwrap();
    }
    assert_eq!(hand.actor(), Some(3));
    assert_eq!(hand.legal().min_raise_to, Some(260));
    hand.apply(Action::Call).unwrap();
    assert_eq!(hand.actor(), Some(4));
    assert_eq!(hand.legal().min_raise_to, Some(260));
    hand.apply(Action::Call).unwrap();
    assert_eq!(hand.actor(), Some(1));
    assert_eq!(hand.legal().call_amount, 50);
    assert_eq!(hand.legal().min_raise_to, None);
}

#[test]
fn cumulative_short_all_ins_reopen_postflop_and_reset_on_next_street() {
    let mut rng = SmallRng::seed_from_u64(703);
    let mut hand = Hand::new(&[200, 1000, 1000, 150], 0, 10, 20, &mut rng);
    for action in [Action::Call, Action::Call, Action::Call, Action::Check] {
        hand.apply(action).unwrap();
    }
    assert_eq!(hand.street, Street::Flop);
    // Seats1 and2 bet/call100; 3 shoves130; 0 shoves180.
    for action in [Action::RaiseTo(100), Action::Call, Action::AllIn, Action::AllIn] {
        hand.apply(action).unwrap();
    }
    // Opening bet100 means 80 accumulated is still short.
    assert_eq!(hand.actor(), Some(1));
    assert_eq!(hand.legal().min_raise_to, None);
    hand.apply(Action::Call).unwrap();
    hand.apply(Action::Call).unwrap();
    assert_eq!(hand.street, Street::Turn);
    assert_eq!(hand.legal().min_raise_to, Some(20));

    let mut hand = Hand::new(&[220, 1000, 1000, 150], 0, 10, 20, &mut rng);
    for action in
        [Action::Call, Action::Call, Action::Call, Action::Check, Action::RaiseTo(100), Action::Call, Action::AllIn, Action::AllIn]
    {
        hand.apply(action).unwrap();
    }
    assert_eq!(hand.actor(), Some(1));
    assert_eq!(hand.legal().min_raise_to, Some(300));
}
