//! The logic a policy uses to decide.

/// The most exploit the `Balanced` kind may spend. The style's own bound is this number, and the
/// policy clamps to it even when a slot was seeded before the bound existed.
pub const BALANCED_EXPLOIT_CAP: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyKind {
    Solving,
    /// The pricer with a mixing draw and a blended opening boundary, reads held and no profile.
    Balanced,
    /// The playbook per opponent type, gated by the opponent's adaptivity class, and the
    /// re-solve pinned to the read.
    Exploiting,
    Pricing,
}

impl PolicyKind {
    pub const ALL: [Self; 4] = [
        Self::Solving,
        Self::Balanced,
        Self::Exploiting,
        Self::Pricing,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Solving => "solving",
            Self::Balanced => "balanced",
            Self::Exploiting => "exploiting",
            Self::Pricing => "pricing",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}
