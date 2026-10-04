//! When experiment mode is on, and for whom (0266): a pure state machine over validated
//! leaderboard readings, driven by a five-minute poll and tested with a fake clock.
//!
//! Entry needs three consecutive qualifying readings of one active season, five minutes apart:
//! all five fleet bots on the official score board and four of them at #1–#4. Entry freezes the
//! protected trio (the fleet bots at #1–#3) and the experiment pair (the bot at #4 and the fifth).
//! Any failed reading, a season change, a lost top four, a protected bot out of #1–#3 or a pair bot
//! in #1–#3 ends the mode; so does a reading older than [`STALE_AFTER_SECS`] at the moment a hand
//! starts. A restart begins in champion mode.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Poll interval of the official leaderboard.
pub const POLL_SECS: u64 = 300;
/// Qualifying readings needed to enter.
pub const QUALIFYING_READINGS: usize = 3;
/// A qualifying reading counts only this long after the previous one (five minutes, less poll jitter).
pub const MIN_SPACING_SECS: f64 = 270.0;
/// Readings further apart than this are not consecutive: a missed poll restarts the count.
pub const MAX_SPACING_SECS: f64 = 480.0;
/// The mode is in force only while its last validated reading is at most this old (one late poll).
pub const STALE_AFTER_SECS: f64 = 480.0;
/// Fleet bots that must hold the top places.
const TOP: i64 = 4;

/// One validated reading of the active season's official score leaderboard.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// Season identity (`season_id`, else the season number).
    pub season: String,
    /// Unix seconds of the reading.
    pub at: f64,
    /// Rank of every fleet bot.
    pub ranks: BTreeMap<String, i64>,
}

impl Reading {
    /// Validate `GET /season/current` and `GET /season/leaderboard?sort_by=score` for `fleet`.
    /// Every failure is a reason the mode cannot use this reading.
    pub fn parse(season: &Value, entries: &Value, fleet: &[String], at: f64) -> Result<Reading, String> {
        if season["status"].as_str() != Some("active") {
            return Err(format!("season status is {}, not active", season["status"]));
        }
        let id = season["season_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| season["season_number"].as_i64().map(|n| n.to_string()))
            .ok_or("season has no identity")?;
        let entries = entries.as_array().ok_or("leaderboard is not a list")?;
        let mut ranks = BTreeMap::new();
        let mut seen_ranks = std::collections::BTreeSet::new();
        for (i, e) in entries.iter().enumerate() {
            let rank = e["rank"].as_i64().ok_or_else(|| format!("leaderboard row {} has no rank", i + 1))?;
            let name = e["bot_name"].as_str().ok_or_else(|| format!("leaderboard row {} has no name", i + 1))?;
            if e["score"].as_i64().is_none() && e["score"].as_f64().is_none() {
                return Err(format!("leaderboard row {} has no score", i + 1));
            }
            if rank < 1 || !seen_ranks.insert(rank) {
                return Err(format!("leaderboard rank {rank} is invalid or repeated"));
            }
            if fleet.iter().any(|f| f == name) {
                ranks.insert(name.to_string(), rank);
            }
        }
        if !(1..=TOP).all(|r| seen_ranks.contains(&r)) {
            return Err("leaderboard is missing one of ranks #1–#4".into());
        }
        if let Some(missing) = fleet.iter().find(|f| !ranks.contains_key(*f)) {
            return Err(format!("fleet bot {missing} is not on the leaderboard"));
        }
        Ok(Reading { season: id, at, ranks })
    }

    /// Fleet bots at #1–#4, best first, when there are exactly four of them.
    fn top_four(&self) -> Option<Vec<String>> {
        let mut top: Vec<(i64, &String)> = self.ranks.iter().filter(|(_, r)| **r <= TOP).map(|(n, r)| (*r, n)).collect();
        top.sort();
        (top.len() == TOP as usize).then(|| top.into_iter().map(|(_, n)| n.clone()).collect())
    }
}

/// Whether the experiment pair may run experiments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Every bot plays the champion.
    #[default]
    Champion,
    /// The frozen pair may play an experiment; the protected trio plays the champion.
    Active,
}

/// The last mode change and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    /// Unix seconds.
    pub at: f64,
    /// The status entered.
    pub to: Status,
    /// Why.
    pub reason: String,
}

