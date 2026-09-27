//! Exact board strengths: river equity and combo strength tables (0261).

use super::ranges::equity_vs_ranges;
use sv10_cards::cards::{Card, CardMask};
use sv10_cards::eval::eval;
use sv10_cards::range::{NUM_COMBOS, Range, combo_mask, combos};
use sv10_rng::{Rng, RngExt};
/// Exact river equity of hero against one range (weighted by combo weights).
pub fn river_equity_exact(hero: [Card; 2], board: &[Card; 5], opp: &Range) -> f64 {
    let hero_mask = hero[0].bit() | hero[1].bit();
    let board_mask = board.iter().fold(0u64, |m, c| m | c.bit());
    let dead = hero_mask | board_mask;
    let hv = eval(hero_mask | board_mask);
    let (mut won, mut total) = (0.0, 0.0);
    for i in 0..NUM_COMBOS {
        let w = opp.w[i] as f64;
        let m = combo_mask(i);
        if w <= 0.0 || m & dead != 0 {
            continue;
        }
        let v = eval(m | board_mask);
        total += w;
        if hv > v {
            won += w;
        } else if hv == v {
            won += w * 0.5;
        }
    }
    if total == 0.0 { 0.5 } else { won / total }
}

/// Exact river equity of every live combo against a uniformly random live hand, identical to
/// `river_equity_exact(combo, board, full)` for each combo but from one evaluation per combo:
/// combos are sorted by rank and each counts the lower and tied combos that share none of its
/// cards, via running per-card counters (card removal).
pub fn river_strengths(board_mask: CardMask) -> Vec<f32> {
    let t = combos();
    let mut made: Vec<(u32, u16)> =
        (0..NUM_COMBOS).filter(|&i| combo_mask(i) & board_mask == 0).map(|i| (eval(combo_mask(i) | board_mask), i as u16)).collect();
    made.sort_unstable();
    let live = made.len() as i64;
    let mut per_card = [0i64; 52];
    for &(_, i) in &made {
        let (a, b) = t.cards[i as usize];
        per_card[a.0 as usize] += 1;
        per_card[b.0 as usize] += 1;
    }
    let mut out = vec![0f32; NUM_COMBOS];
    let mut below = 0i64;
    let mut below_card = [0i64; 52];
    let mut k = 0;
    while k < made.len() {
        let mut j = k;
        let mut group_card = [0i64; 52];
        while j < made.len() && made[j].0 == made[k].0 {
            let (a, b) = t.cards[made[j].1 as usize];
            group_card[a.0 as usize] += 1;
            group_card[b.0 as usize] += 1;
            j += 1;
        }
        let group = (j - k) as i64;
        for &(_, i) in &made[k..j] {
            let (a, b) = t.cards[i as usize];
            let (a, b) = (a.0 as usize, b.0 as usize);
            let total = live - per_card[a] - per_card[b] + 1;
            let wins = below - below_card[a] - below_card[b];
            let ties = group - group_card[a] - group_card[b] + 1;
            out[i as usize] = if total == 0 { 0.5 } else { ((wins as f64 + ties as f64 * 0.5) / total as f64) as f32 };
        }
        for c in 0..52 {
            below_card[c] += group_card[c];
        }
        below += group;
        k = j;
    }
    out
}

/// Exact flop/turn strengths: the same blend as `combo_strengths` (made-hand rank percentile and
/// equity against a random hand), with the equity computed exactly by averaging
/// `river_strengths` over every runout that does not contain the combo's cards.
pub fn exact_strengths(board: &[Card]) -> Vec<f32> {
    let board_mask = board.iter().fold(0u64, |m, c| m | c.bit());
    let rest: Vec<Card> = (0..52u8).map(Card).filter(|c| board_mask & c.bit() == 0).collect();
    let mut sum = vec![0f64; NUM_COMBOS];
    let mut count = vec![0u32; NUM_COMBOS];
    let mut add = |mask: CardMask| {
        let s = river_strengths(mask);
        for i in 0..NUM_COMBOS {
            if combo_mask(i) & mask == 0 {
                sum[i] += s[i] as f64;
                count[i] += 1;
            }
        }
    };
    match board.len() {
        4 => rest.iter().for_each(|r| add(board_mask | r.bit())),
        3 => {
            for (k, t) in rest.iter().enumerate() {
                for r in &rest[k + 1..] {
                    add(board_mask | t.bit() | r.bit());
                }
            }
        }
        _ => panic!("exact_strengths needs a flop or turn"),
    }
    let made_pct = made_percentiles(board_mask);
    (0..NUM_COMBOS).map(|i| if count[i] == 0 { 0.0 } else { 0.55 * made_pct[i] + 0.45 * (sum[i] / count[i] as f64) as f32 }).collect()
}

