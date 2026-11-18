//! The sparring archetypes (`GLOSSARY.md`): scripted opponents the sim plays against, each
//! standing in for a kind of real opponent. They decide on one number, their equity against any
//! two cards, so they are cheap, and none of them is good: the Arena measures how much a policy
//! wins from each, and whether a policy tuned against one loses to another.
//!
//! - The **calling station** calls any bet and bets only its best hands.
//! - The **nit** folds all but strong hands to a bet, and raises those.
//! - The **maniac** bets and raises most of the time with anything.
//! - The **switching** archetype is a nit, then a maniac, turn about, every [`PERIOD`] hands.
//! - The **learning** archetype plays a plain solid game and adapts to how often we bet: it
//!   calls lighter against a bettor, and folds more and steals more against a passive player.

use std::cell::Cell;

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::rng::Rng;
use svanbot_core::rules::Action;
use svanbot_core::spot::{ActKind, Spot};

use crate::equity::sampled;

/// A kind of sparring opponent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Archetype {
    Station,
    Nit,
    Maniac,
    Switching,
    Learning,
}

impl Archetype {
    pub const ALL: [Self; 5] = [
        Self::Station,
        Self::Nit,
        Self::Maniac,
        Self::Switching,
        Self::Learning,
    ];

    /// The archetype's name on a scoreboard.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Station => "calling station",
            Self::Nit => "nit",
            Self::Maniac => "maniac",
            Self::Switching => "switching",
            Self::Learning => "learning",
        }
    }
}

/// Hands the switching archetype plays one way before it plays the other. Short enough that the
/// Arena's smallest run, two bouts of 50 deals, plays both halves: at 200 a 100-deal run would
/// never reach the second one, and the switching row would be the nit row under another name.
pub const PERIOD: u64 = 20;

/// Deals sampled for an archetype's equity.
const SAMPLES: u64 = 256;

/// One sparring opponent. The learning archetype remembers what it has seen of us, so one
/// opponent is one table's worth of hands, played in order.
#[derive(Clone, Debug)]
pub struct Sparring {
    archetype: Archetype,
    /// Our last actions it has looked at, and how many were bets or raises.
    seen: Cell<(u32, u32)>,
}

impl Sparring {
    pub fn new(archetype: Archetype) -> Self {
        Self {
            archetype,
            seen: Cell::new((0, 0)),
        }
    }

    /// Forgets what it has seen of us, as a fresh opponent would. Both hands of a paired deal
    /// face the same opponent, so the deal still cancels the luck of the cards.
    pub fn reset(&self) {
        self.seen.set((0, 0));
    }

    /// The opponent's action at `spot`, in hand number `hand` of its table, drawing from `seed`.
    pub fn decide(&self, spot: &Spot, seed: u64, hand: u64) -> Action {
        let equity = sampled(
            spot.hole,
            &[&any_two(spot.hole | spot.board)],
            spot.board,
            SAMPLES,
            seed,
        );
        let roll = (Rng::stream(seed, 1).next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        match self.archetype {
            Archetype::Station => station(spot, equity),
            Archetype::Nit => nit(spot, equity),
            Archetype::Maniac => maniac(spot, equity, roll),
            Archetype::Switching if (hand / PERIOD).is_multiple_of(2) => nit(spot, equity),
            Archetype::Switching => maniac(spot, equity, roll),
            Archetype::Learning => self.learning(spot, equity, roll),
        }
    }

    fn learning(&self, spot: &Spot, equity: f64, roll: f64) -> Action {
        let (mut acts, mut bets) = self.seen.get();
        if let Some(last) = spot.lines.first().and_then(|line| line.last()) {
            acts += 1;
            bets += u32::from(matches!(last.kind, ActKind::Bet { .. }));
            self.seen.set((acts, bets));
        }
        // How often we bet, shrunk toward three in ten until it has seen enough.
        let aggression = (f64::from(bets) + 3.0) / (f64::from(acts) + 10.0);
        let (lighter, steal) = if aggression > 0.45 {
            (0.2, 0.0)
        } else if aggression < 0.2 {
            (-0.1, 0.4)
        } else {
            (0.0, 0.1)
        };
        match spot.legal.call {
            Some(call) => {
                let price = f64::from(call) / f64::from(spot.pot + call);
                if equity > 0.8 {
                    bet(spot, 1.0)
                } else if equity >= price + 0.12 - lighter {
                    Action::Call
                } else {
                    Action::Fold
                }
            }
            None if equity > 0.6 || roll < steal => bet(spot, 0.75),
            None => Action::Check,
        }
    }
}

fn station(spot: &Spot, equity: f64) -> Action {
    match spot.legal.call {
        Some(_) => Action::Call,
        None if equity > 0.92 => bet(spot, 0.5),
        None => Action::Check,
    }
}

fn nit(spot: &Spot, equity: f64) -> Action {
    match spot.legal.call {
        _ if equity > 0.92 => bet(spot, 1.0),
        Some(_) => Action::Fold,
        None => Action::Check,
    }
}

fn maniac(spot: &Spot, equity: f64, roll: f64) -> Action {
    let stacked = spot
        .legal
        .call
        .is_some_and(|call| call > spot.stack / 2 && equity < 0.35);
    if stacked {
        Action::Fold
    } else if roll < 0.65 {
        bet(spot, 1.0)
    } else if spot.legal.call.is_some() {
        Action::Call
    } else {
        Action::Check
    }
}

/// A bet or raise adding `share` of the pot, or the nearest legal action when none is offered.
fn bet(spot: &Spot, share: f64) -> Action {
    match (spot.legal.raise, spot.legal.call) {
        (Some(range), call) => {
            let to = spot.committed + call.unwrap_or(0) + (f64::from(spot.pot) * share) as u32;
            Action::RaiseTo(to.clamp(range.min, range.max))
        }
        (None, Some(_)) => Action::Call,
        (None, None) => Action::Check,
    }
}

/// Every pair of hole cards that misses `dead`, each as likely as any other.
fn any_two(dead: CardSet) -> Vec<(CardSet, f64)> {
    let deck: Vec<Card> = Rank::ALL
        .into_iter()
        .flat_map(|rank| Suit::ALL.map(|suit| Card::new(rank, suit)))
        .filter(|&card| !dead.contains(card))
        .collect();
    let mut range = Vec::with_capacity(deck.len() * (deck.len() - 1) / 2);
    for (i, &first) in deck.iter().enumerate() {
        for &second in &deck[i + 1..] {
            range.push(([first, second].into_iter().collect(), 1.0));
        }
    }
    range
}
