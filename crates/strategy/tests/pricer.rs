//! The pricer: the fallback way to decide. It prices each legal action by hero's equity against
//! the opponents' ranges (uniform, until V11-06 brings real ones) and takes the best.

use svanbot_core::card::{Card, CardSet};
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Legal, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_strategy::pricer::{Budget, decide};

const BUDGET: Budget = Budget {
    samples: 3_200,
    seed: 5,
};

fn spot(hole: &str, board: &str, pot: u32, legal: Legal, opponents: usize) -> Spot {
    Spot {
        hole: hole.parse().unwrap(),
        board: board.parse().unwrap(),
        pot,
        legal,
        opponents,
        stack: 2000,
        callable_pot: None,
        committed: 0,
        big_blind: 20,
        in_position: false,
        lines: vec![],
        seats: vec![],
    }
}

fn facing(call: u32, raise: Option<(u32, u32)>) -> Legal {
    Legal {
        call: Some(call),
        raise: raise.map(|(min, max)| RaiseRange { min, max }),
    }
}

#[test]
fn short_call_cannot_win_the_deep_stacks_side_pot() {
    use svanbot_core::rules::Blinds;
    use svanbot_sim::table::{deal, play};

    let cards = deal(3, &mut Rng::seeded(1));
    for bet in [100, 1000] {
        let mut decision = None;
        play(
            &cards,
            &[1000, 1000, 100],
            0,
            Blinds { small: 10, big: 20 },
            |seat, spot| match seat {
                0 => Action::RaiseTo(bet),
                1 => Action::Call,
                _ => {
                    assert_eq!(spot.legal.call, Some(80));
                    let d = decide(
                        spot,
                        None,
                        Budget {
                            samples: 5,
                            seed: 4,
                        },
                    );
                    decision = Some(d);
                    d.action
                }
            },
        );
        let d = decision.unwrap();
        assert_eq!(d.price, Some(80.0 / 300.0), "{d:?}");
        assert_eq!(d.equity, 0.2, "{d:?}");
        assert_eq!(d.action, Action::Fold);
    }
}

#[test]
fn aces_raise_preflop() {
    let decision = decide(
        &spot("AhAs", "", 30, facing(20, Some((40, 2000))), 1),
        None,
        BUDGET,
    );
    assert!(
        matches!(decision.action, Action::RaiseTo(_)),
        "{decision:?}"
    );
}

#[test]
fn a_premium_hand_opens_a_full_table_to_two_and_a_half_blinds() {
    // First to act of six, the blinds posted: kings are worth more raised than limped, and the
    // raise is the table's standard open, not a shove.
    let decision = decide(
        &spot("KhKs", "", 30, facing(20, Some((40, 2000))), 5),
        None,
        BUDGET,
    );
    assert_eq!(decision.action, Action::RaiseTo(50), "{decision:?}");
}

#[test]
fn a_weak_hand_folds_to_a_big_bet() {
    // 7-2 offsuit on a board it missed, facing a pot-sized bet: it needs a third of the pot.
    let decision = decide(
        &spot("7c2d", "AhKsQh", 400, facing(200, Some((400, 1800))), 1),
        None,
        BUDGET,
    );
    assert_eq!(decision.action, Action::Fold, "{decision:?}");
}

#[test]
fn a_free_look_is_never_folded() {
    let free = Legal {
        call: None,
        raise: Some(RaiseRange { min: 20, max: 2000 }),
    };
    let decision = decide(&spot("7c2d", "AhKsQh", 40, free, 2), None, BUDGET);
    assert_eq!(decision.action, Action::Check, "{decision:?}");
}

#[test]
fn the_nuts_on_the_river_bet_for_value() {
    let free = Legal {
        call: None,
        raise: Some(RaiseRange { min: 20, max: 1500 }),
    };
    let decision = decide(&spot("AhKh", "QhJhTh2c3d", 300, free, 1), None, BUDGET);
    assert!(
        matches!(decision.action, Action::RaiseTo(_)),
        "{decision:?}"
    );
    assert!(decision.equity > 0.99);
}

#[test]
fn every_decision_is_legal_and_the_same_seed_repeats_it() {
    let mut rng = Rng::seeded(9);
    let deck: Vec<String> = "23456789TJQKA"
        .chars()
        .flat_map(|rank| "cdhs".chars().map(move |suit| format!("{rank}{suit}")))
        .collect();
    for _ in 0..200 {
        let board_len = [0, 3, 4, 5][rng.below(4) as usize];
        let mut seen = CardSet::EMPTY;
        let mut dealt: Vec<Card> = Vec::new();
        while dealt.len() < 2 + board_len {
            let card: Card = deck[rng.below(52) as usize].parse().unwrap();
            if seen.insert(card) {
                dealt.push(card);
            }
        }
        let (hole, board) = dealt.split_at(2);
        let to_call = [0, 20, 100, 600][rng.below(4) as usize];
        let raise = (rng.below(3) > 0).then(|| RaiseRange {
            min: to_call.max(20) * 2,
            max: 2000,
        });
        let legal = Legal {
            call: (to_call > 0).then_some(to_call),
            raise,
        };
        let spot = Spot {
            hole: hole.iter().copied().collect(),
            board: board.iter().copied().collect(),
            pot: 30 + to_call * 2,
            legal,
            opponents: 1 + rng.below(5) as usize,
            stack: 2000,
            callable_pot: None,
            committed: 0,
            big_blind: 20,
            in_position: false,
            lines: vec![],
            seats: vec![],
        };
        let budget = Budget {
            samples: 640,
            seed: rng.next_u64(),
        };
        let decision = decide(&spot, None, budget);
        let legal_now = match decision.action {
            Action::Fold => to_call > 0,
            Action::Check => to_call == 0,
            Action::Call => to_call > 0,
            Action::RaiseTo(to) => raise.is_some_and(|r| (r.min..=r.max).contains(&to)),
        };
        assert!(legal_now, "{decision:?} at {spot:?}");
        assert_eq!(decide(&spot, None, budget), decision);
    }
}

