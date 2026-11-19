//! The action model (`GLOSSARY.md`): how likely an opponent is to fold, check, call or bet each
//! size at a spot, given their hole cards. A range follows from it by Bayes' rule.
//!
//! It is a proper probability model: at every situation and strength bucket, the probabilities of
//! the moves open there sum to 1. That is what lets `learn` fit it on every hand, not only on the
//! ones that reach a showdown (`docs/research/2026-11/02-range-calibration.md`).
//!
//! - A **situation** is the street and whether the player faces a bet.
//! - A **bucket** is where the player's hole cards rank among every live pair on the board at that
//!   moment, cut into [`BUCKETS`] equal shares, 0 the weakest. Preflop the cards rank by a fixed
//!   score of their own, and after the flop by the strength of the best five cards.
//! - A **move** is a fold, check, call, or a bet or raise sized by the chips it adds to the pot.

use std::collections::BTreeMap;

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::eval::{Strength, strength};
use svanbot_core::rules::Street;
use svanbot_core::spot::{Act, ActKind};

/// Strength buckets per situation.
pub const BUCKETS: usize = 8;

/// A move as the model counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Move {
    Fold,
    Check,
    Call,
    /// A bet or raise adding less than half the pot.
    Small,
    /// One adding half the pot up to the whole pot.
    Medium,
    /// One adding the whole pot or more, all-ins included.
    Large,
}

impl Move {
    pub const ALL: [Self; 6] = [
        Self::Fold,
        Self::Check,
        Self::Call,
        Self::Small,
        Self::Medium,
        Self::Large,
    ];

    const fn index(self) -> usize {
        self as usize
    }

    /// The size of a bet or raise that adds `added` chips to a pot of `pot` before it.
    pub fn bet(added: u32, pot: u32) -> Self {
        let share = f64::from(added) / f64::from(pot.max(1));
        if share < 0.5 {
            Self::Small
        } else if share < 1.0 {
            Self::Medium
        } else {
            Self::Large
        }
    }
}

/// Where a player acts: the street, and whether a bet faces them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Situation {
    pub street: Street,
    pub facing: bool,
}

const STREETS: [Street; 4] = [Street::Preflop, Street::Flop, Street::Turn, Street::River];
const FACING: [Move; 5] = [
    Move::Fold,
    Move::Call,
    Move::Small,
    Move::Medium,
    Move::Large,
];
const OPEN: [Move; 4] = [Move::Check, Move::Small, Move::Medium, Move::Large];

impl Situation {
    /// How many situations there are.
    pub const COUNT: usize = 8;

    /// Every situation.
    pub fn all() -> impl Iterator<Item = Self> {
        STREETS
            .into_iter()
            .flat_map(|street| [false, true].map(|facing| Self { street, facing }))
    }

    fn index(self) -> usize {
        self.street as usize * 2 + usize::from(self.facing)
    }

    /// The moves open here: a fold and no check when facing a bet, a check and no fold when not.
    pub fn moves(self) -> &'static [Move] {
        if self.facing { &FACING } else { &OPEN }
    }

    /// `mv` as the model counts it here. A fold with a free check costs nothing more than the
    /// check, so it counts as one; a check facing a bet cannot happen, and counts as the call.
    pub fn counted(self, mv: Move) -> Move {
        match (self.facing, mv) {
            (false, Move::Fold | Move::Call) => Move::Check,
            (true, Move::Check) => Move::Call,
            _ => mv,
        }
    }
}

/// One action an opponent was seen to take: the board then, where they stood, and what they did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    pub board: CardSet,
    pub situation: Situation,
    pub mv: Move,
}

impl Seen {
    /// An action the table saw, as the model counts it.
    pub fn of(act: &Act) -> Self {
        let mv = match act.kind {
            ActKind::Fold => Move::Fold,
            ActKind::Check => Move::Check,
            ActKind::Call => Move::Call,
            ActKind::Bet { added, pot } => Move::bet(added, pot),
        };
        let situation = Situation {
            street: act.street,
            facing: act.facing,
        };
        Self {
            board: act.board,
            situation,
            mv: situation.counted(mv),
        }
    }
}

