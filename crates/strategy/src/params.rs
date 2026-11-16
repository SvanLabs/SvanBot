//! Parameter sets (`GLOSSARY.md`): the tuned numbers a policy plays by, named by a digest.
//!
//! The digest is FNV-1a over each number's name and bits, written here: a pure layer takes no
//! crate for it. Two equal parameter sets have one digest, on any machine.

use std::fmt;
use std::str::FromStr;

/// The tuned numbers of the policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// The exploit budget, 0 to 1: how far the re-solve holds a villain to their fitted action
    /// model past its depth limit.
    pub exploit: f64,
    /// How far from its fair share to the whole pot the pricer's equity must be before it raises.
    pub value_margin: f64,
    /// The share of the pot a pricer's raise adds to the smallest raise.
    pub raise_share: f64,
}

impl Params {
    /// What the bots play by until a challenger is promoted.
    pub const DEFAULT: Self = Self {
        exploit: 0.5,
        value_margin: 0.4,
        raise_share: 0.75,
    };

    /// Each number with its name.
    pub fn named(&self) -> [(&'static str, f64); 3] {
        [
            ("exploit", self.exploit),
            ("value_margin", self.value_margin),
            ("raise_share", self.raise_share),
        ]
    }

    /// This parameter set with the number called `name` set to `value`, or `None` for a name no
    /// number has.
    #[must_use]
    pub fn with(mut self, name: &str, value: f64) -> Option<Self> {
        match name {
            "exploit" => self.exploit = value,
            "value_margin" => self.value_margin = value,
            "raise_share" => self.raise_share = value,
            _ => return None,
        }
        Some(self)
    }

    /// The digest that names this parameter set.
    pub fn digest(&self) -> Digest {
        let mut hash = FNV_OFFSET;
        for (name, value) in self.named() {
            let bytes = name.bytes().chain([0]).chain(value.to_bits().to_le_bytes());
            for byte in bytes {
                hash = (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
            }
        }
        Digest(hash)
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The content hash that names a parameter set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest(u64);

impl Digest {
    /// The digest as a number.
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// Text that is not 16 hex digits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseDigestError;

impl FromStr for Digest {
    type Err = ParseDigestError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != 16 {
            return Err(ParseDigestError);
        }
        u64::from_str_radix(text, 16)
            .map(Self)
            .map_err(|_| ParseDigestError)
    }
}
