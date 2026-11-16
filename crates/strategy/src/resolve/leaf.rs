//! What the end of a street is worth: the leaves of the re-solve.
//!
//! A river ends in a showdown. Earlier, the end is valued over sampled run-outs of the board,
//! and on each run-out one more round of betting is played out by **continuation strategies**
//! (depth-limited solving, Brown, Sandholm and Amos, 2018; Pluribus, 2019): the first seat checks
//! or bets [`BET`] of the pot, and a bet is called or folded to. A continuation says how often a
//! seat bets and calls in each strength bucket. Each seat chooses among [`CHOICES`] of them for
//! each pair it may hold, by regret matching like any other decision: its base, and the base
//! biased to raise, to call and to fold, so neither side can lean on one fixed future.
//!
//! Our base is the baseline model on the next street. The villain's first choice is their action
//! model there, the opponent model beyond the depth limit (ABD, 2025), and the rest are the
//! baseline's. The exploit budget pins that share of their choice to their model; the rest stays
//! free to answer us.

use svanbot_core::rules::Street;

use super::showdown::Showdown;
use crate::action::{ActionModel, BUCKETS, Move, Situation};

/// Continuations a seat chooses among at a street's end.
pub const CHOICES: usize = 4;

/// Reaches per sweep: all of the other seat's, the part that bets, and the part that calls. The
/// sweep is linear, so their choices are summed before it and not after.
const WIDE: usize = 3;

/// The continuation bet, as a share of the pot.
pub const BET: f64 = 0.75;

/// How a seat plays the round after the depth limit: how often it bets when it may, and how
/// often it calls a bet, in each strength bucket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Continuation {
    pub bet: [f64; BUCKETS],
    pub call: [f64; BUCKETS],
}

impl Continuation {
    /// How `model` plays `street`: it bets as often as it does not check, and calls as often as
    /// it does not fold.
    pub fn of(model: &ActionModel, street: Street) -> Self {
        let at = |facing| Situation { street, facing };
        Self {
            bet: std::array::from_fn(|b| 1.0 - model.prob(at(false), b, Move::Check)),
            call: std::array::from_fn(|b| 1.0 - model.prob(at(true), b, Move::Fold)),
        }
    }

    /// This continuation with the odds of a bet and of a call multiplied.
    fn biased(self, bet: f64, call: f64) -> Self {
        let tilt = |p: f64, odds: f64| p * odds / (p * odds + 1.0 - p);
        Self {
            bet: self.bet.map(|p| tilt(p, bet)),
            call: self.call.map(|p| tilt(p, call)),
        }
    }

    /// The choices a seat with this base has: the base, then biased to raise, to call, to fold.
    pub fn portfolio(self) -> [Self; CHOICES] {
        [
            self,
            self.biased(3.0, 1.0),
            self.biased(0.5, 3.0),
            self.biased(0.5, 1.0 / 3.0),
        ]
    }
}

/// The round played past the depth limit.
#[derive(Clone, Debug)]
pub struct Deep {
    /// The continuations each seat chooses among: ours, then the villain's.
    pub portfolio: [[Continuation; CHOICES]; 2],
    /// The seat that acts first after the flop: 0 is us.
    pub first: usize,
    /// Our chips behind at the root. The villain is taken to cover us.
    pub stack: u32,
    /// The share of the villain's choice pinned to their first, their model: the exploit budget.
    pub pin: f64,
}

/// A seat's regrets over its continuations at one leaf, per pair.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    regrets: [Vec<[f64; CHOICES]>; 2],
}

/// Regret matching over the continuations, with `pin` of the weight on the base.
fn pick(regrets: &[f64; CHOICES], pin: f64) -> [f64; CHOICES] {
    let total: f64 = regrets.iter().map(|r| r.max(0.0)).sum();
    let mut chosen = regrets.map(|r| match total > 0.0 {
        true => (1.0 - pin) * r.max(0.0) / total,
        false => (1.0 - pin) / CHOICES as f64,
    });
    chosen[0] += pin;
    chosen
}

/// Run-outs an iteration values a street's end on. The iterations take them in turn, so each
/// run-out is met as often as any other (chance sampling: Zinkevich et al., 2007).
pub const PER_ITERATION: usize = 2;

/// The leaves of one re-solve.
#[derive(Clone, Debug)]
pub struct Leaves {
    /// The run-outs a street's end is valued on.
    ends: Vec<Showdown>,
    /// `None` on the river: nothing is left to play.
    deep: Option<Deep>,
}

impl Leaves {
    pub fn new(ends: Vec<Showdown>, deep: Option<Deep>) -> Self {
        Self { ends, deep }
    }

    /// The run-outs iteration `turn` uses, and for each pair one over the number of them it can
    /// be held on.
    fn sample(&self, turn: u32, pairs: usize) -> (Vec<&Showdown>, Vec<f64>) {
        let count = PER_ITERATION.min(self.ends.len());
        let ends: Vec<&Showdown> = (0..count)
            .map(|k| &self.ends[(turn as usize * count + k) % self.ends.len()])
            .collect();
        let scale = (0..pairs)
            .map(|h| {
                let held = ends.iter().filter(|end| end.live()[h]).count();
                1.0 / held.max(1) as f64
            })
            .collect();
        (ends, scale)
    }