/// Expected move counts per situation and bucket, which a fit adds up.
#[derive(Clone, Debug, PartialEq)]
pub struct Counts(Vec<[f64; 6]>);

impl Default for Counts {
    fn default() -> Self {
        Self(vec![[0.0; 6]; Situation::COUNT * BUCKETS])
    }
}

impl Counts {
    /// Counts `weight` more of `mv` at `situation` and `bucket`.
    pub fn add(&mut self, situation: Situation, bucket: usize, mv: Move, weight: f64) {
        let mv = situation.counted(mv);
        self.0[situation.index() * BUCKETS + bucket][mv.index()] += weight;
    }
}

/// The first line of a model written as text.
const TEXT_HEADER: &str = "svanbot action model 1";

/// The action model: a probability for each move open at each situation and bucket.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionModel {
    probs: Vec<[f64; 6]>,
}

impl ActionModel {
    /// Every open move equally likely, whatever the cards. A range stays uniform under it: the
    /// incumbent any fit must beat.
    pub fn uniform() -> Self {
        Self::from_counts(&Counts::default(), 1.0)
    }

    /// The baseline: how a sound opponent is taken to play until a fit on real hands passes the
    /// gate. The stronger the bucket, the more it bets and raises, the bigger it sizes, and the
    /// less it folds; the weakest bucket bluffs a little. It is set by hand, so it claims little:
    /// enough that a bet or a raise narrows a range to strong hands and some bluffs.
    pub fn baseline() -> Self {
        let mut counts = Counts::default();
        for situation in Situation::all() {
            for bucket in 0..BUCKETS {
                let x = bucket as f64 / (BUCKETS - 1) as f64;
                let bluff = if bucket == 0 { 1.0 } else { 0.0 };
                // How the bets split over small, medium and large: bigger with strength.
                let sizes = [0.5 - 0.3 * x, 0.35, 0.15 + 0.3 * x];
                let bets = [Move::Small, Move::Medium, Move::Large];
                if situation.facing {
                    let raise = 0.02 + 0.33 * x.powi(4) + 0.04 * bluff;
                    let fold = (1.0 - raise) * 0.9 * (1.0 - x).powf(1.5);
                    counts.add(situation, bucket, Move::Fold, fold);
                    counts.add(situation, bucket, Move::Call, 1.0 - raise - fold);
                    for (mv, share) in bets.into_iter().zip(sizes) {
                        counts.add(situation, bucket, mv, raise * share);
                    }
                } else {
                    let bet = 0.06 + 0.64 * x * x + 0.12 * bluff;
                    counts.add(situation, bucket, Move::Check, 1.0 - bet);
                    for (mv, share) in bets.into_iter().zip(sizes) {
                        counts.add(situation, bucket, mv, bet * share);
                    }
                }
            }
        }
        Self::from_counts(&counts, 0.0)
    }

    /// The model the counts give, each open move's count raised by `alpha`, a Dirichlet prior,
    /// so no open move has probability 0.
    pub fn from_counts(counts: &Counts, alpha: f64) -> Self {
        let mut probs = vec![[0.0; 6]; Situation::COUNT * BUCKETS];
        for situation in Situation::all() {
            for bucket in 0..BUCKETS {
                let at = situation.index() * BUCKETS + bucket;
                let open = situation.moves();
                let total: f64 = open.iter().map(|mv| counts.0[at][mv.index()] + alpha).sum();
                for mv in open {
                    probs[at][mv.index()] = (counts.0[at][mv.index()] + alpha) / total;
                }
            }
        }
        Self { probs }
    }

    /// This model with each move's probability at each situation multiplied by `tilt`, then
    /// normalised again over the open moves. A tilt of 1 everywhere leaves it as it is.
    pub fn tilted(&self, tilt: impl Fn(Situation, Move) -> f64) -> Self {
        let mut probs = self.probs.clone();
        for situation in Situation::all() {
            let open = situation.moves();
            let factors: Vec<f64> = open
                .iter()
                .map(|&mv| tilt(situation, mv).max(0.0))
                .collect();
            for bucket in 0..BUCKETS {
                let row = &mut probs[situation.index() * BUCKETS + bucket];
                let total: f64 = open
                    .iter()
                    .zip(&factors)
                    .map(|(mv, factor)| row[mv.index()] * factor)
                    .sum();
                if total <= 0.0 {
                    continue;
                }
                for (mv, factor) in open.iter().zip(&factors) {
                    row[mv.index()] = row[mv.index()] * factor / total;
                }
            }
        }
        Self { probs }
    }

