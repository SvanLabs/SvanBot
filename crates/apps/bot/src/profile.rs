//! Compute profiles (0187): how much of the machine the fleet spends, set from the dashboard.
//!
//! Operator (2026-09-23): "add so user can change different profiles on the web to tune everything
//! to their hardware on the fly." A profile sets only compute: the live Monte Carlo budget and
//! the learner's and analyst's thread counts, never a strategy knob. The live budget never drops
//! below the learner's own simulation budget (`Tuning::decision_samples`), the budget the promotion
//! gate measured the policy at. Each process applies a change on its next poll: the live bots at
//! once, the learner between cycles and the analyst between audit batches, by exiting for their
//! supervisors to restart them with the new thread pool (as they do for a hot swap).
//!
//! Without a stored profile everything runs as before (`max`: the whole machine, 0098).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Store key of the operator's [`ComputeProfile`].
pub const PROFILE_KEY: &str = "compute.profile";

/// Smallest share of the live Monte Carlo budget a profile may keep.
pub const MIN_LIVE_SCALE: f64 = 0.1;
/// Most times the hardware-sized live budget a profile may spend (#758): the turn clock allows far more than the
/// default uses, but ten times the equity samples measured +0.27 bb/100 (95% -1.90 .. +2.44) over 480,000 paired
/// hands, so the default stays 1 and this is the operator's lever, not a recommendation.
pub const MAX_LIVE_SCALE: f64 = 8.0;

/// Compute the fleet may spend.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputeProfile {
    /// `quiet`, `balanced`, `max`, `deep` or `custom`.
    pub name: String,
    /// Share of the hardware-sized live budget, [`MIN_LIVE_SCALE`]..=[`MAX_LIVE_SCALE`] (above 1 spends more than the default).
    pub live_scale: f64,
    /// Learner threads, 1..=logical cores.
    pub learner_threads: usize,
    /// Analyst threads, 1..=logical cores.
    pub analyst_threads: usize,
}

impl ComputeProfile {
    /// The presets for a machine with `logical` threads: `quiet` leaves most of it free for other
    /// work, `balanced` about half, and `max` is the default whole-machine sizing.
    pub fn presets(logical: usize) -> Vec<ComputeProfile> {
        let n = logical.max(1);
        let p = |name: &str, live_scale: f64, learner_threads: usize, analyst_threads: usize| ComputeProfile {
            name: name.into(),
            live_scale,
            learner_threads: learner_threads.clamp(1, n),
            analyst_threads: analyst_threads.clamp(1, n),
        };
        vec![p("quiet", 0.25, n / 4, 1), p("balanced", 0.5, n / 2, n / 4), p("max", 1.0, n, n), p("deep", 4.0, n, n)]
    }

    /// The stored profile, clamped to this machine; `None` when none is stored or it is unreadable.
    pub fn stored(json: Option<&str>, logical: usize) -> Option<ComputeProfile> {
        let p: ComputeProfile = serde_json::from_str(json?).ok()?;
        let n = logical.max(1);
        p.live_scale.is_finite().then(|| ComputeProfile {
            live_scale: p.live_scale.clamp(MIN_LIVE_SCALE, MAX_LIVE_SCALE),
            learner_threads: p.learner_threads.clamp(1, n),
            analyst_threads: p.analyst_threads.clamp(1, n),
            ..p
        })
    }

    /// A dashboard request: `{"name": "quiet" | "balanced" | "max"}` or `{"name": "custom",
    /// "live_scale", "learner_threads", "analyst_threads"}`. Out-of-range values are rejected.
    pub fn from_request(body: &Value, logical: usize) -> Result<ComputeProfile, String> {
        let n = logical.max(1);
        let name = body["name"].as_str().unwrap_or("");
        if let Some(p) = Self::presets(n).into_iter().find(|p| p.name == name) {
            return Ok(p);
        }
        if name != "custom" {
            return Err("name must be quiet, balanced, max, deep or custom".into());
        }
        let live_scale = body["live_scale"]
            .as_f64()
            .filter(|s| s.is_finite() && (MIN_LIVE_SCALE..=MAX_LIVE_SCALE).contains(s))
            .ok_or(format!("live_scale must be a number from {MIN_LIVE_SCALE} to {MAX_LIVE_SCALE}"))?;
        let threads = |k: &str| {
            body[k].as_u64().map(|t| t as usize).filter(|t| (1..=n).contains(t)).ok_or(format!("{k} must be a whole number from 1 to {n}"))
        };
        Ok(ComputeProfile {
            name: "custom".into(),
            live_scale,
            learner_threads: threads("learner_threads")?,
            analyst_threads: threads("analyst_threads")?,
        })
    }

