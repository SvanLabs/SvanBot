//! The session: venue frames in, messages out, with the fair-play guard on every seat list.

use std::cell::RefCell;

use svanbot_core::rules::{Action, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_runtime::session::{Incident, Session};
use svanbot_runtime::wire::{Inbound, Offer, Outbound, Player, Turn, VenueAction};

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

fn turn(offer: Offer) -> Inbound {
    Inbound::YourTurn(Turn {
        hand_id: Some("h-1".into()),
        turn_token: "tok-1".into(),
        offer,
        pot: 30,
        board: vec![],
        players: vec![player(0, "alpha_bot", 1980), player(2, "SurSvan", 1990)],
    })
}

fn facing_a_blind() -> Offer {
    Offer {
        fold: true,
        check: false,
        call: Some(20),
        raise: Some(RaiseRange { min: 40, max: 2000 }),
        all_in: Some(2000),
    }
}

fn never(_: &Spot) -> Action {
    unreachable!("no decision is due")
}

#[test]
fn it_joins_the_lobby_with_auto_rebuy_on() {
    assert_eq!(
        session().on_connect(),
        [
            Outbound::JoinLobby { buy_in: 2000 },
            Outbound::SetAutoRebuy { enabled: true }
        ]
    );
}

#[test]
fn at_its_turn_it_decides_on_the_spot_and_echoes_the_hand_and_token() {
    let mut session = session();
    session.on_frame(
        joined(vec![
            player(0, "alpha_bot", 2000),
            player(2, "SurSvan", 2000),
        ]),
        never,
    );
    session.on_frame(start(), never);
    session.on_frame(
        Inbound::HoleCards {
            cards: vec!["Ah".parse().unwrap(), "As".parse().unwrap()],
        },
        never,
    );
    let seen = RefCell::new(None);
    let step = session.on_frame(turn(facing_a_blind()), |spot: &Spot| {
        *seen.borrow_mut() = Some(spot.clone());
        Action::RaiseTo(60)
    });
    let spot = seen.into_inner().expect("the decision was asked");
    assert_eq!(spot.hole, "AhAs".parse().unwrap());
    assert_eq!(
        (spot.pot, spot.legal.call, spot.opponents, spot.stack),
        (30, Some(20), 1, 1990)
    );
    let [
        Outbound::Action {
            hand_id,
            action,
            client_action_id,
            turn_token,
        },
    ] = &step.send[..]
    else {
        panic!("{step:?}")
    };
    assert_eq!(
        (hand_id.as_str(), turn_token.as_str(), *action),
        ("h-1", "tok-1", VenueAction::Raise(60))
    );
    assert!(!client_action_id.is_empty());
    assert_eq!(step.incidents, []);
}

#[test]
fn it_never_plays_an_action_the_venue_did_not_offer() {
    let play = |offer: Offer, decided: Action| {
        let mut session = session();
        session.on_frame(start(), never);
        session.on_frame(
            Inbound::HoleCards {
                cards: vec!["7c".parse().unwrap(), "2d".parse().unwrap()],
            },
            never,
        );
        match &session.on_frame(turn(offer), |_| decided).send[..] {
            [Outbound::Action { action, .. }] => *action,
            other => panic!("{other:?}"),
        }
    };
    let short = Offer {
        fold: true,
        check: false,
        call: Some(500),
        raise: None,
        all_in: Some(1990),
    };
    assert_eq!(play(short, Action::RaiseTo(5000)), VenueAction::AllIn);
    assert_eq!(play(facing_a_blind(), Action::Check), VenueAction::Fold);
    let free = Offer {
        fold: true,
        check: true,
        call: None,
        raise: Some(RaiseRange { min: 20, max: 2000 }),
        all_in: None,
    };
    assert_eq!(play(free, Action::Fold), VenueAction::Check);
    assert_eq!(play(free, Action::RaiseTo(9000)), VenueAction::Raise(2000));
}

#[test]
fn a_stablemate_at_the_table_makes_it_leave_and_raises_an_incident() {
    let mut session = session();
    let step = session.on_frame(
        joined(vec![player(0, "Svanism", 2000), player(2, "SurSvan", 2000)]),
        never,
    );
    assert_eq!(step.send, [Outbound::LeaveTable]);
    assert_eq!(
        step.incidents,
        [Incident::SharedTable {
            table_id: "t-1".into(),
            stablemate: "Svanism".into()
        }]
    );
}

#[test]
fn a_stablemate_who_sits_down_later_is_caught_too_whatever_the_case_of_the_name() {
    let mut session = session();
    session.on_frame(
        joined(vec![
            player(0, "alpha_bot", 2000),
            player(2, "SurSvan", 2000),
        ]),
        never,
    );
    let step = session.on_frame(Inbound::PlayerJoined(player(4, "svanar", 2000)), never);
    assert_eq!(step.send, [Outbound::LeaveTable]);
    assert!(matches!(step.incidents[..], [Incident::SharedTable { .. }]));
}

#[test]
fn a_hand_dealt_with_a_stablemate_is_a_shared_hand_and_is_folded() {
    let mut session = session();
    session.on_frame(
        joined(vec![
            player(0, "alpha_bot", 2000),
            player(2, "SurSvan", 2000),
        ]),
        never,
    );
    session.on_frame(start(), never);
    session.on_frame(
        Inbound::HoleCards {
            cards: vec!["Ah".parse().unwrap(), "As".parse().unwrap()],
        },
        never,
    );
    let Inbound::YourTurn(mut frame) = turn(facing_a_blind()) else {
        unreachable!()
    };
    frame.players.push(player(5, "SuraGunnar", 2000));
    let step = session.on_frame(Inbound::YourTurn(frame), never);
    assert!(step.send.iter().any(|m| matches!(
        m,
        Outbound::Action {
            action: VenueAction::Fold,
            ..
        }
    )));
    assert!(step.send.contains(&Outbound::LeaveTable));
    assert_eq!(
        step.incidents,
        [Incident::SharedHand {
            hand_id: "h-1".into(),
            stablemate: "SuraGunnar".into()
        }]
    );
}

#[test]
fn a_closed_table_or_a_new_season_sends_it_back_to_the_lobby() {
    let mut session = session();
    assert_eq!(
        session.on_frame(Inbound::TableClosed, never).send,
        [Outbound::JoinLobby { buy_in: 2000 }]
    );
    assert_eq!(
        session.on_frame(Inbound::SeasonEnded, never).send,
        [Outbound::JoinLobby { buy_in: 2000 }]
    );
}

#[test]
fn a_shared_hand_is_folded_even_when_the_check_is_free() {
    let mut session = session();
    session.on_frame(start(), never);
    let free = Offer {
        fold: true,
        check: true,
        call: None,
        raise: None,
        all_in: None,
    };
    let Inbound::YourTurn(mut frame) = turn(free) else {
        unreachable!()
    };
    frame.players.push(player(5, "Svanar", 2000));
    let step = session.on_frame(Inbound::YourTurn(frame), never);
    assert!(
        matches!(
            step.send[0],
            Outbound::Action {
                action: VenueAction::Fold,
                ..
            }
        ),
        "{step:?}"
    );
}
