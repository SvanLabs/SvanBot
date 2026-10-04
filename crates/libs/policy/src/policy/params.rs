//! Strategy parameters: the knobs the learner tunes and promotes.

use serde::{Deserialize, Serialize};
use sv10_engine::engine::Street;
use sv10_engine::situation::Situation;
use sv10_model::model::aggressive;

/// Strategy parameters: the knobs the learner tunes and promotes (serialized as the live champion).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    /// Monte Carlo samples per equity estimate.
    pub samples: usize,
    /// Realization bonus for the aggressor when called.
    pub initiative: f64,
    /// Preflop open size in big blinds.
    pub open_bb: f64,
    /// Big blinds added to an open per limper.
    pub limper_bb: f64,
    /// Open size in big blinds when the effective stack is at or below `preflop_jam_bb` (0171);
    /// the default equals the default `open_bb`, so it changes nothing until the learner moves it.
    pub short_open_bb: f64,
    /// Effective stack in big blinds at or below which a preflop jam is considered and
    /// `short_open_bb` applies (was a fixed 30 before 0171).
    pub preflop_jam_bb: f64,
    /// 3-bet size as a multiple of the raise faced, in position.
    pub three_bet_ip: f64,
    /// 3-bet size as a multiple of the raise faced, out of position.
    pub three_bet_oop: f64,
    /// 4-bet-and-later size as a multiple of the current bet.
    pub four_bet: f64,
    /// Postflop bet sizes as pot fractions.
    pub bet_sizes: Vec<f64>,
    /// Mixing temperature as a fraction of the pot.
    pub temperature: f64,
    /// Fold-probability scale for players facing a raise instead of a bet.
    pub raise_fold_bonus: f64,
    /// Multiplier on every modeled fold probability (<1 = assume more calls).
    pub fold_scale: f64,
    /// Strength of the equity-realization penalty (1 = model default, 0 = realize raw equity).
    pub realize_weight: f64,
    /// Minimum EV (fraction of the pot) a call must show over folding.
    pub call_margin: f64,
    /// Jam allowed when the all-in is at most this many pots (postflop).
    pub jam_pot_ratio: f64,
    /// Weight of the raise-back branch when pricing postflop bets (0 = treat all continues as calls).
    pub raise_risk: f64,
    /// Learned EV corrections in big blinds per spot category (self-calibration).
    pub ev_bias: std::collections::HashMap<String, f64>,
    /// Largest penalty per category as a fraction of pot + call: the per-pot residual the
    /// evidence supports (self-calibration, 0207). A category without one is not capped.
    pub ev_bias_pot_cap: std::collections::HashMap<String, f64>,
    /// Range-reconstruction shape constants (fitted to showdowns by `calibrate`).
    pub range: sv10_model::oprange::RangeParams,
    /// Price every candidate on one shared, importance-reweighted set of Monte Carlo deals
    /// (common random numbers) instead of fresh samples per candidate.
    pub reuse_deals: bool,
    /// Flop/turn board strengths exact (precomputed tables) instead of the Monte Carlo blend (0063).
    pub exact_strengths: bool,
    /// Extra multiplier on preflop fold estimates (steals, 3-bets); 1.0 = the global `fold_scale` only.
    pub preflop_fold_scale: f64,
    /// Added fold probability when every live opponent has only checked on every postflop street so
    /// far (passive lines that lose at showdown, 0046).
    pub passive_fold_bonus: f64,
    /// Extra call margin, as a share of the pot, when facing a preflop 3-bet or more.
    pub three_bet_call_margin: f64,
    /// Weight of the preflop re-raise branch (villain 3-bets / 4-bets hero's raise, 0100); 0 = ignore.
    pub preflop_raise_risk: f64,
    /// Weight of profile-specific responses the stat fold model otherwise ignores: fold-to-c-bet on
    /// flop c-bets and the limper's VPIP facing an iso-raise (0101); 0 = generic curves.
    pub profile_response_weight: f64,
    /// Weight of the check lookahead (an opponent behind bets; hero calls or folds against the
    /// narrowed betting range, 0103) in the check value; 0 = one-street realization only.
    pub check_lookahead: f64,
    /// Multiplier on the equity floor below which a raise is banned once the street has already been
    /// raised (0157's no-bluff-raise-wars guard): 1.0 is the shipped floor — 0.55 with two raises on a
    /// postflop street, 0.5 on the river after one. 0 frees the raise at any equity, which is the
    /// river ceiling the Phase 3 strength work measures ([`Params::raise_allowed`]).
    pub raise_gate: f64,
    /// Weight of the image our own observed play gives the pricing (0321): the range opponents read
    /// us for (`ModelStore::hero_seen_view`) is built from the fleet's tallies for our seat instead
    /// of the population rates, scaled by this. 0 = the population view, as before 0321.
    pub hero_image: f64,
    /// Live-fitted logit shift on each postflop street's final fold estimate (flop, turn, river;
    /// 0156): opponents' realized folds against our heads-up bets are fitted on live hands and the
    /// shift is used only for streets where it beats the uncorrected prediction on held-out hands.
    /// Local like `range` and `ev_bias`: never promoted, 0 in the learner and the golden snapshot.
    pub fold_logit_shift: [f64; 3],
    /// Live-fitted equity shift subtracted from a river call against an all-in (either side all-in;
    /// 0159): those calls realized 0.068 less equity than estimated against the shown hands. Fitted
    /// on older hands and used only while it saves chips on newer ones. Local like
    /// `fold_logit_shift`: never promoted, 0 in the learner and the golden snapshot.
    pub river_jam_call_shift: f64,
    /// Live-fitted equity shift subtracted from any call against an all-in once the pot reaches
    /// [`crate::policy::DEEP_CALL_MIN_POT_BB`] (2026-09-23): calls in 500+ bb pots over-estimated
    /// equity by 0.154 ± 0.080 while smaller pots were calibrated. Fitted on older hands and used only
    /// while it wins on newer ones; local like `river_jam_call_shift`, never promoted.
    pub deep_call_shift: f64,
    /// Live-fitted logit shift on the everyone-folds estimate of our preflop raises (2026-09-23:
    /// 36,080 raises predicted about twice the folds that happened). Fitted from real outcomes and
    /// installed only on a held-out log-loss win; local, never promoted, 0 in the learner and golden.
    pub preflop_fold_logit_shift: f64,
    /// Live-fitted equity shift subtracted from a call against an all-in whose bet was at least
    /// [`crate::policy::OVERBET_CALL_MIN_RATIO`] times the pot before it (2026-09-23: against 4x+ pot
    /// shoves our estimate was 0.623 vs 0.297 exact, n 36). Local, never promoted.
    pub overbet_call_shift: f64,
    /// Live-fitted slope of the size-scaled overbet call shift (0233): against an all-in of `r` times
    /// the pot (r ≥ [`crate::policy::OVERBET_CALL_MIN_RATIO`]) the estimate is lowered by
    /// `slope × overbet_size_x(r)`, at most [`crate::policy::OVERBET_SLOPE_CAP`]. The flat shift pooled
    /// 1.5–4x bets (over-estimate +0.115) with 4x+ shoves (+0.291); the gap grows with the size.
    /// Local, never promoted, 0 in the learner and the golden snapshot.
    pub overbet_call_slope: f64,
    /// Parallel chunks for the shared Monte Carlo deals of one decision (`SharedDeals::new_parallel`);
    /// 1 = the single-stream path the learner and the golden snapshot use. Live play sets it to the core
    /// count with a larger `samples` budget; like `samples`, it is local and never promoted.
    pub deal_chunks: usize,
    /// Price unequal all-in pot tiers with their eligible ranges after a raise. Off until a paired
    /// strength result supports replacing the existing one-pot approximation (#665).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tiered_all_in_fold_pricing: bool,
    /// Share of each player still to act behind a preflop raise that is priced as continuing: their
    /// `call_open + three_bet` rate (capped at 0.8) times this takes that share of hero's equity away
    /// (#746). 0.5 is the long-standing flat factor, so a station and a nit differ only by their rates.
    pub preflop_discount: f64,
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
            ev_bias: std::collections::HashMap::new(),
            ev_bias_pot_cap: std::collections::HashMap::new(),
            range: sv10_model::oprange::RangeParams::DEFAULT,
            reuse_deals: true,
            exact_strengths: true,
            preflop_fold_scale: 1.0,
            passive_fold_bonus: 0.0,
            three_bet_call_margin: 0.0,
            preflop_raise_risk: 0.0,
            profile_response_weight: 0.0,
            check_lookahead: 0.0,
            raise_gate: 1.0,
            hero_image: 0.0,
            fold_logit_shift: [0.0; 3],
            river_jam_call_shift: 0.0,
            deep_call_shift: 0.0,
            preflop_fold_logit_shift: 0.0,
            overbet_call_shift: 0.0,
            overbet_call_slope: 0.0,
            deal_chunks: 1,
            tiered_all_in_fold_pricing: false,
            preflop_discount: 0.5,
        }
    }
}

