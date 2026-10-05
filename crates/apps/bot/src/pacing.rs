//! Learner pacing separates evidence refreshes from champion search.
//!
//! An evidence refresh waits for nothing but a new hand (#314): the fits are what live play reads,
//! and hands arrive at the fleet's play rate, so a refresh runs whenever one is not already
//! running. Champion search keeps its own gate — the dashboard's `min_new_hands` threshold after
//! season day three, the cooldown before it, and the no-promotion backoff — because a champion is
//! replaced only on fresh-deal evidence and a handful of new hands cannot pay for a search. A
//! search yields to a refresh only when the fits are a whole evidence epoch behind (over a
//! population the ledger no longer speaks for); within the epoch it goes first, and the refresh
//! resumes the moment it ends. Separate watermarks prevent early-season searches from repeatedly
//! fitting unchanged evidence. The promotion gate is independent of this scheduler.

use serde::{Deserialize, Serialize};

/// Key of the persisted [`PacingState`], so restarts and hot swaps do not trigger a job.
pub const PACING_KEY: &str = "learner.pacing";

/// Key of the operator's learner settings ([`LearnerSettings`]), written by the dashboard.
pub const SETTINGS_KEY: &str = "learner.settings";

/// Longest cooldown the dashboard accepts, in minutes (one day).
pub const MAX_COOLDOWN_MINUTES: f64 = 1440.0;

/// Largest new-hands limit the dashboard accepts (about two days of fleet play).
pub const MAX_MIN_NEW_HANDS: i64 = 20_000;

/// Hands per evidence epoch: the granularity at which the population a search measures against
/// counts as changed.
///
/// Refreshes run on hands as they arrive (#314), so the refresh watermark alone would retire the
/// rejection ledger (0285) and the experiment target queue every time an opponent played a hand,
/// and every search would re-spend its budget re-measuring transitions it had already decided. Both
/// are scoped to the epoch instead: a bar measured within the epoch stands, and the fits inside the
/// epoch are what make a search's measurements current.
///
/// 500 hands is the batch threshold a refresh used to wait for, so a bar never outlives more
/// population drift than it did before (the batch refresh ran first and retired the scope anyway) —
/// and a search threshold below the epoch, which the dashboard can set, now lets a bar outlive the
/// refresh that lands inside it.
pub const EVIDENCE_EPOCH_HANDS: i64 = 500;

/// The evidence epoch of a hand rowid: the rowid rounded down to [`EVIDENCE_EPOCH_HANDS`]. Derived
/// rather than stored, so a restarted or hot-swapped learner computes the same scope the search it
/// resumes was measured under.
pub fn evidence_epoch(rowid: i64) -> i64 {
    rowid - rowid.rem_euclid(EVIDENCE_EPOCH_HANDS)
}

/// Operator-set learner settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LearnerSettings {
    /// Minutes between the end of a champion search and the start of the next one.
    pub cooldown_minutes: Option<f64>,
    /// New live hands that start the next champion search at once (0: back to back); overrides
    /// `LEARNER_MIN_NEW_HANDS`. Evidence refreshes run on new hands whatever this is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_new_hands: Option<i64>,
}

fn valid_cooldown(m: f64) -> bool {
    m.is_finite() && (0.0..=MAX_COOLDOWN_MINUTES).contains(&m)
}

fn valid_hands(n: i64) -> bool {
    (0..=MAX_MIN_NEW_HANDS).contains(&n)
}

impl LearnerSettings {
    /// Parse a stored settings value; unreadable or out-of-range values are ignored.
    pub fn parse(json: Option<&str>) -> LearnerSettings {
        let mut s: LearnerSettings = json.and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
        s.cooldown_minutes = s.cooldown_minutes.filter(|m| valid_cooldown(*m));
        s.min_new_hands = s.min_new_hands.filter(|n| valid_hands(*n));
        s
    }