/// The persisted mode (`experiment.mode.v1`), written by the process that polls.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeState {
    /// Champion or active.
    pub status: Status,
    /// Season of the qualifying run or of the activation.
    pub season: Option<String>,
    /// Times of the consecutive qualifying readings so far (champion mode only).
    pub qualifying: Vec<f64>,
    /// Fleet bots frozen at #1–#3 for this activation, best first.
    pub protected: Vec<String>,
    /// Frozen experiment pair: the bot at #4, then the fifth fleet bot.
    pub pair: Vec<String>,
    /// When the mode last became active.
    pub activated_at: Option<f64>,
    /// Last successful, validated reading.
    pub last_reading_at: Option<f64>,
    /// Last poll attempt, successful or not.
    pub last_attempt_at: Option<f64>,
    /// Fleet ranks in the last validated reading.
    pub ranks: BTreeMap<String, i64>,
    /// Why the last reading could not be used, if it could not.
    pub last_error: Option<String>,
    /// The last mode change.
    pub last_transition: Option<Transition>,
    /// When this state machine started (a restart starts over in champion mode).
    pub started_at: f64,
}

impl ModeState {
    /// Champion mode at process start.
    pub fn start(now: f64) -> Self {
        ModeState {
            started_at: now,
            last_transition: Some(Transition {
                at: now,
                to: Status::Champion,
                reason: "restart: champion until three new qualifying readings".into(),
            }),
            ..Default::default()
        }
    }

    /// Why this fleet can never enter the mode, when it cannot: qualifying needs all five bots on the
    /// board with four at #1–#4 (#748). Silent before, so a solo box looked like a fleet still
    /// waiting for readings.
    pub fn unavailable(&self) -> Option<String> {
        match self.ranks.len() {
            0 | 5.. => None,
            1 => Some("solo — experiments unavailable: the mode needs the five-bot fleet on the official board".into()),
            n => Some(format!("{n} of 5 fleet bots — experiments unavailable: the mode needs all five on the official board")),
        }
    }

    /// Whether the pair may play an experiment at `now`: active, and its last reading fresh.
    pub fn active_at(&self, now: f64) -> bool {
        self.status == Status::Active && self.last_reading_at.is_some_and(|t| now - t <= STALE_AFTER_SECS && now >= t - 60.0)
    }

    /// Fold one poll result in. Returns the reason when the status changed.
    pub fn observe(&mut self, reading: Result<Reading, String>, now: f64) -> Option<String> {
        self.last_attempt_at = Some(now);
        let reading = match reading {
            Ok(r) => r,
            Err(e) => {
                self.last_error = Some(e.clone());
                self.qualifying.clear();
                return self.end(now, format!("leaderboard reading failed: {e}"));
            }
        };
        self.last_error = None;
        self.last_reading_at = Some(reading.at);
        self.ranks = reading.ranks.clone();
        match self.status {
            Status::Active => {
                let why = self.active_violation(&reading);
                match why {
                    Some(why) => {
                        self.qualifying.clear();
                        self.season = Some(reading.season.clone());
                        // This reading may itself start a new qualifying run.
                        let changed = self.end(now, why);
                        self.qualify(&reading);
                        changed
                    }
                    None => None,
                }
            }
            Status::Champion => {
                if self.season.as_deref() != Some(&reading.season) {
                    self.qualifying.clear();
                    self.season = Some(reading.season.clone());
                }
                self.qualify(&reading);
                if self.qualifying.len() >= QUALIFYING_READINGS {
                    let top = reading.top_four().expect("a qualifying reading has a fleet top four");
                    let fifth = reading.ranks.keys().find(|n| !top.contains(n)).cloned();
                    self.protected = top[..3].to_vec();
                    self.pair = std::iter::once(top[3].clone()).chain(fifth).collect();
                    self.status = Status::Active;
                    self.activated_at = Some(now);
                    self.qualifying.clear();
                    let reason = format!(
                        "{QUALIFYING_READINGS} qualifying readings: protected {}; experiment pair {}",
                        self.protected.join(", "),
                        self.pair.join(", ")
                    );
                    self.last_transition = Some(Transition { at: now, to: Status::Active, reason: reason.clone() });
                    Some(reason)
                } else {
                    None
                }
            }
        }
    }

    /// Mark a stale reading at hand time: the mode ends and must requalify.
    pub fn expire(&mut self, now: f64) -> Option<String> {
        if self.status == Status::Active && !self.active_at(now) {
            self.qualifying.clear();
            return self.end(now, "leaderboard reading is stale".into());
        }
        None
    }

    fn qualify(&mut self, r: &Reading) {
        if r.top_four().is_none() || r.ranks.len() < 5 {
            self.qualifying.clear();
            return;
        }
        match self.qualifying.last() {
            Some(last) if r.at - last < MIN_SPACING_SECS => {} // too soon to count; keeps the run
            Some(last) if r.at - last > MAX_SPACING_SECS => self.qualifying = vec![r.at],
            _ => self.qualifying.push(r.at),
        }
    }

