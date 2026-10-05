//! Shared runtime state: what every bot is doing right now, for the API.

use crate::config::Config;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use sv10_core::model::ModelStore;
use sv10_core::policy::{Candidate, Params};
use sv10_store::store::Store;
use sv10_venue::tracker::SeatView;
use tokio::sync::broadcast;

/// Big blind assumed before any table or stored hand has told us the real one.
pub const DEFAULT_BIG_BLIND: i64 = 20;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DecisionView {
    pub ts: String,
    pub hand_id: String,
    pub street: String,
    pub action: String,
    pub amount: Option<i64>,
    /// Hero's equity against the estimated ranges; `None` when the draw could not measure one and
    /// the decision was refused (#424). Every dashboard reader treats a missing value as
    /// unavailable rather than as a zero.
    pub equity: Option<f64>,
    pub pot_odds: f64,
    pub pot: i64,
    pub to_call: i64,
    pub latency_ms: f64,
    pub reason: String,
    pub hole: Vec<String>,
    pub board: Vec<String>,
    pub candidates: Vec<Candidate>,
    pub opponents: usize,
    /// Champion (promotion lineage entry) whose parameters made this decision.
    pub version: String,
    /// Fold-calibration logit shifts in force (0156); recorded so refits can undo them. Not part
    /// of the dashboard contract.
    #[serde(skip)]
    pub fold_shift: [f64; 3],
    /// Preflop fold calibration shift in effect for this decision (recorded so refits un-shift it).
    pub preflop_fold_shift: f64,
    /// Per-opponent fold offsets in force for the live opponents (0214), recorded for the same
    /// reason: a refit that could not undo them measured each opponent against its own correction.
    #[serde(skip)]
    pub fold_offsets: Vec<(String, f32)>,
    /// Experiment provenance of the hand this decision belongs to (0291); absent in ordinary play.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment: Option<serde_json::Value>,
}

impl DecisionView {
    /// Dashboard projection of one answered decision: the legalized action sent, the decision's
    /// numbers and explanation, the situation's cards and pot, and the champion version. Built
    /// here, next to the type, so the four parallel records of a decision (`Decision`,
    /// `ReplayRecord`, this view, the audit row) gain their dashboard projection in exactly one
    /// place instead of inline at the call site.
    pub fn for_decision(
        hand_id: &str,
        sit: &sv10_core::situation::Situation,
        d: &sv10_core::policy::Decision,
        action: String,
        amount: Option<i64>,
        latency_ms: f64,
        version: String,
    ) -> DecisionView {
        DecisionView {
            ts: chrono::Utc::now().to_rfc3339(),
            hand_id: hand_id.to_string(),
            street: sit.street.name().to_string(),
            action,
            amount,
            equity: d.equity,
            pot_odds: d.pot_odds,
            pot: sit.pot,
            to_call: sit.call_amount,
            latency_ms,
            reason: d.reason.clone(),
            hole: vec![sit.hole[0].to_string(), sit.hole[1].to_string()],
            board: sit.board.iter().map(|c| c.to_string()).collect(),
            candidates: d.candidates.clone(),
            opponents: sit.live_opponents().count(),
            version,
            fold_shift: [0.0; 3],
            preflop_fold_shift: 0.0,
            fold_offsets: Vec::new(),
            experiment: None,
        }
    }
}

/// Lifetime `state_hash` tallies for one bot (0301), keyed by bot name in the store.
///
/// The in-memory counters reset at every hot swap, so after a release the Runtime health panel read
/// e.g. `367 / 0` for hours: a healthy fleet and one whose serializer just started diverging looked
/// the same. These totals are the same counts kept across restarts, flushed with the model
/// checkpoint, so the panel can show both the session and the lifetime rate.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StateHashTotals {
    /// Snapshots whose recomputed hash matched the server's.
    pub ok: u64,
    /// Snapshots that did not.
    pub bad: u64,
    /// The newest mismatch, what the panel links into `review state-hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_mismatch: Option<StateHashMismatch>,
}

/// The newest `state_hash` mismatch a bot saw (0301). `review state-hash` prints the same incidents
/// from the `hash_incidents` table (0265); this is the one the panel shows without a query.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateHashMismatch {
    /// Unix seconds of the frame; the incident row carries the same moment as RFC 3339.
    pub at: f64,
    /// Table the mismatching snapshot described, as the incident row names it.
    pub table: String,
    /// `STALE` (a frame from before the last one we applied) or `DIVERGED`.
    pub verdict: String,
    /// The venue's field census for the frame, as logged.
    pub summary: String,
}