/// Exact made-hand rank percentile of every live combo on a partial board (ties share the midpoint).
fn made_percentiles(board_mask: CardMask) -> Vec<f32> {
    let mut made: Vec<(u32, usize)> =
        (0..NUM_COMBOS).filter(|&i| combo_mask(i) & board_mask == 0).map(|i| (eval(combo_mask(i) | board_mask), i)).collect();
    made.sort();
    let mut out = vec![0f32; NUM_COMBOS];
    let n = made.len() as f32;
    let mut k = 0;
    while k < made.len() {
        let mut j = k;
        while j < made.len() && made[j].0 == made[k].0 {
            j += 1;
        }
        let mid = (k + j) as f32 * 0.5 / n;
        for &(_, i) in &made[k..j] {
            out[i] = mid;
        }
        k = j;
    }
    out
}

/// Showdown strength of every combo on a complete-or-partial board. River:
/// exact equity against a random hand. Flop/turn: a blend of the exact
/// made-hand rank percentile (separates kickers deterministically) and Monte
/// Carlo equity against a random hand (credits draws).
pub fn combo_strengths<R: Rng>(board: &[Card], samples_per_combo: usize, rng: &mut R) -> Vec<f32> {
    let board_mask = board.iter().fold(0u64, |m, c| m | c.bit());
    let full = Range::full();
    let t = combos();
    if board.len() == 5 {
        return river_strengths(board_mask);
    }
    let live: Vec<usize> = (0..NUM_COMBOS).filter(|&i| combo_mask(i) & board_mask == 0).collect();
    let mut made: Vec<(u32, usize)> = live.iter().map(|&i| (eval(combo_mask(i) | board_mask), i)).collect();
    made.sort();
    let mut made_pct = vec![0f32; NUM_COMBOS];
    let n = made.len() as f32;
    let mut k = 0;
    while k < made.len() {
        let mut j = k;
        while j < made.len() && made[j].0 == made[k].0 {
            j += 1;
        }
        let mid = (k + j) as f32 * 0.5 / n;
        for &(_, i) in &made[k..j] {
            made_pct[i] = mid;
        }
        k = j;
    }
    use sv10_rng::SeedableRng;
    let base_seed: u64 = rng.random();
    // Sequential on purpose: this runs inside decisions, where a nested rayon job would let a
    // waiting worker run another table's task under this decision's thread-local strength mode.
    let eqs: Vec<(usize, f32)> = live
        .iter()
        .map(|&i| {
            let (a, b) = t.cards[i];
            let mut r = sv10_rng::rngs::SmallRng::seed_from_u64(base_seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            (i, equity_vs_ranges([a, b], board, &[&full], samples_per_combo, &mut r) as f32)
        })
        .collect();
    let mut out = vec![0f32; NUM_COMBOS];
    for (i, eq) in eqs {
        out[i] = 0.55 * made_pct[i] + 0.45 * eq;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::deals::SharedDeals;
    use super::*;
    use sv10_cards::cards::parse_cards;
    use sv10_rng::SeedableRng;
    use sv10_rng::rngs::SmallRng;

    fn two(a: &str, b: &str) -> [Card; 2] {
        [Card::parse(a).unwrap(), Card::parse(b).unwrap()]
    }

    fn cards(s: &[&str]) -> Vec<Card> {
        parse_cards(s).unwrap()
    }

    #[test]
    fn known_preflop_equities() {
        let mut rng = SmallRng::seed_from_u64(7);
        let full = Range::full();
        let aa = equity_vs_ranges(two("Ah", "As"), &[], &[&full], 200_000, &mut rng);
        assert!((aa - 0.852).abs() < 0.006, "AA vs random {aa}");
        let mut kk = Range::empty();
        for &(a, b) in combos().cards.iter() {
            if a.rank() == 11 && b.rank() == 11 {
                kk.w[sv10_cards::range::combo_index(a, b).expect("a table combo")] = 1.0;
            }
        }
        let aa_kk = equity_vs_ranges(two("Ah", "As"), &[], &[&kk], 200_000, &mut rng);
        assert!((aa_kk - 0.819).abs() < 0.008, "AA vs KK {aa_kk}");
        let aa3 = equity_vs_ranges(two("Ah", "As"), &[], &[&full, &full], 200_000, &mut rng);
        assert!((aa3 - 0.735).abs() < 0.008, "AA vs 2 random {aa3}");
    }

    #[test]
    fn shared_deals_match_direct_monte_carlo() {
        let board = parse_cards(&["Qd", "8s", "3c"]).unwrap();
        let hero = two("Qh", "Jh");
        let full = Range::full();
        let mut rng = SmallRng::seed_from_u64(11);
        let deals = SharedDeals::new(hero, &board, &[&full, &full], 60_000, &mut rng);
        let both = deals.equity(&[(0, None), (1, None)]).unwrap();
        let direct = equity_vs_ranges(hero, &board, &[&full, &full], 60_000, &mut rng);
        assert!((both - direct).abs() < 0.01, "shared {both} direct {direct}");
        // Narrow the first opponent to pairs and broadways; reweighted equity must match a fresh run.
        let mut narrow = Range::empty();
        for (i, &(a, b)) in combos().cards.iter().enumerate() {
            if a.rank() == b.rank() || (a.rank() >= 8 && b.rank() >= 8) {
                narrow.w[i] = 0.5;
            }
        }
        let reweighted = deals.equity(&[(0, Some(&narrow))]).unwrap();
        let fresh = equity_vs_ranges(hero, &board, &[&narrow], 60_000, &mut rng);
        assert!((reweighted - fresh).abs() < 0.015, "reweighted {reweighted} fresh {fresh}");
        // A range with no weight in the deals cannot be estimated.
        assert!(deals.equity(&[(0, Some(&Range::empty()))]).is_none());
    }

    /// Brute-force exact heads-up equity against `opp` over every combo and board completion.
    fn brute_exact(hero: [Card; 2], board: &[Card], opp: &Range) -> f64 {
        let dead = hero[0].bit() | hero[1].bit() | board.iter().fold(0u64, |m, c| m | c.bit());
        let rest: Vec<u64> = (0..52u8).map(|c| Card(c).bit()).filter(|b| dead & b == 0).collect();
        let (mut won, mut total) = (0.0f64, 0.0f64);
        for i in 0..NUM_COMBOS {
            let (w, m) = (opp.w[i] as f64, combo_mask(i));
            if w <= 0.0 || m & dead != 0 {
                continue;
            }
            let free: Vec<u64> = rest.iter().copied().filter(|b| m & b == 0).collect();
            let mut boards = Vec::new();
            match 5 - board.len() {
                0 => boards.push(0),
                1 => boards.extend(free.iter().copied()),
                _ => {
                    for x in 0..free.len() {
                        for y in x + 1..free.len() {
                            boards.push(free[x] | free[y]);
                        }
                    }
                }
            }
            let b0 = board.iter().fold(0u64, |acc, c| acc | c.bit());
            for extra in boards {
                let (hv, v) = (eval(hero[0].bit() | hero[1].bit() | b0 | extra), eval(m | b0 | extra));
                total += w;
                won += w * if hv > v {
                    1.0
                } else if hv == v {
                    0.5
                } else {
                    0.0
                };
            }
        }
        won / total
    }

    #[test]
    fn heads_up_deals_are_exact_when_the_enumeration_fits_the_budget() {
        let mut rng = SmallRng::seed_from_u64(3);
        let hero = two("Ah", "Kd");
        let mut narrow = Range::empty();
        for (i, &(a, b)) in combos().cards.iter().enumerate() {
            if a.rank() == b.rank() || a.rank() >= 9 || b.rank() >= 9 {
                narrow.w[i] = if a.rank() == b.rank() { 1.0 } else { 0.4 };
            }
        }
        for board in [cards(&["7s", "8s", "2c", "Jd", "Kh"]), cards(&["7s", "8s", "2c", "Jd"])] {
            let deals = SharedDeals::new_parallel(hero, &board, &[&narrow], 1_600_000, 4, &mut rng);
            assert!(deals.is_exact(), "board {board:?}");
            let got = deals.equity(&[(0, None)]).unwrap();
            let want = brute_exact(hero, &board, &narrow);
            assert!((got - want).abs() < 1e-9, "board {board:?}: {got} vs {want}");
            // Narrowing reweights exactly too: equity against a sub-range equals its exact equity.
            let mut pairs = Range::empty();
            for (i, &(a, b)) in combos().cards.iter().enumerate() {
                if a.rank() == b.rank() {
                    pairs.w[i] = 1.0;
                }
            }
            let got = deals.equity(&[(0, Some(&pairs))]).unwrap();
            let want = brute_exact(hero, &board, &pairs);
            assert!((got - want).abs() < 1e-9, "narrowed: {got} vs {want}");
        }
        // The sequential path enumerates too, and matches the parallel one exactly.
        let board = cards(&["7s", "8s", "2c", "Jd", "Kh"]);
        let a = SharedDeals::new(hero, &board, &[&narrow], 5_000, &mut rng).equity(&[(0, None)]).unwrap();
        let b = SharedDeals::new_parallel(hero, &board, &[&narrow], 5_000, 8, &mut rng).equity(&[(0, None)]).unwrap();
        assert_eq!(a, b);
        assert!((a - river_equity_exact(hero, &[board[0], board[1], board[2], board[3], board[4]], &narrow)).abs() < 1e-9);
        // Too large for the budget, multiway, or preflop: Monte Carlo as before.
        let flop = cards(&["7s", "8s", "2c"]);
        assert!(!SharedDeals::new(hero, &flop, &[&narrow], 2_500, &mut rng).is_exact());
        let full = Range::full();
        assert!(!SharedDeals::new(hero, &board, &[&full, &full], 1_600_000, &mut rng).is_exact());
        assert!(!SharedDeals::new(hero, &[], &[&full], 1_600_000, &mut rng).is_exact());
    }

    #[test]
    fn parallel_shared_deals_are_reproducible_and_agree_with_sequential_deals() {
        let board = parse_cards(&["Qd", "8s", "3c"]).unwrap();
        let hero = two("Qh", "Jh");
        let full = Range::full();
        let deal =
            |chunks: usize| SharedDeals::new_parallel(hero, &board, &[&full, &full], 40_001, chunks, &mut SmallRng::seed_from_u64(5));
        let (a, b) = (deal(4), deal(4));
        assert_eq!(a.len(), 40_001);
        assert_eq!((&a.combos, &a.hero_rank, &a.opp_rank), (&b.combos, &b.hero_rank, &b.opp_rank));
        // One chunk is exactly the sequential path.
        let one = deal(1);
        let seq = SharedDeals::new(hero, &board, &[&full, &full], 40_001, &mut SmallRng::seed_from_u64(5));
        assert_eq!((&one.combos, &one.hero_rank), (&seq.combos, &seq.hero_rank));
        let (pa, ps) = (a.equity(&[(0, None), (1, None)]).unwrap(), seq.equity(&[(0, None), (1, None)]).unwrap());
        assert!((pa - ps).abs() < 0.012, "parallel {pa} sequential {ps}");
    }

    #[test]
    fn river_strengths_equal_per_combo_exact_equity() {
        let full = Range::full();
        for b in
            [["7d", "Ts", "2c", "Kh", "3s"], ["As", "Ks", "Qs", "Js", "Ts"], ["2c", "2d", "2h", "7s", "7c"], ["9h", "8h", "4c", "4d", "Jh"]]
        {
            let board: [Card; 5] = parse_cards(&b).unwrap().try_into().unwrap();
            let mask = board.iter().fold(0u64, |m, c| m | c.bit());
            let fast = river_strengths(mask);
            for (i, &(a, c)) in combos().cards.iter().enumerate() {
                let want = if combo_mask(i) & mask != 0 { 0.0 } else { river_equity_exact([a, c], &board, &full) as f32 };
                assert_eq!(fast[i], want, "board {b:?} combo {a}{c}");
            }
        }
    }

    #[test]
    fn exact_strengths_match_high_sample_monte_carlo() {
        let full = Range::full();
        for b in [vec!["Qd", "8s", "3c", "Th"], vec!["9h", "8h", "2c"]] {
            let board = parse_cards(&b).unwrap();
            let exact = exact_strengths(&board);
            let mask = board.iter().fold(0u64, |m, c| m | c.bit());
            let made = made_percentiles(mask);
            let mut rng = SmallRng::seed_from_u64(9);
            for hole in [two("Qh", "Jh"), two("7h", "6h"), two("2d", "2s")] {
                let i = sv10_cards::range::combo_index(hole[0], hole[1]).expect("two named cards");
                let eq = ((exact[i] - 0.55 * made[i]) / 0.45) as f64;
                let mc = equity_vs_ranges(hole, &board, &[&full], 150_000, &mut rng);
                assert!((eq - mc).abs() < 0.006, "board {b:?} hole {}{}: exact {eq} mc {mc}", hole[0], hole[1]);
            }
        }
    }

    #[test]
    fn river_exact_matches_monte_carlo() {
        let board: [Card; 5] = parse_cards(&["7d", "Ts", "2c", "Kh", "3s"]).unwrap().try_into().unwrap();
        let hero = two("Ah", "Td");
        let full = Range::full();
        let exact = river_equity_exact(hero, &board, &full);
        let mut rng = SmallRng::seed_from_u64(3);
        let mc = equity_vs_ranges(hero, &board, &[&full], 200_000, &mut rng);
        assert!((exact - mc).abs() < 0.005, "exact {exact} mc {mc}");
    }
}
