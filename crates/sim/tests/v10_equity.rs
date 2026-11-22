//! v10.0.2's range equity and shared deals, against the numbers the v10.0.2 code gave for the
//! same hands, ranges and seeds.

use svanbot_core::rng::Rng;
use svanbot_sim::v10::cards::{Card, NUM_COMBOS, Range};
use svanbot_sim::v10::deals::SharedDeals;
use svanbot_sim::v10::equity::{equity_vs_ranges, equity_vs_ranges_chunked};

fn cards(text: &str) -> Vec<Card> {
    text.split(' ')
        .filter(|c| !c.is_empty())
        .map(|c| Card::parse(c).unwrap())
        .collect()
}

/// A spread of weights over every combo, and a narrowing of it.
fn ranges() -> (Range, Range) {
    let mut top = Range::empty();
    for i in 0..NUM_COMBOS {
        top.w[i] = ((i * 7919) % 1000) as f32 / 1000.0;
    }
    let mut narrow = top.clone();
    for i in 0..NUM_COMBOS {
        narrow.w[i] *= ((i * 31) % 10) as f32 / 10.0;
    }
    (top, narrow)
}

const HERO: &str = "Ah Kd";
const TURN: &str = "7c 8d Ts 2h";

fn hero() -> [Card; 2] {
    let h = cards(HERO);
    [h[0], h[1]]
}

#[test]
fn the_range_equity_matches_v10() {
    let (top, narrow) = ranges();
    let full = Range::full();
    let pre = equity_vs_ranges(hero(), &[], &[&top, &full], 2000, &mut Rng::seeded(3));
    assert_eq!(pre, Some(0.47908333333333336));
    let flop = equity_vs_ranges(
        hero(),
        &cards("7c 8d Ts"),
        &[&top],
        1500,
        &mut Rng::seeded(4),
    );
    assert_eq!(flop, Some(0.45566666666666666));
    let chunked = equity_vs_ranges_chunked(
        hero(),
        &cards(TURN),
        &[&top, &narrow],
        2001,
        4,
        &mut Rng::seeded(5),
    );
    assert_eq!(chunked, Some(0.20964517741129435));
}

#[test]
fn the_shared_deals_match_v10() {
    let (top, narrow) = ranges();
    let full = Range::full();
    let deals = SharedDeals::new(hero(), &[], &[&top, &full], 2500, &mut Rng::seeded(6));
    assert_eq!(deals.len(), 2500);
    assert_eq!(
        deals.equity(&[(0, None), (1, None)]),
        Some(0.4887333333333333)
    );
    assert_eq!(deals.equity(&[(0, Some(&narrow))]), Some(0.637289953456999));
    assert_eq!(deals.equity(&[(1, None)]), Some(0.6608));
    let flop = cards("7c 8d Ts");
    let deals =
        SharedDeals::new_chunked(hero(), &flop, &[&top, &full], 2500, 3, &mut Rng::seeded(7));
    assert_eq!(deals.len(), 2500);
    assert_eq!(
        deals.equity(&[(0, Some(&narrow)), (1, None)]),
        Some(0.27290746874264404)
    );
    assert_eq!(
        deals.equity(&[(1, Some(&narrow))]),
        Some(0.4831932224337327)
    );
}

#[test]
fn few_enough_combos_are_dealt_exactly_as_v10_dealt_them() {
    let (top, narrow) = ranges();
    let mut few = Range::empty();
    for i in (0..NUM_COMBOS).step_by(23) {
        few.w[i] = 0.5 + (i % 3) as f32;
    }
    let deals = SharedDeals::new(hero(), &cards(TURN), &[&few], 2500, &mut Rng::seeded(8));
    assert!(deals.is_exact());
    assert_eq!(deals.len(), 1716);
    assert_eq!(deals.equity(&[(0, None)]), Some(0.600488354620586));
    assert_eq!(
        deals.equity(&[(0, Some(&narrow))]),
        Some(0.6063619680535656)
    );
    let river = cards("7c 8d Ts 2h Kc");
    let deals = SharedDeals::new(hero(), &river, &[&top], 2500, &mut Rng::seeded(9));
    assert!(deals.is_exact());
    assert_eq!(deals.len(), 988);
    assert_eq!(
        deals.equity(&[(0, Some(&narrow))]),
        Some(0.8667104493359744)
    );
}
