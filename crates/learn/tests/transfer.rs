//! Gene transfer (V11-80, spec Phase 2 item 5): a candidate that cleared one bot's search is
//! offered to another bot, which accepts it only inside its own style's pins and bounds and then
//! screens and confirms it against its own champion on its own deals.
//!
//! The offer is the donor's parameter set, the numbers only: no verdict, no seed and no opponent
//! read travels with it. The receiver's search log is keyed by its own champion, so the donor's
//! evidence cannot be read as the receiver's.

use svanbot_learn::search::{BotNext, Entry, Stage, Step, next_bot_with};
use svanbot_learn::transfer::{Offer, offer, push};
use svanbot_strategy::params::Params;
use svanbot_strategy::profile::ACTIVE;
use svanbot_strategy::style::{STANDARD, Style};

const BOUNDED: Style = Style {
    name: "bounded",
    open_scale: 0.8,
    mechanics: svanbot_strategy::style::NEUTRAL,
    profile: ACTIVE,
    bounds: &[
        ("exploit", 0.0, 0.45),
        ("three_bet_value", 0.04, 0.04),
        ("three_bet_bluff", 0.0, 0.0),
        ("cbet_dry", 0.6, 0.6),
        ("cbet_wet", 0.3, 0.3),
        ("size_small", 0.33, 0.33),
        ("size_big", 1.0, 1.0),
        ("station_vpip", 0.85, 0.85),
        ("station_pfr", 0.15, 0.15),
        ("maniac_vpip", 0.8, 0.8),
        ("maniac_pfr", 0.5, 0.5),
        ("nit_vpip", 0.2, 0.2),
        ("nit_fold", 0.7, 0.7),
    ],
};

const PINNED: Style = Style {
    name: "pinned",
    open_scale: 0.8,
    mechanics: svanbot_strategy::style::NEUTRAL,
    profile: ACTIVE,
    bounds: &[
        ("exploit", 0.5, 0.5),
        ("three_bet_value", 0.04, 0.04),
        ("three_bet_bluff", 0.0, 0.0),
        ("cbet_dry", 0.6, 0.6),
        ("cbet_wet", 0.3, 0.3),
        ("size_small", 0.33, 0.33),
        ("size_big", 1.0, 1.0),
        ("station_vpip", 0.85, 0.85),
        ("station_pfr", 0.15, 0.15),
        ("maniac_vpip", 0.8, 0.8),
        ("maniac_pfr", 0.5, 0.5),
        ("nit_vpip", 0.2, 0.2),
        ("nit_fold", 0.7, 0.7),
    ],
};

fn at(exploit: f64) -> Params {
    Params {
        exploit,
        spread: 0.0,
        ..Params::DEFAULT
    }
}

fn found(mbb: f64) -> svanbot_learn::paired::Verdict {
    svanbot_learn::paired::Verdict {
        deals: 200,
        mbb_per_100: mbb,
        ci95: 10.0,
    }
}

fn screened(champion: &Params, challenger: Params, seed: u64, mbb: f64) -> Entry {
    Entry::Simmed {
        step: Step {
            champion: champion.digest(),
            challenger,
            stage: Stage::First,
            seed,
        },
        verdict: found(mbb),
    }
}

fn confirmed(champion: &Params, challenger: Params, seed: u64, mbb: f64) -> Entry {
    Entry::Simmed {
        step: Step {
            champion: champion.digest(),
            challenger,
            stage: Stage::Confirmation,
            seed,
        },
        verdict: found(mbb),
    }
}

#[test]
fn a_donor_candidate_is_offered_only_inside_the_receiver_s_style() {
    let receiver = at(0.7);
    let donor = at(0.4);
    let held = offer(donor, &receiver, &STANDARD).expect("standard allows the move");
    assert_eq!(held.challenger, donor);

    // A pin at 0.5 brings the candidate to the pinned value (a move from the receiver's 0.7),
    // and a bound of 0.45 brings a candidate at 0.6 inside it rather than refusing it.
    assert_eq!(
        offer(donor, &receiver, &PINNED).map(|offer| offer.challenger.exploit),
        Some(0.5)
    );
    assert_eq!(
        offer(at(0.6), &receiver, &BOUNDED).map(|offer| offer.challenger.exploit),
        Some(0.45)
    );
    assert_eq!(
        offer(donor, &receiver, &BOUNDED).map(|offer| offer.challenger),
        Some(donor)
    );
    // The receiver's own champion is no move.
    assert_eq!(offer(receiver, &receiver, &STANDARD), None);
}

