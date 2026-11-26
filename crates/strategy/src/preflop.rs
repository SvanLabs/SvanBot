//! The preflop tables (V11-39): what each starting hand is worth against one to five random hands,
//! known ahead of time, and the hands each position opens with.
//!
//! - A **class** is one of the 169 kinds of starting hand: a pair, or two ranks suited or offsuit.
//!   Suits do not matter before the flop, so every pair of hole cards has one class.
//! - The **table** is each class's all-in equity against one to five opponents holding random
//!   hands. It is measured once by our own sampler (`svanbot tables preflop`), shipped as
//!   `data/preflop.txt` and read in at the first use, so no decision spends samples on it.
//! - A **position**, before the flop, is how many opponents are still to act behind us and whether we
//!   play the later streets in position. At six-handed UTG has 5 behind, HJ 4, CO 3, the button 2
//!   and the small blind 1; heads up the button has 1 behind and is in position.
//! - A position **opens** the share of hands its rank allows: a class's rank is the share of all hands
//!   that are worth more than it against the players behind.

use std::sync::OnceLock;

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_sim::equity::sampled;

/// Kinds of starting hand.
pub const CLASSES: usize = 169;

/// The most opponents the table is measured against: a full 6-max table.
pub const MOST: usize = 5;

/// The class of a pair of hole cards. Classes sit on a 13 × 13 grid of ranks, `high * 13 + low`
/// for a suited hand, `low * 13 + high` for an offsuit one, and the diagonal for pairs.
pub fn class_of(hole: CardSet) -> usize {
    let mut cards = hole.iter();
    let (Some(a), Some(b)) = (cards.next(), cards.next()) else {
        return 0;
    };
    let (x, y) = (a.rank() as usize, b.rank() as usize);
    let (high, low) = (x.max(y), x.min(y));
    if a.suit() == b.suit() {
        high * 13 + low
    } else {
        low * 13 + high
    }
}

/// A class's name in the usual notation: `AA`, `AKs`, `72o`.
pub fn name(class: usize) -> String {
    let (row, column) = (class / 13, class % 13);
    let letter = |rank: usize| "23456789TJQKA".as_bytes()[rank] as char;
    match row.cmp(&column) {
        std::cmp::Ordering::Equal => format!("{}{}", letter(row), letter(row)),
        std::cmp::Ordering::Greater => format!("{}{}s", letter(row), letter(column)),
        std::cmp::Ordering::Less => format!("{}{}o", letter(column), letter(row)),
    }
}

/// How many pairs of hole cards a class holds: 6 for a pair, 4 suited, 12 offsuit.
pub fn combos(class: usize) -> f64 {
    match (class / 13).cmp(&(class % 13)) {
        std::cmp::Ordering::Equal => 6.0,
        std::cmp::Ordering::Greater => 4.0,
        std::cmp::Ordering::Less => 12.0,
    }
}

/// One pair of hole cards of `class`, the one the table is measured with.
pub fn example(class: usize) -> CardSet {
    let (row, column) = (class / 13, class % 13);
    let (high, low) = (Rank::ALL[row.max(column)], Rank::ALL[row.min(column)]);
    let second = if row > column {
        Suit::Hearts
    } else {
        Suit::Spades
    };
    [Card::new(high, Suit::Hearts), Card::new(low, second)]
        .into_iter()
        .collect()
}

/// The equity of `class` against `opponents` random hands, measured from `samples` seeded deals.
/// This is what the shipped table holds, at the samples and seed it was written with.
pub fn measure(class: usize, opponents: usize, samples: u64, seed: u64) -> f64 {
    let hole = example(class);
    let uniform: Vec<(CardSet, f64)> = every_pair(hole);
    let ranges: Vec<&[(CardSet, f64)]> = vec![uniform.as_slice(); opponents];
    sampled(hole, &ranges, CardSet::EMPTY, samples, seed)
}

/// Every pair of hole cards that misses `dead`, each with weight 1.
fn every_pair(dead: CardSet) -> Vec<(CardSet, f64)> {
    crate::action::combos(dead)
        .into_iter()
        .map(|pair| (pair, 1.0))
        .collect()
}

/// The table's text: a line a class in class order, its name and its equity against one to
/// [`MOST`] opponents. `equities[class][opponents - 1]`.
pub fn table_text(equities: &[[f64; MOST]]) -> String {
    let mut text = String::new();
    for (class, row) in equities.iter().enumerate() {
        text.push_str(&name(class));
        for value in row {
            text.push_str(&format!(" {value:.6}"));
        }
        text.push('\n');
    }
    text
}