    /// These settings with a dashboard update applied: only the fields it sends change, and any
    /// invalid field (or none at all) rejects the whole update.
    pub fn update(&self, body: &serde_json::Value) -> Result<LearnerSettings, String> {
        let mut next = self.clone();
        let mut any = false;
        if let Some(v) = body.get("cooldown_minutes") {
            let m = v
                .as_f64()
                .filter(|m| valid_cooldown(*m))
                .ok_or(format!("cooldown_minutes must be a number from 0 to {MAX_COOLDOWN_MINUTES}"))?;
            next.cooldown_minutes = Some(m.round());
            any = true;
        }
        if let Some(v) = body.get("min_new_hands") {
            let n = v.as_f64().filter(|n| n.is_finite()).map(|n| n.round() as i64).filter(|n| valid_hands(*n));
            next.min_new_hands = Some(n.ok_or(format!("min_new_hands must be a whole number from 0 to {MAX_MIN_NEW_HANDS}"))?);
            any = true;
        }
        if any { Ok(next) } else { Err("send cooldown_minutes and/or min_new_hands".into()) }
    }
}

/// Pacing configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Pacing {
    /// Hands required before a champion search after season day three. Evidence refreshes do not
    /// wait for a threshold: they run on the hands as they arrive (#314).
    pub min_new_hands: i64,
    /// Largest multiplier the no-promotion backoff reaches.
    pub max_backoff: i64,
    /// A champion search runs after this long regardless of new hands.
    pub max_idle_secs: f64,
    /// Minimum time from the end of one search to the start of the next.
    pub cooldown_secs: f64,
}

impl Default for Pacing {
    fn default() -> Self {
        Pacing { min_new_hands: 500, max_backoff: 1, max_idle_secs: 6.0 * 3600.0, cooldown_secs: 3600.0 }
    }
}

impl Pacing {
    /// Defaults overridden by `LEARNER_MIN_NEW_HANDS` (the champion-search threshold; refreshes run
    /// on new hands regardless), `LEARNER_MAX_BACKOFF`, `LEARNER_MAX_IDLE_HOURS` and
    /// `LEARNER_COOLDOWN_MINUTES` (`LEARNER_MIN_NEW_HANDS=0 LEARNER_COOLDOWN_MINUTES=0` restores
    /// back-to-back searches).
    pub fn from_env() -> Self {
        let d = Pacing::default();
        let get = |k: &str| std::env::var(k).ok().and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| *v >= 0.0);
        Pacing {
            min_new_hands: get("LEARNER_MIN_NEW_HANDS").map_or(d.min_new_hands, |v| v as i64),
            max_backoff: get("LEARNER_MAX_BACKOFF").map_or(d.max_backoff, |v| (v as i64).max(1)),
            max_idle_secs: get("LEARNER_MAX_IDLE_HOURS").map_or(d.max_idle_secs, |v| v * 3600.0),
            cooldown_secs: get("LEARNER_COOLDOWN_MINUTES").map_or(d.cooldown_secs, |v| v * 60.0),
        }
    }

    /// This pacing with the dashboard's settings applied.
    pub fn with_settings(&self, settings: &LearnerSettings) -> Pacing {
        Pacing {
            cooldown_secs: settings.cooldown_minutes.map_or(self.cooldown_secs, |m| m * 60.0),
            min_new_hands: settings.min_new_hands.unwrap_or(self.min_new_hands),
            ..self.clone()
        }
    }

    /// Hands needed after `streak` consecutive searches without a promotion.
    pub fn required(&self, streak: u32) -> i64 {
        let mult = 1i64 << streak.saturating_sub(1).min(20);
        self.min_new_hands.saturating_mul(mult.min(self.max_backoff))
    }

    /// Champion-search hand target at `now`. Search is cooldown-paced for the first three season
    /// days, then capped by the same evidence threshold as refits. Missing season state is treated
    /// as late season.
    pub fn search_required(&self, streak: u32, season_started_at: Option<f64>, now: f64) -> i64 {
        let early = season_started_at.is_some_and(|started| now >= started && now - started < 3.0 * 86_400.0);
        if early { 0 } else { self.required(streak) }
    }

    /// Select the next learner job. `forced_at` is the latest operator start request.
    pub fn gate(&self, st: &PacingState, max_rowid: i64, now: f64, forced_at: Option<f64>, season_started_at: Option<f64>) -> Gate {
        let early = season_started_at.is_some_and(|started| now >= started && now - started < 3.0 * 86_400.0);
        let search_needed = if early { 0 } else { self.required(st.streak) };
        let refit_have = (max_rowid - st.refit_rowid()).max(0);
        let search_have = (max_rowid - st.search_rowid()).max(0);
        let cooldown_until = st.search_end().max(st.search_run()) + self.cooldown_secs;
        let forced = forced_at.is_some_and(|t| t > st.search_run());
        let first_search = st.search_run() <= 0.0;
        let search_due = forced
            || first_search
            || (!early && search_have >= search_needed)
            || (now >= cooldown_until && (early || st.follow_up || now - st.search_run() >= self.max_idle_secs));

        // An evidence refresh waits for nothing but a new hand (#314). It goes first when the fits
        // are a whole epoch behind — a search that starts on models older than the scope it will be
        // measured under is what the batch threshold used to prevent — and otherwise whenever a
        // search is not itself due. The other order would starve the search: hands arrive while a
        // refresh runs, so a refresh that always went first would always be due again.
        let refit_due = refit_have > 0;
        let fits_stale = evidence_epoch(max_rowid) != evidence_epoch(st.refit_rowid());
        if refit_due && (fits_stale || !search_due) {
            return Gate::Run { job: LearnerJob::Refit, reason: "new hands since the last refresh" };
        }
        if search_due {
            let reason = if forced {
                "operator request"
            } else if first_search {
                "first champion search"
            } else if st.follow_up {
                "follow-up search around the new champion"
            } else if early {
                "season days 1-3 accelerated search"
            } else if search_have >= search_needed {
                "enough new hands"
            } else {
                "maximum idle time reached"
            };
            return Gate::Run { job: LearnerJob::Search, reason };
        }

        // Only a search waits here: reaching this point means no hand has arrived since the last
        // refresh, so the fits are current and the search's own gate has not opened. The search is
        // what the panel counts towards; the hands that arrive while it waits are refreshed on the
        // way, and a refresh never holds the search back by more than its own duration.
        let reason = if now < cooldown_until && (early || st.follow_up) {
            format!(
                "champion search cooling down for {:.0} more min{}",
                ((cooldown_until - now) / 60.0).ceil(),
                if early { " during season days 1-3" } else { "" }
            )
        } else {
            format!("{search_have} of {search_needed} new hands for champion search")
        };
        Gate::Wait {
            job: LearnerJob::Search,
            have: search_have,
            needed: search_needed,
            cooldown_until: (now < cooldown_until).then_some(cooldown_until),
            reason,
        }
    }
}

