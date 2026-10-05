//! The champion's tunable knobs, defined once: what the search may try, and what the dashboard draws.
//!
//! #322: the Champion profile carried its own hand-written copy of this list — a label, a min, a max
//! and a sentence per knob, typed into `web/src/training.tsx`. A knob the search gained did not reach
//! the panel, and a bound the search widened left the panel drawing against the old one, so the
//! Experiments panel could evaluate `call_margin -0.045 -> -0.050` for weeks beside a bar whose track
//! said the knob stopped at -0.10.
//!
//! So the bounds live here, `learner::pool` clamps its proposals with them and `api::state` serves
//! them: a knob added to this table is searched and drawn, and one that is not in it is neither.
//!
//! A bound is only a limit on what the gate gets to test (LESSONS 29): a champion sitting on one is
//! an untested direction, not a forbidden value, which is why widening one is a normal change.

use sv10_core::policy::Params;

/// One tunable knob: what the search may move it to, and what the dashboard needs to draw it.
pub struct Knob {
    /// The field name, as `Params` spells it and the payload keys it.
    pub key: &'static str,
    /// The name the dashboard prints.
    pub label: &'static str,
    /// Lowest value the search may propose.
    pub min: f64,
    /// Highest value the search may propose.
    pub max: f64,
    /// Decimal places the value is printed with; a knob whose whole range is under a tenth needs more
    /// than two (`temperature` spans 0.002..0.05, and `0.00` is not a reading).
    pub decimals: u8,
    /// What the knob does, in one sentence, for the operator.
    pub description: &'static str,
    read: fn(&Params) -> f64,
}

impl Knob {
    /// This knob's value in `p`.
    pub fn get(&self, p: &Params) -> f64 {
        (self.read)(p)
    }
}

/// The knob with this key, if it is one.
pub fn find(key: &str) -> Option<&'static Knob> {
    KNOBS.iter().find(|k| k.key == key)
}

/// A knob's value in `p`, or `None` when `key` is not a knob.
pub fn get(key: &str, p: &Params) -> Option<f64> {
    find(key).map(|k| k.get(p))
}

/// `new` held inside the knob's bounds, in the direction it moved: a step up stops at `max` and a
/// step down at `min`. Only the travelled bound applies, so a champion already outside the far side
/// of the range is never dragged across the table by a step the other way. A key that is not a knob
/// (`bet_size_set` proposes a whole size list, not a number) passes through.
pub fn bound(key: &str, old: f64, new: f64) -> f64 {
    match find(key) {
        Some(k) if new > old => new.min(k.max),
        Some(k) if new < old => new.max(k.min),
        _ => new,
    }
}

/// The postflop bet-size scale as a multiple of the champion's set: the 0.55-pot size is index 1 of
/// the four-size set and index 2 of the seven-size set (0170), so it names the same bet in both.
fn bet_size_scale(p: &Params) -> f64 {
    p.bet_sizes.get(if p.bet_sizes.len() == 7 { 2 } else { 1 }).copied().unwrap_or(0.55) / 0.55
}

