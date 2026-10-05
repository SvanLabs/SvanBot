//! Auto-renew Pro when it lapses (#776), and the alarm when it lapses with auto-renew off (#775).
//!
//! Pro is per season and the venue refuses extra bots on a Free account (`portfolio_pro_required`),
//! so a lapse at a season turn sits four of five bots out until someone buys it. The owner key
//! (`config.bots[0]`, the `SVANBOT_API_KEY` one) is the only key whose `/season/me` reads the
//! account's tier; the child keys read `false` under Pro.
//!
//! **Off by default.** `SVANBOT_AUTO_RENEW_PRO=1` turns the purchaser on; off, nothing here calls a
//! purchase endpoint and nothing makes an extra venue request, it only logs the alarm. On, it buys
//! only when the owner key's tier is *measured* Free — a fresh `/season/me` read right before the
//! purchase, on a box with more than one key, outside dry run — with the widest bundle up to
//! `SVANBOT_AUTO_RENEW_SEASONS` (1, 3 or 6; default 3) that the credit balance covers: a `402`
//! charges nothing, so the ladder steps down on it. Each rung gets its own `request_id`, written to
//! the store before the request is sent, and a request of unknown fate is retried with that same id
//! (a fresh id is a new charge). After a purchase nothing more is bought for six hours.

use crate::live::Shared;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// Kv key of the persisted [`State`].
pub const STATE_KEY: &str = "pro.renewal.v1";
/// Kv key a worker writes (unix seconds) when the venue refuses a join with `portfolio_pro_required`.
pub const LAPSE_KEY: &str = "pro.lapse.seen";
/// A purchase request of unknown fate is retried with its own id for this long.
const RETRY_FOR_SECS: f64 = 24.0 * 3600.0;
/// Nothing more is bought this long after a purchase: the tier read may lag, a second charge may not happen.
const QUIET_AFTER_PURCHASE_SECS: f64 = 6.0 * 3600.0;
/// After a failed renewal (no credit on any rung, or a hard error) the next try waits this long.
const BACKOFF_SECS: f64 = 3600.0;
/// A purchase this recent with the tier still reading Free means the owner read is wrong: no second charge.
const RECENT_RECEIPT_SECS: f64 = 7.0 * 24.0 * 3600.0;
/// A request of unknown fate is retried no sooner than this after the last attempt.
const RETRY_GAP_SECS: f64 = 300.0;
/// Fresh owner reads are at least this far apart.
const READ_GAP_SECS: f64 = 60.0;
/// How often the loop looks.
const TICK: Duration = Duration::from_secs(20);

/// What the owner key's `/season/me` says about the account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Pro,
    Free,
    /// Not read, or no `pro_tier` field: nothing is decided on it.
    Unknown,
}

impl Tier {
    pub fn of(season: Option<&Value>) -> Tier {
        match season.and_then(|v| v["pro_tier"].as_bool()) {
            Some(true) => Tier::Pro,
            Some(false) => Tier::Free,
            None => Tier::Unknown,
        }
    }
}

/// Settings read from the environment once at start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    /// Widest bundle the ladder may try: 1, 3 or 6 seasons.
    pub max_seasons: u8,
}

impl Settings {
    pub fn from_env() -> Settings {
        Settings::parse(|k| std::env::var(k).ok())
    }

    /// The settings from any source of variables (the environment, or a test's table).
    pub fn parse(get: impl Fn(&str) -> Option<String>) -> Settings {
        let var = |k: &str| get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        Settings {
            enabled: matches!(var("SVANBOT_AUTO_RENEW_PRO").as_deref(), Some("1" | "true")),
            max_seasons: match var("SVANBOT_AUTO_RENEW_SEASONS").and_then(|v| v.parse::<u8>().ok()) {
                Some(1) => 1,
                Some(6) => 6,
                _ => 3,
            },
        }
    }
}

/// A purchase request that may have reached the venue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub request_id: String,
    pub seasons: u8,
    pub at: f64,
}

