//! Self-calibration's correction term at pricing time: which fitted class a candidate reads, and the
//! correction's size in chips.

use super::{DEEP_CALL_MIN_POT_BB, Params};
use std::borrow::Cow;
use sv10_engine::situation::Situation;

/// Categories whose calibration is fitted and read per pot depth (2026-10-18 design): flop and turn
/// calls realize more than predicted in small pots and far less past 400 bb, so one pooled
/// correction taxed the small pots with a penalty fitted on the deep ones. River calls miss by a
/// near-constant share of the pot at every depth and stay pooled.
pub const DEPTH_SPLIT_CATEGORIES: [&str; 2] = ["flop:call", "turn:call"];

/// Suffix of a split category's deep class (`turn:call:deep`).
pub const DEEP_SUFFIX: &str = ":deep";

/// The calibration class of a decision in `category` sized against `scale_bb` (pot + call, big
/// blinds): a split category's deep class from [`DEEP_CALL_MIN_POT_BB`], the same threshold the
/// deep-call fit tests (there against the pot before the call, here against pot + call); the
/// category itself otherwise. The store groups stored rows by the same rule (`DepthSplit`, handed
/// these constants by the bot) and a test there holds the two together.
pub fn calibration_key(category: &str, scale_bb: f64) -> Cow<'_, str> {
    if scale_bb >= DEEP_CALL_MIN_POT_BB && DEPTH_SPLIT_CATEGORIES.contains(&category) {
        Cow::Owned(format!("{category}{DEEP_SUFFIX}"))
    } else {
        Cow::Borrowed(category)
    }
}

/// The installed correction for a candidate in `category`, in chips, or `None` when its class has
/// none. Classes are read per pot depth only when `params.calibration_depth_split` says the
/// installed corrections were fitted that way, so a record made before the split prices as it did.
/// The class is keyed on `received_scale`, pot + call as the decision received it — the scale the
/// calibration row stores (`client/decide.rs`) — not on the pot left once uncallable chips go back
/// (`sit`), so a short stack facing an overshove reads the class its row is fitted in.
pub(super) fn calibration_term(category: &str, sit: &Situation, received_scale: i64, params: &Params) -> Option<f64> {
    let scale = sit.pot + sit.call_amount.max(0);
    let key = match params.calibration_depth_split {
        true => calibration_key(category, received_scale as f64 / sit.bb.max(1) as f64),
        false => Cow::Borrowed(category),
    };
    let bias = params.ev_bias.get(key.as_ref())?;
    Some(applied_bias(*bias, sit.bb, scale, params.ev_bias_pot_cap.get(key.as_ref()).copied()))
}

