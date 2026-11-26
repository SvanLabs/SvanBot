//! A policy's opening width and profile rules.

use std::fmt;

use crate::kind::BALANCED_EXPLOIT_CAP;
use crate::params::Params;
use crate::profile::{ACTIVE, EMPTY, Profile};

pub struct Style {
    pub name: &'static str,
    pub open_scale: f64,
    pub profile: Profile<'static>,
    /// What the style does beyond the numbers every style shares (M3–M5).
    pub mechanics: Mechanics,
    /// The range a parameter-set number may take under this style, by name. A zero-width range
    /// is a pin: the style fixes the number, and the search never moves it. A number not listed
    /// is left to the parameter set's own range.
    pub bounds: &'static [(&'static str, f64, f64)],
}

impl fmt::Debug for Style {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name)
    }
}

/// The style's three-bet, sizing and c-bet rules (M3–M5). A style with no rule leaves the spot
/// to the pricer, which is what every style but Grinder's and Gambler's does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mechanics {
    pub three_bet: Option<ThreeBet>,
    pub sizes: Sizes,
    pub cbet: Option<Cbet>,
}

/// M3, three-bet logic: three-bet the best `three_bet_value` of classes by rank; a polarised
/// style also three-bets the worst `three_bet_bluff`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreeBet {
    pub polarised: bool,
}

/// M4, the size menu: which of the pricer's raise sizes the style may name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sizes {
    /// Every size the pricer prices.
    All,
    /// The small end: the smallest size the pricer prices.
    Small,
    /// The big end: the largest size the pricer prices.
    Big,
}

/// M5, texture c-bets: bet a checked-to board when its texture is the style's, with the style's
/// small or big size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cbet {
    pub dry: bool,
    pub wet: bool,
    pub big: bool,
}

/// The mechanics no style but Grinder and Gambler has: the pricer decides every spot.
pub const NEUTRAL: Mechanics = Mechanics {
    three_bet: None,
    sizes: Sizes::All,
    cbet: None,
};

impl Style {
    /// `params` with this style's bounds applied: a number outside its range is clamped into it,
    /// and a zero-width bound (a pin) sets the number exactly. A new champion slot starts here.
    /// The width is pinned in the style itself, and the policy reads it from there, so seeding the
    /// width moves no parameter-set number.
    #[must_use]
    pub fn seed(&self, params: Params) -> Params {
        self.bounds
            .iter()
            .fold(params, |params, (name, low, high)| {
                params
                    .with(name, params_value(&params, name).clamp(*low, *high))
                    .unwrap_or(params)
            })
    }

    /// Whether this style lets `value` be searched for the number `name`: inside its bound when
    /// the style names one, and anywhere when it does not. A pinned number is allowed only at
    /// its pin.
    pub fn allows(&self, name: &str, value: f64) -> bool {
        match self.bounds.iter().find(|(bounded, _, _)| *bounded == name) {
            Some((_, low, high)) => (*low..=*high).contains(&value),
            None => true,
        }
    }
}

/// The current value of `name`, for a seed to clamp. A name the set does not have reads as the
/// neutral zero, though every bound here names a real number.
fn params_value(params: &Params, name: &str) -> f64 {
    params
        .named()
        .into_iter()
        .find(|(each, _)| *each == name)
        .map_or(0.0, |(_, value)| value)
}

