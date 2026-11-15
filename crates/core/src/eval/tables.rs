//! The evaluator's two tables, built from the rules of hold'em on first use.
//!
//! A hand is first scored as a "class": the category, then the ranks that decide ties, four bits
//! each, most significant first. The 7,462 classes of five cards, sorted, give each class its
//! strength.

use super::{Category, RANK_WEIGHTS};

pub(super) struct Tables {
    /// The strength of a flush, by the 13-bit rank mask of its suit's cards (five to seven).
    pub(super) flushes: Vec<u16>,
    /// The strength of five to seven cards with no flush, by the sum of their rank weights.
    pub(super) ranks: PerfectHash,
}

impl Tables {
    pub(super) fn build() -> Self {
        let classes = classes();
        let strength = |class: u32| {
            let at = classes
                .binary_search(&class)
                .expect("every hand has a class");
            u16::try_from(at).expect("7,462 strengths fit in a u16")
        };
        let flushes = (0..1u16 << 13)
            .map(|mask| match mask.count_ones() {
                5.. => strength(suited_class(mask)),
                _ => 0,
            })
            .collect();
        let entries = rank_counts(5..=7)
            .iter()
            .map(|counts| (rank_key(counts), strength(unsuited_class(counts))))
            .collect();
        Self {
            flushes,
            ranks: PerfectHash::build(entries),
        }
    }
}

/// A two-level perfect hash from a rank key to a strength: the key's low bits pick a row, and the
/// row's offset places its high bits in one shared array, where no two keys land in one slot. The
/// low bits come from the small weights of the low ranks, so they spread the keys evenly over the
/// rows, and an even spread packs quickly and tightly.
pub(super) struct PerfectHash {
    offsets: Vec<u32>,
    values: Vec<u16>,
}

const ROW_BITS: u32 = 14;
const ROWS: u32 = 1 << ROW_BITS;

impl PerfectHash {
    pub(super) fn get(&self, key: u32) -> u16 {
        let offset = self.offsets[(key & (ROWS - 1)) as usize];
        self.values[(offset + (key >> ROW_BITS)) as usize]
    }

    /// Places the fullest rows first, each at the lowest offset where its keys all land on free
    /// slots. Panics if two entries share a key: the rank weights must keep every sum apart.
    fn build(entries: Vec<(u32, u16)>) -> Self {
        let max_key = entries.iter().map(|&(key, _)| key).max().unwrap_or(0);
        let mut rows: Vec<Vec<(u32, u16)>> = vec![Vec::new(); ROWS as usize];
        for (key, value) in entries {
            rows[(key & (ROWS - 1)) as usize].push((key >> ROW_BITS, value));
        }
        let mut order: Vec<usize> = (0..rows.len()).filter(|&r| !rows[r].is_empty()).collect();
        order.sort_by_key(|&r| std::cmp::Reverse(rows[r].len()));

        let mut offsets = vec![0; rows.len()];
        // One bit per slot, set once the slot holds a value.
        let mut taken: Vec<u64> = Vec::new();
        let mut values: Vec<u16> = Vec::new();
        let mut first_free: usize = 0;
        for r in order {
            let row = &mut rows[r];
            row.sort_unstable();
            assert!(
                row.windows(2).all(|pair| pair[0].0 != pair[1].0),
                "two rank sets share a key: the rank weights must keep every sum apart"
            );
            // Test 64 offsets at a time: bit i of `blocked` is set when offset + i would land a
            // key of this row on a taken slot.
            let mut offset = first_free.saturating_sub(row[0].0 as usize);
            loop {
                let blocked = row.iter().fold(0, |blocked, &(column, _)| {
                    blocked | bits_from(&taken, offset + column as usize)
                });
                if blocked != u64::MAX {
                    offset += (!blocked).trailing_zeros() as usize;
                    break;
                }
                offset += 64;
            }
            for &(column, value) in row.iter() {
                let slot = offset + column as usize;
                if slot / 64 >= taken.len() {
                    taken.resize(slot / 64 + 1, 0);
                }
                if slot >= values.len() {
                    values.resize(slot + 1, 0);
                }
                taken[slot / 64] |= 1 << (slot % 64);
                values[slot] = value;
            }
            while bits_from(&taken, first_free) & 1 == 1 {
                first_free += 1;
            }
            offsets[r] = u32::try_from(offset).expect("the table fits in a u32");
        }
        // Any key up to the largest indexes in bounds, even one no hand makes.
        let end =
            offsets.iter().max().map_or(0, |&o| o as usize) + (max_key >> ROW_BITS) as usize + 1;
        values.resize(end, 0);
        Self { offsets, values }
    }
}