impl Params {
    /// Whether hero may raise on this street at all (0157): no bluff raise wars — once a postflop
    /// street has two raises, or the river one, only a hand above the equity floor may raise. Both
    /// floors are scaled by [`Params::raise_gate`], so 1.0 is the shipped 0.55 / 0.5 and 0 removes the
    /// gate; the river ceiling the Phase 3 strength work names is this predicate, and it lives next to
    /// the field that scales it rather than in the (500-line-frozen) `policy/mod.rs`.
    pub fn raise_allowed(&self, sit: &Situation, eq: f64) -> bool {
        let street_raises = sit.history.iter().filter(|h| h.street == sit.street && aggressive(h)).count();
        !(sit.street != Street::Preflop && street_raises >= 2 && eq < 0.55 * self.raise_gate)
            && !(sit.street == Street::River && street_raises >= 1 && eq < 0.5 * self.raise_gate)
    }

    /// Adopt a learner-promoted champion: every strategy knob comes from `promoted`, while the
    /// Monte Carlo budget (`samples`, `deal_chunks`) and the live-fitted state (`ev_bias`, `ev_bias_pot_cap`,
    /// `range`, `fold_logit_shift`, `river_jam_call_shift`, `deep_call_shift`, `preflop_fold_logit_shift`, `overbet_call_shift`, `overbet_call_slope`) stay local to this process. The promotion contract lives here, next to the
    /// fields it governs, instead of at the call site that applies it.
    pub fn adopt_promoted(&mut self, promoted: Params) {
        let samples = self.samples;
        let deal_chunks = self.deal_chunks;
        let ev_bias = std::mem::take(&mut self.ev_bias);
        let ev_bias_pot_cap = std::mem::take(&mut self.ev_bias_pot_cap);
        let range = self.range;
        let fold_logit_shift = self.fold_logit_shift;
        let river_jam_call_shift = self.river_jam_call_shift;
        let deep_call_shift = self.deep_call_shift;
        let preflop_fold_logit_shift = self.preflop_fold_logit_shift;
        let overbet_call_shift = self.overbet_call_shift;
        let overbet_call_slope = self.overbet_call_slope;
        *self = promoted;
        self.samples = samples;
        self.deal_chunks = deal_chunks;
        self.ev_bias = ev_bias;
        self.ev_bias_pot_cap = ev_bias_pot_cap;
        self.range = range;
        self.fold_logit_shift = fold_logit_shift;
        self.river_jam_call_shift = river_jam_call_shift;
        self.deep_call_shift = deep_call_shift;
        self.preflop_fold_logit_shift = preflop_fold_logit_shift;
        self.overbet_call_shift = overbet_call_shift;
        self.overbet_call_slope = overbet_call_slope;
    }

