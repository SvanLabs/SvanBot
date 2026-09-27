//! Live fits (0205): the corrections fitted from the fleet's own hands and installed in the live
//! [`Params`] — per-street and preflop fold calibration ([`crate::foldcal`]) and the river,
//! deep-pot and overbet all-in call shifts ([`crate::raisewar`]). They are local: never promoted,
//! and the learner and the golden snapshot play with [`LiveFits::NONE`].
//!
//! Every consumer goes through this module: the learner refits and stores them, the fleet loads
//! them at startup and whenever the stored fits change, and the dashboard shows the ones in use.
//! A new fit is one field here plus its fit module.

use crate::{foldcal, raisewar};
use serde::Serialize;
use std::time::Instant;
use sv10_core::policy::Params;
use sv10_store::store::Store;

/// The installed shifts, named as their [`Params`] fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct LiveFits {
    /// Logit shift on the final fold estimate per postflop street (flop, turn, river).
    pub fold_logit_shift: [f64; 3],
    /// Logit shift on the everyone-folds estimate of our preflop raises.
    pub preflop_fold_logit_shift: f64,
    /// Equity shift on river calls against an all-in.
    pub river_jam_call_shift: f64,
    /// Equity shift on calls against an all-in in deep pots.
    pub deep_call_shift: f64,
    /// Equity shift on calls against an overbet all-in.
    pub overbet_call_shift: f64,
    /// Slope of the size-scaled overbet call shift (0233).
    pub overbet_call_slope: f64,
}

impl LiveFits {
    /// No corrections: what the learner and the golden snapshot play with.
    pub const NONE: LiveFits = LiveFits {
        fold_logit_shift: [0.0; 3],
        preflop_fold_logit_shift: 0.0,
        river_jam_call_shift: 0.0,
        deep_call_shift: 0.0,
        overbet_call_shift: 0.0,
        overbet_call_slope: 0.0,
    };

    /// The installed shifts from the stored fits (0 for any fit absent, unreadable or inactive).
    pub fn load(store: &Store) -> anyhow::Result<LiveFits> {
        let fold = store.get_kv(foldcal::FOLD_CAL_KEY)?;
        Ok(LiveFits {
            fold_logit_shift: foldcal::installed_shift(fold.as_deref()),
            preflop_fold_logit_shift: foldcal::installed_preflop_shift(fold.as_deref()),
            river_jam_call_shift: raisewar::installed_river_jam_shift(store.get_kv(raisewar::RIVER_JAM_KEY)?.as_deref()),
            deep_call_shift: raisewar::installed_deep_call_shift(store.get_kv(raisewar::DEEP_CALL_KEY)?.as_deref()),
            // The overbet fit shares the deep-pot fit's type.
            overbet_call_shift: raisewar::installed_deep_call_shift(store.get_kv(raisewar::OVERBET_CALL_KEY)?.as_deref()),
            overbet_call_slope: raisewar::installed_overbet_slope(store.get_kv(raisewar::OVERBET_SLOPE_KEY)?.as_deref()),
        })
    }

    /// The shifts `params` plays with.
    pub fn of(params: &Params) -> LiveFits {
        LiveFits {
            fold_logit_shift: params.fold_logit_shift,
            preflop_fold_logit_shift: params.preflop_fold_logit_shift,
            river_jam_call_shift: params.river_jam_call_shift,
            deep_call_shift: params.deep_call_shift,
            overbet_call_shift: params.overbet_call_shift,
            overbet_call_slope: params.overbet_call_slope,
        }
    }

    /// Install these shifts in `params`.
    pub fn apply(&self, params: &mut Params) {
        params.fold_logit_shift = self.fold_logit_shift;
        params.preflop_fold_logit_shift = self.preflop_fold_logit_shift;
        params.river_jam_call_shift = self.river_jam_call_shift;
        params.deep_call_shift = self.deep_call_shift;
        params.overbet_call_shift = self.overbet_call_shift;
        params.overbet_call_slope = self.overbet_call_slope;
    }

    /// One log line per shift that differs from `prev`, naming the value now in use.
    pub fn changes_from(&self, prev: &LiveFits) -> Vec<String> {
        let mut lines = Vec::new();
        if self.fold_logit_shift != prev.fold_logit_shift {
            lines.push(format!("fold calibration: logit shifts {:?} (flop, turn, river)", self.fold_logit_shift));
        }
        if self.preflop_fold_logit_shift != prev.preflop_fold_logit_shift {
            lines.push(format!("preflop fold calibration: logit shift {:+.2}", self.preflop_fold_logit_shift));
        }
        if self.river_jam_call_shift != prev.river_jam_call_shift {
            lines.push(format!("river all-in calls: equity shift -{:.3}", self.river_jam_call_shift));
        }
        if self.deep_call_shift != prev.deep_call_shift {
            lines.push(format!("deep-pot all-in calls: equity shift -{:.3}", self.deep_call_shift));
        }
        if self.overbet_call_shift != prev.overbet_call_shift {
            lines.push(format!("overbet all-in calls: equity shift -{:.3}", self.overbet_call_shift));
        }
        if self.overbet_call_slope != prev.overbet_call_slope {
            lines.push(format!("overbet all-in calls: size-scaled shift, slope {:.3} per size unit", self.overbet_call_slope));
        }
        lines
    }
}

