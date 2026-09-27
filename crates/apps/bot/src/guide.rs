//! Starting-hand guide derived from the live policy: for each hand class and
//! position, does the policy (with the live opponent models, parameters and
//! neural response model) open-raise when folded to it at 100bb? The BB tab
//! shows hands defended (call or 3-bet) against a button open.

use serde_json::{Value, json};
use sv10_core::cards::{Card, RANK_CHARS};
use sv10_core::engine::{Action, Hand};
use sv10_core::model::ModelStore;
use sv10_core::nn::Mlp;
use sv10_core::policy::{Params, decide_with};
use sv10_core::preflop;
use sv10_core::range::{combos, hand_class};
use sv10_core::situation::Situation;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

static GUIDE: parking_lot::RwLock<Option<Value>> = parking_lot::RwLock::new(None);

pub fn cached() -> Option<Value> {
    GUIDE.read().clone()
}

pub fn refresh(models: &ModelStore, params: &Params, nn: Option<&Mlp>) {
    let v = compute(models, params, nn);
    *GUIDE.write() = Some(v);
}

/// Hand classes in the conventional 13x13 matrix order: rows and columns
/// A..2, pairs on the diagonal, suited above it, offsuit below.
fn matrix_order() -> Vec<(String, [Card; 2], bool, bool)> {
    let mut out = Vec::new();
    for row in (0..13u8).rev() {
        for col in (0..13u8).rev() {
            let (hi, lo) = if row >= col { (row, col) } else { (col, row) };
            let r = |x: u8| RANK_CHARS[x as usize] as char;
            if row == col {
                out.push((format!("{}{}", r(hi), r(lo)), [Card::new(hi, 0), Card::new(lo, 1)], true, false));
            } else if col < row {
                // Right of the diagonal (lower rank column): suited.
                out.push((format!("{}{}s", r(hi), r(lo)), [Card::new(hi, 0), Card::new(lo, 0)], false, true));
            } else {
                out.push((format!("{}{}o", r(hi), r(lo)), [Card::new(hi, 0), Card::new(lo, 1)], false, false));
            }
        }
    }
    out
}

/// Up to three concrete combos of a class, with different suit patterns.
fn class_combos(sample: [Card; 2]) -> Vec<[Card; 2]> {
    let class = hand_class(sample[0], sample[1]);
    combos().cards.iter().filter(|&&(a, b)| hand_class(a, b) == class).step_by(2).take(3).map(|&(a, b)| [a, b]).collect()
}

fn spot(hole: [Card; 2], hero: usize, actions: &[Action]) -> Option<Hand> {
    let mut used = hole[0].bit() | hole[1].bit();
    let mut next = || {
        let mut c = 51u8;
        while used & (1 << c) != 0 {
            c -= 1;
        }
        used |= 1 << c;
        Card(c)
    };
    let mut seats = Vec::new();
    for i in 0..6 {
        let h = if i == hero { hole } else { [next(), next()] };
        seats.push(Hand::seat_state(2000, h));
    }
    let runout = [next(), next(), next(), next(), next()];
    let mut hand = Hand::with_cards(seats, runout, 0, 10, 20);
    for &a in actions {
        hand.apply(a).ok()?;
    }
    (hand.actor() == Some(hero)).then_some(hand)
}

pub fn compute(models: &ModelStore, params: &Params, nn: Option<&Mlp>) -> Value {
    // ~3,000 decisions: the simulation budget on one thread is plenty for open/fold margins (the live
    // parameters carry the 16x parallel budget, which would make the guide take minutes).
    let params = &Params { samples: (params.samples / sv10_core::hardware::LIVE_SAMPLE_FACTOR).max(600), deal_chunks: 1, ..params.clone() };
    let names: Vec<String> = (0..6).map(|i| format!("\u{0}guide{i}")).collect();
    let table = preflop::table();
    let mut rows = Vec::new();
    for (label, sample, _pair, _suited) in matrix_order() {
        let class_combos = class_combos(sample);
        let mut open = Vec::new();
        // Button at seat 0: UTG=3, HJ=4, CO=5, BTN=0, SB=1 act first-in after the folds before them.
        for (folds, hero) in [(0usize, 3usize), (1, 4), (2, 5), (3, 0), (4, 1)] {
            let mut margin = 0.0;
            let mut n = 0.0;
            for (k, hole) in class_combos.iter().enumerate() {
                let Some(hand) = spot(*hole, hero, &vec![Action::Fold; folds]) else { continue };
                let sit = Situation::from_hand(&hand, hero, &names);
                let mut rng = SmallRng::seed_from_u64(hero as u64 * 1000 + k as u64);
                let d = decide_with(&sit, models, params, nn, &mut rng);
                let best_raise =
                    d.candidates.iter().filter(|c| c.action == "raise" || c.action == "all_in").map(|c| c.ev).fold(f64::MIN, f64::max);
                let passive = d
                    .candidates
                    .iter()
                    .filter(|c| c.action == "call" || c.action == "fold" || c.action == "check")
                    .map(|c| c.ev)
                    .fold(0.0, f64::max);
                margin += best_raise - passive;
                n += 1.0;
            }
            open.push(n > 0.0 && margin / n > 0.0);
        }
        // Big blind: defend (call or 3-bet) against a 2.5bb button open with the small blind folded.
        let mut defend = 0.0;
        let mut n = 0.0;
        for (k, hole) in class_combos.iter().enumerate() {
            let actions = [Action::Fold, Action::Fold, Action::Fold, Action::RaiseTo(50), Action::Fold];
            let Some(hand) = spot(*hole, 2, &actions) else { continue };
            let sit = Situation::from_hand(&hand, 2, &names);
            let mut rng = SmallRng::seed_from_u64(2000 + k as u64);
            let d = decide_with(&sit, models, params, nn, &mut rng);
            defend += d.candidates.iter().filter(|c| c.action != "fold").map(|c| c.ev).fold(f64::MIN, f64::max);
            n += 1.0;
        }
        open.push(n > 0.0 && defend / n > 0.0);
        let class = hand_class(sample[0], sample[1]);
        rows.push(json!({"hand": label, "score": 1.0 - table.percentile[class], "open": open}));
    }
    json!(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_is_conventional() {
        let m: Vec<String> = matrix_order().into_iter().map(|x| x.0).collect();
        assert_eq!(m.len(), 169);
        assert_eq!(&m[..3], &["AA", "AKs", "AQs"]);
        assert_eq!(m[13], "AKo");
        assert_eq!(m[14], "KK");
        assert_eq!(m[168], "22");
        let unique: std::collections::HashSet<&String> = m.iter().collect();
        assert_eq!(unique.len(), 169);
    }
}
