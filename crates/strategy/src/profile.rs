//! Compiled Rust rules that can decide a spot before the re-solve or pricer.

use svanbot_core::card::{CardSet, Rank, Suit};
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Chips, Legal, Street};
use svanbot_core::spot::Spot;

use crate::preflop::{self, Position};

/// The profile used in live play and the Arena.
pub const ACTIVE: Profile<'static> = Profile::new(&[]);

/// One rule. The action is evaluated only when its condition matches.
pub struct Rule {
    pub when: fn(&Facts<'_>) -> bool,
    pub then: fn(&Facts<'_>) -> Action,
}

/// A metric must have this many hands with an observed opportunity before a rule can use it.
pub const MIN_EVIDENCE_HANDS: usize = 30;

/// An observed frequency, with the independent hands behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rate {
    hands: usize,
    hits: usize,
    opportunities: usize,
}

impl Rate {
    /// Adds the outcome of one hand. A hand with no opportunity supplies no evidence.
    pub fn observe(&mut self, hits: usize, opportunities: usize) {
        self.hands += usize::from(opportunities > 0);
        self.hits += hits;
        self.opportunities += opportunities;
    }

    pub fn hands(self) -> usize {
        self.hands
    }

    pub fn value(self) -> Option<f64> {
        (self.hands >= MIN_EVIDENCE_HANDS).then(|| self.hits as f64 / self.opportunities as f64)
    }
}

/// What a bot has observed of one opponent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpponentStats {
    /// Voluntarily put chips in preflop, per hand with an observed preflop action.
    pub vpip: Rate,
    /// Raised preflop, per hand with an observed preflop action.
    pub pfr: Rate,
    /// Folds per postflop decision that faced a bet.
    pub fold_to_bet: Rate,
    /// Bets and raises as a fraction of postflop actions other than checks.
    pub aggression: Rate,
}

/// The profile's stats about the opponent in this seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatedStats {
    pub seat: usize,
    pub stats: OpponentStats,
}

/// A typed value a profile computes from the current spot and its qualified evidence.
pub struct Variable<T>(pub fn(&Facts<'_>) -> T);

impl<T> Variable<T> {
    pub fn get(&self, facts: &Facts<'_>) -> T {
        (self.0)(facts)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardTexture {
    pub paired: bool,
    /// The largest number of cards of one suit.
    pub most_of_suit: usize,
    /// The largest number of distinct board ranks in one straight, ace low or high.
    pub straight_ranks: u32,
}

/// The spot's facts as a rule sees them.
pub struct Facts<'a> {
    spot: &'a Spot,
    seed: u64,
    opponents: &'a [SeatedStats],
}

impl Facts<'_> {
    pub fn street(&self) -> Street {
        Street::of(self.spot.board)
    }

    pub fn board(&self) -> CardSet {
        self.spot.board
    }

    pub fn hole(&self) -> CardSet {
        self.spot.hole
    }

    pub fn board_texture(&self) -> BoardTexture {
        let mut suits = [0; Suit::ALL.len()];
        let mut ranks = 0u16;
        for card in self.board().iter() {
            suits[card.suit() as usize] += 1;
            ranks |= 1 << (card.rank() as usize + 1);
        }
        let paired = (ranks.count_ones() as usize) < self.board().len();
        // Give the ace a second bit below the deuce for the wheel.
        ranks |= ranks >> (Rank::Ace as usize + 1);
        BoardTexture {
            paired,
            most_of_suit: suits.into_iter().max().unwrap_or(0),
            straight_ranks: (0..=9)
                .map(|start| ((ranks >> start) & 31).count_ones())
                .max()
                .unwrap_or(0),
        }
    }

    /// A draw in [0, 1) from a rule-selected stream. Other draws and rules do not advance it.
    pub fn random(&self, stream: u64) -> f64 {
        let bits = Rng::stream(self.seed, stream).next_u64() >> 11;
        bits as f64 / (1u64 << 53) as f64
    }

    pub fn starting_hand_class(&self) -> Option<usize> {
        (self.spot.hole.len() == 2).then(|| preflop::class_of(self.spot.hole))
    }

    pub fn position(&self) -> Option<Position> {
        (self.street() == Street::Preflop).then_some(Position {
            behind: self.players_behind()?,
            in_position: self.in_position()?,
        })
    }

    pub fn players_behind(&self) -> Option<usize> {
        self.spot.players_behind
    }

    pub fn in_position(&self) -> Option<bool> {
        self.players_behind().map(|_| self.spot.in_position)
    }

    pub fn pot(&self) -> Chips {
        self.spot.pot
    }

    pub fn committed(&self) -> Chips {
        self.spot.committed
    }

    pub fn opponent_count(&self) -> usize {
        self.spot.opponents
    }

    pub fn pot_bb(&self) -> Option<f64> {
        self.in_big_blinds(self.spot.pot)
    }

    pub fn stack_bb(&self) -> Option<f64> {
        self.in_big_blinds(self.spot.stack)
    }

    pub fn legal(&self) -> Legal {
        self.spot.legal
    }

    pub fn opponent(&self, seat: usize) -> Option<OpponentStats> {
        self.opponents
            .iter()
            .find(|read| read.seat == seat)
            .map(|read| read.stats)
    }

    pub fn opponents(&self) -> &[SeatedStats] {
        self.opponents
    }

    fn in_big_blinds(&self, chips: Chips) -> Option<f64> {
        let blind = self.spot.big_blind;
        (blind > 0).then(|| chips as f64 / blind as f64)
    }
}

/// Rules run in order. The first match decides.
#[derive(Clone, Copy)]
pub struct Profile<'a> {
    rules: &'a [Rule],
    opponents: &'a [SeatedStats],
}

impl<'a> Profile<'a> {
    pub const fn new(rules: &'a [Rule]) -> Self {
        Self {
            rules,
            opponents: &[],
        }
    }

    pub fn is_empty(self) -> bool {
        self.rules.is_empty()
    }

    pub fn with_opponents(self, opponents: &'a [SeatedStats]) -> Self {
        Self { opponents, ..self }
    }

    /// The first rule's action fitted to the legal choices, if a rule matches.
    pub fn action(self, spot: &Spot, seed: u64) -> Option<Action> {
        let facts = Facts {
            spot,
            seed,
            opponents: self.opponents,
        };
        self.rules
            .iter()
            .find(|rule| (rule.when)(&facts))
            .map(|rule| spot.legal.nearest((rule.then)(&facts)))
    }
}
