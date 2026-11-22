//! v10.0.2's read of a player: tendencies as rates.
//!
//! The sparring opponent plays with an empty model store, as the parity harness ran it, so every
//! seat's profile, hero's image of itself included, is the population default: each rate the
//! prior, rated through an empty own counter over an empty population one, in v10's f32
//! arithmetic. A store fitted on live play is the operator's data and stays out of the repo.

use super::situation::{Position, Street};

/// The population's prior rates.
pub mod defaults {
    pub const VPIP: f32 = 0.32;
    pub const PFR: f32 = 0.18;
    pub const OPEN_RAISE: f32 = 0.20;
    pub const LIMP: f32 = 0.10;
    pub const THREE_BET: f32 = 0.08;
    pub const CALL_OPEN: f32 = 0.22;
    pub const FOLD_TO_3BET: f32 = 0.50;
    pub const FOUR_BET: f32 = 0.08;
    pub const FOLD_TO_4BET: f32 = 0.45;
    pub const BET_FIRST: [f32; 3] = [0.38, 0.32, 0.30];
    pub const FOLD_VS_BET: [f32; 3] = [0.45, 0.42, 0.45];
    pub const RAISE_VS_BET: [f32; 3] = [0.10, 0.08, 0.07];
    pub const FOLD_VS_SIZE: [f32; 3] = [0.36, 0.45, 0.52];
    pub const CBET: f32 = 0.60;
    pub const FOLD_TO_CBET: f32 = 0.45;
    pub const WTSD: f32 = 0.30;
    pub const RIVER_BLUFF: f32 = 0.30;
}

const W_PRE: f32 = 12.0;
const W_POST: f32 = 10.0;
const W_POPULATION: f32 = 150.0;

/// An empty counter's rate: the prior, by v10's `(hit + prior * weight) / (opp + weight)`.
fn rate(prior: f32, weight: f32) -> f32 {
    (0.0 + prior * weight) / (0.0 + weight)
}

/// A rate rated over the population's, both empty.
fn pr(prior: f32, weight: f32) -> f32 {
    rate(rate(prior, W_POPULATION), weight)
}

fn arr(priors: [f32; 3], weight: f32) -> [f32; 3] {
    priors.map(|prior| pr(prior, weight))
}

#[derive(Clone, PartialEq, Debug)]
pub struct Profile {
    pub vpip: f32,
    pub pfr: f32,
    pub open_raise: f32,
    pub limp: f32,
    pub three_bet: f32,
    pub call_open: f32,
    pub fold_to_3bet: f32,
    pub four_bet: f32,
    pub fold_to_4bet: f32,
    pub bet_first: [f32; 3],
    pub fold_vs_bet: [f32; 3],
    pub raise_vs_bet: [f32; 3],
    pub fold_vs_size: [f32; 3],
    pub cbet: f32,
    pub fold_to_cbet: f32,
    pub wtsd: f32,
    pub river_bluff: f32,
    pub open_pos: [f32; 3],
    pub vpip_pos: [f32; 3],
    pub fold_logit_offset: f32,
    pub size_tell: f32,
}

impl Profile {
    /// The profile of a player nothing is known about.
    pub fn population_default() -> Self {
        use defaults as d;
        let open = pr(d::OPEN_RAISE, W_PRE);
        let vpip = pr(d::VPIP, W_PRE);
        let open_scale = [0.7f32, 1.45, 1.0];
        let vpip_scale = [0.8f32, 1.3, 1.05];
        Self {
            vpip,
            pfr: pr(d::PFR, W_PRE),
            open_raise: open,
            limp: pr(d::LIMP, W_PRE),
            three_bet: pr(d::THREE_BET, W_PRE),
            call_open: pr(d::CALL_OPEN, W_PRE),
            fold_to_3bet: pr(d::FOLD_TO_3BET, 6.0),
            four_bet: pr(d::FOUR_BET, 6.0),
            fold_to_4bet: pr(d::FOLD_TO_4BET, 5.0),
            bet_first: arr(d::BET_FIRST, W_POST),
            fold_vs_bet: arr(d::FOLD_VS_BET, W_POST),
            raise_vs_bet: arr(d::RAISE_VS_BET, W_POST),
            fold_vs_size: arr(d::FOLD_VS_SIZE, W_POST),
            cbet: pr(d::CBET, 8.0),
            fold_to_cbet: pr(d::FOLD_TO_CBET, 8.0),
            wtsd: pr(d::WTSD, W_POST),
            river_bluff: pr(d::RIVER_BLUFF, 5.0),
            open_pos: [0, 1, 2].map(|g| rate((open * open_scale[g]).clamp(0.01, 0.95), 15.0)),
            vpip_pos: [0, 1, 2].map(|g| rate((vpip * vpip_scale[g]).clamp(0.02, 0.98), 15.0)),
            fold_logit_offset: 0.0,
            size_tell: 0.0,
        }
    }

    /// The chance of a fold to a bet of `frac` of the pot.
    pub fn fold_to_bet(&self, street: Street, frac: f64) -> f64 {
        let st = street.index().saturating_sub(1).min(2);
        let base_cont = 1.0 - f64::from(self.fold_vs_bet[st]);
        let curve = |x: f64| {
            ((1.0 + 2.0 * 0.66) / (1.0 + 2.0 * x.max(0.05)))
                .powf(0.55)
                .min(1.6)
        };
        let b = size_bucket(frac);
        let learned_cont =
            (1.0 - f64::from(self.fold_vs_size[b])) * curve(frac) / curve([0.33, 0.66, 1.2][b]);
        let cont = 0.6 * (base_cont * curve(frac)) + 0.4 * learned_cont;
        (1.0 - cont).clamp(0.02, 0.97)
    }
}

/// A bet's size class: under 0.45 of the pot, under 0.85, or more.
pub fn size_bucket(frac: f64) -> usize {
    if frac < 0.45 {
        0
    } else if frac < 0.85 {
        1
    } else {
        2
    }
}

/// The positional group: early, late, or the blinds.
pub fn position_group(p: Position) -> usize {
    match p {
        Position::Early | Position::Middle => 0,
        Position::Cutoff | Position::Button => 1,
        Position::SmallBlind | Position::BigBlind => 2,
    }
}
