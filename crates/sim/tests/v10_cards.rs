//! v10.0.2's card encoding, combos and hand classes, which every ported table indexes by.

use svanbot_sim::v10::cards::{
    Card, NUM_COMBOS, Range, class_combos, class_name, combo_index, combo_mask, combos, hand_class,
};

#[test]
fn a_card_is_its_suit_times_thirteen_plus_its_rank() {
    assert_eq!(Card::parse("2c"), Some(Card(0)));
    assert_eq!(Card::parse("Ah"), Some(Card(38)));
    assert_eq!(Card::parse("As"), Some(Card(51)));
    assert_eq!(Card::parse("??"), None);
    for i in 0..52 {
        assert_eq!(Card::parse(&Card(i).to_string()), Some(Card(i)));
    }
}

#[test]
fn the_combos_run_in_v10s_order() {
    let table = combos();
    assert_eq!(table.cards[0], (Card(0), Card(1)));
    assert_eq!(table.cards[51], (Card(1), Card(2)));
    assert_eq!(table.cards[NUM_COMBOS - 1], (Card(50), Card(51)));
    for (i, &(a, b)) in table.cards.iter().enumerate() {
        assert_eq!(combo_index(a, b), Some(i));
        assert_eq!(combo_index(b, a), Some(i));
        assert_eq!(combo_mask(i), (1 << a.0) | (1 << b.0));
    }
    assert_eq!(combo_index(Card(3), Card(3)), None);
}

#[test]
fn the_classes_cover_every_combo() {
    let mut counts = [0usize; 169];
    for &(a, b) in &combos().cards {
        counts[hand_class(a, b)] += 1;
    }
    for (class, &n) in counts.iter().enumerate() {
        assert_eq!(n as f64, class_combos(class), "{}", class_name(class));
    }
    let card = |text| Card::parse(text).unwrap();
    assert_eq!(class_name(hand_class(card("Ah"), card("Kh"))), "AKs");
    assert_eq!(class_name(hand_class(card("2d"), card("Tc"))), "T2o");
    assert_eq!(class_name(12), "AA");
}

#[test]
fn a_range_loses_the_combos_a_dead_card_touches() {
    let mut range = Range::full();
    range.remove_dead(Card::parse("Ah").unwrap().bit());
    assert_eq!(range.total(), 1275.0);
    range.normalize();
    assert!((range.total() - 1.0).abs() < 1e-4);
    assert!(Range::empty().is_empty());
}