/// What one bot is doing, as the dashboard reads it.
///
/// The `#[serde(skip)]` fields below are runtime state and must not travel with this value: `Instant`
/// timers that mean nothing in another process, per-connection turn tokens, the situation behind
/// `last_decision` (behind `s.models`, not here), and the hand in progress. The two pieces that must
/// outlive a process persist on their own keys — `open_hand` through [`OPEN_HANDS_KEY`] and
/// `state_hash_lifetime` with the model checkpoint — so a reload takes them from the store, never
/// from this struct's serialized form (0325).
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BotLive {
    pub slot: usize,
    pub name: String,
    /// connecting | lobby | playing | paused | stopped | offline | error
    pub mode: String,
    /// run | pause | stop (operator intent)
    pub desired: String,
    pub connected: bool,
    pub table_id: Option<String>,
    pub seat: Option<usize>,
    pub hand_id: Option<String>,
    pub street: Option<String>,
    pub dealer: usize,
    pub stack: i64,
    pub pot: i64,
    pub big_blind: i64,
    pub hole: Vec<String>,
    pub board: Vec<String>,
    pub seats: Vec<SeatView>,
    pub actor_seat: Option<usize>,
    pub last_decision: Option<DecisionView>,
    pub session_hands: u64,
    pub session_net: i64,
    pub decisions: u64,
    /// Decisions that hit the 8 s cap and took the safe action (0252). A run of these is a stall,
    /// not a normal turn, so the count is on the dashboard rather than only in a log line.
    pub decision_timeouts: u64,
    pub latencies_ms: VecDeque<f64>,
    pub season: Option<serde_json::Value>,
    pub last_error: Option<String>,
    pub connected_since: Option<String>,
    pub rejections: u64,
    /// Table snapshots whose `state_hash` matched / did not match our canonical serialization.
    pub state_hash_ok: u64,
    pub state_hash_bad: u64,
    /// The same two tallies since this bot first ran (0301), loaded from the store at startup and
    /// flushed with the model checkpoint.
    #[serde(default)]
    pub state_hash_lifetime: StateHashTotals,
    #[serde(skip)]
    pub last_hash_resync: Option<std::time::Instant>,
    /// Resyncs asked for after an action rejection in the current hand (hand id, count): at most
    /// [`crate::client::MAX_REJECT_RESYNCS`] per hand, so a persistent rejection cannot loop.
    #[serde(skip)]
    pub reject_resyncs: (Option<String>, u32),
    /// Last table/hand whose missing decision state requested recovery: at most one snapshot
    /// per hand, so an incomplete resync cannot loop while the safe action still meets the deadline.
    #[serde(skip)]
    pub missing_state_resync: Option<(String, String)>,
    /// Unix seconds when our current turn began (None when not our turn).
    pub turn_started: Option<f64>,
    /// Hand and token of the last action accepted by the connection writer.
    #[serde(skip)]
    pub last_acted_turn_token: Option<String>,
    #[serde(skip)]
    pub last_acted_hand_id: Option<String>,
    /// Sequence of that turn when the authority supplied one; older turns in the same hand are stale.
    #[serde(skip)]
    pub last_acted_turn_seq: Option<i64>,
    /// Latest action label per seat this hand, for seat bubbles.
    pub last_actions: std::collections::HashMap<usize, String>,
    #[serde(skip)]
    pub last_table_switch: Option<std::time::Instant>,
    /// Due time of a server-scheduled auto-rebuy (`auto_rebuy_scheduled`), cleared on `rebuy_confirmed`.
    #[serde(skip)]
    pub auto_rebuy_at: Option<std::time::Instant>,
    /// When the bot last checked (and possibly left) to rebuy deeper.
    #[serde(skip)]
    pub last_topup: Option<std::time::Instant>,
    /// The situation behind `last_decision`, for the range explorer.
    #[serde(skip)]
    pub last_situation: Option<std::sync::Arc<sv10_core::situation::Situation>>,
    /// The policy fixed for the current hand (experiment arm or champion, 0291).
    #[serde(skip)]
    pub hand_policy: Option<crate::experiment::HandPolicy>,
    /// This bot's own lineage knobs (`params.slot.<bot>`, ADR 0002), `None` while the shared champion plays.
    #[serde(skip)]
    pub slot_params: Option<Params>,
    /// Version name of that lineage's newest champion (recorded with each decision), when it has one.
    #[serde(skip)]
    pub slot_version: Option<String>,
    /// The hand in progress, saved when the process exits so the next one can finish it (0315).
    #[serde(skip)]
    pub open_hand: Option<sv10_venue::tracker::OpenHand>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LogLine {
    pub ts: String,
    pub bot: String,
    pub level: String,
    pub message: String,
}

pub struct Shared {
    pub config: Config,
    pub bots: Vec<RwLock<BotLive>>,
    pub models: RwLock<ModelStore>,
    pub params: RwLock<Params>,
    pub nn: RwLock<Option<std::sync::Arc<sv10_core::nn::Mlp>>>,
    pub reputation: RwLock<crate::reputation::ReputationBook>,
    /// Measured results against each opponent (refreshed every 15 minutes).
    pub head_to_head: RwLock<std::collections::HashMap<String, crate::headtohead::HeadToHead>>,
    /// Avatar URL per player name, as `table_state` last showed it at one of our tables (0180).
    pub avatars: RwLock<std::collections::HashMap<String, String>>,
    pub store: Store,
    pub log: Mutex<VecDeque<LogLine>>,
    pub events: broadcast::Sender<String>,
    pub started_at: String,
    /// `GET /season/current` reading (wind-down freezes voluntary table moves).
    pub season_clock: RwLock<crate::season::SeasonClock>,
    /// Identity and start of the season now being played, for panels that present this season.
    pub current_season: RwLock<Option<crate::season::CurrentSeason>>,
    /// Lineage name of the parameters in `params` (updated together with them on promotion).
    pub champion_version: RwLock<String>,
    /// A saved bot setup waits for the fleet to restart (checked by the release watch).
    pub restart_requested: std::sync::atomic::AtomicBool,
    /// The live database failed its structural check (#744): the release watch exits 70 for the restore at the first
    /// moment no bot is mid-turn, instead of the backup task exiting in the middle of a hand.
    pub restore_requested: std::sync::atomic::AtomicBool,
    /// Finished hands whose insert failed (e.g. `database is locked` while another process held a
    /// write lock: 3 hands lost on 2026-09-19/20). Already observed by the models; retried by
    /// `tasks::retry_unstored_hands` (0152), each with its failed retry count.
    pub unstored_hands: Mutex<Vec<(sv10_store::store::HandRow, u32)>>,
    /// Current bot name -> every name its API key played under (current first; `identity`).
    pub aliases: RwLock<std::collections::HashMap<String, Vec<String>>>,
    /// Experiment mode as this process last read it (0291).
    pub experiment: RwLock<crate::experiment::Live>,
    /// Hands the previous process left in progress, by bot, waiting for their replayed result (0315).
    pub resumable: Mutex<OpenHands>,
    /// The TV's last projection per slot, shared by every spectator connection (#334): the first
    /// connection whose slot is due projects, the rest read the entry while it is fresher than
    /// [`crate::api::tv`] 's tick. Bounded by the seat count; entries go stale, never wrong, and a
    /// slot re-projects past the tick.
    pub tv_cache: Mutex<std::collections::HashMap<usize, (std::time::Instant, String)>>,
    /// Bounds the decision searches running at once (#780): tokio's 512-thread `spawn_blocking` cap is no
    /// backpressure for CPU-bound work, and five unbounded searches on a few cores stretched ~130 ms to
    /// ~0.6 s. Over-limit searches queue (inside the decision's 8 s cap), never run degraded.
    pub decision_gate: std::sync::Arc<tokio::sync::Semaphore>,
}

/// How many decision searches may run at once on a machine with `logical_cores`: half of them, at
/// least two. A fleet of five bots on twelve cores never queues; a small box does instead of thrashing.
pub fn decision_permits(logical_cores: usize) -> usize {
    (logical_cores / 2).max(2)
}

/// `configured` with every alias each of those names has played under, each once, order preserved.
pub fn with_aliases(configured: &[String], aliases: &std::collections::HashMap<String, Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in configured {
        for candidate in aliases.get(name).cloned().unwrap_or_else(|| vec![name.clone()]) {
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

impl Shared {
    pub fn log(&self, bot: &str, level: &str, message: impl Into<String>) {
        let message = message.into();
        match level {
            "error" => tracing::error!(bot, "{message}"),
            "warn" => tracing::warn!(bot, "{message}"),
            _ => tracing::info!(bot, "{message}"),
        }
        let line = LogLine { ts: chrono::Utc::now().to_rfc3339(), bot: bot.to_string(), level: level.to_string(), message };
        if level != "debug" {
            self.store.log_event(&line.bot, &line.level, &line.message);
        }
        let mut log = self.log.lock();
        log.push_back(line.clone());
        while log.len() > 500 {
            log.pop_front();
        }
        drop(log);
        let _ = self.events.send(serde_json::json!({"type": "log", "line": line}).to_string());
    }

    /// The one big-blind source for bb-denominated display and analysis: a seated bot's tracked
    /// blind, else the last stored hand's, else [`DEFAULT_BIG_BLIND`].
    pub fn big_blind(&self) -> f64 {
        self.bots
            .iter()
            .map(|b| b.read().big_blind)
            .find(|bb| *bb > 0)
            .or_else(|| self.store.latest_big_blind().ok().flatten())
            .unwrap_or(DEFAULT_BIG_BLIND) as f64
    }

    /// Save every bot's hand in progress for the process that starts next (0315). A failed write only
    /// costs those hands' rows, so it is logged, not fatal. Merged into what is already saved: a split
    /// fleet saves once per process and each process plays its own bot (the head's copy of a worker's
    /// state carries no hand, `BotLive::open_hand` is not serialised), so one process's save must not
    /// drop another's (0128).
    pub fn save_open_hands(&self) {
        let mine: OpenHands =
            self.bots.iter().filter_map(|b| b.read().open_hand.clone().map(|hand| (b.read().name.clone(), hand))).collect();
        let mut open = OpenHands::default();
        // Read and written in one transaction: the processes of a split fleet exit together on a hot
        // swap, and two of them reading the same saved map each wrote it back with only its own hand.
        let saved = self.store.update_kv(OPEN_HANDS_KEY, |old| {
            open = old.and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
            open.extend(mine);
            Ok(serde_json::to_string(&open)?)
        });
        match saved {
            Err(e) => tracing::warn!("hands in progress not saved for the next process: {e}"),
            // One line per exit, so a lost row can be traced from its save to its resync (0315).
            Ok(()) if !open.is_empty() => {
                let hands: Vec<String> = open.iter().map(|(bot, h)| format!("{bot} {}", h.hand_id)).collect();
                tracing::info!("saved {} hands in progress for the next process: {}", open.len(), hands.join(", "));
            }
            Ok(()) => {}
        }
    }

    /// The current name of a bot that stored rows under `name`, which may be one of its earlier
    /// names (SvanBotV7 is SvanBotV10). Anything that groups stored rows per bot must go through
    /// this, or a renamed bot shows up twice.
    pub fn current_name(&self, name: &str) -> String {
        self.aliases
            .read()
            .iter()
            .find(|(_, names)| names.iter().any(|n| n == name))
            .map_or_else(|| name.to_string(), |(current, _)| current.clone())
    }

    /// Every name a bot's hands are stored under (current name first), so a renamed bot's season
    /// stays one record (`identity`).
    pub fn names_of(&self, name: &str) -> Vec<String> {
        self.aliases.read().get(name).cloned().unwrap_or_else(|| vec![name.to_string()])
    }

    /// Every name the configured fleet's hands are stored under, aliases included.
    ///
    /// A panel that reads the store by name needs this rather than `config.bots`: after a rename the
    /// hands are stored under the old name too, so a list built by hand shows a fraction of the store
    /// and a number that disagrees with the terminal (#609, #610).
    pub fn fleet_names(&self) -> Vec<String> {
        with_aliases(&self.config.bots.iter().map(|b| b.name.clone()).collect::<Vec<_>>(), &self.aliases.read())
    }

    /// Identity and start of the season now being played, when it is known. Panels that present
    /// this season's performance scope their stored hands by it; when it is `None` nothing is
    /// scoped and the panel says so, rather than presenting all-time figures as this season's.
    pub fn season(&self) -> Option<crate::season::CurrentSeason> {
        self.current_season.read().clone()
    }

    /// Push a realtime event to dashboard subscribers immediately.
    pub fn emit(&self, kind: &str, mut payload: serde_json::Value) {
        payload["type"] = serde_json::json!(kind);
        payload["ts"] = serde_json::json!(chrono::Utc::now().timestamp_millis() as f64 / 1000.0);
        let _ = self.events.send(payload.to_string());
    }

    pub fn update<F: FnOnce(&mut BotLive)>(&self, slot: usize, f: F) {
        let mut b = self.bots[slot].write();
        f(&mut b);
        let _ = self.events.send(serde_json::json!({"type": "bot", "slot": slot}).to_string());
    }
}

/// KV key of the hands in progress a process left at exit (0315).
pub const OPEN_HANDS_KEY: &str = "fleet.open-hands.v1";

/// Hands in progress saved for a later process, per bot name (0315).
pub type OpenHands = std::collections::HashMap<String, sv10_venue::tracker::OpenHand>;

/// kv key holding every bot's lifetime state-hash tallies (0301), keyed by bot name.
pub const STATE_HASH_TOTALS_KEY: &str = "state_hash.totals.v1";

/// Count one verified snapshot into a bot's counters: the since-restart tallies the panel has
/// always shown, and the lifetime totals that survive a restart (0301). `mismatch` is the
/// diagnosis to remember when the snapshot did not match.
pub fn count_state_hash(b: &mut BotLive, ok: bool, mismatch: Option<StateHashMismatch>) {
    if ok {
        b.state_hash_ok += 1;
        b.state_hash_lifetime.ok += 1;
        return;
    }
    b.state_hash_bad += 1;
    b.state_hash_lifetime.bad += 1;
    if let Some(mismatch) = mismatch {
        b.state_hash_lifetime.last_mismatch = Some(mismatch);
    }
}

/// Take the lifetime tallies back into `bots` at startup (0301). The since-restart counters always
/// start at zero; a missing or unreadable value leaves the lifetime tallies there too.
pub fn load_state_hash_totals(store: &Store, bots: &[RwLock<BotLive>]) {
    let stored: std::collections::HashMap<String, StateHashTotals> =
        store.get_kv(STATE_HASH_TOTALS_KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    for b in bots {
        let mut b = b.write();
        b.state_hash_lifetime = stored.get(&b.name).cloned().unwrap_or_default();
    }
}

/// Persist the lifetime tallies; called where the models are checkpointed, so a hot swap carries
/// the counters into the next process (0301). `put_kv` skips an unchanged value: an idle fleet
/// writes nothing.
pub fn save_state_hash_totals(store: &Store, bots: &[RwLock<BotLive>]) {
    let totals: std::collections::BTreeMap<String, StateHashTotals> = bots
        .iter()
        .map(|b| {
            let b = b.read();
            (b.name.clone(), b.state_hash_lifetime.clone())
        })
        .collect();
    match serde_json::to_string(&totals) {
        Ok(json) => {
            if let Err(e) = store.put_kv(STATE_HASH_TOTALS_KEY, &json) {
                tracing::warn!("saving state-hash totals failed: {e}");
            }
        }
        Err(e) => tracing::warn!("serializing state-hash totals failed: {e}"),
    }
}

/// Take the hands in progress the previous process saved, for the bots this process plays; what
/// belongs to another process is left for it, since a split fleet starts one process per bot and the
/// first to start would otherwise swallow the rest (0128). Taking clears them: a hand is settled
/// once, from one resync replay.
pub fn take_open_hands(store: &Store, names: &[String]) -> OpenHands {
    let mut mine = OpenHands::default();
    // One transaction, as in the save: a process starting beside this one must not put back what
    // this one took, or take what this one is about to leave.
    let left = store.update_kv(OPEN_HANDS_KEY, |old| {
        let saved: OpenHands = old.and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
        let (taken, theirs): (OpenHands, OpenHands) = saved.into_iter().partition(|(name, _)| names.iter().any(|n| n == name));
        mine = taken;
        Ok(serde_json::to_string(&theirs)?)
    });
    if let Err(e) = left {
        tracing::warn!("hands in progress of other processes not left for them: {e}");
    }
    mine
}

// The split-fleet seam (heartbeat/want keys, the lineage head) lives in its own file (0320); it is
// production code and other modules reach it as `crate::live::heartbeat_key` and friends, so it is
// re-exported here at the path they already use.
mod fleet;
pub use fleet::*;

#[cfg(test)]
mod tests;
