//! What live play has installed from the learner's store (0222): promoted parameters, the neural
//! response model, the fitted range model, the compute profile's live budget, the live fits
//! ([`crate::livefits`]) and the per-opponent corrections ([`crate::playerfits`]).
//!
//! One rule for all of them: an installed artifact changes only when a store read succeeds and
//! returns something different. A failed read keeps what is in play (0205 set this for the live
//! fits; before 0222 a failed read of the neural or range key uninstalled that model until the
//! next tick) and is logged once per failure streak.

use crate::live::Shared;
use crate::{NN_KEY, PARAMS_KEY, StoredNet};
use sv10_core::policy::Params;
use sv10_store::store::Store;

/// One watched store key: the value last read successfully.
#[derive(Clone, Debug)]
struct Watched {
    key: &'static str,
    /// `None` until the first successful read; then the stored value (`Some(None)` = absent).
    seen: Option<Option<String>>,
}

impl Watched {
    fn unseen(key: &'static str) -> Self {
        Watched { key, seen: None }
    }

    fn seen_as(key: &'static str, value: Option<String>) -> Self {
        Watched { key, seen: Some(value) }
    }

    /// `Ok(Some(value))` when the stored value differs from the last successful read, `Ok(None)`
    /// when it does not; a failed read changes nothing.
    fn changed(&mut self, store: &Store) -> anyhow::Result<Option<Option<String>>> {
        let now = store.get_kv(self.key)?;
        if self.seen.as_ref() == Some(&now) {
            return Ok(None);
        }
        self.seen = Some(now.clone());
        Ok(Some(now))
    }
}

/// The fleet's view of what it has installed, refreshed from the store every watcher tick.
#[derive(Debug)]
pub struct Installs {
    profile: Watched,
    range: Watched,
    nn: Watched,
    params: Watched,
    /// Whether the last refresh hit a read error (the first error of a streak is logged).
    failing: bool,
}

impl Installs {
    /// Start from what startup installed: the stored params and range model count as seen; the
    /// compute profile and neural model install on the first refresh that finds them stored.
    pub fn after_startup(store: &Store) -> Self {
        let seen = |key| store.get_kv(key).map_or_else(|_| Watched::unseen(key), |v| Watched::seen_as(key, v));
        Installs {
            profile: Watched::seen_as(crate::profile::PROFILE_KEY, None),
            range: seen(crate::RANGE_PARAMS_KEY),
            nn: Watched::seen_as(NN_KEY, None),
            params: seen(PARAMS_KEY),
            failing: false,
        }
    }

    /// Install every artifact whose stored value changed, logging each change. A store read
    /// error keeps the installed artifact; the first error of a streak is logged as a warning.
    pub fn refresh(&mut self, shared: &Shared) {
        let mut errors = Vec::new();
        if let Err(e) = self.refresh_profile(shared) {
            errors.push(format!("compute profile: {e}"));
        }
        if let Err(e) = self.refresh_range(shared) {
            errors.push(format!("range model: {e}"));
        }
        match crate::livefits::LiveFits::load(&shared.store) {
            Ok(fits) => {
                let current = crate::livefits::LiveFits::of(&shared.params.read());
                if fits != current {
                    fits.apply(&mut shared.params.write());
                    for line in fits.changes_from(&current) {
                        shared.log("learner", "info", line);
                    }
                }
            }
            Err(e) => errors.push(format!("live fits: {e}")),
        }
        match crate::playerfits::PlayerFits::load(&shared.store) {
            Ok(fits) => {
                let current = crate::playerfits::PlayerFits::of(&shared.models.read());
                if fits != current {
                    fits.apply(&mut shared.models.write());
                    for line in fits.changes_from(&current) {
                        shared.log("learner", "info", line);
                    }
                }
            }
            Err(e) => errors.push(format!("per-opponent corrections: {e}")),
        }
        if let Err(e) = self.refresh_nn(shared) {
            errors.push(format!("neural model: {e}"));
        }
        if let Err(e) = self.refresh_params(shared) {
            errors.push(format!("promoted parameters: {e}"));
        }
        if !errors.is_empty() && !self.failing {
            shared.log("learner", "warn", format!("store unreadable, keeping what is installed ({})", errors.join("; ")));
        }
        if errors.is_empty() && self.failing {
            shared.log("learner", "info", "store readable again");
        }
        self.failing = !errors.is_empty();
    }