/// What a successful purchase returned.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub seasons_purchased: i64,
    pub seasons_remaining: i64,
    pub amount_charged_cents: i64,
    pub request_id: String,
    pub at: f64,
}

/// The persisted renewal state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub pending: Option<Pending>,
    pub receipt: Option<Receipt>,
    pub last_attempt_at: Option<f64>,
    pub last_error: Option<String>,
    /// No new purchase before this (unix seconds).
    pub quiet_until: f64,
}

/// What to do about the tier right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Auto-renew is off: no purchase call, ever.
    Off,
    /// Nothing to buy, and why.
    Nothing(&'static str),
    /// Retry the request of unknown fate with its own id.
    Retry { seasons: u8, request_id: String },
    /// Start a purchase down this ladder, widest first.
    Buy { ladder: Vec<u8> },
}

/// The decision, pure. `tier` is the fresh read taken right before this call.
pub fn plan(settings: Settings, state: &State, tier: Tier, now: f64) -> Plan {
    if !settings.enabled {
        return Plan::Off;
    }
    if tier != Tier::Free {
        return Plan::Nothing(if tier == Tier::Pro { "Pro is active" } else { "tier not read" });
    }
    if let Some(p) = state.pending.as_ref().filter(|p| now - p.at < RETRY_FOR_SECS) {
        if state.last_attempt_at.is_some_and(|t| now - t < RETRY_GAP_SECS) {
            return Plan::Nothing("retrying shortly");
        }
        return Plan::Retry { seasons: p.seasons, request_id: p.request_id.clone() };
    }
    if state.receipt.as_ref().is_some_and(|r| now - r.at < RECENT_RECEIPT_SECS) {
        return Plan::Nothing("a renewal was bought this week and the tier still reads Free: check the account");
    }
    if now < state.quiet_until {
        return Plan::Nothing("a renewal was tried recently");
    }
    Plan::Buy { ladder: [6u8, 3, 1].into_iter().filter(|s| *s <= settings.max_seasons).collect() }
}

/// Where the purchase endpoints are and whose key signs them.
pub struct Rest<'a> {
    pub http: &'a reqwest::Client,
    pub base: &'a str,
    pub key: &'a str,
}

/// The owner key's tier from a fresh `GET /season/me`.
pub async fn read_tier(rest: &Rest<'_>) -> Tier {
    let Ok(r) = rest.http.get(format!("{}/season/me", rest.base)).bearer_auth(rest.key).send().await else { return Tier::Unknown };
    if !r.status().is_success() {
        return Tier::Unknown;
    }
    Tier::of(r.json::<Value>().await.ok().as_ref())
}

enum Bought {
    Done(Receipt),
    /// `402`: the credit balance does not cover this bundle; nothing was charged.
    NoCredit,
    /// Anything else: the request may or may not have been applied.
    Unknown(String),
}

async fn post_bundle(rest: &Rest<'_>, seasons: u8, request_id: &str, now: f64) -> Bought {
    let body = json!({"seasons": seasons, "request_id": request_id});
    let sent = rest.http.post(format!("{}/season/pro-bundle", rest.base)).bearer_auth(rest.key).json(&body).send().await;
    let r = match sent {
        Ok(r) => r,
        Err(e) => return Bought::Unknown(format!("request failed: {e}")),
    };
    let status = r.status();
    if status.as_u16() == 402 {
        return Bought::NoCredit;
    }
    let text = r.text().await.unwrap_or_default();
    if !status.is_success() {
        return Bought::Unknown(format!("HTTP {status}: {}", text.chars().take(200).collect::<String>()));
    }
    // A 2xx means it was charged, whatever the body looks like: never retry after this.
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Bought::Done(Receipt {
        seasons_purchased: v["seasons_purchased"].as_i64().unwrap_or(seasons as i64),
        seasons_remaining: v["seasons_remaining"].as_i64().unwrap_or(-1),
        amount_charged_cents: v["amount_charged_cents"].as_i64().unwrap_or(-1),
        request_id: request_id.to_string(),
        at: now,
    })
}

