//! The ring (V11-29): the 6-max form of the paired sim. One deal is played once from every seat
//! of a full table, the challenger moving one seat round each time and the opponents in the
//! rest, so the challenger holds every hand in every position and the luck of the cards cancels
//! the way the heads-up mirror cancels it (`GLOSSARY.md`).
//!
//! What a seat draws its decisions from belongs to the seat, not to who sits in it: every
//! rotation hands the seats the same seeds, so a policy playing itself plays the same hand six
//! times and breaks even on the deal.

use svanbot_core::rng::Rng;
use svanbot_core::rules::Action;
use svanbot_core::spot::Spot;
use svanbot_sim::table::{deal, play_corrected};

use crate::paired::Match;

/// Seats at a full 6-max table: the venue's game.
pub const SIX_MAX: usize = 6;

/// Sets the ring's seeds apart from the deals' and from the heads-up mirror's.
const DECISIONS: u64 = 0x5eed_0fde_c151_0606;

/// Sets the ring's run-out seeds apart from its decisions'.
const RUN_OUTS: u64 = 0x5eed_0f00_7500_0606;

/// Plays deal number `number` at `seats` seats, once with the challenger in each, and returns what
/// the challenger won in big blinds a hand. The button stays on seat 0, so a rotation moves the
/// challenger, not the cards. `opponent` is asked for the action of the seat it is given, so a
/// stateful opponent can keep one state a seat.
pub fn ring(
    game: &Match,
    seats: usize,
    number: u64,
    challenger: &impl Fn(&Spot, u64) -> Action,
    opponent: &impl Fn(usize, &Spot, u64) -> Action,
) -> f64 {
    ring_with(game, seats, number, challenger, opponent, |_| {})
}

/// [`ring`], with `reset` run before each rotation and handed the challenger's seat in it, so a
/// stateful opponent meets every rotation as it was when the deal began, and knows who to watch.
pub fn ring_with(
    game: &Match,
    seats: usize,
    number: u64,
    challenger: &impl Fn(&Spot, u64) -> Action,
    opponent: &impl Fn(usize, &Spot, u64) -> Action,
    reset: impl Fn(usize),
) -> f64 {
    let cards = deal(seats, &mut Rng::stream(game.seed, number));
    let block = number * seats as u64;
    // Every rotation pays an all-in over the same run-outs.
    let run_outs = Rng::stream(game.seed ^ RUN_OUTS, number).next_u64();
    let stacks = vec![game.stack; seats];
    let mut chips = 0.0;
    for us in 0..seats {
        reset(us);
        let mut seeds: Vec<Rng> = (0..seats)
            .map(|seat| Rng::stream(game.seed ^ DECISIONS, block + seat as u64))
            .collect();
        let won = play_corrected(&cards, &stacks, 0, game.blinds, run_outs, |at, spot| {
            let seed = seeds[at].next_u64();
            if at == us {
                challenger(spot, seed)
            } else {
                opponent(at, spot, seed)
            }
        });
        chips += won[us];
    }
    chips / (seats as f64 * f64::from(game.blinds.big))
}