    /// Compute profile (0187): the live Monte Carlo budget follows the dashboard's profile.
    fn refresh_profile(&mut self, shared: &Shared) -> anyhow::Result<()> {
        let mut pending = self.profile.clone();
        let Some(profile_json) = pending.changed(&shared.store)? else { return Ok(()) };
        let hardware = shared.store.get_kv(crate::HARDWARE_PROFILE_KEY)?;
        self.profile = pending;
        let logical = hardware
            .as_deref()
            .and_then(|h| serde_json::from_str::<serde_json::Value>(h).ok())
            .and_then(|h| h["logical_cores"].as_u64())
            .unwrap_or(1) as usize;
        if let Some((base, floor)) = crate::profile::hardware_budget(hardware.as_deref()) {
            let profile = crate::profile::ComputeProfile::stored(profile_json.as_deref(), logical);
            let samples = profile.as_ref().map_or(base, |p| p.live_samples(base, floor));
            if shared.params.read().samples != samples {
                shared.params.write().samples = samples;
                let name = profile.map_or("max".to_string(), |p| p.name);
                shared.log("learner", "info", format!("compute profile {name}: live decisions use {samples} samples"));
            }
        }
        Ok(())
    }

    /// The showdown-fitted range model, or the defaults when none is stored and active.
    fn refresh_range(&mut self, shared: &Shared) -> anyhow::Result<()> {
        let Some(range_json) = self.range.changed(&shared.store)? else { return Ok(()) };
        let fitted = crate::fitted_range_params(range_json.as_deref());
        let active = fitted.is_some();
        shared.params.write().range = fitted.unwrap_or_default();
        shared.log(
            "learner",
            "info",
            format!("range model: {}", if active { "showdown-fitted parameters in use" } else { "default parameters" }),
        );
        Ok(())
    }

    /// The neural response model, exposed only once approved ([`crate::neural::next_live_net`]).
    fn refresh_nn(&mut self, shared: &Shared) -> anyhow::Result<()> {
        let was_stored = matches!(self.nn.seen, Some(Some(_)));
        let Some(nn_json) = self.nn.changed(&shared.store)? else { return Ok(()) };
        let stored = nn_json.as_deref().and_then(|j| serde_json::from_str::<StoredNet>(j).ok());
        let current = shared.nn.read().clone();
        let loaded = crate::neural::next_live_net(current, stored);
        let active = loaded.is_some();
        *shared.nn.write() = loaded;
        if was_stored || active {
            shared.log("learner", "info", format!("neural response model {}", if active { "active" } else { "inactive" }));
        }
        Ok(())
    }

