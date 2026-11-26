//! A policy's opening width and profile rules.

use std::fmt;

use crate::kind::BALANCED_EXPLOIT_CAP;
use crate::params::Params;
use crate::profile::{ACTIVE, EMPTY, Profile};

pub struct Style {
    pub name: &'static str,
    pub open_scale: f64,
    pub profile: Profile<'static>,
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
    bounds: &[("spread", 0.0, 0.0)],
};
pub const TIGHT: Style = Style {
    name: "tight",
    open_scale: 0.75,
    profile: ACTIVE,
    bounds: &[("spread", 0.0, 0.0)],
};
pub const LOOSE: Style = Style {
    name: "loose",
    open_scale: 1.25,
    profile: ACTIVE,
    bounds: &[("spread", 0.0, 0.0)],
};
/// Anchor, the `Balanced` brain: the baseline made a brain. It mixes near-best actions and
/// blends the opening boundary, holds reads, and caps exploit low. Its slot learns the spread
/// and the capped exploit between the bounds.
pub const ANCHOR: Style = Style {
    name: "anchor",
    open_scale: 1.0,
    profile: EMPTY,
    bounds: &[("exploit", 0.0, BALANCED_EXPLOIT_CAP), ("spread", 0.1, 0.5)],
};

pub const ALL: [&Style; 4] = [&STANDARD, &TIGHT, &LOOSE, &ANCHOR];

pub fn named(name: &str) -> Option<&'static Style> {
    ALL.into_iter().find(|style| style.name == name)
}
