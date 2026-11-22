//! v10.0.2's policy knobs, at the values `Params::default()` gave it.
//!
//! The port keeps the knobs the decision reads with an empty model store. v10's EV bias tables,
//! its check lookahead, its hero image and its response net were off by default and are left
//! out.

use super::ranges::RangeParams;
use super::situation::{Situation, Street};

#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub samples: usize,
    pub initiative: f64,
    pub open_bb: f64,
    pub limper_bb: f64,
    pub short_open_bb: f64,
    pub preflop_jam_bb: f64,
    pub three_bet_ip: f64,
    pub three_bet_oop: f64,
    pub four_bet: f64,
    pub bet_sizes: Vec<f64>,
    pub temperature: f64,
    pub raise_fold_bonus: f64,
    pub fold_scale: f64,
    pub realize_weight: f64,
    pub call_margin: f64,
    pub jam_pot_ratio: f64,
    pub raise_risk: f64,
    pub range: RangeParams,
    pub reuse_deals: bool,
    pub preflop_fold_scale: f64,
    pub passive_fold_bonus: f64,
    pub three_bet_call_margin: f64,
    pub preflop_raise_risk: f64,
    pub profile_response_weight: f64,
    pub raise_gate: f64,
    pub fold_logit_shift: [f64; 3],
    pub river_jam_call_shift: f64,
    pub deep_call_shift: f64,
    pub preflop_fold_logit_shift: f64,
    pub overbet_call_shift: f64,
    pub overbet_call_slope: f64,
    pub deal_chunks: usize,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            samples: 2500,
            initiative: 0.03,
            open_bb: 2.5,
            limper_bb: 1.0,
            short_open_bb: 2.5,
            preflop_jam_bb: 30.0,
            three_bet_ip: 3.0,
            three_bet_oop: 3.8,
            four_bet: 2.3,
            bet_sizes: vec![0.33, 0.55, 0.8, 1.2],
            temperature: 0.01,
            raise_fold_bonus: 0.06,
            fold_scale: 1.0,
            realize_weight: 1.0,
            call_margin: 0.0,
            jam_pot_ratio: 2.2,
            raise_risk: 1.0,
            range: RangeParams::DEFAULT,
            reuse_deals: true,
            preflop_fold_scale: 1.0,
            passive_fold_bonus: 0.0,
            three_bet_call_margin: 0.0,
            preflop_raise_risk: 0.0,
            profile_response_weight: 0.0,
            raise_gate: 1.0,
            fold_logit_shift: [0.0; 3],
            river_jam_call_shift: 0.0,
            deep_call_shift: 0.0,
            preflop_fold_logit_shift: 0.0,
            overbet_call_shift: 0.0,
            overbet_call_slope: 0.0,
            deal_chunks: 1,
        }
    }
}

impl Params {
    /// Whether a raise is worth pricing: not into a re-raised pot postflop, nor a raised river,
    /// without the equity for it.
    pub fn raise_allowed(&self, sit: &Situation, eq: f64) -> bool {
        let street_raises = sit
            .history
            .iter()
            .filter(|h| h.street == sit.street && h.aggressive())
            .count();
        !(sit.street != Street::Preflop && street_raises >= 2 && eq < 0.55 * self.raise_gate)
            && !(sit.street == Street::River && street_raises >= 1 && eq < 0.5 * self.raise_gate)
    }
}
