use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use svanbot_core::rules::Action;
use svanbot_core::spot::{Act, ActKind, Spot};
use svanbot_sim::table::Record;
use svanbot_strategy::action::Seen;

use crate::paired::Challenger;
use crate::profile::Observation;
use crate::read_sim::OPPONENT;
use crate::reads::{Reads, Witness};

/// Heads up the same opponent follows the challenger through the mirror. At a full table each
/// sparring seat keeps its own history when the challenger rotates through the other seats.
pub fn opponent_name(seats: usize, seat: usize) -> String {
    if seats == 2 {
        OPPONENT.to_owned()
    } else {
        format!("seat {seat}")
    }
}

/// The challenger's reads for one bout. All rotations see the read as it stood before the deal.
pub struct BoutReader<F> {
    decide: F,
    reads: RefCell<Reads>,
    pending: RefCell<Vec<Witness>>,
    deal: Cell<usize>,
}

impl<F: Fn(&Spot, u64, &Reads) -> Action> BoutReader<F> {
    /// Freeze the observations before measuring fresh deals.
    pub fn into_reads(self) -> Reads {
        self.reads.into_inner()
    }

    pub fn new(decide: F) -> Self {
        Self {
            decide,
            reads: RefCell::default(),
            pending: RefCell::default(),
            deal: Cell::default(),
        }
    }
}

impl<F: Fn(&Spot, u64, &Reads) -> Action> Challenger for BoutReader<F> {
    fn decide(&self, spot: &Spot, seed: u64) -> Action {
        (self.decide)(spot, seed, &self.reads.borrow())
    }

    fn finished_hand(&self, us: usize, lines: &[Vec<Act>], actions: &[Record]) {
        let mut pending = self.pending.borrow_mut();
        for (seat, line) in lines.iter().enumerate().filter(|(seat, _)| *seat != us) {
            let mut witness = Witness {
                name: opponent_name(lines.len(), seat),
                actions: line.iter().map(Seen::of).collect(),
                aggressive: line
                    .iter()
                    .any(|act| matches!(act.kind, ActKind::Bet { .. })),
                profile: Some(Observation::of(line)),
                vs_us: None,
                vs_others: None,
            };
            let mut bettor = None;
            for act in actions {
                if let Action::RaiseTo(_) = act.action {
                    bettor = Some((act.street, act.seat));
                } else if act.seat == seat
                    && act.owed_before > 0
                    && matches!(act.action, Action::Fold | Action::Call)
                {
                    let response = if bettor == Some((act.street, us)) {
                        &mut witness.vs_us
                    } else {
                        &mut witness.vs_others
                    };
                    response.get_or_insert(act.action == Action::Fold);
                }
            }
            pending.push(witness);
        }
    }

    fn finished_deal(&self) {
        let mut reads = self.reads.borrow_mut();
        let mut pending = self.pending.borrow_mut();
        // Rotations reuse hole cards. Each opponent contributes one independent hand to the
        // profile per deal. Rotate which perspective supplies it to cover every seat.
        let mut by_name = BTreeMap::<&str, Vec<usize>>::new();
        for (index, witness) in pending.iter().enumerate() {
            by_name.entry(&witness.name).or_default().push(index);
        }
        let deal = self.deal.get();
        let mut kept = vec![false; pending.len()];
        for indices in by_name.values() {
            kept[indices[deal % indices.len()]] = true;
        }
        for (mut witness, keep) in pending.drain(..).zip(kept) {
            if !keep {
                witness.profile = None;
            }
            reads.observe(witness);
        }
        self.deal.set(deal + 1);
    }
}