    /// The model as text: a first line naming the format, then one line of six numbers for each
    /// situation and bucket. Every number reads back to the same bits.
    pub fn to_text(&self) -> String {
        let mut text = String::from(TEXT_HEADER);
        for row in &self.probs {
            text.push('\n');
            let numbers: Vec<String> = row.iter().map(|p| format!("{p:?}")).collect();
            text.push_str(&numbers.join(" "));
        }
        text.push('\n');
        text
    }

    /// The model `text` holds, or `None` unless it is whole: the header, every row, and in
    /// every row a distribution over the moves open there.
    pub fn from_text(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != TEXT_HEADER {
            return None;
        }
        let mut probs = Vec::with_capacity(Situation::COUNT * BUCKETS);
        for line in lines {
            let numbers: Vec<f64> = line
                .split_whitespace()
                .map(|number| number.parse().ok())
                .collect::<Option<_>>()?;
            probs.push(<[f64; 6]>::try_from(numbers).ok()?);
        }
        if probs.len() != Situation::COUNT * BUCKETS {
            return None;
        }
        for situation in Situation::all() {
            for bucket in 0..BUCKETS {
                let row = &probs[situation.index() * BUCKETS + bucket];
                let open: f64 = situation.moves().iter().map(|mv| row[mv.index()]).sum();
                let all: f64 = row.iter().sum();
                let sound = row.iter().all(|p| (0.0..=1.0).contains(p));
                if !sound || (open - 1.0).abs() > 1e-9 || (all - 1.0).abs() > 1e-9 {
                    return None;
                }
            }
        }
        Some(Self { probs })
    }

    /// The chance of `mv` at `situation` from a player whose bucket is not known: the mean over
    /// the buckets, which are equal shares of the pairs.
    pub fn marginal(&self, situation: Situation, mv: Move) -> f64 {
        (0..BUCKETS)
            .map(|bucket| self.prob(situation, bucket, mv))
            .sum::<f64>()
            / BUCKETS as f64
    }

    /// The probability of `mv` at `situation` from a player in `bucket`.
    pub fn prob(&self, situation: Situation, bucket: usize, mv: Move) -> f64 {
        self.probs[situation.index() * BUCKETS + bucket][situation.counted(mv).index()]
    }
}

/// Every pair of hole cards that misses `board` and `dead`, with its bucket on `board`, in the
/// order of [`combos`].
pub fn buckets(board: CardSet, dead: CardSet) -> Vec<(CardSet, usize)> {
    let live = combos(board | dead);
    let mut scored: Vec<(Score, usize)> = live
        .iter()
        .enumerate()
        .map(|(at, &combo)| (score(combo, board), at))
        .collect();
    scored.sort_unstable();
    let total = scored.len();
    let mut bucket = vec![0; total];
    let mut start = 0;
    while start < total {
        let end = start + scored[start..].partition_point(|&(s, _)| s == scored[start].0);
        // Equal scores share a bucket, from the middle of their run.
        let share = (start + end) as f64 / 2.0 / total as f64;
        let at = ((share * BUCKETS as f64) as usize).min(BUCKETS - 1);
        for &(_, index) in &scored[start..end] {
            bucket[index] = at;
        }
        start = end;
    }
    live.into_iter().zip(bucket).collect()
}

/// Every pair of hole cards that misses `dead`, lowest cards first.
pub fn combos(dead: CardSet) -> Vec<CardSet> {
    let deck: Vec<Card> = Rank::ALL
        .into_iter()
        .flat_map(|rank| Suit::ALL.map(|suit| Card::new(rank, suit)))
        .filter(|&card| !dead.contains(card))
        .collect();
    let mut all = Vec::with_capacity(deck.len() * deck.len().saturating_sub(1) / 2);
    for (i, &first) in deck.iter().enumerate() {
        for &second in &deck[i + 1..] {
            all.push([first, second].into_iter().collect());
        }
    }
    all
}