#[test]
fn duplicate_offers_are_kept_once() {
    let receiver = at(0.7);
    let donor = at(0.4);
    let mut offers = Vec::new();
    assert!(push(
        &mut offers,
        offer(donor, &receiver, &STANDARD).unwrap()
    ));
    assert!(!push(
        &mut offers,
        offer(donor, &receiver, &STANDARD).unwrap()
    ));
    assert!(!push(&mut offers, Offer { challenger: donor }));
    assert!(push(
        &mut offers,
        offer(at(0.5), &receiver, &STANDARD).unwrap()
    ));
    assert_eq!(offers.len(), 2);
}

#[test]
fn a_transferred_offer_is_screened_on_the_receiver_s_deals_not_the_donor_s() {
    let receiver = at(0.7);
    let candidate = at(0.4);
    let moved = offer(candidate, &receiver, &STANDARD).unwrap();

    let BotNext::Screen { challenger, seed } = next_bot_with(&receiver, &STANDARD, &[], &[moved])
    else {
        panic!("a transfer starts as a screen");
    };
    assert_eq!(challenger, candidate);

    // The same deals the receiver's own ring screens on: its champion names the stream, not the
    // donor's champion or the donor's candidate.
    let BotNext::Screen { seed: own_seed, .. } = next_bot_with(&receiver, &STANDARD, &[], &[])
    else {
        panic!("the receiver has its own ring");
    };
    assert_eq!(seed, own_seed);
}

#[test]
fn a_transferred_move_cannot_skip_the_receiver_s_screen_or_confirmation() {
    let receiver = at(0.7);
    let candidate = at(0.4);
    let moved = offer(candidate, &receiver, &BOUNDED).unwrap();
    // BOUNDED leaves the receiver's own ring outside it, so the offer is the only challenger.
    let BotNext::Screen { challenger, seed } =
        next_bot_with(&receiver, &BOUNDED, &[], std::slice::from_ref(&moved))
    else {
        panic!("a transfer is screened first");
    };
    assert_eq!(challenger, candidate);

    // A losing screen drops it: no candidate.
    let lost = vec![screened(&receiver, challenger, seed, -50.0)];
    assert!(matches!(
        next_bot_with(&receiver, &BOUNDED, &lost, std::slice::from_ref(&moved)),
        BotNext::Held
    ));

    // A winning screen asks for a fresh confirmation, not a candidate.
    let won = vec![screened(&receiver, challenger, seed, 100.0)];
    let BotNext::Confirm {
        challenger: asked,
        seed: confirm_seed,
    } = next_bot_with(&receiver, &BOUNDED, &won, std::slice::from_ref(&moved))
    else {
        panic!("a winning screen asks for its confirmation");
    };
    assert_eq!(asked, candidate);
    assert_ne!(confirm_seed, seed, "the confirmation plays fresh deals");

    // Only the confirmation above zero makes it a candidate.
    let mut both = won;
    both.push(confirmed(&receiver, candidate, confirm_seed, 100.0));
    match next_bot_with(&receiver, &BOUNDED, &both, &[moved]) {
        BotNext::Candidate { challenger, .. } => assert_eq!(challenger, candidate),
        other => panic!("expected the receiver's candidate, got {other:?}"),
    }
}

#[test]
fn a_stored_offer_is_checked_again_against_the_receiver_s_current_style() {
    let receiver = at(0.7);
    let moved = offer(at(0.4), &receiver, &STANDARD).unwrap();
    // The receiver moved to a style that pins 0.5: the stored offer at 0.4 is ignored.
    assert_eq!(
        next_bot_with(&receiver, &PINNED, &[], &[moved]),
        BotNext::Held
    );
}