    /// Live Monte Carlo samples under this profile: `base` scaled, never below `floor` (the
    /// learner's simulation budget) nor above [`MAX_LIVE_SCALE`] times `base`.
    pub fn live_samples(&self, base: usize, floor: usize) -> usize {
        ((base as f64 * self.live_scale).round() as usize).clamp(floor.min(base), (base as f64 * MAX_LIVE_SCALE) as usize)
    }
}

/// `(live_samples, decision_samples)` from the stored hardware profile (`HARDWARE_PROFILE_KEY`).
pub fn hardware_budget(hardware_json: Option<&str>) -> Option<(usize, usize)> {
    let v: Value = serde_json::from_str(hardware_json?).ok()?;
    let t = &v["tuning"];
    Some((t["live_samples"].as_u64()? as usize, t["decision_samples"].as_u64()? as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn presets_fit_the_machine() {
        let p = ComputeProfile::presets(8);
        let names: Vec<_> = p.iter().map(|p| (p.name.as_str(), p.live_scale, p.learner_threads, p.analyst_threads)).collect();
        assert_eq!(names, [("quiet", 0.25, 2, 1), ("balanced", 0.5, 4, 2), ("max", 1.0, 8, 8), ("deep", 4.0, 8, 8)]);
        // A two-thread box still gets at least one thread everywhere.
        assert!(ComputeProfile::presets(2).iter().all(|p| p.learner_threads >= 1 && p.analyst_threads >= 1));
    }

    #[test]
    fn requests_pick_a_preset_or_a_checked_custom_profile() {
        assert_eq!(ComputeProfile::from_request(&json!({"name": "balanced"}), 8).unwrap().learner_threads, 4);
        let custom = json!({"name": "custom", "live_scale": 0.75, "learner_threads": 6, "analyst_threads": 3});
        assert_eq!(
            ComputeProfile::from_request(&custom, 8).unwrap(),
            ComputeProfile { name: "custom".into(), live_scale: 0.75, learner_threads: 6, analyst_threads: 3 }
        );
        assert!(
            ComputeProfile::from_request(&json!({"name": "custom", "live_scale": 0.75, "learner_threads": 9, "analyst_threads": 3}), 8)
                .is_err()
        );
        assert!(
            ComputeProfile::from_request(&json!({"name": "custom", "live_scale": 0.0, "learner_threads": 2, "analyst_threads": 1}), 8)
                .is_err()
        );
        assert!(ComputeProfile::from_request(&json!({"name": "turbo"}), 8).is_err());
        assert_eq!(ComputeProfile::from_request(&json!({"name": "deep"}), 8).unwrap().live_scale, 4.0);
        assert!(
            ComputeProfile::from_request(&json!({"name": "custom", "live_scale": 9.0, "learner_threads": 2, "analyst_threads": 1}), 8)
                .is_err()
        );
    }

    #[test]
    fn stored_profiles_are_clamped_to_this_machine() {
        let big = r#"{"name":"custom","live_scale":30.0,"learner_threads":32,"analyst_threads":0}"#;
        let p = ComputeProfile::stored(Some(big), 8).unwrap();
        assert_eq!((p.live_scale, p.learner_threads, p.analyst_threads), (MAX_LIVE_SCALE, 8, 1));
        assert_eq!(
            ComputeProfile::stored(Some(r#"{"name":"deep","live_scale":4.0,"learner_threads":8,"analyst_threads":8}"#), 8)
                .unwrap()
                .live_scale,
            4.0
        );
        assert_eq!(ComputeProfile::stored(Some("junk"), 8), None);
        assert_eq!(ComputeProfile::stored(None, 8), None);
    }

    #[test]
    fn live_budget_never_drops_below_the_measured_policy_budget() {
        let quiet = &ComputeProfile::presets(8)[0];
        // 4,000 live samples, learner sims at 1,000: a quarter lands exactly on the floor.
        assert_eq!(quiet.live_samples(4_000, 1_000), 1_000);
        assert_eq!(ComputeProfile { live_scale: 0.1, ..quiet.clone() }.live_samples(4_000, 1_000), 1_000);
        assert_eq!(ComputeProfile::presets(8)[1].live_samples(4_000, 1_000), 2_000);
        assert_eq!(ComputeProfile::presets(8)[2].live_samples(4_000, 1_000), 4_000);
        // `deep` spends four times the default, and nothing spends more than the ceiling.
        assert_eq!(ComputeProfile::presets(8)[3].live_samples(4_000, 1_000), 16_000);
        assert_eq!(ComputeProfile { live_scale: 100.0, ..quiet.clone() }.live_samples(4_000, 1_000), 32_000);
        assert_eq!(hardware_budget(Some(r#"{"tuning":{"live_samples":4000,"decision_samples":1000}}"#)), Some((4_000, 1_000)));
        assert_eq!(hardware_budget(Some("{}")), None);
    }
}