/// Persisted pacing state. Legacy cycle fields remain the search state for JSON compatibility.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PacingState {
    /// Newest hand rowid when the last champion search started.
    pub last_rowid: i64,
    /// Unix time the last champion search started (0 before the first).
    pub last_run: f64,
    /// Consecutive searches without a promotion.
    pub streak: u32,
    /// The last search promoted, so another search follows after cooldown.
    pub follow_up: bool,
    /// Unix time the last champion search ended.
    #[serde(default)]
    pub last_end: f64,
    /// Newest hand included in the last evidence refresh.
    #[serde(default)]
    pub refit_rowid: i64,
    /// Unix time the last evidence refresh started.
    #[serde(default)]
    pub refit_run: f64,
    /// Unix time the last evidence refresh ended.
    #[serde(default)]
    pub refit_end: f64,
}

impl PacingState {
    fn refit_rowid(&self) -> i64 {
        if self.refit_rowid == 0 { self.last_rowid } else { self.refit_rowid }
    }

    fn search_rowid(&self) -> i64 {
        self.last_rowid
    }

    fn search_run(&self) -> f64 {
        self.last_run
    }

    fn search_end(&self) -> f64 {
        self.last_end.max(self.last_run)
    }

    /// Fill split refit fields from the legacy search fields after deserialization.
    pub fn migrate(&mut self) {
        if self.refit_rowid == 0 && self.last_rowid != 0 {
            self.refit_rowid = self.last_rowid;
        }
        if self.refit_run == 0.0 && self.last_run != 0.0 {
            self.refit_run = self.last_run;
            self.refit_end = self.last_end.max(self.last_run);
        }
    }

    /// Record an evidence refresh.
    pub fn finish_refit(&mut self, start_rowid: i64, started: f64, ended: f64) {
        self.refit_rowid = start_rowid;
        self.refit_run = started;
        self.refit_end = ended;
    }

    /// Record a champion search.
    pub fn finish_search(&mut self, start_rowid: i64, started: f64, ended: f64, promoted: bool) {
        self.last_rowid = start_rowid;
        self.last_run = started;
        self.last_end = ended;
        self.follow_up = promoted;
        self.streak = if promoted { 0 } else { self.streak.saturating_add(1) };
    }
}