/// A table whose players, facing a bet, always take `answer`, and otherwise check.
fn answering(answer: svanbot_strategy::action::Move) -> svanbot_strategy::action::ActionModel {
    use svanbot_strategy::action::{ActionModel, BUCKETS, Counts, Move, Situation};
    let mut counts = Counts::default();
    for situation in Situation::all() {
        for bucket in 0..BUCKETS {
            let mv = if situation.facing {
                answer
            } else {
                Move::Check
            };
            counts.add(situation, bucket, mv, 1.0);
        }
    }
    ActionModel::from_counts(&counts, 0.0)
}

#[test]
fn a_bet_is_worth_the_pot_it_takes_when_everyone_folds() {
    use svanbot_strategy::action::Move;
    use svanbot_strategy::params::Params;
    use svanbot_strategy::pricer::price_lines;
    // 7-2 offsuit missed the board and checking wins it rarely; against two players who fold to
    // any bet, a bet takes the whole pot every time.
    let free = Legal {
        call: None,
        raise: Some(RaiseRange { min: 20, max: 2000 }),
    };
    let at = spot("7c2d", "AhKsQh", 400, free, 2);
    let folds = answering(Move::Fold);
    let decision = price_lines(&at, &[&folds, &folds], BUDGET, &Params::DEFAULT);
    assert!(
        matches!(decision.action, Action::RaiseTo(_)),
        "{decision:?}"
    );
    // Against two who never fold, the same bet only puts chips behind a losing hand.
    let calls = answering(Move::Call);
    let decision = price_lines(&at, &[&calls, &calls], BUDGET, &Params::DEFAULT);
    assert_eq!(decision.action, Action::Check, "{decision:?}");
}

#[test]
fn a_call_is_priced_against_the_players_left_to_act_as_they_answer_it() {
    use svanbot_core::rules::Street;
    use svanbot_core::spot::{Act, ActKind};
    use svanbot_strategy::action::{ActionModel, BUCKETS, Counts, Move, Situation};
    use svanbot_strategy::params::Params;
    use svanbot_strategy::pricer::price_lines;
    // One player opened to three blinds; three are still to act, and this table folds 95% of
    // the time to any bet, whatever it holds. Against four players who all stay, nines fall
    // short of the price; against the opener and the few who call behind, they are worth it.
    let mut counts = Counts::default();
    for situation in Situation::all() {
        for bucket in 0..BUCKETS {
            if situation.facing {
                counts.add(situation, bucket, Move::Fold, 0.95);
                counts.add(situation, bucket, Move::Large, 0.05);
            } else {
                counts.add(situation, bucket, Move::Check, 1.0);
            }
        }
    }
    let table = ActionModel::from_counts(&counts, 0.0);
    let mut at = spot("9h9s", "", 90, facing(60, None), 4);
    let opened = Act {
        street: Street::Preflop,
        board: CardSet::EMPTY,
        facing: true,
        kind: ActKind::Bet { added: 60, pot: 30 },
    };
    at.lines = vec![vec![opened], vec![], vec![], vec![]];
    let decision = price_lines(&at, &[&table; 4], BUDGET, &Params::DEFAULT);
    assert!(decision.equity < 0.4, "{decision:?}");
    assert_eq!(decision.action, Action::Call, "{decision:?}");
}

#[test]
fn a_big_river_bet_from_a_player_who_bets_big_only_when_strong_folds_top_pair() {
    use svanbot_core::rules::Street;
    use svanbot_core::spot::{Act, ActKind};
    use svanbot_strategy::action::{ActionModel, BUCKETS, Counts, Move, Situation};
    // The bettor bets the pot on the river only with the top eighth of their range.
    let mut counts = Counts::default();
    let open = Situation {
        street: Street::River,
        facing: false,
    };
    for bucket in 0..BUCKETS {
        let mv = if bucket == BUCKETS - 1 {
            Move::Large
        } else {
            Move::Check
        };
        counts.add(open, bucket, mv, 200.0);
    }
    let polar = ActionModel::from_counts(&counts, 1.0);
    let board = "Ks9d4c7h2s";
    let mut spot = spot("KcJd", board, 200, facing(100, None), 1);
    spot.lines = vec![vec![Act {
        street: Street::River,
        board: board.parse().unwrap(),
        facing: false,
        kind: ActKind::Bet {
            added: 100,
            pot: 100,
        },
    }]];
    // Against any two cards, top pair calls a third of the pot. Against this line it folds.
    assert_eq!(decide(&spot, None, BUDGET).action, Action::Call);
    assert_eq!(decide(&spot, Some(&polar), BUDGET).action, Action::Fold);
}
