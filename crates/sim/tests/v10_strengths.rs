//! v10.0.2's preflop tables and board strengths, against what the v10.0.2 code gave. A table is
//! compared by the FNV-1a digest of its values' bits, so one changed float fails it.

use svanbot_core::rng::Rng;
use svanbot_sim::v10::cards::{Card, mask_of};
use svanbot_sim::v10::preflop;
use svanbot_sim::v10::strengths::{combo_strengths, exact_strengths, river_strengths};

fn digest(values: &[f32]) -> u64 {
    values.iter().fold(0xcbf2_9ce4_8422_2325, |hash, x| {
        x.to_bits().to_le_bytes().iter().fold(hash, |hash, &b| {
            (hash ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        })
    })
}

fn board() -> Vec<Card> {
    "7c 8d Ts 2h Kc"
        .split(' ')
        .map(|c| Card::parse(c).unwrap())
        .collect()
}

#[test]
fn the_preflop_tables_match_v10() {
    let table = preflop::table();
    assert_eq!(digest(&table.combo_percentile), 326778510300171953);
    assert_eq!(table.order[..5], [12, 11, 10, 9, 8]);
    assert_eq!(
        [table.percentile[0], table.percentile[168]],
        [0.5633484, 0.04223228]
    );
    let scores = preflop::combo_scores_vs_top(0.05);
    assert_eq!(digest(&scores.score), 12356295017592346385);
    assert_eq!(scores.order[..5], [558, 571, 584, 987, 1000]);
    assert_eq!(
        preflop::class_equity_vs_top(0.3)[..3],
        [0.43733335, 0.48733333, 0.48033333]
    );
}

#[test]
fn the_river_and_turn_strengths_match_v10() {
    let board = board();
    let river = river_strengths(mask_of(&board));
    assert_eq!(river[..3], [0.4070707, 0.4070707, 0.4070707]);
    assert_eq!(digest(&river), 15662847670454069211);
    let turn = exact_strengths(&board[..4]);
    assert_eq!(turn[..3], [0.4804879, 0.48096222, 0.48096222]);
    assert_eq!(digest(&turn), 16417995187559845639);
}

#[test]
fn the_flop_strengths_match_v10_exact_and_drawn() {
    let board = board();
    let flop = exact_strengths(&board[..3]);
    assert_eq!(flop[..3], [0.084497735, 0.09632939, 0.11312871]);
    assert_eq!(digest(&flop), 13093518760697573672);
    let drawn = combo_strengths(&board[..3], 48, &mut Rng::seeded(12));
    assert_eq!(drawn[..3], [0.11155399, 0.10966198, 0.11056547]);
    assert_eq!(digest(&drawn), 8805999909251447681);
}