/// The training commands the learner acts on. Anything else is refused where it arrives: stored and
/// answered `ok`, a `stop`, `automatic` or `rollback` read as queued and nothing ever happened.
pub fn learner_acts_on(command: &str) -> bool {
    matches!(command, "start" | "run")
}

/// Time of an operator start request in a `learner.command` value: the dashboard's "Start next
/// search early" sends `start`; `run` is accepted as an alias.
pub fn operator_start_at(command_json: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(command_json).ok()?;
    v["command"].as_str().is_some_and(learner_acts_on).then(|| v["ts"].as_f64()).flatten()
}

/// A schedulable learner job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LearnerJob {
    /// Refresh evidence-derived fits and models.
    Refit,
    /// Search and confirm a champion challenger.
    Search,
}

impl LearnerJob {
    /// Dashboard label.
    pub fn label(self) -> &'static str {
        match self {
            LearnerJob::Refit => "Evidence refresh",
            LearnerJob::Search => "Champion search",
        }
    }
}

/// Pacing decision.
#[derive(Clone, Debug, PartialEq)]
pub enum Gate {
    /// Run one job, with the reason.
    Run {
        /// Selected job.
        job: LearnerJob,
        /// Why it is due.
        reason: &'static str,
    },
    /// Keep waiting for the search's gate or its cooldown. A refresh is never what a wait is for:
    /// it waits only for a hand, and a hand that arrives starts one before the search.
    Wait {
        /// Job expected to run next (a search today).
        job: LearnerJob,
        /// New hands accumulated for it.
        have: i64,
        /// New hands required.
        needed: i64,
        /// When the search cooldown ends, if it currently binds.
        cooldown_until: Option<f64>,
        /// Why this is the next job.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pacing() -> Pacing {
        Pacing { min_new_hands: 500, max_backoff: 1, max_idle_secs: 6.0 * 3600.0, cooldown_secs: 3600.0 }
    }

    fn state() -> PacingState {
        PacingState {
            last_rowid: 1_000,
            last_run: 10_000.0,
            last_end: 10_600.0,
            streak: 1,
            follow_up: false,
            refit_rowid: 1_000,
            refit_run: 10_000.0,
            refit_end: 10_600.0,
        }
    }

    #[test]
    fn early_season_searches_after_cooldown_without_new_hands() {
        let p = pacing();
        let st = state();
        let season_start = 9_000.0;
        assert_eq!(
            p.gate(&st, 1_000, 11_000.0, None, Some(season_start)),
            Gate::Wait {
                job: LearnerJob::Search,
                have: 0,
                needed: 0,
                cooldown_until: Some(14_200.0),
                reason: "champion search cooling down for 54 more min during season days 1-3".into(),
            }
        );
        assert_eq!(
            p.gate(&st, 1_000, 14_200.0, None, Some(season_start)),
            Gate::Run { job: LearnerJob::Search, reason: "season days 1-3 accelerated search" }
        );
    }

    #[test]
    fn late_or_unknown_season_search_waits_for_dashboard_threshold() {
        let p = pacing();
        let mut st = state();
        st.refit_rowid = 1_500;
        let late = Some(10_000.0 - 4.0 * 86_400.0);
        for season in [late, None] {
            assert_eq!(p.search_required(1, season, 20_000.0), 500);
            assert!(matches!(
                p.gate(&st, 1_499, 20_000.0, None, season),
                Gate::Wait { job: LearnerJob::Search, have: 499, needed: 500, .. }
            ));
            assert_eq!(p.gate(&st, 1_500, 20_000.0, None, season), Gate::Run { job: LearnerJob::Search, reason: "enough new hands" });
        }
    }

    #[test]
    fn a_refresh_runs_on_the_hands_as_they_arrive() {
        let p = pacing();
        let st = state();
        // One new hand, far short of any threshold: the fits are refreshed anyway (#314).
        assert_eq!(
            p.gate(&st, 1_001, 20_000.0, None, None),
            Gate::Run { job: LearnerJob::Refit, reason: "new hands since the last refresh" }
        );
    }