    fn active_violation(&self, r: &Reading) -> Option<String> {
        if self.season.as_deref() != Some(&r.season) {
            return Some(format!("season changed to {}", r.season));
        }
        if r.top_four().is_none() {
            return Some("the fleet no longer holds #1–#4".into());
        }
        if let Some(p) = self.protected.iter().find(|p| r.ranks.get(*p).is_none_or(|rank| *rank > 3)) {
            return Some(format!("protected bot {p} left #1–#3"));
        }
        if let Some(p) = self.pair.iter().find(|p| r.ranks.get(*p).is_some_and(|rank| *rank <= 3)) {
            return Some(format!("experiment bot {p} entered #1–#3"));
        }
        None
    }

    fn end(&mut self, now: f64, reason: String) -> Option<String> {
        if self.status != Status::Active {
            return None;
        }
        self.status = Status::Champion;
        self.protected.clear();
        self.pair.clear();
        self.activated_at = None;
        self.last_transition = Some(Transition { at: now, to: Status::Champion, reason: reason.clone() });
        Some(reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FLEET: [&str; 5] = ["A", "B", "C", "D", "E"];

    fn fleet() -> Vec<String> {
        FLEET.iter().map(|s| s.to_string()).collect()
    }

    /// A board with the given names in rank order (others fill the gaps).
    fn board(order: &[&str]) -> Value {
        json!(
            order.iter().enumerate().map(|(i, n)| json!({"rank": i + 1, "bot_name": n, "score": 1_000_000 - i as i64})).collect::<Vec<_>>()
        )
    }

    fn season(id: &str) -> Value {
        json!({"season_id": id, "season_number": 13, "status": "active"})
    }

    fn reading(order: &[&str], at: f64) -> Result<Reading, String> {
        Reading::parse(&season("s13"), &board(order), &fleet(), at)
    }

    const TOP4: [&str; 7] = ["A", "B", "C", "D", "x", "y", "E"];

    fn activated(t0: f64) -> ModeState {
        let mut m = ModeState::start(t0);
        for i in 0..3 {
            m.observe(reading(&TOP4, t0 + 300.0 * i as f64), t0 + 300.0 * i as f64);
        }
        assert_eq!(m.status, Status::Active);
        m
    }

    #[test]
    fn a_fleet_smaller_than_five_says_why_it_never_qualifies() {
        let mut m = ModeState::start(0.0);
        assert_eq!(m.unavailable(), None, "no reading yet: nothing to say");
        let solo = Reading::parse(&season("s13"), &board(&["x", "y", "z", "w", "A"]), &["A".to_string()], 100.0);
        m.observe(solo, 100.0);
        assert!(m.unavailable().is_some_and(|r| r.starts_with("solo — experiments unavailable")), "{:?}", m.unavailable());
        assert_eq!(m.status, Status::Champion);
        let mut full = ModeState::start(0.0);
        full.observe(reading(&TOP4, 100.0), 100.0);
        assert_eq!(full.unavailable(), None, "five bots: qualifying readings are still counted");
        let three = Reading::parse(&season("s13"), &board(&["A", "B", "C", "x"]), &["A", "B", "C"].map(String::from), 100.0);
        let mut m3 = ModeState::start(0.0);
        m3.observe(three, 100.0);
        assert_eq!(m3.unavailable().as_deref().map(|r| &r[..7]), Some("3 of 5 "));
    }

    #[test]
    fn three_qualifying_readings_five_minutes_apart_activate_and_freeze_identities() {
        let mut m = ModeState::start(0.0);
        assert_eq!(m.observe(reading(&TOP4, 0.0), 0.0), None);
        assert_eq!(m.observe(reading(&TOP4, 120.0), 120.0), None, "a quick poll does not count");
        assert_eq!(m.qualifying.len(), 1);
        assert_eq!(m.observe(reading(&TOP4, 300.0), 300.0), None);
        let why = m.observe(reading(&["B", "A", "C", "D", "x", "E"], 600.0), 600.0).expect("activates");
        assert!(why.contains("experiment pair D, E"), "{why}");
        assert_eq!((m.protected.clone(), m.pair.clone()), (vec!["B".into(), "A".into(), "C".into()], vec!["D".into(), "E".into()]));
        assert!(m.active_at(700.0));
        assert!(!m.active_at(600.0 + STALE_AFTER_SECS + 1.0), "stale readings fail closed");
    }

    #[test]
    fn a_failure_or_a_gap_restarts_qualification() {
        let mut m = ModeState::start(0.0);
        m.observe(reading(&TOP4, 0.0), 0.0);
        m.observe(reading(&TOP4, 300.0), 300.0);
        m.observe(Err("timeout".into()), 600.0);
        assert!(m.qualifying.is_empty() && m.status == Status::Champion);
        m.observe(reading(&TOP4, 900.0), 900.0);
        m.observe(reading(&TOP4, 1_500.0), 1_500.0);
        assert_eq!(m.qualifying, vec![1_500.0], "a missed poll is not consecutive");
        m.observe(reading(&["A", "B", "C", "x", "D", "E"], 1_800.0), 1_800.0);
        assert!(m.qualifying.is_empty(), "top three is not top four");
    }

    #[test]
    fn every_fleet_bot_must_be_identifiable_and_the_board_well_formed() {
        assert!(reading(&["A", "B", "C", "D", "x"], 0.0).unwrap_err().contains("E is not on the leaderboard"));
        let bad = json!([{"rank": 1, "bot_name": "A", "score": 5}, {"rank": 1, "bot_name": "B", "score": 4}]);
        assert!(Reading::parse(&season("s"), &bad, &fleet(), 0.0).is_err());
        let ended = json!({"season_id": "s", "status": "ended"});
        assert!(Reading::parse(&ended, &board(&TOP4), &fleet(), 0.0).unwrap_err().contains("not active"));
        assert!(Reading::parse(&season("s"), &json!({"error": "x"}), &fleet(), 0.0).is_err());
    }

    #[test]
    fn every_exit_condition_returns_to_champion_and_requires_requalifying() {
        let exits: [(&[&str], &str, &str); 5] = [
            (&["A", "B", "C", "x", "D", "E"], "s13", "no longer holds #1–#4"),
            (&["A", "B", "D", "C", "x", "E"], "s13", "protected bot C left #1–#3"),
            (&["A", "B", "C", "E", "D"], "s13", "E"),
            (&TOP4, "s14", "season changed"),
            (&["A", "B", "x", "C", "D", "E"], "s13", "no longer holds #1–#4"),
        ];
        for (order, season_id, expect) in exits {
            let mut m = activated(0.0);
            let r = Reading::parse(&season(season_id), &board(order), &fleet(), 900.0);
            // "A B C E D": E at #4 is a swap inside the pair, which does not end the mode.
            let why = m.observe(r, 900.0);
            if expect == "E" {
                assert_eq!(why, None, "order inside the pair may change");
                assert_eq!(m.pair, vec!["D".to_string(), "E".to_string()], "identities stay frozen");
                continue;
            }
            let why = why.unwrap_or_else(|| panic!("{order:?} must end the mode"));
            assert!(why.contains(expect), "{why} vs {expect}");
            assert_eq!(m.status, Status::Champion);
            assert!(m.pair.is_empty() && m.protected.is_empty());
            assert!(m.qualifying.len() <= 1, "re-entry needs three new readings");
        }
    }

    #[test]
    fn order_changes_inside_the_trio_keep_the_mode_and_failures_end_it() {
        let mut m = activated(0.0);
        assert_eq!(m.observe(reading(&["C", "A", "B", "D", "E"], 900.0), 900.0), None);
        assert!(m.observe(Err("HTTP 503".into()), 1_200.0).unwrap().contains("HTTP 503"));
        let mut m = activated(0.0);
        assert!(m.expire(600.0 + STALE_AFTER_SECS + 1.0).unwrap().contains("stale"));
        assert_eq!(m.status, Status::Champion);
    }

    #[test]
    fn a_pair_bot_climbing_into_the_trio_ends_it_under_every_permutation() {
        // Every placement of the five fleet bots over ranks 1..=6: the mode survives exactly when
        // the frozen trio holds #1–#3 and the pair holds #4 (the other pair bot anywhere else).
        let slots = [1i64, 2, 3, 4, 5, 6];
        let mut checked = 0;
        for perm in permutations(&slots, 5) {
            let mut m = activated(0.0);
            let mut order: Vec<&str> = vec!["x"; 6];
            for (bot, rank) in FLEET.iter().zip(&perm) {
                order[*rank as usize - 1] = bot;
            }
            let survives = m.observe(reading(&order, 900.0), 900.0).is_none();
            let expected = ["A", "B", "C"].iter().all(|p| perm[FLEET.iter().position(|f| f == p).unwrap()] <= 3)
                && ["D", "E"].iter().any(|p| perm[FLEET.iter().position(|f| f == p).unwrap()] == 4);
            assert_eq!(survives, expected, "{order:?}");
            assert_eq!(m.status == Status::Active, survives);
            checked += 1;
        }
        assert_eq!(checked, 720);
    }

    fn permutations(items: &[i64], k: usize) -> Vec<Vec<i64>> {
        if k == 0 {
            return vec![vec![]];
        }
        let mut out = Vec::new();
        for (i, x) in items.iter().enumerate() {
            let rest: Vec<i64> = items.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, v)| *v).collect();
            for mut p in permutations(&rest, k - 1) {
                p.insert(0, *x);
                out.push(p);
            }
        }
        out
    }

    #[test]
    fn a_restart_begins_in_champion_mode() {
        let m = ModeState::start(42.0);
        assert_eq!(m.status, Status::Champion);
        assert!(!m.active_at(42.0));
        assert!(m.last_transition.unwrap().reason.contains("restart"));
    }
}
