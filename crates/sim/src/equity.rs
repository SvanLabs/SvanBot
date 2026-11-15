//! Equity: each player's share of the pot over the ways the board can run out, by enumerating every
//! runout or by sampling seeded deals.
//!
//! Shares are counted in whole units of 1/60, so a tie among up to six players adds exact
//! integers. A result then has the same bits whatever order its parts were summed in, and work
//! cut into blocks can run on any number of threads.

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::eval::{Key, Strength};
use svanbot_core::rng::Rng;

/// The units a whole pot is counted in: 60 splits evenly among one to six players.
const WHOLE: u64 = 60;

/// Sampled work is cut into this many blocks, each drawn from its own stream of the seed. The
/// count is fixed, not set by the machine, so a seed gives the same deals everywhere.
pub const BLOCKS: u64 = 64;

/// Each player's equity with these hole cards on `board` (zero to five cards), over every way the
/// rest of the board can come. The shares sum to 1.
pub fn exact(hole_cards: &[CardSet], board: CardSet) -> Vec<f64> {
    let dead = hole_cards.iter().fold(board, |dead, &cards| dead | cards);
    let unseen: Vec<Card> = deck().filter(|&card| !dead.contains(card)).collect();
    let mut units = vec![0u64; hole_cards.len()];
    let mut runouts = 0u64;
    each_runout(&unseen, 5 - board.len(), Key::of(board), &mut |key| {
        runouts += 1;
        let strengths: Vec<Strength> = hole_cards
            .iter()
            .map(|&cards| add(key, cards).strength())
            .collect();
        share(&strengths, &mut units);
    });
    units
        .iter()
        .map(|&u| u as f64 / (WHOLE * runouts) as f64)
        .collect()
}

/// The equity of `hero` on `board` against one opponent per range, from `samples` seeded deals.
/// A range lists hole cards with weights. Each deal draws every opponent's cards by weight, and
/// redraws cards that clash with ones already dealt.
pub fn sampled(
    hero: CardSet,
    ranges: &[&[(CardSet, f64)]],
    board: CardSet,
    samples: u64,
    seed: u64,
) -> f64 {
    let ranges: Vec<Weighted> = ranges.iter().map(|range| Weighted::new(range)).collect();
    let mut units = 0u64;
    let mut dealt = 0u64;
    for block in 0..BLOCKS {
        let mut rng = Rng::stream(seed, block);
        let count = samples / BLOCKS + u64::from(block < samples % BLOCKS);
        for _ in 0..count {
            if let Some(share) = deal_once(hero, &ranges, board, &mut rng) {
                units += share;
                dealt += 1;
            }
        }
    }
    units as f64 / (WHOLE * dealt.max(1)) as f64
}

/// Hero's share of one seeded deal, in units, or `None` if a range could not be dealt around the
/// cards already out.
fn deal_once(hero: CardSet, ranges: &[Weighted], board: CardSet, rng: &mut Rng) -> Option<u64> {
    let mut dead = hero | board;
    let mut opponents = Vec::with_capacity(ranges.len());
    for range in ranges {
        let cards = (0..100)
            .map_while(|_| range.draw(rng))
            .find(|&cards| cards.is_disjoint(dead))?;
        dead |= cards;
        opponents.push(cards);
    }
    let mut full = board;
    while full.len() < 5 {
        let index = rng.below(52) as usize;
        let card = Card::new(Rank::ALL[index / 4], Suit::ALL[index % 4]);
        if !dead.contains(card) {
            dead.insert(card);
            full.insert(card);
        }
    }
    let key = Key::of(full);
    let mut strengths = vec![add(key, hero).strength()];
    strengths.extend(opponents.iter().map(|&cards| add(key, cards).strength()));
    let mut units = vec![0; strengths.len()];
    share(&strengths, &mut units);
    Some(units[0])
}

/// Hole cards drawn by weight, through the running total of the weights.
struct Weighted {
    cards: Vec<CardSet>,
    running: Vec<f64>,
}

impl Weighted {
    fn new(range: &[(CardSet, f64)]) -> Self {
        let mut total = 0.0;
        let running = range
            .iter()
            .map(|&(_, weight)| {
                total += weight.max(0.0);
                total
            })
            .collect();
        Self {
            cards: range.iter().map(|&(cards, _)| cards).collect(),
            running,
        }
    }

    /// Hole cards drawn by weight, or `None` for an empty range.
    fn draw(&self, rng: &mut Rng) -> Option<CardSet> {
        let total = self.running.last().copied()?;
        // 53 random bits as a fraction of the total weight.
        let point = (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64 * total;
        let at = self.running.partition_point(|&below| below <= point);
        self.cards.get(at.min(self.cards.len() - 1)).copied()
    }
}

/// Splits one pot's units among the strongest of `strengths`.
fn share(strengths: &[Strength], units: &mut [u64]) {
    let best = strengths.iter().max().copied();
    let winners = strengths.iter().filter(|&&s| Some(s) == best).count() as u64;
    for (unit, &strength) in units.iter_mut().zip(strengths) {
        if Some(strength) == best {
            *unit += WHOLE / winners;
        }
    }
}

fn add(key: Key, cards: CardSet) -> Key {
    cards.iter().fold(key, Key::add)
}

fn deck() -> impl Iterator<Item = Card> {
    Rank::ALL
        .into_iter()
        .flat_map(|rank| Suit::ALL.map(|suit| Card::new(rank, suit)))
}

/// Calls `visit` with the key of every way to add `left` of the `unseen` cards to `key`.
fn each_runout(unseen: &[Card], left: usize, key: Key, visit: &mut impl FnMut(Key)) {
    if left == 0 {
        return visit(key);
    }
    for (i, &card) in unseen.iter().enumerate() {
        each_runout(&unseen[i + 1..], left - 1, key.add(card), visit);
    }
}
