//! The per-bot promotion gate (V11-77): what a bot's candidate must show before the live guard
//! may start. This is arithmetic and data only; `runtime` stores the evidence and `apps` runs
//! the sims that fill it.

use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;

use crate::paired::Verdict;

/// The most a designated row may lose against its opponent, in mbb/100: 5 bb/100. The row's
/// lower 95% bound must stay at or above minus this.
pub const GAUNTLET_MARGIN: f64 = 5_000.0;

/// The opponents a promotion must show a fresh 6-max row for. These are the Arena's release rows:
/// a candidate is proven against the same field a release is.
pub const DESIGNATED: [&str; 7] = [
    "pricer",
    "v10.0.2",
    "calling station",
    "nit",
    "maniac",
    "switching",
    "learning",
];

/// One designated opponent's row: the challenger's paired edge over the incumbent, in mbb/100.
#[derive(Clone, Debug, PartialEq)]
pub struct GauntletRow {
    pub opponent: String,
    pub verdict: Verdict,
}

/// The evidence a bot's promotion needs before its live guard: the incumbent and the candidate
/// it was measured against, the kind and style it was simulated with, one fresh paired 6-max
/// confirmation, and a row for every designated opponent.
#[derive(Clone, Debug, PartialEq)]
pub struct Evidence {
    pub incumbent: Params,
    pub challenger: Params,
    pub kind: PolicyKind,
    pub style: &'static str,
    pub fresh: Verdict,
    pub gauntlet: Vec<GauntletRow>,
}

/// What the gate says about a set of evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate {
    /// The fresh confirmation is above zero and every designated row rules out a loss beyond
    /// [`GAUNTLET_MARGIN`].
    Admitted,
    /// The evidence is complete but does not clear a bar.
    Refused(Vec<String>),
    /// A required row is missing or undecided: no verdict either way, so the incumbent stays.
    Incomplete(Vec<String>),
}

/// Reads `evidence`: an incomplete row is no verdict and wins over a refusal, so a candidate is
/// never refused for a row that has not been played.
pub fn gate(evidence: &Evidence) -> Gate {
    let mut incomplete = Vec::new();
    let mut refusals = Vec::new();
    if !decided(&evidence.fresh) {
        incomplete.push("the fresh confirmation has no verdict".to_owned());
    } else if !evidence.fresh.above_zero() {
        refusals.push(format!(
            "the fresh confirmation is not above the incumbent: {:+.0} mbb/100, 95% ±{:.0}",
            evidence.fresh.mbb_per_100, evidence.fresh.ci95
        ));
    }
    for opponent in DESIGNATED {
        let Some(row) = evidence
            .gauntlet
            .iter()
            .find(|row| row.opponent == opponent)
        else {
            incomplete.push(format!("no gauntlet row for {opponent}"));
            continue;
        };
        if !decided(&row.verdict) {
            incomplete.push(format!("the gauntlet row for {opponent} has no verdict"));
            continue;
        }
        let lower = row.verdict.mbb_per_100 - row.verdict.ci95;
        if lower < -GAUNTLET_MARGIN {
            refusals.push(format!(
                "the gauntlet row for {opponent} may lose more than 5 bb/100: lower {lower:+.0} mbb/100"
            ));
        }
    }
    if !incomplete.is_empty() {
        Gate::Incomplete(incomplete)
    } else if !refusals.is_empty() {
        Gate::Refused(refusals)
    } else {
        Gate::Admitted
    }
}

/// Whether a verdict was actually measured: a row with no deals, or a non-finite number, is no
/// evidence and must not read as a pass.
fn decided(verdict: &Verdict) -> bool {
    verdict.deals > 0
        && verdict.mbb_per_100.is_finite()
        && verdict.ci95.is_finite()
        && verdict.ci95 >= 0.0
}