/// Refit and store every live fit (the learner, each search cycle).
pub fn refit_all(store: &Store, now: f64) {
    refit_fold(store, now);
    refit_calls(store);
}

/// Refit the call fits, and the fold calibration when it is missing or stale (the learner, at
/// start and hourly between search cycles: a restart must not play uncalibrated until the next
/// cycle, which waits for enough new hands).
pub fn refit_stale(store: &Store, now: f64) {
    if foldcal::refit_due(store.get_kv(foldcal::FOLD_CAL_KEY).ok().flatten().as_deref(), now) {
        refit_fold(store, now);
    }
    refit_calls(store);
}

fn put(store: &Store, key: &str, what: &str, fit: &impl Serialize) {
    match serde_json::to_string(fit) {
        Ok(j) => {
            if let Err(e) = store.put_kv(key, &j) {
                tracing::warn!("{what} not stored: {e}");
            }
        }
        Err(e) => tracing::warn!("{what} not serialized: {e}"),
    }
}

fn verdict(active: bool, shift: String) -> String {
    if active { shift } else { "not installed".into() }
}

/// Re-fit the per-street fold calibration (0156): a few seconds of SQLite reads and a
/// one-dimensional fit per street, gated on held-out live hands inside `foldcal::fit`.
fn refit_fold(store: &Store, now: f64) {
    let t0 = Instant::now();
    let mut samples = match foldcal::samples_from_store(store) {
        Ok(s) => s,
        Err(e) => return tracing::warn!("fold calibration samples unreadable: {e}"),
    };
    match foldcal::preflop_samples_from_store(store) {
        Ok(pre) => samples.extend(pre),
        Err(e) => tracing::warn!("preflop fold samples unreadable: {e}"),
    }
    let cal = foldcal::fit(&samples, now);
    let names = ["preflop", "flop", "turn", "river"];
    for (name, s) in names.iter().zip(std::iter::once(&cal.preflop).chain(&cal.streets)) {
        tracing::info!(
            "fold calibration {name}: n {} predicted {:.3} actual {:.3}; held-out gain {:+.1} mnats (95% lower {:+.1}) -> {}",
            s.n,
            s.predicted,
            s.actual,
            s.held_out_gain * 1000.0,
            s.held_out_lower * 1000.0,
            verdict(s.active, format!("shift {:+.2}", s.shift))
        );
    }
    put(store, foldcal::FOLD_CAL_KEY, "fold calibration", &cal);
    tracing::info!("fold calibration fitted on {} samples in {:.1}s", samples.len(), t0.elapsed().as_secs_f64());
}

