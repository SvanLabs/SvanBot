//! v10.0.2's samplers and evaluator, against values the v10.0.2 code gave for the same seeds.

use svanbot_core::rng::Rng;
use svanbot_sim::v10::cards::{Card, mask_of};
use svanbot_sim::v10::draw;
use svanbot_sim::v10::eval::{FLUSH, STRAIGHT, STRAIGHT_FLUSH, category, eval};

#[test]
fn the_draws_match_v10_for_seed_7() {
    let mut rng = Rng::seeded(7);
    let f64s: Vec<f64> = (0..4).map(|_| draw::f64(&mut rng)).collect();
    assert_eq!(
        f64s,
        [
            0.05536043647833311,
            0.17211585444811772,
            0.7175761283586594,
            0.42720981929150526
        ]
    );
    let f32s: Vec<f32> = (0..4).map(|_| draw::f32(&mut rng)).collect();
    assert_eq!(f32s, [0.96365947, 0.46570367, 0.72390705, 0.3298394]);
    let cards: Vec<u32> = (0..12).map(|_| draw::below(52, &mut rng)).collect();
    assert_eq!(cards, [51, 3, 5, 8, 38, 5, 25, 5, 8, 9, 5, 41]);
    let positions: Vec<u32> = (0..6).map(|_| draw::below(7, &mut rng)).collect();
    assert_eq!(positions, [4, 2, 5, 3, 5, 0]);
    assert_eq!(draw::u64(&mut rng), 99951794812778521);
    assert_eq!(draw::u64(&mut rng), 2223757807435418317);
}

#[test]
fn the_evaluator_values_match_v10() {
    let recorded: [(u64, u32); 12] = [
        (1145830702587968, 2840064),
        (2216204468258, 2435072),
        (291379179683864, 1354096),
        (52785181689858, 833381),
        (20067160949856, 1428064),
        (68790913281, 1096336),
        (71485578281217, 2628096),
        (9298772352, 1558912),
        (1478849581817856, 759653),
        (1688849907450368, 2792448),
        (20476919808, 1419328),
        (211930870448145, 1100160),
    ];
    for (mask, value) in recorded {
        assert_eq!(eval(mask), value, "mask {mask:#x}");
    }
}

#[test]
fn the_wheel_and_the_royal_rank_as_v10_ranks_them() {
    let hand = |text: &str| {
        let cards: Vec<Card> = text
            .split(' ')
            .map(|c| Card::parse(c).unwrap())
            .collect();
        eval(mask_of(&cards))
    };
    let wheel = hand("Ah 2c 3d 4s 5h 9c Kd");
    assert_eq!(category(wheel), STRAIGHT);
    assert!(hand("2h 3c 4d 5s 6h 9c Kd") > wheel);
    assert_eq!(category(hand("Ah Kh Qh 2h 7h 9c 3d")), FLUSH);
    assert_eq!(
        hand("As Ks Qs Js Ts 2c 3d"),
        (STRAIGHT_FLUSH << 20) | (12 << 16)
    );
}