    /// Adopt a recorded decision's own state: its Monte Carlo budget (`samples`, `deal_chunks`) and
    /// its live-fitted set (`ev_bias`, `ev_bias_pot_cap`, `range`, `fold_logit_shift`,
    /// `river_jam_call_shift`, `deep_call_shift`, `preflop_fold_logit_shift`, `overbet_call_shift`,
    /// `overbet_call_slope`) — exactly the fields `adopt_promoted` keeps local — leaving every
    /// strategy knob as it is.
    ///
    /// A re-solve of a recorded action adopts this so that the only thing it varies against that
    /// action is the knob set under test (0358): the prices the decision was made at, and the budget
    /// and deal decomposition it was solved with, are the record's. The list is defined here once,
    /// next to `adopt_promoted`'s, so an instrument cannot carry a private copy of it (`analyst.drift`
    /// and `review replay --current` both take it; 0366, 0359).
    pub fn adopt_recorded_local(&mut self, recorded: &Params) {
        self.samples = recorded.samples;
        self.deal_chunks = recorded.deal_chunks;
        self.ev_bias = recorded.ev_bias.clone();
        self.ev_bias_pot_cap = recorded.ev_bias_pot_cap.clone();
        self.range = recorded.range;
        self.fold_logit_shift = recorded.fold_logit_shift;
        self.river_jam_call_shift = recorded.river_jam_call_shift;
        self.deep_call_shift = recorded.deep_call_shift;
        self.preflop_fold_logit_shift = recorded.preflop_fold_logit_shift;
        self.overbet_call_shift = recorded.overbet_call_shift;
        self.overbet_call_slope = recorded.overbet_call_slope;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_raise_gate_floors_default_to_the_shipped_guard_and_scale_with_the_knob() {
        // Phase 3 river ceiling: the two floors in "no bluff raise wars" were literals, so "on the
        // river, once anyone has raised, a hand under 0.5 equity may never raise" was a fixed ceiling
        // rather than one the learner can price. 1.0 must be the shipped guard and 0 must free it.
        use sv10_engine::engine::{ActionKind, ActionRecord};
        let raise = |street| ActionRecord {
            seat: 1,
            street,
            kind: ActionKind::Raise,
            to: 200,
            pot_before: 100,
            to_call_before: 0,
            bet_before: 0,
            full_raise: true,
            think_ms: None,
            street_open: false,
        };
        let river = sv10_engine::situation::fixtures::uncallable_overshove();
        assert_eq!(river.street, Street::River);
        let shipped = Params::default();
        assert_eq!(shipped.raise_gate, 1.0, "the shipped guard is the field's default");
        assert!(shipped.raise_allowed(&river, 0.0), "no raise on the street yet: the river clause cannot fire");
        let mut faced = river.clone();
        faced.history = vec![raise(Street::River)];
        assert!(!shipped.raise_allowed(&faced, 0.49), "the shipped river floor bans a raise under 0.5");
        assert!(shipped.raise_allowed(&faced, 0.51), "and leaves one above it alone");
        assert!(Params { raise_gate: 0.0, ..shipped.clone() }.raise_allowed(&faced, 0.0), "a zeroed gate frees it");
        assert!(Params { raise_gate: 0.98, ..shipped.clone() }.raise_allowed(&faced, 0.49), "a relaxed one at its floor");
        let mut two = river.clone();
        two.street = Street::Turn;
        two.history = vec![raise(Street::Turn), raise(Street::Turn)];
        assert!(!shipped.raise_allowed(&two, 0.54), "two raises on a postflop street hold the 0.55 floor");
        assert!(Params { raise_gate: 0.0, ..shipped.clone() }.raise_allowed(&two, 0.0));
        let mut preflop = river.clone();
        preflop.street = Street::Preflop;
        preflop.history = vec![raise(Street::Preflop), raise(Street::Preflop)];
        assert!(shipped.raise_allowed(&preflop, 0.0), "the guard is postflop only");
    }

    #[test]
    fn promotion_adopts_knobs_but_keeps_local_budget_and_fits() {
        let mut live = Params {
            samples: 9999,
            deal_chunks: 8,
            ev_bias: [("river:call".to_string(), 1.5)].into_iter().collect(),
            fold_logit_shift: [0.2, -0.5, -0.7],
            river_jam_call_shift: 0.05,
            deep_call_shift: 0.15,
            preflop_fold_logit_shift: -1.2,
            overbet_call_shift: 0.3,
            overbet_call_slope: 0.06,
            ..Params::default()
        };
        let promoted = Params {
            call_margin: 0.05,
            samples: 100,
            ev_bias: [("river:call".to_string(), -9.0)].into_iter().collect(),
            ..Params::default()
        };
        live.adopt_promoted(promoted);
        assert_eq!(live.call_margin, 0.05);
        assert_eq!((live.samples, live.deal_chunks), (9999, 8));
        assert_eq!(live.ev_bias["river:call"], 1.5);
        assert_eq!(live.range, sv10_model::oprange::RangeParams::DEFAULT);
        assert_eq!(live.fold_logit_shift, [0.2, -0.5, -0.7], "the live fold fit is never promoted over");
        assert_eq!(live.river_jam_call_shift, 0.05, "the live river-jam fit is never promoted over");
        assert_eq!(live.deep_call_shift, 0.15, "the live deep-call fit is never promoted over");
        assert_eq!(live.preflop_fold_logit_shift, -1.2, "the live preflop fold fit is never promoted over");
        assert_eq!(live.overbet_call_shift, 0.3, "the live overbet call fit is never promoted over");
    }

    #[test]
    fn a_re_solve_adopts_the_recorded_budget_and_fits_but_keeps_its_own_knobs() {
        let recorded = Params {
            samples: 9_000,
            deal_chunks: 8,
            ev_bias: [("river:call".to_string(), 1.5)].into_iter().collect(),
            ev_bias_pot_cap: [("river:call".to_string(), 0.2)].into_iter().collect(),
            range: sv10_model::oprange::RangeParams { soft_width: 0.31, ..Default::default() },
            fold_logit_shift: [0.2, 0.0, -0.7],
            preflop_fold_logit_shift: -1.4,
            river_jam_call_shift: 0.07,
            deep_call_shift: 0.15,
            overbet_call_shift: 0.3,
            overbet_call_slope: 0.06,
            ..Params::default()
        };
        let mut knobs = Params { call_margin: 0.05, initiative: 0.09, samples: 2_500, ..Params::default() };
        knobs.adopt_recorded_local(&recorded);
        // The budget and the live-fitted set are the record's, field for field (0358's convention:
        // a re-solve of a recorded action varies only the knobs under test).
        assert_eq!((knobs.samples, knobs.deal_chunks), (9_000, 8));
        assert_eq!(knobs.ev_bias, recorded.ev_bias);
        assert_eq!(knobs.ev_bias_pot_cap, recorded.ev_bias_pot_cap);
        assert_eq!(knobs.range, recorded.range);
        assert_eq!(knobs.fold_logit_shift, [0.2, 0.0, -0.7]);
        assert_eq!(knobs.preflop_fold_logit_shift, -1.4);
        assert_eq!(knobs.river_jam_call_shift, 0.07);
        assert_eq!(knobs.deep_call_shift, 0.15);
        assert_eq!(knobs.overbet_call_shift, 0.3);
        assert_eq!(knobs.overbet_call_slope, 0.06);
        // And the strategy knobs are untouched.
        assert_eq!((knobs.call_margin, knobs.initiative), (0.05, 0.09), "a strategy knob moved");
        assert_eq!(knobs.bet_sizes, Params::default().bet_sizes);
    }
}