/// Carry out `plan`. `save` stores the state; it runs before every request, so a crash after the
/// send still finds the request id on disk. Returns the receipt when something was bought.
pub async fn execute(rest: &Rest<'_>, state: &mut State, plan: Plan, now: f64, save: &mut (dyn FnMut(&State) + Send)) -> Option<Receipt> {
    if matches!(plan, Plan::Off | Plan::Nothing(_)) {
        return None;
    }
    state.last_attempt_at = Some(now);
    let ladder = match plan {
        Plan::Retry { seasons, request_id } => {
            return settle(rest, state, seasons, request_id, now, save).await.0;
        }
        Plan::Buy { ladder } => ladder,
        Plan::Off | Plan::Nothing(_) => return None,
    };
    for seasons in ladder {
        let request_id = sv10_rt::uuid_v4();
        state.pending = Some(Pending { request_id: request_id.clone(), seasons, at: now });
        let (receipt, no_credit) = settle(rest, state, seasons, request_id, now, save).await;
        if receipt.is_some() || !no_credit {
            return receipt;
        }
    }
    state.last_error = Some("the credit balance does not cover even one season ($5): add credits at openpoker.ai".into());
    state.quiet_until = now + BACKOFF_SECS;
    save(state);
    None
}

/// One request with the id already in `state.pending`. `(receipt, no_credit)`.
async fn settle(
    rest: &Rest<'_>,
    state: &mut State,
    seasons: u8,
    request_id: String,
    now: f64,
    save: &mut (dyn FnMut(&State) + Send),
) -> (Option<Receipt>, bool) {
    state.pending = Some(Pending { request_id: request_id.clone(), seasons, at: state.pending.as_ref().map_or(now, |p| p.at) });
    save(state);
    match post_bundle(rest, seasons, &request_id, now).await {
        Bought::Done(receipt) => {
            state.pending = None;
            state.receipt = Some(receipt.clone());
            state.last_error = None;
            state.quiet_until = now + QUIET_AFTER_PURCHASE_SECS;
            save(state);
            (Some(receipt), false)
        }
        Bought::NoCredit => {
            state.pending = None;
            save(state);
            (None, true)
        }
        Bought::Unknown(e) => {
            // Keep the id: the next pass retries this same purchase.
            state.last_error = Some(e);
            state.quiet_until = now + BACKOFF_SECS.min(300.0);
            save(state);
            (None, false)
        }
    }
}

fn now_secs() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}