/// The shipped table, read once.
fn table() -> &'static [[f64; MOST]; CLASSES] {
    static TABLE: OnceLock<[[f64; MOST]; CLASSES]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [[0.0; MOST]; CLASSES];
        let text = include_str!("data/preflop.txt");
        for (class, line) in text.lines().enumerate().take(CLASSES) {
            let mut words = line.split(' ');
            assert_eq!(
                words.next(),
                Some(name(class).as_str()),
                "preflop.txt line {class}"
            );
            for (at, word) in words.take(MOST).enumerate() {
                table[class][at] = word.parse().expect("preflop.txt holds numbers");
            }
        }
        table
    })
}

/// The equity of `class` against `opponents` random hands, from the shipped table. The count is
/// held to one through [`MOST`].
pub fn equity(class: usize, opponents: usize) -> f64 {
    table()[class.min(CLASSES - 1)][opponents.clamp(1, MOST) - 1]
}

/// Each class's rank against `opponents`: the share of all hands worth more, plus half its own.
fn ranks() -> &'static [[f64; CLASSES]; MOST] {
    static RANKS: OnceLock<[[f64; CLASSES]; MOST]> = OnceLock::new();
    RANKS.get_or_init(|| {
        let mut ranks = [[0.0; CLASSES]; MOST];
        let total: f64 = (0..CLASSES).map(combos).sum();
        for (at, rank) in ranks.iter_mut().enumerate() {
            let mut order: Vec<usize> = (0..CLASSES).collect();
            // Best first; equal equities in class order, so the order is the same everywhere.
            order.sort_by(|&a, &b| table()[b][at].total_cmp(&table()[a][at]).then(a.cmp(&b)));
            let mut above = 0.0;
            for class in order {
                rank[class] = (above + combos(class) / 2.0) / total;
                above += combos(class);
            }
        }
        ranks
    })
}

/// Where a decision sits before the flop: the opponents still to act behind it, and whether it
/// plays the later streets in position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub behind: usize,
    pub in_position: bool,
}

impl Position {
    /// The share of all hands this position opens an unopened pot with. The standard 6-max opening
    /// ranges: tight up front, wide on the button, and widest heads up, where the button is in
    /// position against one player.
    pub fn share(self) -> f64 {
        match (self.behind, self.in_position) {
            (0 | 1, true) => 0.80,
            (0 | 1, false) => 0.42,
            (2, _) => 0.45,
            (3, _) => 0.27,
            (4, _) => 0.19,
            _ => 0.15,
        }
    }

    /// The opening share at this width, capped at every hand.
    pub fn share_at(self, scale: f64) -> f64 {
        (self.share() * scale).min(1.0)
    }
}

/// Whether `position` opens an unopened pot with `class`: its rank against the players behind is
/// within the position's share.
pub fn opens(class: usize, position: Position) -> bool {
    opens_at(class, position, 1.0)
}

/// Whether the position opens this class with the scaled opening share.
pub fn opens_at(class: usize, position: Position, scale: f64) -> bool {
    rank(class, position) < position.share_at(scale)
}

/// The share of hands over which the `Balanced` kind turns the opening cut into a chance: half
/// inside the width and half outside. It is a compiled rule, not a number the slot learns.
pub const OPEN_BLEND: f64 = 0.02;

/// The chance the position opens this class under a blended boundary. It is 1 well inside the
/// width, 0 well outside it, and runs linearly across [`OPEN_BLEND`] around the cut, so a class
/// exactly on the boundary opens half the time. `opens_at` is the same boundary without the
/// blend.
pub fn open_chance(class: usize, position: Position, scale: f64) -> f64 {
    let edge = position.share_at(scale) - rank(class, position);
    (edge / OPEN_BLEND + 0.5).clamp(0.0, 1.0)
}

/// A class's rank by position: the share of all hands worth less than it, 0 the best. The
/// mechanics read it, as the opening cut does.
pub fn rank_at(class: usize, position: Position) -> f64 {
    rank(class, position)
}

fn rank(class: usize, position: Position) -> f64 {
    ranks()[position.behind.clamp(1, MOST) - 1][class.min(CLASSES - 1)]
}
