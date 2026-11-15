//! The evaluator, against the venue's hand rankings (<https://docs.openpoker.ai/llms-full.txt>,
//! "Hand Rankings"): the best five of up to seven cards.

use std::collections::BTreeSet;

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::eval::{Category, Key, Strength, strength};

fn of(cards: &str) -> Strength {
    strength(cards.parse::<CardSet>().unwrap())
}

#[test]
fn each_ranking_beats_the_one_below_it() {
    let ladder = [
        ("AsKsQsJsTs", Category::StraightFlush),
        ("9c9d9h9s2c", Category::FourOfAKind),
        ("KcKdKh2s2c", Category::FullHouse),
        ("Ah9h7h4h2h", Category::Flush),
        ("Tc9d8h7s6c", Category::Straight),
        ("QcQdQh7s2c", Category::ThreeOfAKind),
        ("JcJd4h4s2c", Category::TwoPair),
        ("AcAd9h7s2c", Category::OnePair),
        ("AcKd9h7s2c", Category::HighCard),
    ];
    for pair in ladder.windows(2) {
        let [(higher, _), (lower, _)] = pair else {
            unreachable!()
        };
        assert!(of(higher) > of(lower), "{higher} should beat {lower}");
    }
    for (cards, category) in ladder {
        assert_eq!(of(cards).category(), category, "{cards}");
    }
}

#[test]
fn the_best_five_of_seven_play() {
    // Hole cards AhKh on a board of QhJhTh2c2d: a royal flush, whatever the pair on the board.
    assert_eq!(of("AhKhQhJhTh2c2d"), of("AhKhQhJhTh"));
    // Two pairs on the board and a third in the hand: the best two pairs and the best kicker play.
    assert_eq!(of("KcKd9c9d4c4dAs"), of("KcKd9c9dAs"));
    // Kickers past the fifth card do not count.
    assert_eq!(of("AcAd9h7s5c3d2h"), of("AcAd9h7s5c"));
}

#[test]
fn the_wheel_is_the_lowest_straight() {
    assert_eq!(of("Ac2d3h4s5c").category(), Category::Straight);
    assert!(of("Ac2d3h4s5c") < of("2c3d4h5s6c"));
    assert!(of("Ac2d3h4s5c") > of("AcAdKhQsJc"));
    assert!(of("Ah2h3h4h5h") < of("2h3h4h5h6h"));
}

#[test]
fn equal_hands_in_different_suits_split() {
    assert_eq!(of("AcKd9h7s2c"), of("AdKh9s7c2d"));
    assert_eq!(of("Ah9h7h4h2h"), of("As9s7s4s2s"));
}

#[test]
fn a_board_key_plus_two_cards_is_the_seven_card_strength() {
    let board: CardSet = "QhJhTh2c2d".parse().unwrap();
    let key = Key::of(board);
    let ah = "Ah".parse().unwrap();
    let kh = "Kh".parse().unwrap();
    assert_eq!(key.add(ah).add(kh).strength(), of("AhKhQhJhTh2c2d"));
}

fn deck() -> Vec<Card> {
    Rank::ALL
        .into_iter()
        .flat_map(|rank| Suit::ALL.map(|suit| Card::new(rank, suit)))
        .collect()
}

#[test]
fn every_five_card_hand_falls_in_the_published_counts() {
    // The number of five-card hands in each category, and of distinct hands, as published for
    // the 2,598,960 deals of five cards.
    let deck = deck();
    let mut counts = [0u32; 9];
    let mut strengths = BTreeSet::new();
    for (a, &c0) in deck.iter().enumerate() {
        let k0 = Key::EMPTY.add(c0);
        for (b, &c1) in deck.iter().enumerate().skip(a + 1) {
            let k1 = k0.add(c1);
            for (c, &c2) in deck.iter().enumerate().skip(b + 1) {
                let k2 = k1.add(c2);
                for (d, &c3) in deck.iter().enumerate().skip(c + 1) {
                    let k3 = k2.add(c3);
                    for &c4 in &deck[d + 1..] {
                        let strength = k3.add(c4).strength();
                        counts[strength.category() as usize] += 1;
                        strengths.insert(strength);
                    }
                }
            }
        }
    }
    assert_eq!(
        counts,
        [
            1_302_540, 1_098_240, 123_552, 54_912, 10_200, 5_108, 3_744, 624, 40
        ]
    );
    assert_eq!(strengths.len(), 7_462);
}