fn load(shared: &Shared) -> State {
    shared.store.get_kv(STATE_KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

fn store(shared: &Shared, state: &State) {
    match serde_json::to_string(state) {
        Ok(j) => {
            if let Err(e) = shared.store.put_kv(STATE_KEY, &j) {
                shared.log("fleet", "error", format!("Pro renewal state not stored: {e}"));
            }
        }
        Err(e) => shared.log("fleet", "error", format!("Pro renewal state not serializable: {e}")),
    }
}

/// A worker saw the venue refuse a join for lack of Pro: tell the head, at most every 20 s.
pub fn note_lapse(shared: &Shared) {
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = now_secs() as u64;
    use std::sync::atomic::Ordering::Relaxed;
    // The clock restarts only when a signal is written. Restarted on every call, a run of refusals
    // closer than 20 s apart wrote the first and then nothing for as long as it lasted (#878).
    let last = LAST.load(Relaxed);
    if now.saturating_sub(last) >= 20 && LAST.compare_exchange(last, now, Relaxed, Relaxed).is_ok() {
        let _ = shared.store.put_kv(LAPSE_KEY, &now.to_string());
    }
}

/// `GET /api/state`'s renewal block: the switch, what the owner key reads, the last receipt or error.
pub fn view(shared: &Shared) -> Value {
    let (settings, state) = (Settings::from_env(), load(shared));
    let tier = Tier::of(shared.bots.first().and_then(|b| b.read().season.clone()).as_ref());
    json!({
        "enabled": settings.enabled,
        "max_seasons": settings.max_seasons,
        "owner_tier": match tier { Tier::Pro => "pro", Tier::Free => "free", Tier::Unknown => "unknown" },
        "multi_key": shared.config.bots.len() > 1,
        "receipt": state.receipt,
        "pending": state.pending.is_some(),
        "last_attempt_at": state.last_attempt_at,
        "last_error": state.last_error,
    })
}

/// Start the loop (the head or an all-in-one process; workers never buy). With auto-renew off it
/// makes no venue request at all: it reads the owner tier the season poller already holds and logs
/// the alarm once an hour.
pub fn spawn(shared: &Arc<Shared>) {
    if shared.config.worker || shared.config.bots.len() < 2 {
        return;
    }
    let settings = Settings::from_env();
    // Only the owner key reads the account's tier; a child key reads Free under Pro and would buy again and again.
    if std::env::var("SVANBOT_API_KEY").ok().as_deref() != Some(shared.config.bots[0].api_key.as_str()) {
        shared.log("fleet", "warn", "Pro auto-renew needs SVANBOT_API_KEY as the first bot (the owner key); it stays off");
        return;
    }
    shared.log(
        "fleet",
        "info",
        if settings.enabled {
            format!("Pro auto-renew is on: up to {} seasons when the owner key reads Free", settings.max_seasons)
        } else {
            "Pro auto-renew is off (SVANBOT_AUTO_RENEW_PRO=1 turns it on); a lapse is logged".to_string()
        },
    );
    crate::tasks::supervision::spawn("pro renewal", shared, move |s| async move { run(s, settings).await });
}

async fn run(shared: Arc<Shared>, settings: Settings) {
    let http = reqwest::Client::builder().user_agent(crate::USER_AGENT).timeout(Duration::from_secs(20)).build().unwrap();
    let owner = shared.config.bots[0].clone();
    let (mut last_read, mut last_alarm, mut seen) = (0.0_f64, 0.0_f64, 0.0_f64);
    loop {
        tokio::time::sleep(TICK).await;
        let now = now_secs();
        let signal = shared.store.get_kv(LAPSE_KEY).ok().flatten().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let cached = Tier::of(shared.bots[0].read().season.as_ref());
        let suspect = cached == Tier::Free || signal > seen;
        if !suspect {
            continue;
        }
        if !settings.enabled {
            // The alarm only: no venue request, no purchase.
            if cached == Tier::Free && now - last_alarm > 3600.0 {
                last_alarm = now;
                shared.log(
                    &owner.name,
                    "warn",
                    "Pro lapsed on the owner key: the other bots are refused (portfolio_pro_required). Renew Pro at openpoker.ai, or set SVANBOT_AUTO_RENEW_PRO=1 to renew from credits",
                );
            }
            seen = signal;
            continue;
        }
        if shared.config.dry_run || now - last_read < READ_GAP_SECS {
            continue;
        }
        last_read = now;
        seen = signal;
        let rest = Rest { http: &http, base: &shared.config.rest_base, key: &owner.api_key };
        let tier = read_tier(&rest).await;
        let mut state = load(&shared);
        if tier == Tier::Pro && state.pending.is_some() {
            state.pending = None;
            store(&shared, &state);
        }
        let plan = plan(settings, &state, tier, now);
        if matches!(plan, Plan::Nothing(_) | Plan::Off) {
            continue;
        }
        shared.log(&owner.name, "warn", "Pro lapsed on the owner key (fresh read): renewing from credits");
        let mut save = |s: &State| store(&shared, s);
        match execute(&rest, &mut state, plan, now, &mut save).await {
            Some(r) => shared.log(
                &owner.name,
                "info",
                format!(
                    "Pro renewed: {} season(s) bought for ${:.2}, {} remaining; the refused bots rejoin on their next attempt",
                    r.seasons_purchased,
                    r.amount_charged_cents as f64 / 100.0,
                    r.seasons_remaining
                ),
            ),
            None => shared.log(
                &owner.name,
                "error",
                format!(
                    "Pro lapsed and the renewal did not complete: {}",
                    state.last_error.as_deref().unwrap_or("see the venue's answer in the log")
                ),
            ),
        }
    }
}

#[cfg(test)]
mod tests;
