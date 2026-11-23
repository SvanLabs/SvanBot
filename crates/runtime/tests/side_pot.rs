//! The live price of a short call: the chips already in that a call can win, reconstructed from
//! each seat's opening stack, and only when those reconcile with the venue's pot (V11-28).

use svanbot_core::rules::Action;
use svanbot_core::spot::Spot;
use svanbot_runtime::session::Session;
use svanbot_runtime::wire::{Inbound, Offer, Player, Turn};

const ROSTER: [&str; 5] = ["SvanBotV10", "Svanar", "Svanism", "SuraGunnar", "SurSvan"];

fn session() -> Session {
    Session::new("SurSvan", ROSTER.map(String::from))
}

fn player(seat: usize, name: &str, stack: u32) -> Player {
    Player {
        seat,
        name: name.into(),
        stack,
    }
}

fn joined(players: Vec<Player>) -> Inbound {
    Inbound::TableJoined {
        table_id: "t-1".into(),
        seat: 2,
        players,
    }
}

fn start() -> Inbound {
    Inbound::HandStart {
        hand_id: "h-1".into(),
        seat: 2,
        dealer_seat: 0,
        small_blind: 10,
        big_blind: 20,
    }
}

fn never(_: &Spot) -> Action {
    unreachable!("no decision is due")
}

#[test]
fn a_missed_result_cannot_price_the_next_hand_from_the_old_distribution() {
    // We resumed mid-hand, without its start. The hero wins 100 from the other two
    // seats. The payout changes the distribution but not the table's chip total.
    use svanbot_runtime::wire::parse;
    let players = |x, y, z| {
        vec![
            player(0, "a", x),
            player(1, "b", y),
            player(2, "SurSvan", z),
        ]
    };
    let turn = |id: &str, pot, players, offer| {
        Inbound::YourTurn(Turn {
            hand_id: Some(id.into()),
            turn_token: format!("{id}-turn"),
            pot,
            players,
            offer,
            board: vec![],
        })
    };
    let free = Offer {
        check: true,
        ..Offer::default()
    };
    let short = Offer {
        fold: true,
        call: Some(80),
        ..Offer::default()
    };
    for snapshot in [false, true] {
        for paid in [false, true] {
            let mut s = session();
            s.on_frame(joined(players(1100, 1100, 100)), never);
            if snapshot {
                let state = parse(
                    r#"{"type":"table_state","hand_id":"prior","pot":150,"board":[],"seats":[
                    {"seat":0,"name":"a","stack":1050},{"seat":1,"name":"b","stack":1050},
                    {"seat":2,"name":"SurSvan","stack":50}]}"#,
                )
                .unwrap();
                s.on_frame(state, never);
            } else {
                s.on_frame(turn("prior", 150, players(1050, 1050, 50), free), |_| {
                    Action::Check
                });
            }
            if paid {
                let result = Inbound::HandResult {
                    final_stacks: vec![(0, 1050), (1, 1050), (2, 200)],
                    shown: vec![],
                };
                s.on_frame(result, never);
            }
            s.on_frame(start(), never);
            let mut observed = None;
            s.on_frame(turn("h-1", 1820, players(200, 200, 80), short), |spot| {
                observed = Some((spot.callable_pot, svanbot_strategy::pricer::price_of(spot)));
                Action::Call
            });
            let pot = paid.then_some(520);
            assert_eq!(
                observed,
                Some((pot, Some(80.0 / if paid { 600.0 } else { 1900.0 })))
            );
        }
    }
}

#[test]
fn a_live_short_call_uses_only_reconciled_hand_contributions() {
    // Seat 3 folded after putting in 10. Its dead chips are still winnable.
    for (pot, expected) in [(2030, Some(230)), (2040, None)] {
        let mut s = session();
        s.on_frame(
            joined(vec![
                player(0, "a", 1200),
                player(1, "b", 1300),
                player(2, "SurSvan", 100),
                player(3, "c", 400),
            ]),
            never,
        );
        s.on_frame(start(), never);
        s.on_frame(
            Inbound::PlayerAction {
                seat: 3,
                name: None,
                action: "fold".into(),
                amount: 0,
                stack: 390,
                detail: Default::default(),
            },
            never,
        );
        let mut saw_turn = false;
        s.on_frame(
            Inbound::YourTurn(Turn {
                hand_id: Some("h-1".into()),
                turn_token: "t".into(),
                board: vec![],
                pot,
                players: vec![
                    player(0, "a", 200),
                    player(1, "b", 300),
                    player(2, "SurSvan", 80),
                    player(3, "c", 390),
                ],
                offer: Offer {
                    fold: true,
                    call: Some(80),
                    ..Offer::default()
                },
            }),
            |spot| {
                saw_turn = true;
                assert_eq!(spot.callable_pot, expected);
                assert_eq!(
                    svanbot_strategy::pricer::price_of(spot),
                    Some(80.0 / if expected.is_some() { 310.0 } else { 2120.0 })
                );
                Action::Fold
            },
        );
        assert!(saw_turn);
    }
}