/// Every knob the learner searches and the dashboard draws, in the order the profile shows them.
pub const KNOBS: &[Knob] = &[
    Knob {
        key: "fold_scale",
        label: "Fold-probability scale",
        min: 0.4,
        max: 1.2,
        decimals: 2,
        description: "Multiplier on every modeled opponent fold probability. Below 1 assumes opponents call more often than their stats suggest.",
        read: |p| p.fold_scale,
    },
    Knob {
        key: "preflop_fold_scale",
        label: "Preflop fold scale",
        min: 0.5,
        max: 1.8,
        decimals: 2,
        description: "Extra multiplier on preflop fold estimates (steals and 3-bets) on top of the global scale.",
        read: |p| p.preflop_fold_scale,
    },
    Knob {
        key: "realize_weight",
        label: "Equity realization weight",
        min: 0.0,
        max: 1.6,
        decimals: 2,
        description: "How strongly equity is discounted or boosted for the share a hand actually realizes when play continues.",
        read: |p| p.realize_weight,
    },
    Knob {
        key: "passive_fold_bonus",
        label: "Passive-line fold bonus",
        min: -0.25,
        max: 0.25,
        decimals: 2,
        description: "Added fold probability when every opponent has only checked after the flop.",
        read: |p| p.passive_fold_bonus,
    },
    Knob {
        key: "initiative",
        label: "Aggressor initiative",
        min: -0.06,
        max: 0.12,
        decimals: 2,
        description: "Extra equity realization credited to the aggressor when a bet or raise is called.",
        read: |p| p.initiative,
    },
    Knob {
        key: "open_bb",
        label: "Open size · bb",
        min: 2.0,
        max: 4.5,
        decimals: 2,
        description: "First-in raise size in big blinds (plus one big blind per limper).",
        read: |p| p.open_bb,
    },
    Knob {
        key: "short_open_bb",
        label: "Short-stack open · bb",
        min: 1.8,
        max: 3.5,
        decimals: 2,
        description: "Open size used when the effective stack is at or below the jam threshold (0171).",
        read: |p| p.short_open_bb,
    },
    Knob {
        key: "preflop_jam_bb",
        label: "Preflop jam at or below · bb",
        min: 12.0,
        max: 50.0,
        decimals: 2,
        description: "Effective stack in big blinds at or below which preflop jams are considered and the short-stack open applies.",
        read: |p| p.preflop_jam_bb,
    },
    Knob {
        key: "three_bet_ip",
        label: "3-bet size IP · x",
        min: 2.5,
        max: 4.5,
        decimals: 2,
        description: "3-bet size in position as a multiple of the raise faced.",
        read: |p| p.three_bet_ip,
    },
    Knob {
        key: "three_bet_oop",
        label: "3-bet size OOP · x",
        min: 2.5,
        max: 5.0,
        decimals: 2,
        description: "3-bet size out of position as a multiple of the open.",
        read: |p| p.three_bet_oop,
    },
    Knob {
        key: "four_bet",
        label: "4-bet size · x",
        min: 2.0,
        max: 3.0,
        decimals: 2,
        description: "4-bet-and-later size as a multiple of the current bet.",
        read: |p| p.four_bet,
    },
    Knob {
        key: "three_bet_call_margin",
        label: "3-bet call margin",
        min: -0.14,
        max: 0.1,
        decimals: 2,
        description: "Extra price, as a share of the pot, required to call a 3-bet or more.",
        read: |p| p.three_bet_call_margin,
    },
    Knob {
        key: "call_margin",
        label: "Call margin",
        min: -0.1,
        max: 0.08,
        decimals: 2,
        description: "Minimum EV a call must show over folding, as a share of the pot (negative calls lighter).",
        read: |p| p.call_margin,
    },
    Knob {
        key: "raise_risk",
        label: "Postflop raise-back weight",
        min: 0.0,
        max: 2.0,
        decimals: 2,
        description: "Weight of the postflop branch where an opponent raises our bet back.",
        read: |p| p.raise_risk,
    },
    Knob {
        key: "preflop_raise_risk",
        label: "Preflop re-raise weight",
        min: 0.0,
        max: 2.0,
        decimals: 2,
        description: "Weight of the preflop branch where an opponent 3-bets or 4-bets our raise, from their own re-raise rates.",
        read: |p| p.preflop_raise_risk,
    },
    Knob {
        key: "limper_bb",
        label: "Per-limper open add · bb",
        min: 0.0,
        max: 2.5,
        decimals: 2,
        description: "Big blinds added to an open raise per limper.",
        read: |p| p.limper_bb,
    },
    Knob {
        key: "jam_pot_ratio",
        label: "Max jam · pots",
        min: 1.0,
        max: 4.0,
        decimals: 2,
        description: "Largest postflop all-in considered, in pots.",
        read: |p| p.jam_pot_ratio,
    },
    Knob {
        key: "short_jam_pot_ratio",
        label: "Max jam · pots, short stack",
        min: 0.0,
        max: 10.0,
        decimals: 1,
        description: "Largest postflop all-in considered, in pots, when the effective stack is at most three pots; the larger of this and the max jam applies. 0 turns it off.",
        read: |p| p.short_jam_pot_ratio,
    },
    Knob {
        key: "raise_fold_bonus",
        label: "Raise fold bonus",
        min: -0.1,
        max: 0.2,
        decimals: 2,
        description: "Extra fold probability assumed when an opponent faces a raise rather than a bet.",
        read: |p| p.raise_fold_bonus,
    },
    Knob {
        key: "bet_size_scale",
        label: "Bet size scale",
        min: 0.6,
        max: 1.8,
        decimals: 2,
        description: "Multiplier on the postflop bet sizes tried (33/55/80/120% pot at 1.0).",
        read: bet_size_scale,
    },
    Knob {
        key: "profile_response_weight",
        label: "Profile response weight",
        min: 0.0,
        max: 1.5,
        decimals: 2,
        description: "Weight of the profile-specific responses the stat fold model ignores — fold-to-c-bet against a flop c-bet, a limper's VPIP facing an iso-raise. 0 uses the generic curves.",
        read: |p| p.profile_response_weight,
    },
    Knob {
        key: "check_lookahead",
        label: "Check lookahead weight",
        min: 0.0,
        max: 1.0,
        decimals: 2,
        description: "Weight of the next street when checking is priced: an opponent behind bets and we call or fold against their narrowed betting range. 0 prices one street only.",
        read: |p| p.check_lookahead,
    },
    Knob {
        key: "temperature",
        label: "Mixing temperature · pots",
        min: 0.002,
        max: 0.05,
        decimals: 3,
        description: "How much EV, as a share of the pot, a chosen action may give up so the choice is not always the same one.",
        read: |p| p.temperature,
    },
    Knob {
        key: "raise_gate",
        label: "Raise guard · floor scale",
        min: 0.0,
        max: 1.2,
        decimals: 2,
        description: "Scales the equity floor under which a raise is banned once the street has been raised (0.55 postflop after two raises, 0.5 on the river after one). 1 is the shipped guard, 0 frees the raise.",
        read: |p| p.raise_gate,
    },
    Knob {
        key: "hero_image",
        label: "Own-image weight",
        min: 0.0,
        max: 1.0,
        decimals: 2,
        description: "How much the range opponents read us for comes from our own observed play instead of the population rates. 0 uses the population view.",
        read: |p| p.hero_image,
    },
    Knob {
        key: "preflop_discount",
        label: "Preflop discount · players behind",
        min: 0.0,
        max: 1.0,
        decimals: 2,
        description: "How much of the chance that a player still to act behind a preflop raise continues is taken off our equity (their call + 3-bet rate times this). 0.5 is the long-standing factor; 0 ignores players behind.",
        read: |p| p.preflop_discount,
    },
    Knob {
        key: "tiered_all_in_fold_pricing",
        label: "Tiered all-in fold pricing",
        min: 0.0,
        max: 1.0,
        decimals: 0,
        description: "Uses each side pot's eligible opponents when pricing a raise that makes all players with chips behind fold.",
        read: |p| f64::from(p.tiered_all_in_fold_pricing as u8),
    },
    Knob {
        key: "caller_mix",
        label: "Caller-count mixture",
        min: 0.0,
        max: 1.0,
        decimals: 0,
        description: "Prices a raise that three or more players could call against the top three callers, as a mixture over one, two and three-plus callers, instead of treating every multiway call as the top two.",
        read: |p| f64::from(p.caller_mix as u8),
    },
];

#[cfg(test)]
mod tests;
