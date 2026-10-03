pub mod analysis;
pub mod api;
pub mod badges;
pub mod client;
pub mod compaction;
pub mod config;
pub mod derived;
pub mod experiment;
pub mod findings;
pub mod foldcal;
pub mod guide;
pub mod headtohead;
pub mod history;
pub mod hostcheck;
pub mod identity;
pub mod installs;
pub mod jobs;
pub mod knobs;
pub mod learner;
pub mod live;
pub mod livefits;
pub mod luck;
pub mod monitor;
pub mod multiway;
pub mod neural;
pub mod nnresidual;
pub mod pacing;
pub mod pacing_study;
pub mod playerfits;
pub mod playerfold;
pub mod playersize;
pub mod profile;
pub mod promotion;
pub mod raisewar;
pub mod rangefit;
pub mod release;
pub mod replay;
pub mod reputation;
pub mod review_calls;
pub mod review_drift;
pub mod review_fleet;
pub mod review_recent;
pub mod review_rerun;
pub mod review_rival;
pub mod review_season;
pub mod review_wiring;
pub mod search_ledger;
pub mod season;
pub mod seasons;
pub mod setup;
pub mod stories;
pub mod style;
pub mod tasks;
#[cfg(test)]
pub(crate) mod testlog;
pub mod unrecorded;
pub mod watchdog;

// Extracted crates live at their own paths (`sv10_store::<module>`, `sv10_rt`, `sv10_venue::<module>`);
// this crate uses them directly, with no re-export fan.

/// Project version (workspace `Cargo.toml` is the single source of truth).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Git commit baked in at build time (`SVANBOT_COMMIT`, exported by `scripts/release.sh`).
pub const BUILD_COMMIT: &str = match option_env!("SVANBOT_COMMIT") {
    Some(commit) => commit,
    None => "dev",
};
/// HTTP and WebSocket user agent for every outbound request.
pub const USER_AGENT: &str = concat!("svanbot10/", env!("CARGO_PKG_VERSION"));

pub const MODELS_KEY: &str = "models.v1";
/// Recency half-life of live opponent decision tallies, in a player's own hands (0168). Replaying
/// 78,183 stored hands, 751,322 later opponent decisions were predicted better than from all-time
/// tallies at every half-life tried: 5,000 +0.33, 2,000 +0.69, 1,000 +1.01 ± 0.08, 500 +1.06 ± 0.12
/// and 250 +0.50 mnats per decision. 1,000 sits on the plateau with the tighter interval.
pub const OPPONENT_HALF_LIFE_HANDS: f32 = 1_000.0;
pub const PARAMS_KEY: &str = "params.v1";
pub const LEARNER_STATUS_KEY: &str = "learner.status";
pub const LEARNER_EXPERIMENTS_KEY: &str = "learner.experiments";
pub const NN_KEY: &str = "nn.response.v1";
/// A trained response model awaiting the paired poker gate; the fleet never reads it (0142).
pub const NN_CANDIDATE_KEY: &str = "nn.response.candidate.v1";
pub const RANGE_PARAMS_KEY: &str = "range_params.v1";
/// Self-calibration table (per-spot predicted-vs-realized corrections).
pub const CALIBRATION_KEY: &str = "calibration.v1";
/// Machine profile recorded at startup (CPU, memory, disks).
pub const HARDWARE_PROFILE_KEY: &str = "hardware.profile";
/// Operator command for the learner (dashboard `control`, pacing gate).
pub const LEARNER_COMMAND_KEY: &str = "learner.command";
/// Learner cycle counter.
pub const LEARNER_CYCLE_KEY: &str = "learner.cycle";
/// Champion lineage (promoted parameter versions, newest last).
pub const LEARNER_LINEAGE_KEY: &str = "learner.lineage";
/// Post-promotion replay drift summary written by the analyst.
pub const ANALYST_DRIFT_KEY: &str = "analyst.drift";
/// Analyst heartbeat (queue depth, deep-search pace), read by the dashboard.
pub const ANALYST_STATUS_KEY: &str = "analyst.status";
/// Identity and start of the season now being played (`season::CurrentSeason`), refreshed by the
/// season poller. Persisted so a restart scopes its panels correctly before the first poll lands.
pub const SEASON_KEY: &str = "season.current.v1";

/// Range-reconstruction constants fitted to showdowns by `calibrate`.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct StoredRangeParams {
    /// Whether the fit beat the defaults on held-out showdowns (only then is it installed).
    pub active: bool,
    pub params: sv10_core::oprange::RangeParams,
    pub report: serde_json::Value,
    pub fitted_at: f64,
}

/// An avatar the server sent, as a URL the dashboard can load (0180): absolute URLs pass through,
/// site paths (`/api/public-avatar/...`) resolve against the REST base's origin.
pub fn avatar_url(rest_base: &str, raw: &str) -> Option<String> {
    if raw.starts_with("https://") || raw.starts_with("http://") {
        return Some(raw.to_string());
    }
    if !raw.starts_with('/') || raw.starts_with("//") {
        return None;
    }
    let rest = rest_base.split_once("://")?;
    let host = rest.1.split('/').next().filter(|h| !h.is_empty())?;
    Some(format!("{}://{host}{raw}", rest.0))
}

#[cfg(test)]
mod avatar_tests {
    #[test]
    fn site_paths_resolve_against_the_rest_origin() {
        let base = "https://api.openpoker.ai/api";
        assert_eq!(
            super::avatar_url(base, "/api/public-avatar/a/b.webp").as_deref(),
            Some("https://api.openpoker.ai/api/public-avatar/a/b.webp")
        );
        assert_eq!(super::avatar_url(base, "https://cdn.x/y.png").as_deref(), Some("https://cdn.x/y.png"));
        assert_eq!(super::avatar_url(base, "//evil/x.png"), None);
        assert_eq!(super::avatar_url(base, "avatars/x.png"), None);
        assert_eq!(super::avatar_url("not a url", "/a.png"), None);
    }
}

/// The stored showdown-fitted range parameters, if they beat the defaults on held-out hands.
pub fn fitted_range_params(stored: Option<&str>) -> Option<sv10_core::oprange::RangeParams> {
    stored.and_then(|j| serde_json::from_str::<StoredRangeParams>(j).ok()).filter(|s| s.active).map(|s| s.params)
}

/// A trained response model as stored by the learner.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct StoredNet {
    pub net: sv10_core::nn::Mlp,
    pub active: bool,
    /// Feature/profile chronology contract used to build train and validation samples. Missing on
    /// legacy artifacts, which therefore cannot activate after chronology became mandatory.
    #[serde(default)]
    pub training_contract: String,
    /// Set only after a fresh-deal paired poker gate approves this exact artifact.
    #[serde(default)]
    pub paired_poker_approved: bool,
    pub val_loss: f64,
    pub baseline_loss: f64,
    pub train_samples: usize,
    pub val_samples: usize,
    pub trained_at: f64,
}

/// Log timestamps in the machine's local time with its UTC offset (e.g. `2026-09-17 02:31:30.123+02:00`),
/// so logs read like the operator's clock; stored data stays UTC.
pub struct LocalTime;

impl tracing_subscriber::fmt::time::FormatTime for LocalTime {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f%:z"))
    }
}

/// Plain logging for tools: local-time stamps, no target.
pub fn init_tool_logging() {
    tracing_subscriber::fmt().with_timer(LocalTime).with_target(false).init();
}

/// A stored RFC 3339 timestamp (UTC) shown in local time, `YYYY-MM-DD HH:MM:SS`; unparseable input
/// is returned unchanged.
pub fn local_time(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|_| rfc3339.to_string())
}