/// The live calibration's per-category correction in chips. Residuals scale with the pot (river
/// calls: -0.7 bb in pots up to 50 bb, -326 bb past 1,000 bb, 2026-09-24), so a penalty fitted in
/// big blinds never exceeds `pot_cap` (the category's supported per-pot residual) of `scale`
/// (pot + call, the calibration's unit); upward corrections are capped by the calibration step.
fn applied_bias(bias_bb: f64, bb: i64, scale: i64, pot_cap: Option<f64>) -> f64 {
    let chips = bias_bb * bb as f64;
    match pot_cap {
        Some(cap) if chips < 0.0 => chips.max(-cap * scale as f64),
        _ => chips,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_engine::engine::Street;
    use sv10_model::model::ModelStore;
    use sv10_rng::SeedableRng;

    #[test]
    fn a_penalty_is_capped_by_its_own_per_pot_evidence() {
        // Live 2026-09-24: river:call carried -8.8 bb from big-pot residuals while calls in pots up to
        // 100 bb realized their prediction; its per-pot residual bounds the penalty in a small pot.
        assert_eq!(applied_bias(-8.8, 20, 280, Some(0.1)), -0.1 * 280.0);
        assert_eq!(applied_bias(-8.8, 20, 100_000, Some(0.1)), -8.8 * 20.0);
        // preflop:raise:over missed by more than the pot: its own cap leaves the bb penalty whole.
        assert_eq!(applied_bias(-1.91, 20, 60, Some(1.3)), -1.91 * 20.0);
        // No per-pot evidence (records before 0207): the bb correction as fitted.
        assert_eq!(applied_bias(-8.8, 20, 280, None), -8.8 * 20.0);
        // Upward corrections keep their own small cap in the calibration step.
        assert_eq!(applied_bias(3.0, 20, 30, Some(0.1)), 60.0);
    }

    #[test]
    fn only_flop_and_turn_calls_split_and_only_from_500_bb() {
        assert_eq!(calibration_key("turn:call", 499.99), "turn:call");
        assert_eq!(calibration_key("turn:call", 500.0), "turn:call:deep");
        assert_eq!(calibration_key("flop:call", 10_000.0), "flop:call:deep");
        assert_eq!(calibration_key("river:call", 10_000.0), "river:call", "river calls stay pooled");
        assert_eq!(calibration_key("turn:raise:big", 10_000.0), "turn:raise:big");
        assert_eq!(calibration_key("preflop:call", 10_000.0), "preflop:call");
    }

    /// A turn call in a pot of `pot` chips with `call` to pay at a 20-chip blind.
    fn turn_call(pot: i64, call: i64) -> Situation {
        let mut sit = sv10_engine::situation::fixtures::uncallable_overshove();
        sit.street = Street::Turn;
        (sit.bb, sit.pot, sit.call_amount) = (20, pot, call);
        sit
    }

    #[test]
    fn a_deep_call_reads_its_class_only_when_the_fit_was_split() {
        let split = Params {
            ev_bias: [("turn:call".to_string(), -1.0), ("turn:call:deep".to_string(), -15.0)].into_iter().collect(),
            calibration_depth_split: true,
            ..Params::default()
        };
        let deep = turn_call(9_000, 1_000); // 500 bb pot + call
        let small = turn_call(1_000, 200); // 60 bb
        assert_eq!(calibration_term("turn:call", &deep, 10_000, &split), Some(-15.0 * 20.0));
        assert_eq!(calibration_term("turn:call", &small, 1_200, &split), Some(-20.0));
        let pooled = Params { calibration_depth_split: false, ..split.clone() };
        assert_eq!(calibration_term("turn:call", &deep, 10_000, &pooled), Some(-20.0), "a record without the flag prices pooled");
        assert_eq!(calibration_term("river:call", &deep, 10_000, &split), None, "no class, no correction");
    }

    #[test]
    fn a_zero_big_blind_prices_without_dividing_by_zero() {
        let mut sit = turn_call(9_000, 1_000);
        sit.bb = 0;
        let p = Params {
            ev_bias: [("turn:call:deep".to_string(), -15.0)].into_iter().collect(),
            calibration_depth_split: true,
            ..Params::default()
        };
        assert!(calibration_term("turn:call", &sit, 10_000, &p).is_some_and(f64::is_finite));
    }

    #[test]
    fn the_decision_prices_a_deep_turn_call_with_its_own_class() {
        let mut turn = sv10_engine::situation::fixtures::uncallable_overshove();
        turn.street = Street::Turn;
        turn.bb = 10;
        // After the uncallable chips go back, pot + call is about 922 bb: the deep class.
        let adj = turn.without_uncallable();
        assert!((adj.pot + adj.call_amount) as f64 / adj.bb as f64 >= DEEP_CALL_MIN_POT_BB);
        let call_bias = |flag: bool| {
            let params = Params {
                ev_bias: [("turn:call".to_string(), -1.0), ("turn:call:deep".to_string(), -15.0)].into_iter().collect(),
                calibration_depth_split: flag,
                ..Params::default()
            };
            let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(7);
            let d = crate::policy::decide_with(&turn, &ModelStore::default(), &params, None, &mut rng);
            d.candidates.iter().find(|c| c.action == "call").expect("a call is offered").bias
        };
        assert_eq!(call_bias(true), -15.0 * 10.0, "the deep class");
        assert_eq!(call_bias(false), -10.0, "a record without the split prices as before");
    }

    /// The live overshove (bb 20): the calibration row stores pot + call as the decision received it,
    /// 45,607 bb, so it is fitted as `turn:call:deep`; pricing sees 461 bb once the uncallable chips go
    /// back. The class priced must be the class fitted, so the key reads the received pot.
    #[test]
    fn an_overshove_call_reads_the_class_its_row_is_stored_under() {
        let mut turn = sv10_engine::situation::fixtures::uncallable_overshove();
        turn.street = Street::Turn;
        let stored_bb = (turn.pot + turn.call_amount) as f64 / turn.bb as f64;
        let adj = turn.without_uncallable();
        assert!(stored_bb >= DEEP_CALL_MIN_POT_BB, "stored as deep: {stored_bb}");
        assert!(((adj.pot + adj.call_amount) as f64 / adj.bb as f64) < DEEP_CALL_MIN_POT_BB, "priced pot is shallow");
        let params = Params {
            ev_bias: [("turn:call".to_string(), -1.0), ("turn:call:deep".to_string(), -15.0)].into_iter().collect(),
            calibration_depth_split: true,
            ..Params::default()
        };
        let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(7);
        let d = crate::policy::decide_with(&turn, &ModelStore::default(), &params, None, &mut rng);
        let call = d.candidates.iter().find(|c| c.action == "call").expect("a call is offered");
        assert_eq!(call.bias, -15.0 * 20.0, "the class its calibration row is stored under");
    }
}
