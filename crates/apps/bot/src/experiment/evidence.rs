//! Live evidence for one target (0267): treatment minus champion control on the pair's hands,
//! with the predeclared gates. It never promotes and never mixes with the simulated estimate:
//! harm retires the target, support asks the learner to finish its fresh-deal confirmation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use sv10_store::store::{ArmHand, CONTROL_ARM, TREATMENT_ARM};

/// KV key of the fleet's live verdicts, by target id.
pub const VERDICTS_KEY: &str = "experiment.verdicts.v1";
/// Hands per arm before the safety stop may retire a target.
pub const MIN_EFFECTIVE_HANDS: u64 = 2_000;
/// Hands per arm at which the target gets its verdict.
pub const BOUNDARY_HANDS: u64 = 10_000;
/// Hands a pair bot plays in one arm before the two bots exchange arms (a multiple of six seats,
/// so positions balance inside a block).
pub const BLOCK_HANDS: u64 = 120;

/// Which policy a pair bot plays this hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    /// The learner challenger.
    Treatment,
    /// The champion, as the experiment's control.
    Control,
}

impl Arm {
    /// The store's arm name.
    pub fn as_str(self) -> &'static str {
        match self {
            Arm::Treatment => TREATMENT_ARM,
            Arm::Control => CONTROL_ARM,
        }
    }

    /// The provenance kind recorded with every decision and hand (0267).
    pub fn provenance(self) -> &'static str {
        match self {
            Arm::Treatment => "parameter_challenger",
            Arm::Control => "champion_control",
        }
    }
}

/// The arm of pair bot `pair_index` (0 or 1) after it has played `hands_on_target` hands of the
/// target: the two bots start on opposite arms and swap every [`BLOCK_HANDS`], so neither bot's
/// identity, table or time of day becomes the treatment.
pub fn arm_for(pair_index: usize, hands_on_target: u64) -> Arm {
    if (hands_on_target / BLOCK_HANDS + pair_index as u64).is_multiple_of(2) { Arm::Treatment } else { Arm::Control }
}

/// Mean and standard error of one arm, big blinds per hand.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ArmStats {
    /// Hands with a known result.
    pub hands: u64,
    /// Mean big blinds per hand (all-in luck removed where filled).
    pub mean_bb: f64,
    /// Standard error of the mean.
    pub se_bb: f64,
}

impl ArmStats {
    fn of(values: &[f64]) -> Self {
        let n = values.len() as f64;
        if values.is_empty() {
            return ArmStats::default();
        }
        let mean = values.iter().sum::<f64>() / n;
        let var = if values.len() > 1 { values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0) } else { 0.0 };
        ArmStats { hands: values.len() as u64, mean_bb: mean, se_bb: (var / n).sqrt() }
    }
}

/// Treatment minus control.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    /// Challenger arm.
    pub treatment: ArmStats,
    /// Champion arm.
    pub control: ArmStats,
    /// Difference in bb/100.
    pub diff_bb100: f64,
    /// 95% interval of the difference, bb/100.
    pub lower_bb100: f64,
    /// Upper end of the 95% interval, bb/100.
    pub upper_bb100: f64,
    /// Hands in the smaller arm: the sample the gates count.
    pub effective_hands: u64,
}

impl Estimate {
    /// Estimate from the target's stored hands. A hand without a result or a big blind is left out.
    pub fn of(hands: &[ArmHand]) -> Self {
        let values = |arm: &str| -> Vec<f64> {
            hands
                .iter()
                .filter(|h| h.arm == arm)
                .filter_map(|h| {
                    let bb = h.bb.filter(|b| *b > 0)? as f64;
                    Some(h.ev_net.or(h.net.map(|n| n as f64))? / bb)
                })
                .collect()
        };
        let (treatment, control) = (ArmStats::of(&values(TREATMENT_ARM)), ArmStats::of(&values(CONTROL_ARM)));
        let diff = treatment.mean_bb - control.mean_bb;
        let se = (treatment.se_bb.powi(2) + control.se_bb.powi(2)).sqrt();
        Estimate {
            diff_bb100: diff * 100.0,
            lower_bb100: (diff - 1.96 * se) * 100.0,
            upper_bb100: (diff + 1.96 * se) * 100.0,
            effective_hands: treatment.hands.min(control.hands),
            treatment,
            control,
        }
    }