    #[test]
    fn a_due_search_goes_before_a_refresh_within_the_epoch() {
        // A search threshold inside the epoch (the dashboard can set one): the search runs without
        // waiting for a refresh, because the hands it will be measured against were refreshed
        // minutes ago. With the default 500-hand threshold a due search has always crossed an epoch
        // boundary, so it waits for the one refresh first — what the batch gate used to force.
        let p = Pacing { min_new_hands: 100, ..pacing() };
        let st = state();
        assert_eq!(p.gate(&st, 1_200, 20_000.0, None, None), Gate::Run { job: LearnerJob::Search, reason: "enough new hands" });
    }

    #[test]
    fn a_refresh_goes_first_when_the_fits_are_an_epoch_behind() {
        let p = pacing();
        let st = state();
        // A search is due at a hand two epochs past the one the fits were refitted in: it waits for
        // one refresh, then runs. A search measured under a scope the population has left is what
        // the batch threshold used to prevent.
        let behind = 4 * EVIDENCE_EPOCH_HANDS;
        assert_eq!(
            p.gate(&st, behind, 20_000.0, None, None),
            Gate::Run { job: LearnerJob::Refit, reason: "new hands since the last refresh" }
        );
        let mut refitted = state();
        refitted.finish_refit(behind, 20_000.0, 20_010.0);
        assert_eq!(p.gate(&refitted, behind + 1, 20_020.0, None, None), Gate::Run { job: LearnerJob::Search, reason: "enough new hands" });
    }

    #[test]
    fn an_epoch_is_five_hundred_hands_of_rowid() {
        assert_eq!((evidence_epoch(0), evidence_epoch(499), evidence_epoch(500), evidence_epoch(1_499)), (0, 0, 500, 1_000));
    }

    #[test]
    fn refit_and_search_watermarks_advance_independently() {
        let mut st = state();
        st.finish_refit(1_500, 20_000.0, 20_010.0);
        assert_eq!((st.refit_rowid, st.last_rowid, st.last_run), (1_500, 1_000, 10_000.0));
        st.finish_search(1_600, 21_000.0, 21_600.0, true);
        assert_eq!((st.refit_rowid, st.last_rowid, st.follow_up, st.streak), (1_500, 1_600, true, 0));
    }

    #[test]
    fn legacy_state_migrates_refit_watermark() {
        let mut st: PacingState =
            serde_json::from_str(r#"{"last_rowid":42,"last_run":100.0,"last_end":120.0,"streak":2,"follow_up":false}"#).unwrap();
        st.migrate();
        assert_eq!((st.refit_rowid, st.refit_run, st.refit_end), (42, 100.0, 120.0));
    }

    #[test]
    fn operator_request_forces_search_and_promotion_resets_streak() {
        let p = pacing();
        let mut st = state();
        assert_eq!(p.gate(&st, 1_100, 11_000.0, Some(10_500.0), None), Gate::Run { job: LearnerJob::Search, reason: "operator request" });
        st.streak = 3;
        st.finish_search(1_100, 11_000.0, 11_500.0, true);
        assert_eq!((st.streak, st.follow_up), (0, true));
    }

    #[test]
    fn dashboard_settings_parse_update_and_override() {
        let p = Pacing { min_new_hands: 569, ..pacing() };
        let set = LearnerSettings::parse(Some(r#"{"cooldown_minutes":30,"min_new_hands":150}"#));
        assert_eq!((set.cooldown_minutes, set.min_new_hands), (Some(30.0), Some(150)));
        assert_eq!((p.with_settings(&set).min_new_hands, p.with_settings(&set).cooldown_secs), (150, 1800.0));
        let merged = set.update(&serde_json::json!({"min_new_hands": 800})).unwrap();
        assert_eq!((merged.cooldown_minutes, merged.min_new_hands), (Some(30.0), Some(800)));
        assert!(set.update(&serde_json::json!({"min_new_hands": "lots"})).is_err());
        assert_eq!(LearnerSettings::parse(Some(r#"{"min_new_hands":-5}"#)).min_new_hands, None);
    }

    #[test]
    fn dashboard_start_and_run_alias_request_a_search() {
        assert_eq!(operator_start_at(r#"{"command":"start","ts":1700.5}"#), Some(1700.5));
        assert_eq!(operator_start_at(r#"{"command":"run","ts":12}"#), Some(12.0));
        assert_eq!(operator_start_at(r#"{"command":"stop","ts":12}"#), None);
        assert_eq!(operator_start_at("not json"), None);
    }
}