    /// `player`'s counterfactual values, per pair, where a street ends with `pot` in the middle
    /// and `added` put in since the root, against the other seat's reach `theirs`. `positive` is
    /// the discount of a positive regret in iteration `turn`.
    pub fn value(
        &self,
        player: usize,
        pot: f64,
        added: [u32; 2],
        theirs: &[f64],
        choice: &mut Choice,
        (turn, positive): (u32, f64),
    ) -> Vec<f64> {
        let paid = f64::from(added[player]);
        let pairs = theirs.len();
        let (ends, scale) = self.sample(turn, pairs);
        let deep = self.deep.as_ref().and_then(|deep| {
            let bet = (BET * pot).min(f64::from(deep.stack.saturating_sub(added[0])));
            (bet > 0.0).then_some((deep, bet))
        });
        let Some((deep, bet)) = deep else {
            let mut value = vec![0.0; pairs];
            for end in ends {
                let (share, compatible) = end.against(theirs);
                for (h, total) in value.iter_mut().enumerate() {
                    *total += scale[h] * (pot * share[h] - paid * compatible[h]);
                }
            }
            return value;
        };
        for regrets in &mut choice.regrets {
            regrets.resize(pairs, [0.0; CHOICES]);
        }
        let pin = |seat: usize| if seat == 1 { deep.pin } else { 0.0 };
        let other = 1 - player;
        let theirs_choice: Vec<[f64; CHOICES]> = choice.regrets[other]
            .iter()
            .map(|regrets| pick(regrets, pin(other)))
            .collect();
        let mut values = vec![[0.0; CHOICES]; pairs];
        for end in ends {
            let round = Round {
                end,
                first: player == deep.first,
                pot,
                paid,
                bet,
            };
            round.add(
                &deep.portfolio[player],
                &deep.portfolio[other],
                theirs,
                &theirs_choice,
                &scale,
                &mut values,
            );
        }
        let regrets = &mut choice.regrets[player];
        (0..pairs)
            .map(|h| {
                let chosen = pick(&regrets[h], pin(player));
                let value: f64 = (0..CHOICES).map(|i| chosen[i] * values[h][i]).sum();
                for i in 0..CHOICES {
                    let regret = &mut regrets[h][i];
                    *regret *= if *regret > 0.0 { positive } else { 0.5 };
                    *regret += values[h][i] - value;
                }
                value
            })
            .collect()
    }
}

/// The continuation round on one run-out, from one seat's side.
pub struct Round<'a> {
    pub end: &'a Showdown,
    /// Whether the seat valued acts first.
    pub first: bool,
    pub pot: f64,
    /// What the seat has put in since the root.
    pub paid: f64,
    pub bet: f64,
}

impl Round<'_> {
    /// Adds to `values`, per pair and per choice of `mine`, `scale` times the round's value
    /// against `theirs`, each of whose pairs plays its continuations as `theirs_choice` says.
    pub fn add(
        &self,
        mine: &[Continuation; CHOICES],
        other: &[Continuation; CHOICES],
        theirs: &[f64],
        theirs_choice: &[[f64; CHOICES]],
        scale: &[f64],
        values: &mut [[f64; CHOICES]],
    ) {
        let bucket = self.end.bucket();
        let live = self.end.live();
        // Three reaches of theirs: all of it, the part that bets when it may, and the part that
        // calls a bet (after its own check, when it acts first), each summed over their choices.
        let wide: Vec<[f64; WIDE]> = (0..theirs.len())
            .map(|h| {
                let mut row = [0.0; WIDE];
                if live[h] && theirs[h] != 0.0 {
                    let b = usize::from(bucket[h]);
                    for (j, plays) in other.iter().enumerate() {
                        let reach = theirs[h] * theirs_choice[h][j];
                        let calls = match self.first {
                            true => plays.call[b],
                            false => (1.0 - plays.bet[b]) * plays.call[b],
                        };
                        row[0] += reach;
                        row[1] += reach * plays.bet[b];
                        row[2] += reach * calls;
                    }
                }
                row
            })
            .collect();
        let (share, compatible) = self.end.against_wide(&wide);
        let (pot, paid, bet) = (self.pot, self.paid, self.bet);
        let big = pot + 2.0 * bet;
        for h in 0..theirs.len() {
            if !live[h] {
                continue;
            }
            // The share won and the reach met: of all, of their bettors, of their callers.
            let [s_all, s_bet, s_call] = share[h];
            let [c_all, c_bet, c_call] = compatible[h];
            let b = usize::from(bucket[h]);
            for (i, plays) in mine.iter().enumerate() {
                let (bets, calls) = (plays.bet[b], plays.call[b]);
                // A bet called is a showdown for the bigger pot; a fold gives up what was paid.
                let called = |s: f64, c: f64| big * s - (paid + bet) * c;
                let value = if self.first {
                    // We check: they check behind, or bet and we fold or call.
                    (1.0 - bets)
                        * (pot * (s_all - s_bet) - paid * (c_all - c_bet)
                            + (1.0 - calls) * -paid * c_bet
                            + calls * called(s_bet, c_bet))
                        // We bet: they fold or call.
                        + bets * ((pot - paid) * (c_all - c_call) + called(s_call, c_call))
                } else {
                    let (s_check, c_check) = (s_all - s_bet, c_all - c_bet);
                    // They check: we check behind, or bet and they fold or call.
                    (1.0 - bets) * (pot * s_check - paid * c_check)
                        + bets * ((pot - paid) * (c_check - c_call) + called(s_call, c_call))
                        // They bet: we fold or call.
                        + (1.0 - calls) * -paid * c_bet
                        + calls * called(s_bet, c_bet)
                };
                values[h][i] += scale[h] * value;
            }
        }
    }
}
