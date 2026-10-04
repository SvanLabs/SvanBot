//! What live play has installed from the learner's store (0222): promoted parameters, the neural
//! response model, the fitted range model, the compute profile's live budget, the live fits
//! ([`crate::livefits`]) and the per-opponent corrections ([`crate::playerfits`]).
//!
//! One rule for all of them: an installed artifact changes only when a store read succeeds and
//! returns something different. A failed read or malformed record keeps what is in play (0205 set this for the live
//! fits; before 0222 a failed read of the neural or range key uninstalled that model until the
//! next tick) and is logged once per failure streak.

use crate::live::Shared;
use crate::{NN_KEY, PARAMS_KEY, StoredNet};
use sv10_core::policy::Params;
use sv10_store::store::Store;

/// Read a present artifact fallibly, including its typed JSON contract. Absence is legitimate;
/// unreadable records are not deletion requests and must preserve an installed incumbent.
pub(crate) fn read_checked<T: serde::de::DeserializeOwned>(store: &Store, key: &str) -> anyhow::Result<Option<String>> {
    use anyhow::Context;
    let json = store.get_kv(key)?;
    if let Some(j) = &json {
        serde_json::from_str::<T>(j).with_context(|| format!("artifact {key} is malformed"))?;
    }
    Ok(json)
}

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
    /// Per bot slot: the `params.slot.<bot>` value last read (`Some(None)` = absent), `None` until read.
    slots: Vec<Option<Option<String>>>,
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
            slots: Vec::new(),
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
        if let Err(e) = self.refresh_slots(shared) {
            errors.push(format!("per-bot parameters: {e}"));
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
        if let Some(json) = &profile_json {
            serde_json::from_str::<crate::profile::ComputeProfile>(json)?;
        }
        let logical = hardware
            .as_deref()
            .and_then(|h| serde_json::from_str::<serde_json::Value>(h).ok())
            .and_then(|h| h["logical_cores"].as_u64())
            .unwrap_or(1) as usize;
        let budget = crate::profile::hardware_budget(hardware.as_deref());
        if hardware.is_some() && budget.is_none() {
            anyhow::bail!("hardware profile has no valid live budget");
        }
        self.profile = pending;
        if let Some((base, floor)) = budget {
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
        let mut pending = self.range.clone();
        let Some(range_json) = pending.changed(&shared.store)? else { return Ok(()) };
        let stored = range_json.as_deref().map(serde_json::from_str::<crate::StoredRangeParams>).transpose()?;
        let fitted = stored.filter(|s| s.active).map(|s| s.params);
        self.range = pending;
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
        let mut pending = self.nn.clone();
        let Some(nn_json) = pending.changed(&shared.store)? else { return Ok(()) };
        let stored = nn_json.as_deref().map(serde_json::from_str::<StoredNet>).transpose()?;
        self.nn = pending;
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
        let mut pending = self.params.clone();
        let Some(current) = pending.changed(&shared.store)? else { return Ok(()) };
        let promoted = current.as_deref().map(serde_json::from_str::<Params>).transpose()?;
        self.params = pending;
        if let Some(p) = promoted {
            shared.params.write().adopt_promoted(p);
            if let Some(v) = crate::live::lineage_head(&shared.store) {
                *shared.champion_version.write() = v;
            }
            shared.log("learner", "info", "promoted strategy parameters are now live");
        }
        Ok(())
    }
}

impl Installs {
    /// Each bot's own lineage knobs, read from `params.slot.<bot>`; a bot without the key plays the
    /// shared champion. A failed read or malformed record keeps what that bot has (ADR 0002 stage 1).
    fn refresh_slots(&mut self, shared: &Shared) -> anyhow::Result<()> {
        self.slots.resize(shared.bots.len(), None);
        let mut failed = None;
        for slot in 0..shared.bots.len() {
            let name = shared.bots[slot].read().name.clone();
            let key = crate::slot_params_key(&name);
            let outcome = shared.store.get_kv(&key).and_then(|raw| {
                if self.slots[slot].as_ref() == Some(&raw) {
                    return Ok(None);
                }
                let own = raw.as_deref().map(serde_json::from_str::<Params>).transpose()?;
                Ok(Some((raw, own)))
            });
            match outcome {
                Ok(None) => {}
                Ok(Some((raw, own))) => {
                    self.slots[slot] = Some(raw);
                    let changed = own.is_some() || shared.bots[slot].read().slot_params.is_some();
                    let msg = if own.is_some() { "own strategy parameters are now live" } else { "plays the shared champion again" };
                    let version = shared
                        .store
                        .get_kv(&crate::learner::lane::Lane::bot(&name).key(crate::LEARNER_LINEAGE_KEY))?
                        .and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok())
                        .and_then(|l| l.last().cloned());
                    let mut bot = shared.bots[slot].write();
                    bot.slot_params = own;
                    bot.slot_version = version;
                    drop(bot);
                    if changed {
                        shared.log(&name, "info", msg);
                    }
                }
                Err(e) => failed = Some(format!("{key}: {e}")),
            }
        }
        failed.map_or(Ok(()), |e| Err(anyhow::anyhow!(e)))
    }
}

#[cfg(test)]
mod tests;

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