/// The 64 bits of `taken` from bit `start` on, with slots past its end read as free.
fn bits_from(taken: &[u64], start: usize) -> u64 {
    let (word, shift) = (start / 64, start % 64);
    let low = taken.get(word).copied().unwrap_or(0) >> shift;
    let high = match shift {
        0 => 0,
        _ => taken.get(word + 1).copied().unwrap_or(0) << (64 - shift),
    };
    low | high
}

/// The sum of the rank weights of `counts[r]` cards of each rank `r`.
fn rank_key(counts: &[u8; 13]) -> u32 {
    counts
        .iter()
        .zip(RANK_WEIGHTS)
        .map(|(&count, weight)| u32::from(count) * weight)
        .sum()
}

/// Every way to hold `totals` cards by rank, at most four of each.
fn rank_counts(totals: std::ops::RangeInclusive<u8>) -> Vec<[u8; 13]> {
    fn fill(rank: usize, left: u8, counts: &mut [u8; 13], out: &mut Vec<[u8; 13]>) {
        if rank == 13 {
            if left == 0 {
                out.push(*counts);
            }
            return;
        }
        for count in 0..=left.min(4) {
            counts[rank] = count;
            fill(rank + 1, left - count, counts, out);
        }
        counts[rank] = 0;
    }
    let mut out = Vec::new();
    for total in totals {
        fill(0, total, &mut [0; 13], &mut out);
    }
    out
}

/// The 7,462 classes of five cards, sorted: from every five-card rank multiset, and every suited
/// mask of five ranks.
fn classes() -> Vec<u32> {
    let mut classes: Vec<u32> = rank_counts(5..=5).iter().map(unsuited_class).collect();
    classes.extend(
        (0..1u16 << 13)
            .filter(|mask| mask.count_ones() == 5)
            .map(suited_class),
    );
    classes.sort_unstable();
    classes.dedup();
    classes
}

fn class(category: Category, ranks: &[usize]) -> u32 {
    let packed = ranks
        .iter()
        .fold(category as u32, |acc, &rank| acc << 4 | rank as u32);
    packed << (4 * (5 - ranks.len()))
}

/// The top rank of the highest straight in `present`, a 13-bit rank mask. The ace also plays low.
fn straight_top(present: u16) -> Option<usize> {
    // Bit 0 is the ace played low, and bit r + 1 is rank r.
    let with_low_ace = present << 1 | (present >> 12 & 1);
    (0..=9)
        .rev()
        .find(|&low| with_low_ace >> low & 0b11111 == 0b11111)
        .map(|low| low + 3)
}

/// The best five of five to seven cards of one suit, by their rank mask.
fn suited_class(mask: u16) -> u32 {
    match straight_top(mask) {
        Some(top) => class(Category::StraightFlush, &[top]),
        None => class(Category::Flush, &highest(mask, 5, &[])),
    }
}

/// The best five of five to seven cards with no flush, by how many of each rank they hold.
fn unsuited_class(counts: &[u8; 13]) -> u32 {
    let ranks_with =
        |least: u8| -> Vec<usize> { (0..13).rev().filter(|&r| counts[r] >= least).collect() };
    let present = ranks_with(1).iter().fold(0u16, |mask, &r| mask | 1 << r);
    let with_kickers =
        |made: &[usize], kickers: usize| [made, &highest(present, kickers, made)].concat();
    let (quads, trips, pairs) = (ranks_with(4), ranks_with(3), ranks_with(2));
    if let Some(&quad) = quads.first() {
        return class(Category::FourOfAKind, &with_kickers(&[quad], 1));
    }
    if let Some(&trip) = trips.first()
        && let Some(&pair) = pairs.iter().find(|&&r| r != trip)
    {
        return class(Category::FullHouse, &[trip, pair]);
    }
    if let Some(top) = straight_top(present) {
        return class(Category::Straight, &[top]);
    }
    if let Some(&trip) = trips.first() {
        return class(Category::ThreeOfAKind, &with_kickers(&[trip], 2));
    }
    match pairs[..] {
        [high, low, ..] => class(Category::TwoPair, &with_kickers(&[high, low], 1)),
        [pair] => class(Category::OnePair, &with_kickers(&[pair], 3)),
        [] => class(Category::HighCard, &highest(present, 5, &[])),
    }
}

/// The `n` highest ranks in `mask`, leaving out `except`, highest first.
fn highest(mask: u16, n: usize, except: &[usize]) -> Vec<usize> {
    (0..13)
        .rev()
        .filter(|&r| mask >> r & 1 == 1 && !except.contains(&r))
        .take(n)
        .collect()
}