    /// Strategy knobs the learner promoted; the Monte Carlo budget and live fits stay local
    /// (the contract lives on `Params::adopt_promoted`, next to the fields).
    fn refresh_params(&mut self, shared: &Shared) -> anyhow::Result<()> {
        let Some(current) = self.params.changed(&shared.store)? else { return Ok(()) };
        match current.as_deref().map(serde_json::from_str::<Params>) {
            Some(Err(e)) => shared.log("learner", "error", format!("promoted parameters are unreadable, keeping the current ones: {e}")),
            Some(Ok(p)) => {
                shared.params.write().adopt_promoted(p);
                if let Some(v) = crate::live::lineage_head(&shared.store) {
                    *shared.champion_version.write() = v;
                }
                shared.log("learner", "info", "promoted strategy parameters are now live");
            }
            None => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Make every kv read fail (or work again) by renaming the table under a second connection.
    fn break_kv(shared: &Shared, broken: bool) {
        let conn = rusqlite::Connection::open(shared.config.artifacts.join("svanbot10.db")).unwrap();
        let sql = if broken { "ALTER TABLE kv RENAME TO kv_hidden" } else { "ALTER TABLE kv_hidden RENAME TO kv" };
        conn.execute_batch(sql).unwrap();
    }

    fn range_json(soft_width: f32) -> String {
        let p = sv10_core::oprange::RangeParams { soft_width, ..Default::default() };
        serde_json::json!({"active": true, "params": p, "report": {}, "fitted_at": 0.0}).to_string()
    }

    fn fixture(tag: &str) -> Arc<Shared> {
        let shared = Shared::for_test(tag, &["A"]);
        shared.store.put_kv(crate::RANGE_PARAMS_KEY, &range_json(0.5)).unwrap();
        shared
    }

    fn params_json(shared: &Shared) -> serde_json::Value {
        serde_json::to_value(&*shared.params.read()).unwrap()
    }

    #[test]
    fn a_failed_store_read_keeps_every_installed_artifact() {
        let shared = fixture("installs-keep");
        let mut installs = Installs::after_startup(&shared.store);
        // Startup already installed the stored range model; the first refresh leaves it.
        shared.params.write().range = crate::fitted_range_params(Some(&range_json(0.5))).unwrap();
        installs.refresh(&shared);
        let before = params_json(&shared);
        let nn_before = shared.nn.read().is_some();

        break_kv(&shared, true);
        installs.refresh(&shared);
        installs.refresh(&shared);
        assert_eq!(params_json(&shared), before, "an unreadable store must not change the params");
        assert_eq!(shared.nn.read().is_some(), nn_before);
        let warnings = shared.log.lock().iter().filter(|l| l.level == "warn").count();
        assert_eq!(warnings, 1, "one warning per failure streak");

        break_kv(&shared, false);
        installs.refresh(&shared);
        assert_eq!(params_json(&shared), before, "reading the same values again changes nothing");
        assert!(shared.log.lock().iter().any(|l| l.message == "store readable again"));
    }

    #[test]
    fn a_changed_range_model_installs_after_a_read_error() {
        let shared = fixture("installs-change");
        let mut installs = Installs::after_startup(&shared.store);
        break_kv(&shared, true);
        installs.refresh(&shared);
        break_kv(&shared, false);
        shared.store.put_kv(crate::RANGE_PARAMS_KEY, &range_json(0.3)).unwrap();
        installs.refresh(&shared);
        assert_eq!(shared.params.read().range.soft_width, 0.3);
    }

    #[test]
    fn promoted_params_install_once() {
        let shared = fixture("installs-params");
        let mut installs = Installs::after_startup(&shared.store);
        let promoted = Params { call_margin: 0.123, ..Default::default() };
        shared.store.put_kv(PARAMS_KEY, &serde_json::to_string(&promoted).unwrap()).unwrap();
        installs.refresh(&shared);
        assert_eq!(shared.params.read().call_margin, 0.123);
        installs.refresh(&shared);
        let installs_logged = shared.log.lock().iter().filter(|l| l.message.starts_with("promoted strategy")).count();
        assert_eq!(installs_logged, 1);
    }
}

#[cfg(test)]
mod profile_retry_tests {
    use super::*;

    #[test]
    fn unchanged_profile_retries_after_hardware_read_recovers() {
        let shared = Shared::for_test("profile-hardware-retry", &["A"]);
        let hardware = r#"{"logical_cores":8,"tuning":{"live_samples":4000,"decision_samples":1000}}"#;
        shared.store.put_kv(crate::HARDWARE_PROFILE_KEY, hardware).unwrap();
        shared.params.write().samples = 4000;
        let mut installs = Installs::after_startup(&shared.store);
        installs.refresh(&shared);
        shared
            .store
            .put_kv(crate::profile::PROFILE_KEY, r#"{"name":"quiet","live_scale":0.25,"learner_threads":2,"analyst_threads":1}"#)
            .unwrap();
        let writer = rusqlite::Connection::open(shared.config.artifacts.join("svanbot10.db")).unwrap();
        writer.execute("UPDATE kv SET value=x'00' WHERE key=?1", [crate::HARDWARE_PROFILE_KEY]).unwrap();
        installs.refresh(&shared);
        assert_eq!(shared.params.read().samples, 4000);
        installs.refresh(&shared);
        assert_eq!(shared.log.lock().iter().filter(|l| l.level == "warn" && l.message.contains("store unreadable")).count(), 1);
        shared.store.put_kv(crate::HARDWARE_PROFILE_KEY, hardware).unwrap();
        installs.refresh(&shared);
        assert_eq!(
            shared.params.read().samples,
            1000,
            "the unchanged operator profile must still install after the dependent read recovers"
        );
        assert!(shared.log.lock().iter().any(|l| l.message == "store readable again"));
    }
}
