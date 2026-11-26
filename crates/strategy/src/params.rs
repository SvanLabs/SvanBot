//! Parameter sets (`GLOSSARY.md`): the tuned numbers a policy plays by, named by a digest.
//!
//! The digest is FNV-1a over each number's name and bits, written here: a pure layer takes no
//! crate for it. Two equal parameter sets have one digest, on any machine. Only the numbers that
//! differ from their defaults are hashed, except `exploit`, which is always hashed: that keeps
//! the digest of every champion written before a number existed.
//!
//! The pricer once read two numbers of its own, a value margin and a raise share. Since V11-39 it
//! values every action in chips instead, so neither is a number it plays by, and both are gone.

use std::fmt;
use std::str::FromStr;

/// The tuned numbers of the policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// The exploit budget, 0 to 1: how far the re-solve holds a villain to their fitted action
    /// model past its depth limit.
    pub exploit: f64,
    /// The `Balanced` kind's mixing spread, in chips of the pot: actions worth no less than the
    /// best minus this share of the pot are eligible for its draw. Zero is no mixing.
    pub spread: f64,
    /// The playbook's station cut: a read that enters at least this share of its hands, raises
    /// at most `station_pfr` of them and does not fold to bets is a station.
    pub station_vpip: f64,
    /// The playbook's station cut on preflop raises: at most this share.
    pub station_pfr: f64,
    /// The playbook's maniac cut: a read that enters at least this share of its hands and raises
    /// at least `maniac_pfr` of them is a maniac.
    pub maniac_vpip: f64,
    /// The playbook's maniac cut on preflop raises: at least this share.
    pub maniac_pfr: f64,
    /// The playbook's nit cut: a read that enters at most this share of its hands is a nit.
    pub nit_vpip: f64,
    /// The playbook's nit cut on folds: a nit folds at least this share of the bets it faces.
    pub nit_fold: f64,
}

impl Params {
    /// What the bots play by until a challenger is promoted.
    pub const DEFAULT: Self = Self {
        exploit: 0.5,
        spread: 0.0,
        station_vpip: 0.85,
        station_pfr: 0.15,
        maniac_vpip: 0.8,
        maniac_pfr: 0.5,
        nit_vpip: 0.2,
        nit_fold: 0.7,
    };

    /// Each number with its name.
    pub fn named(&self) -> [(&'static str, f64); 8] {
        [
            ("exploit", self.exploit),
            ("spread", self.spread),
            ("station_vpip", self.station_vpip),
            ("station_pfr", self.station_pfr),
            ("maniac_vpip", self.maniac_vpip),
            ("maniac_pfr", self.maniac_pfr),
            ("nit_vpip", self.nit_vpip),
            ("nit_fold", self.nit_fold),
        ]
    }

    /// This parameter set with the number called `name` set to `value`, or `None` for a name no
    /// number has.
    #[must_use]
    pub fn with(mut self, name: &str, value: f64) -> Option<Self> {
        match name {
            "exploit" => self.exploit = value,
            "spread" => self.spread = value,
            "station_vpip" => self.station_vpip = value,
            "station_pfr" => self.station_pfr = value,
            "maniac_vpip" => self.maniac_vpip = value,
            "maniac_pfr" => self.maniac_pfr = value,
            "nit_vpip" => self.nit_vpip = value,
            "nit_fold" => self.nit_fold = value,
            _ => return None,
        }
        Some(self)
    }

    /// Whether every number is inside the range it is defined on: every number is a share, 0 to 1.
    pub fn in_range(&self) -> bool {
        self.named()
            .into_iter()
            .all(|(_, value)| (0.0..=1.0).contains(&value))
    }

    /// The digest that names this parameter set.
    pub fn digest(&self) -> Digest {
        let mut hash = FNV_OFFSET;
        for (name, value) in self.named() {
            // `exploit` is always hashed, so the champion named before `spread` existed keeps its
            // digest. Every other number is hashed only when it differs from its default.
            let default = Self::DEFAULT
                .named()
                .into_iter()
                .find(|(each, _)| *each == name)
                .map_or(f64::NAN, |(_, value)| value);
            if name != "exploit" && value.to_bits() == default.to_bits() {
                continue;
            }
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
