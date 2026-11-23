//! The reads' sim (V11-25): what playing by an opponent's own read wins.
//!
//! A **warm-up** plays deals against a sparring opponent and builds the challenger's read of them
//! with the code live play uses ([`Reads`]), from the opponent's line as the challenger saw it.
//! The read is then frozen, so the measured deals stay independent of each other and of the order
//! threads play them in. [`reads_on_off`] plays each measured deal twice, by the challenger with
//! reads and without, against a fresh opponent each time, and gives the difference: a paired
//! comparison over the same cards.

use std::cell::RefCell;
use std::ops::Range;

use svanbot_core::rules::Action;
use svanbot_core::spot::{ActKind, Spot};
use svanbot_sim::sparring::{Archetype, Sparring};
use svanbot_strategy::action::Seen;

use crate::paired::{Match, duel_with};
use crate::reads::{Reads, Witness};

/// The name the warm-up gives the sparring opponent (`GLOSSARY.md`: opponent), for the decision to read them by.
pub const OPPONENT: &str = "opponent";

/// The challenger's read of `archetype` after the deals `numbers`, each played from both seats.
pub fn warm_up(
    game: &Match,
    archetype: Archetype,
    numbers: Range<u64>,
    challenger: &impl Fn(&Spot, u64) -> Action,
) -> Reads {
    let opponent = Sparring::new(archetype);
    // The opponent's line at the challenger's latest decision of each hand.
    let lines: RefCell<Vec<Vec<svanbot_core::spot::Act>>> = RefCell::new(Vec::new());
    let watching = |spot: &Spot, seed: u64| {
        if let (Some(line), Some(last)) = (spot.lines.first(), lines.borrow_mut().last_mut()) {
            last.clone_from(line);
        }
        challenger(spot, seed)
    };
    for number in numbers {
        duel_with(
            game,
            number,
            &watching,
            &|spot, seed| opponent.decide(spot, seed, number),
            || lines.borrow_mut().push(Vec::new()),
        );
    }
    let mut reads = Reads::default();
    for line in lines.into_inner() {
        // Heads up, every bet the opponent faced was ours.
        let answer = line
            .iter()
            .find(|act| act.facing && matches!(act.kind, ActKind::Fold | ActKind::Call))
            .map(|act| matches!(act.kind, ActKind::Fold));
        reads.observe(Witness {
            name: OPPONENT.to_owned(),
            actions: line.iter().map(Seen::of).collect(),
            aggressive: line
                .iter()
                .any(|act| matches!(act.kind, ActKind::Bet { .. })),
            vs_us: answer,
            vs_others: None,
        });
    }
    reads
}

/// What `on` won over `off` on each deal of `numbers` against `archetype`, in big blinds a hand.
pub fn reads_on_off(
    game: &Match,
    archetype: Archetype,
    numbers: Range<u64>,
    on: &impl Fn(&Spot, u64) -> Action,
    off: &impl Fn(&Spot, u64) -> Action,
) -> Vec<f64> {
    numbers
        .map(|number| {
            let against = |challenger: &dyn Fn(&Spot, u64) -> Action| {
                let opponent = Sparring::new(archetype);
                duel_with(
                    game,
                    number,
                    &|spot, seed| challenger(spot, seed),
                    &|spot, seed| opponent.decide(spot, seed, number),
                    || {},
                )
            };
            against(on) - against(off)
        })
        .collect()
}
