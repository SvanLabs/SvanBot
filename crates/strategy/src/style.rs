//! A policy's opening width and profile rules.

use std::fmt;

use crate::params::Params;
use crate::profile::{ACTIVE, Profile};

pub struct Style {
    pub name: &'static str,
    pub open_scale: f64,
    pub profile: Profile<'static>,
}

impl fmt::Debug for Style {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name)
    }
}

impl Style {
    /// `params` with this style's pins applied: the numbers a new champion slot starts from.
    /// The width is pinned in the style itself, and the policy reads it from there, so seeding
    /// moves no parameter-set number and no digest. A style that pins a parameter-set number
    /// applies it here.
    #[must_use]
    pub fn seed(&self, params: Params) -> Params {
        params
    }
}

pub const STANDARD: Style = Style {
    name: "standard",
    open_scale: 1.0,
    profile: ACTIVE,
};
pub const TIGHT: Style = Style {
    name: "tight",
    open_scale: 0.75,
    profile: ACTIVE,
};
pub const LOOSE: Style = Style {
    name: "loose",
    open_scale: 1.25,
    profile: ACTIVE,
};

pub const ALL: [&Style; 3] = [&STANDARD, &TIGHT, &LOOSE];

pub fn named(name: &str) -> Option<&'static Style> {
    ALL.into_iter().find(|style| style.name == name)
}
