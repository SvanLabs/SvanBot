//! The logic a policy uses to decide.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyKind {
    Solving,
    Pricing,
}

impl PolicyKind {
    pub const ALL: [Self; 2] = [Self::Solving, Self::Pricing];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Solving => "solving",
            Self::Pricing => "pricing",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}