/// A slow evaluator written only from the rules: score each of the 21 ways to pick five of seven
/// cards, and keep the best.
fn reference(cards: &[Card]) -> (Category, Vec<usize>) {
    let mut best = None;
    for skip_a in 0..cards.len() {
        for skip_b in skip_a + 1..cards.len() {
            let five: Vec<Card> = (0..cards.len())
                .filter(|&i| i != skip_a && i != skip_b)
                .map(|i| cards[i])
                .collect();
            let score = reference_five(&five);
            if best.as_ref().is_none_or(|b| score > *b) {
                best = Some(score);
            }
        }
    }
    best.unwrap()
}

fn reference_five(five: &[Card]) -> (Category, Vec<usize>) {
    let mut by_count: Vec<(usize, usize)> = Rank::ALL
        .iter()
        .map(|&rank| {
            (
                five.iter().filter(|c| c.rank() == rank).count(),
                rank as usize,
            )
        })
        .filter(|&(count, _)| count > 0)
        .collect();
    by_count.sort_unstable_by(|a, b| b.cmp(a));
    let ranks: Vec<usize> = by_count.iter().map(|&(_, rank)| rank).collect();
    let flush = five.iter().all(|c| c.suit() == five[0].suit());
    let straight_top = match ranks[..] {
        [12, 3, 2, 1, 0] => Some(3),
        [top, .., low] if ranks.len() == 5 && top - low == 4 => Some(top),
        _ => None,
    };
    let shape: Vec<usize> = by_count.iter().map(|&(count, _)| count).collect();
    let category = match (straight_top, flush, &shape[..]) {
        (Some(_), true, _) => Category::StraightFlush,
        (_, _, [4, 1]) => Category::FourOfAKind,
        (_, _, [3, 2]) => Category::FullHouse,
        (_, true, _) => Category::Flush,
        (Some(_), _, _) => Category::Straight,
        (_, _, [3, 1, 1]) => Category::ThreeOfAKind,
        (_, _, [2, 2, 1]) => Category::TwoPair,
        (_, _, [2, 1, 1, 1]) => Category::OnePair,
        _ => Category::HighCard,
    };
    (category, straight_top.map_or(ranks, |top| vec![top]))
}

#[test]
fn seven_card_strengths_order_hands_as_the_reference_does() {
    let deck = deck();
    // A fixed xorshift walk, so the test sees the same 20,000 deals on every run.
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move |below: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % below as u64) as usize
    };
    let mut deal = |n: usize| {
        let mut cards = deck.clone();
        (0..n)
            .map(|i| {
                let pick = i + next(cards.len() - i);
                cards.swap(i, pick);
                cards[i]
            })
            .collect::<Vec<Card>>()
    };
    for _ in 0..10_000 {
        let (a, b) = (deal(7), deal(7));
        let (strength_a, strength_b) = (seven(&a), seven(&b));
        let (reference_a, reference_b) = (reference(&a), reference(&b));
        assert_eq!(strength_a.category(), reference_a.0, "{a:?}");
        assert_eq!(
            strength_a.cmp(&strength_b),
            reference_a.cmp(&reference_b),
            "{a:?} vs {b:?}"
        );
    }
}

fn seven(cards: &[Card]) -> Strength {
    strength(cards.iter().copied().fold(CardSet::EMPTY, |mut set, card| {
        set.insert(card);
        set
    }))
}
