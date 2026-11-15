//! The fair-play guard at the seat-list seam (V11-01). A stablemate in the joined seat list, in a
//! snapshot, or sitting down mid-session makes the bot leave once, and a stranger never does. After
//! the leave the bot goes back to the lobby (<https://docs.openpoker.ai/llms-full.txt>, rev
//! 2026-09-02, "table_closed", "Error Codes").

use svanbot_core::rules::{Action, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_runtime::session::{Incident, Session, Step};
use svanbot_runtime::wire::{Hero, Inbound, Offer, Outbound, Player, Seat, Snapshot, Turn};

const ROSTER: [&str; 5] = ["SvanBotV10", "Svanar", "Svanism", "SuraGunnar", "SurSvan"];

fn session() -> Session {
    Session::new("SurSvan", ROSTER.map(String::from))
}

fn never(_: &Spot) -> Action {
    unreachable!("no decision is due")
}

fn player(seat: usize, name: &str) -> Player {
    Player {
        seat,
        name: name.into(),
        stack: 2000,
    }
}

fn joined(players: Vec<Player>) -> Inbound {
    Inbound::TableJoined {
        table_id: "t-1".into(),
        seat: 2,
        players,
    }
}

fn snapshot(players: Vec<Player>) -> Inbound {
    Inbound::TableState(Snapshot {
        hand_id: None,
        pot: 0,
        board: vec![],
        seats: players
            .into_iter()
            .map(|player| Seat {
                player,
                folded: None,
            })
            .collect(),
        hero: Some(Hero {
            seat: 2,
            hole: vec![],
            offer: None,
            turn_token: None,
        }),
    })
}

fn turn(players: Vec<Player>) -> Inbound {
    Inbound::YourTurn(Turn {
        hand_id: Some("h-1".into()),
        turn_token: "tok-1".into(),
        offer: Offer {
            fold: true,
            check: false,
            call: Some(20),
            raise: Some(RaiseRange { min: 40, max: 2000 }),
            all_in: None,
        },
        pot: 30,
        board: vec![],
        players,
    })
}

fn leaves(step: &Step) -> usize {
    step.send
        .iter()
        .filter(|message| **message == Outbound::LeaveTable)
        .count()
}

/// Feeds every frame and adds up what the session sent and raised.
fn run(session: &mut Session, frames: Vec<Inbound>) -> Step {
    let mut all = Step::default();
    for frame in frames {
        let step = session.on_frame(frame, |_| Action::Call);
        all.send.extend(step.send);
        all.incidents.extend(step.incidents);
    }
    all
}

fn shared_table(stablemate: &str) -> Incident {
    Incident::SharedTable {
        table_id: "t-1".into(),
        stablemate: stablemate.into(),
    }
}

#[test]
fn a_stablemate_in_the_joined_seat_list_makes_it_leave_once() {
    let mut session = session();
    let seated = vec![player(0, "Svanism"), player(2, "SurSvan")];
    let all = run(
        &mut session,
        vec![
            joined(seated.clone()),
            Inbound::PlayerJoined(player(4, "alpha_bot")),
            snapshot(seated),
        ],
    );
    assert_eq!(leaves(&all), 1, "{all:?}");
    assert_eq!(all.incidents, [shared_table("Svanism")]);
}

#[test]
fn a_stablemate_in_a_snapshot_makes_it_leave_once() {
    let mut session = session();
    let seated = vec![player(2, "SurSvan"), player(5, "SVANAR")];
    let all = run(
        &mut session,
        vec![snapshot(seated.clone()), snapshot(seated)],
    );
    assert_eq!(leaves(&all), 1, "{all:?}");
    assert_eq!(all.incidents.len(), 1);
}

#[test]
fn a_stablemate_who_sits_down_mid_session_makes_it_leave_once() {
    let mut session = session();
    let all = run(
        &mut session,
        vec![
            joined(vec![player(0, "alpha_bot"), player(2, "SurSvan")]),
            Inbound::PlayerJoined(player(3, "suragunnar")),
            Inbound::PlayerJoined(player(4, "beta_bot")),
        ],
    );
    assert_eq!(leaves(&all), 1, "{all:?}");
    assert_eq!(all.incidents, [shared_table("suragunnar")]);
}

#[test]
fn strangers_never_trigger_it_even_with_names_like_ours() {
    let mut session = session();
    let lookalikes = [
        "SvanBot",
        "SvanBot_yw7r8y",
        "BotSvana",
        "ToBSvana",
        "Svanar2",
        "Sur",
    ];
    let mut frames = vec![joined(vec![player(2, "SurSvan")])];
    frames.extend(
        lookalikes
            .iter()
            .enumerate()
            .map(|(i, name)| Inbound::PlayerJoined(player(i, name))),
    );
    frames.push(turn(vec![player(0, "SvanBot"), player(2, "SurSvan")]));
    let all = run(&mut session, frames);
    assert_eq!(leaves(&all), 0, "{all:?}");
    assert_eq!(all.incidents, []);
}

#[test]
fn a_turn_while_leaving_is_folded_as_a_shared_hand_without_a_second_leave() {
    let mut session = session();
    session.on_frame(
        joined(vec![player(0, "Svanism"), player(2, "SurSvan")]),
        never,
    );
    let step = session.on_frame(
        turn(vec![player(0, "Svanism"), player(2, "SurSvan")]),
        never,
    );
    assert_eq!(leaves(&step), 0, "{step:?}");
    assert_eq!(
        step.incidents,
        [Incident::SharedHand {
            hand_id: "h-1".into(),
            stablemate: "Svanism".into()
        }]
    );
}

#[test]
fn once_its_own_seat_is_left_it_forgets_the_table_and_goes_back_to_the_lobby() {
    let mut session = session();
    session.on_frame(
        joined(vec![player(0, "Svanism"), player(2, "SurSvan")]),
        never,
    );
    // Someone else leaving is not our leave.
    assert_eq!(
        session
            .on_frame(Inbound::PlayerLeft { seat: 0 }, never)
            .send,
        []
    );
    let step = session.on_frame(Inbound::PlayerLeft { seat: 2 }, never);
    assert_eq!(step.send, [Outbound::JoinLobby { buy_in: 2000 }]);
    assert!(matches!(
        session.on_connect()[0],
        Outbound::JoinLobby { .. }
    ));
}

#[test]
fn a_table_closed_before_its_own_leave_gets_one_final_leave_then_a_clean_exit() {
    let mut session = session();
    session.on_frame(
        joined(vec![player(0, "Svanar"), player(2, "SurSvan")]),
        never,
    );
    let closed = session.on_frame(Inbound::TableClosed, never);
    assert_eq!(closed.send, [Outbound::LeaveTable]);
    let not_at_table = Inbound::Error {
        code: "not_at_table".into(),
        table_id: None,
    };
    assert_eq!(
        session.on_frame(not_at_table, never).send,
        [Outbound::JoinLobby { buy_in: 2000 }]
    );
}