pub const STANDARD: Style = Style {
    name: "standard",
    open_scale: 1.0,
    profile: ACTIVE,
    mechanics: NEUTRAL,
    bounds: &[
        ("three_bet_value", 0.04, 0.04),
        ("three_bet_bluff", 0.0, 0.0),
        ("cbet_dry", 0.6, 0.6),
        ("cbet_wet", 0.3, 0.3),
        ("size_small", 0.33, 0.33),
        ("size_big", 1.0, 1.0),
        ("spread", 0.0, 0.0),
        ("station_vpip", 0.85, 0.85),
        ("station_pfr", 0.15, 0.15),
        ("maniac_vpip", 0.8, 0.8),
        ("maniac_pfr", 0.5, 0.5),
        ("nit_vpip", 0.2, 0.2),
        ("nit_fold", 0.7, 0.7),
    ],
};
pub const TIGHT: Style = Style {
    name: "tight",
    open_scale: 0.75,
    profile: ACTIVE,
    // Grinder: narrow, value-heavy three-bets, small sizes, c-bets dry boards.
    mechanics: Mechanics {
        three_bet: Some(ThreeBet { polarised: false }),
        sizes: Sizes::Small,
        cbet: Some(Cbet {
            dry: true,
            wet: false,
            big: false,
        }),
    },
    bounds: &[
        ("three_bet_value", 0.02, 0.08),
        ("three_bet_bluff", 0.0, 0.0),
        ("cbet_dry", 0.4, 0.9),
        ("cbet_wet", 0.0, 0.3),
        ("size_small", 0.25, 0.5),
        ("size_big", 0.5, 1.0),
        ("spread", 0.0, 0.0),
        ("station_vpip", 0.85, 0.85),
        ("station_pfr", 0.15, 0.15),
        ("maniac_vpip", 0.8, 0.8),
        ("maniac_pfr", 0.5, 0.5),
        ("nit_vpip", 0.2, 0.2),
        ("nit_fold", 0.7, 0.7),
    ],
};
pub const LOOSE: Style = Style {
    name: "loose",
    open_scale: 1.25,
    profile: ACTIVE,
    // Gambler: polarised three-bets, the big sizes, pressure on wet boards.
    mechanics: Mechanics {
        three_bet: Some(ThreeBet { polarised: true }),
        sizes: Sizes::Big,
        cbet: Some(Cbet {
            dry: false,
            wet: true,
            big: true,
        }),
    },
    bounds: &[
        ("three_bet_value", 0.02, 0.08),
        ("three_bet_bluff", 0.04, 0.16),
        ("cbet_dry", 0.1, 0.5),
        ("cbet_wet", 0.4, 0.9),
        ("size_small", 0.3, 0.66),
        ("size_big", 0.8, 1.0),
        ("spread", 0.0, 0.0),
        ("station_vpip", 0.85, 0.85),
        ("station_pfr", 0.15, 0.15),
        ("maniac_vpip", 0.8, 0.8),
        ("maniac_pfr", 0.5, 0.5),
        ("nit_vpip", 0.2, 0.2),
        ("nit_fold", 0.7, 0.7),
    ],
};
/// Anchor, the `Balanced` brain: the baseline made a brain. It mixes near-best actions and
/// blends the opening boundary, holds reads, and caps exploit low. Its slot learns the spread
/// and the capped exploit between the bounds.
pub const ANCHOR: Style = Style {
    name: "anchor",
    open_scale: 1.0,
    profile: EMPTY,
    mechanics: NEUTRAL,
    bounds: &[
        ("three_bet_value", 0.04, 0.04),
        ("three_bet_bluff", 0.0, 0.0),
        ("cbet_dry", 0.6, 0.6),
        ("cbet_wet", 0.3, 0.3),
        ("size_small", 0.33, 0.33),
        ("size_big", 1.0, 1.0),
        ("exploit", 0.0, BALANCED_EXPLOIT_CAP),
        ("spread", 0.1, 0.5),
        ("station_vpip", 0.85, 0.85),
        ("station_pfr", 0.15, 0.15),
        ("maniac_vpip", 0.8, 0.8),
        ("maniac_pfr", 0.5, 0.5),
        ("nit_vpip", 0.2, 0.2),
        ("nit_fold", 0.7, 0.7),
    ],
};

/// Hunter, the `Exploiting` brain: a playbook per opponent type read from stats and per
/// adaptivity class, and the re-solve pinned to the read. It plays no style profile: the
/// playbook is the kind's logic. Its slot learns the playbook's cuts and the exploit stance
/// between the bounds.
pub const HUNTER: Style = Style {
    name: "hunter",
    open_scale: 1.0,
    profile: EMPTY,
    mechanics: NEUTRAL,
    bounds: &[
        ("three_bet_value", 0.04, 0.04),
        ("three_bet_bluff", 0.0, 0.0),
        ("cbet_dry", 0.6, 0.6),
        ("cbet_wet", 0.3, 0.3),
        ("size_small", 0.33, 0.33),
        ("size_big", 1.0, 1.0),
        ("exploit", 0.0, 1.0),
        ("spread", 0.0, 0.0),
        ("station_vpip", 0.6, 1.0),
        ("station_pfr", 0.0, 0.4),
        ("maniac_vpip", 0.5, 1.0),
        ("maniac_pfr", 0.2, 1.0),
        ("nit_vpip", 0.0, 0.4),
        ("nit_fold", 0.4, 1.0),
    ],
};

pub const ALL: [&Style; 5] = [&STANDARD, &TIGHT, &LOOSE, &ANCHOR, &HUNTER];

pub fn named(name: &str) -> Option<&'static Style> {
    ALL.into_iter().find(|style| style.name == name)
}
