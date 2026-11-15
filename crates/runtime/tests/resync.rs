//! The session across a reconnect: the `table_seq` watermark, `resync_request`, and installing a
//! `resync_response` or a `table_state` snapshot (<https://docs.openpoker.ai/llms-full.txt>,
//! rev 2026-09-02, "Envelope Metadata", "Reconnection" and "resync_response").

use std::cell::Cell;

use svanbot_core::rules::{Action, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_runtime::session::{Incident, Session};
use svanbot_runtime::wire::{
    Frame, Hero, Inbound, Offer, Outbound, Player, Resync, Seat, Snapshot, VenueAction,
};

const ROSTER: [&str; 5] = ["SvanBotV10", "Svanar", "Svanism", "SuraGunnar", "SurSvan"];

fn session() -> Session {
    Session::new("SurSvan", ROSTER.map(String::from))
}

fn never(_: &Spot) -> Action {
    unreachable!("no decision is due")
}

fn at(table_seq: u64, inbound: Inbound) -> Frame {
    Frame {
        table_seq: Some(table_seq),
        inbound,
    }
}

fn seat(seat: usize, name: &str, folded: Option<bool>) -> Seat {
    Seat {
        player: Player {
            seat,
            name: name.into(),
            stack: 1900,
        },
        folded,
    }
}

/// A session seated at `t-1` that has applied events up to `table_seq` 42.
fn seated() -> Session {
    let mut session = session();
    session.on_frame(
        Inbound::TableJoined {
            table_id: "t-1".into(),
            seat: 2,
            players: vec![
                seat(0, "alpha_bot", None).player,
                seat(2, "SurSvan", None).player,
            ],
        },
        never,
    );
    session.on_frame(
        at(
            42,
            Inbound::HandStart {
                hand_id: "h-7".into(),
                seat: 2,
                dealer_seat: 0,
                small_blind: 10,
                big_blind: 20,
            },
        ),
        never,
    );
    session
}

fn offer() -> Offer {
    Offer {
        fold: true,
        check: false,
        call: Some(40),
        raise: Some(RaiseRange { min: 80, max: 1880 }),
        all_in: Some(1880),
    }
}

/// The snapshot the actor gets back: our turn, with the token the venue still holds.
fn acting(seats: Vec<Seat>) -> Snapshot {
    Snapshot {
        hand_id: Some("h-7".into()),
        pot: 120,
        board: ["7d", "Ts", "2c"].map(|c| c.parse().unwrap()).to_vec(),
        seats,
        hero: Some(Hero {
            seat: 2,
            hole: ["Ah", "Kd"].map(|c| c.parse().unwrap()).to_vec(),
            offer: Some(offer()),
            turn_token: Some("tok-9".into()),
        }),
    }
}

#[test]
fn a_reconnect_with_a_table_asks_for_the_events_it_missed_instead_of_rejoining() {
    assert_eq!(
        seated().on_connect(),
        [Outbound::ResyncRequest {
            table_id: "t-1".into(),
            last_table_seq: 42
        }]
    );
    let mut forgot = seated();
    forgot.forget_table();
    assert!(matches!(forgot.on_connect()[0], Outbound::JoinLobby { .. }));
}

#[test]
fn a_duplicate_or_older_event_is_ignored_and_a_forward_jump_is_applied() {
    let mut session = seated();
    session.on_frame(at(50, Inbound::PlayerLeft { seat: 0 }), never);
    let late_stablemate = Inbound::PlayerJoined(seat(4, "Svanar", None).player);
    assert_eq!(
        session
            .on_frame(at(50, late_stablemate.clone()), never)
            .send,
        []
    );
    assert_eq!(session.on_frame(at(49, late_stablemate), never).send, []);
    assert!(matches!(
        &session.on_connect()[0],
        Outbound::ResyncRequest {
            last_table_seq: 50,
            ..
        }
    ));
}

#[test]
fn the_actor_replays_then_acts_on_the_snapshots_token_once() {
    let mut session = seated();
    let resync = Inbound::Resync(Resync {
        player: true,
        to_table_seq: Some(50),
        replayed: vec![at(
            47,
            Inbound::PlayerAction {
                seat: 0,
                action: "fold".into(),
                amount: 0,
                stack: 1940,
            },
        )],
        snapshot: Some(acting(vec![
            seat(0, "alpha_bot", None),
            seat(2, "SurSvan", Some(false)),
            seat(3, "gamma_bot", Some(false)),
        ])),
    });
    let asked = Cell::new(0);
    let step = session.on_frame(resync.clone(), |spot| {
        asked.set(asked.get() + 1);
        // The replayed fold is kept, so one opponent is left in the hand.
        assert_eq!((spot.opponents, spot.pot, spot.board.len()), (1, 120, 3));
        Action::Call
    });
    assert_eq!(
        step.send,
        [Outbound::Action {
            hand_id: "h-7".into(),
            action: VenueAction::Call,
            client_action_id: "h-7-1".into(),
            turn_token: "tok-9".into(),
        }]
    );
    // The same token is never played twice: the venue would reject it.
    let again = session.on_frame(resync, |_| Action::Call);
    assert_eq!((again.send, asked.get()), (vec![], 1));
}

#[test]
fn a_snapshot_without_a_token_is_not_authority_to_act() {
    let mut session = seated();
    let mut snapshot = acting(vec![seat(2, "SurSvan", Some(false))]);
    if let Some(hero) = &mut snapshot.hero {
        hero.turn_token = None;
    }
    assert_eq!(
        session.on_frame(Inbound::TableState(snapshot), never).send,
        []
    );
}

#[test]
fn a_stablemate_in_a_snapshot_is_caught_by_the_seat_list_guard() {
    let mut session = seated();
    let step = session.on_frame(
        Inbound::TableState(acting(vec![
            seat(2, "SurSvan", Some(false)),
            seat(5, "SVANISM", Some(false)),
        ])),
        never,
    );
    assert!(matches!(
        step.send[0],
        Outbound::Action {
            action: VenueAction::Fold,
            ..
        }
    ));
    assert!(step.send.contains(&Outbound::LeaveTable));
    assert_eq!(
        step.incidents,
        [Incident::SharedHand {
            hand_id: "h-7".into(),
            stablemate: "SVANISM".into()
        }]
    );
}

#[test]
fn already_seated_resyncs_that_table_from_its_start() {
    let mut session = session();
    session.on_connect();
    let step = session.on_frame(
        Inbound::Error {
            code: "already_seated".into(),
            table_id: Some("t-3".into()),
        },
        never,
    );
    assert_eq!(
        step.send,
        [Outbound::ResyncRequest {
            table_id: "t-3".into(),
            last_table_seq: 0
        }]
    );
}

#[test]
fn a_lost_seat_sends_it_back_to_the_lobby() {
    let mut spectator = seated();
    spectator.on_connect();
    let step = spectator.on_frame(
        Inbound::Resync(Resync {
            player: false,
            to_table_seq: None,
            replayed: vec![],
            snapshot: None,
        }),
        never,
    );
    assert!(matches!(step.send[0], Outbound::JoinLobby { .. }));
    let mut gone = seated();
    gone.on_connect();
    let step = gone.on_frame(
        Inbound::Error {
            code: "not_at_table".into(),
            table_id: None,
        },
        never,
    );
    assert!(matches!(step.send[0], Outbound::JoinLobby { .. }));
    assert!(matches!(gone.on_connect()[0], Outbound::JoinLobby { .. }));
}