/// Re-fit the all-in call shifts (0159, 0200, 0203) from one read of exact showdown equities,
/// each gated on held-out calls inside `raisewar`.
fn refit_calls(store: &Store) {
    let t0 = Instant::now();
    let commits = match raisewar::commits_from_store(store) {
        Ok((_, commits)) => commits,
        Err(e) => return tracing::warn!("river jam samples unreadable: {e}"),
    };
    let fit = raisewar::fit_river_jam_call(&commits);
    tracing::info!(
        "river all-in calls: n {} over-estimate {:+.3}; held-out saved {:+.0} chips/call (95% lower {:+.0}) -> {} ({:.1}s)",
        fit.n,
        fit.train_shift,
        fit.held_out_saved,
        fit.held_out_lower,
        verdict(fit.active, format!("shift -{:.3}", fit.shift)),
        t0.elapsed().as_secs_f64()
    );
    put(store, raisewar::RIVER_JAM_KEY, "river jam fit", &fit);
    let bb = store.latest_big_blind().ok().flatten().unwrap_or(crate::live::DEFAULT_BIG_BLIND) as f64;
    let bands = [
        ("deep-pot", raisewar::DEEP_CALL_KEY, raisewar::fit_deep_call(&commits, bb)),
        ("overbet", raisewar::OVERBET_CALL_KEY, raisewar::fit_overbet_call(&commits)),
    ];
    for (name, key, band) in bands {
        tracing::info!(
            "{name} all-in calls: n {} over-estimate {:+.3}; held-out {:+.3} (95% lower {:+.3}) -> {}",
            band.n,
            band.train_shift,
            band.held_out_gap,
            band.held_out_gap_lower,
            verdict(band.active, format!("shift -{:.3}", band.shift))
        );
        put(store, key, &format!("{name} call fit"), &band);
    }
    let slope = raisewar::fit_overbet_slope(&commits);
    tracing::info!(
        "overbet all-in calls, size-scaled: n {} slope {:.3}; held-out slope {:.3} (95% lower {:+.3}), gap {:+.3} -> {:+.3}, saved {:+.0} chips/call (95% lower {:+.0}) -> {}",
        slope.n,
        slope.train_slope,
        slope.held_out_slope,
        slope.held_out_slope_lower,
        slope.held_out_gap_before,
        slope.held_out_gap_after,
        slope.held_out_saved,
        slope.held_out_saved_lower,
        verdict(slope.active, format!("slope {:.3}", slope.slope))
    );
    put(store, raisewar::OVERBET_SLOPE_KEY, "overbet slope fit", &slope);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> Store {
        let d = std::env::temp_dir().join(format!("sv10-livefits-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Store::open(&d.join("svanbot10.db")).unwrap()
    }

    fn sample() -> LiveFits {
        LiveFits {
            fold_logit_shift: [0.2, 0.0, -0.7],
            preflop_fold_logit_shift: -1.4,
            river_jam_call_shift: 0.07,
            deep_call_shift: 0.15,
            overbet_call_shift: 0.3,
            overbet_call_slope: 0.06,
        }
    }

    #[test]
    fn each_stored_fit_lands_in_its_own_field() {
        let s = store("load");
        let fits = sample();
        let fold =
            foldcal::FoldCalibration { shift: fits.fold_logit_shift, preflop_shift: fits.preflop_fold_logit_shift, ..Default::default() };
        s.put_kv(foldcal::FOLD_CAL_KEY, &serde_json::to_string(&fold).unwrap()).unwrap();
        let jam = raisewar::RiverJamFit { shift: fits.river_jam_call_shift, active: true, ..Default::default() };
        s.put_kv(raisewar::RIVER_JAM_KEY, &serde_json::to_string(&jam).unwrap()).unwrap();
        let deep = raisewar::DeepCallFit { shift: fits.deep_call_shift, active: true, ..Default::default() };
        s.put_kv(raisewar::DEEP_CALL_KEY, &serde_json::to_string(&deep).unwrap()).unwrap();
        let over = raisewar::DeepCallFit { shift: fits.overbet_call_shift, active: true, ..Default::default() };
        s.put_kv(raisewar::OVERBET_CALL_KEY, &serde_json::to_string(&over).unwrap()).unwrap();
        let slope = raisewar::OverbetSlopeFit { slope: fits.overbet_call_slope, active: true, ..Default::default() };
        s.put_kv(raisewar::OVERBET_SLOPE_KEY, &serde_json::to_string(&slope).unwrap()).unwrap();
        assert_eq!(LiveFits::load(&s).unwrap(), fits);
    }

    #[test]
    fn nothing_stored_or_unreadable_plays_uncorrected() {
        let s = store("empty");
        assert_eq!(LiveFits::load(&s).unwrap(), LiveFits::NONE);
        s.put_kv(raisewar::OVERBET_CALL_KEY, "not json").unwrap();
        assert_eq!(LiveFits::load(&s).unwrap(), LiveFits::NONE);
        assert_eq!(LiveFits::default(), LiveFits::NONE);
    }

    #[test]
    fn a_fresh_store_refits_to_nothing_installed() {
        let s = store("refit");
        refit_all(&s, 1_000.0);
        for key in [
            foldcal::FOLD_CAL_KEY,
            raisewar::RIVER_JAM_KEY,
            raisewar::DEEP_CALL_KEY,
            raisewar::OVERBET_CALL_KEY,
            raisewar::OVERBET_SLOPE_KEY,
        ] {
            assert!(s.get_kv(key).unwrap().is_some(), "{key} not stored");
        }
        assert_eq!(LiveFits::load(&s).unwrap(), LiveFits::NONE, "no evidence, no correction");
    }

    #[test]
    fn apply_and_of_round_trip_without_touching_other_params() {
        let mut p = Params { samples: 123, call_margin: 0.5, ..Default::default() };
        sample().apply(&mut p);
        assert_eq!(LiveFits::of(&p), sample());
        assert_eq!((p.samples, p.call_margin), (123, 0.5));
        LiveFits::NONE.apply(&mut p);
        assert_eq!(LiveFits::of(&p), LiveFits::NONE);
    }

    #[test]
    fn a_promotion_keeps_the_installed_fits() {
        let mut live = Params::default();
        sample().apply(&mut live);
        live.adopt_promoted(Params { call_margin: 0.9, ..Default::default() });
        assert_eq!(LiveFits::of(&live), sample(), "adopt_promoted dropped a live fit");
    }

    #[test]
    fn changes_name_only_the_shifts_that_moved() {
        assert!(sample().changes_from(&sample()).is_empty());
        assert_eq!(sample().changes_from(&LiveFits::NONE).len(), 6);
        let moved = LiveFits { overbet_call_shift: 0.0, ..sample() };
        assert_eq!(moved.changes_from(&sample()), vec!["overbet all-in calls: equity shift -0.000".to_string()]);
        let sloped = LiveFits { overbet_call_slope: 0.024, ..sample() };
        assert_eq!(sloped.changes_from(&sample()), vec!["overbet all-in calls: size-scaled shift, slope 0.024 per size unit".to_string()]);
    }
}
