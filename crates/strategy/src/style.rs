//! A policy's opening width and profile rules.

use std::fmt;

use crate::params::Params;
use crate::profile::{ACTIVE, Profile};

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
    /// `params` with this style's pins applied: the numbers a new champion slot starts from.
    /// The width is pinned in the style itself, and the policy reads it from there, so seeding
    /// moves no parameter-set number and no digest. A parameter-set number the style pins (a
    /// zero-width bound) is set here.
    #[must_use]
    pub fn seed(&self, params: Params) -> Params {
        self.bounds
            .iter()
            .fold(params, |params, (name, low, high)| {
                if low == high {
                    params.with(name, *low).unwrap_or(params)
                } else {
                    params
                }
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

pub const STANDARD: Style = Style {
    name: "standard",
    open_scale: 1.0,
    profile: ACTIVE,
    bounds: &[],
};
pub const TIGHT: Style = Style {
    name: "tight",
    open_scale: 0.75,
    profile: ACTIVE,
    bounds: &[],
};
pub const LOOSE: Style = Style {
    name: "loose",
    open_scale: 1.25,
    profile: ACTIVE,
    bounds: &[],
};

pub const ALL: [&Style; 3] = [&STANDARD, &TIGHT, &LOOSE];

pub fn named(name: &str) -> Option<&'static Style> {
    ALL.into_iter().find(|style| style.name == name)
}