    /// The gate this estimate has reached, if any: the safety stop from [`MIN_EFFECTIVE_HANDS`],
    /// the verdict at [`BOUNDARY_HANDS`].
    pub fn verdict(&self) -> Option<Verdict> {
        if self.effective_hands >= MIN_EFFECTIVE_HANDS && self.upper_bb100 < 0.0 {
            return Some(Verdict::LiveHarmful);
        }
        if self.effective_hands >= BOUNDARY_HANDS {
            return Some(if self.lower_bb100 > 0.0 { Verdict::LiveSupported } else { Verdict::Inconclusive });
        }
        None
    }

    /// The next gate, for the dashboard.
    pub fn next_gate(&self) -> String {
        if self.effective_hands < MIN_EFFECTIVE_HANDS {
            format!("safety check at {MIN_EFFECTIVE_HANDS} hands per arm ({} so far)", self.effective_hands)
        } else {
            format!(
                "verdict at {BOUNDARY_HANDS} hands per arm ({} so far); stops early if the upper bound falls below 0",
                self.effective_hands
            )
        }
    }
}

/// A target's live result. Never `promoted`: only the fresh-deal gate promotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Treatment beat control at the boundary: the learner confirms it next.
    LiveSupported,
    /// Treatment is worse than control: retired.
    LiveHarmful,
    /// The boundary passed without a decision.
    Inconclusive,
}

/// A stored verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VerdictRecord {
    /// The verdict.
    pub verdict: Verdict,
    /// Unix seconds.
    pub at: f64,
    /// Target label.
    pub label: String,
    /// The estimate that reached it.
    pub estimate: Estimate,
}

/// Verdicts by target id.
pub type Verdicts = BTreeMap<String, VerdictRecord>;

#[cfg(test)]
mod tests {
    use super::*;

    fn hands(arm: &str, nets: impl IntoIterator<Item = i64>) -> Vec<ArmHand> {
        nets.into_iter()
            .enumerate()
            .map(|(i, net)| ArmHand {
                bot: "A".into(),
                hand_id: format!("{arm}{i}"),
                arm: arm.into(),
                ts: String::new(),
                net: Some(net),
                ev_net: None,
                bb: Some(20),
            })
            .collect()
    }

    #[test]
    fn the_two_bots_start_opposite_and_swap_every_block() {
        assert_eq!((arm_for(0, 0), arm_for(1, 0)), (Arm::Treatment, Arm::Control));
        assert_eq!((arm_for(0, BLOCK_HANDS - 1), arm_for(0, BLOCK_HANDS)), (Arm::Treatment, Arm::Control));
        assert_eq!(arm_for(1, BLOCK_HANDS), Arm::Treatment);
        let treated = (0..BLOCK_HANDS * 10).filter(|k| arm_for(0, *k) == Arm::Treatment).count() as u64;
        assert_eq!(treated, BLOCK_HANDS * 5, "each bot spends half its hands in each arm");
    }

    #[test]
    fn a_clearly_worse_treatment_is_stopped_only_after_the_minimum_sample() {
        let n = MIN_EFFECTIVE_HANDS as i64;
        let mut all = hands(TREATMENT_ARM, (0..n - 1).map(|i| if i % 2 == 0 { -60 } else { 20 }));
        all.extend(hands(CONTROL_ARM, (0..n).map(|i| if i % 2 == 0 { -20 } else { 60 })));
        let early = Estimate::of(&all);
        assert!(early.upper_bb100 < 0.0);
        assert_eq!(early.verdict(), None, "below the minimum sample only the ordinary safeguards act");
        all.extend(hands(TREATMENT_ARM, [-60]));
        assert_eq!(Estimate::of(&all).verdict(), Some(Verdict::LiveHarmful));
    }

    #[test]
    fn the_boundary_gives_support_or_inconclusive_and_luck_adjusted_nets_are_used() {
        let n = BOUNDARY_HANDS as i64;
        let mut all = hands(TREATMENT_ARM, (0..n).map(|i| if i % 2 == 0 { 30 } else { -10 }));
        all.extend(hands(CONTROL_ARM, (0..n).map(|i| if i % 2 == 0 { 20 } else { -20 })));
        let e = Estimate::of(&all);
        assert!((e.diff_bb100 - 50.0).abs() < 1e-9, "{}", e.diff_bb100);
        assert_eq!(e.verdict(), Some(Verdict::LiveSupported));
        for h in all.iter_mut().filter(|h| h.arm == TREATMENT_ARM) {
            h.ev_net = Some(0.0);
        }
        assert_eq!(Estimate::of(&all).verdict(), Some(Verdict::Inconclusive));
        assert!(serde_json::to_string(&Verdict::LiveSupported).unwrap().contains("live-supported"));
    }
}