/// How a pair of hole cards ranks on a board. Every pair on one board scores the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Score {
    /// Before the flop: a fixed score of the two cards.
    Preflop(u32),
    /// After it: the strength of the best five cards.
    Made(Strength),
}

fn score(combo: CardSet, board: CardSet) -> Score {
    if board.len() >= 3 {
        Score::Made(strength(combo | board))
    } else {
        Score::Preflop(preflop(combo))
    }
}

/// A rough preflop order: pairs by rank, then the high card, the low card, suitedness and
/// connectedness. It only needs to be monotone enough for buckets; the fit does the rest.
fn preflop(combo: CardSet) -> u32 {
    let mut cards = combo.iter();
    let (Some(a), Some(b)) = (cards.next(), cards.next()) else {
        return 0;
    };
    let (low, high) = (a.rank() as u32 + 2, b.rank() as u32 + 2);
    if low == high {
        return 100 + 13 * high;
    }
    let suited = if a.suit() == b.suit() { 8 } else { 0 };
    let connected = match high - low {
        1 => 4,
        2 => 2,
        _ => 0,
    };
    10 * high + 3 * low + suited + connected
}

/// The range that follows from `seen` under `model`: every pair that misses `dead` and the boards
/// seen, weighted by the chance it takes each action seen, and normalised to sum to 1.
pub fn range(model: &ActionModel, seen: &[Seen], dead: CardSet) -> Vec<(CardSet, f64)> {
    let all_boards = seen
        .iter()
        .fold(CardSet::EMPTY, |cards, step| cards | step.board);
    let live = combos(dead | all_boards);
    let mut weights = vec![1.0; live.len()];
    let mut board_buckets: Vec<(CardSet, Vec<(CardSet, usize)>)> = Vec::new();
    for step in seen {
        if !board_buckets.iter().any(|(board, _)| *board == step.board) {
            board_buckets.push((step.board, buckets(step.board, dead)));
        }
        let Some((_, ranked)) = board_buckets.iter().find(|(board, _)| *board == step.board) else {
            continue;
        };
        // Both lists come from `combos` in one order, and `ranked` holds every live pair, so one
        // walk down `ranked` finds each in turn.
        let mut at = 0;
        for (weight, combo) in weights.iter_mut().zip(&live) {
            while ranked[at].0 != *combo {
                at += 1;
            }
            *weight *= model.prob(step.situation, ranked[at].1, step.mv);
        }
    }
    let total: f64 = weights.iter().sum();
    live.into_iter()
        .zip(weights)
        .map(|(combo, weight)| (combo, weight / total))
        .collect()
}

/// `range` as its mass over the strength buckets on `board`: one number a bucket, summing to 1.
///
/// The range itself is a weight per pair, 17 to 21 KB for a board; a trace keeps this instead. It
/// is the same abstraction the model reasons in ([`buckets`]), so a reader of the trace and the
/// solver that read the range are looking at one thing. An empty range summarises to zeros.
pub fn summary(read: &[(CardSet, f64)], board: CardSet) -> Vec<f64> {
    masses(read, &ranked(board))
}

/// Every live pair on `board` with its bucket, once: five opponents on one board share one table.
pub fn ranked(board: CardSet) -> BTreeMap<CardSet, usize> {
    buckets(board, CardSet::EMPTY).into_iter().collect()
}

/// [`summary`] by a table [`ranked`] already built.
pub fn masses(read: &[(CardSet, f64)], ranked: &BTreeMap<CardSet, usize>) -> Vec<f64> {
    let mut masses = vec![0.0; BUCKETS];
    let mut total = 0.0;
    for &(held, weight) in read {
        let Some(&bucket) = ranked.get(&held) else {
            continue;
        };
        masses[bucket] += weight;
        total += weight;
    }
    if total > 0.0 {
        for mass in &mut masses {
            *mass /= total;
        }
    }
    masses
}
