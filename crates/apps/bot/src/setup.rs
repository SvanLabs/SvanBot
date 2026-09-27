//! Bot setup from the dashboard (0085): which bots run, under which name and key, and the table
//! settings, written to `.env` so every tool, the systemd unit and the next restart agree.
//!
//! `.env` stays the single source of truth. Keys are write-only through the API (the view carries a
//! four-character hint), every value is validated against a shell- and parser-safe charset before it
//! is written, the file is replaced atomically at mode 600 with the previous version kept as
//! `artifacts/.env.previous`, and a supervised fleet restarts at the next moment no bot is mid-turn
//! (the hot-swap path) so the change applies without dropping an action.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Bots a Pro portfolio may run concurrently (openpoker.ai fair-play rules).
pub const MAX_BOTS: usize = 5;
/// Highest slot number `Config::from_env` reads (`OPENPOKER_API_KEY_10`); setup clears up to it.
const MAX_SLOTS_READ: usize = 10;

/// One configured bot as the dashboard sees it (never the key itself).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SlotView {
    /// Position in `.env` (0 = `SVANBOT_API_KEY`).
    pub slot: usize,
    /// Bot name.
    pub name: String,
    /// `…` plus the key's last four characters.
    pub key_hint: String,
    /// Plays when the fleet starts.
    pub enabled: bool,
}

/// The setup page state.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SetupView {
    /// Configured bots in slot order.
    pub bots: Vec<SlotView>,
    /// Maximum buy-in (1,000–5,000).
    pub buy_in: i64,
    /// Seek tables with a current-season top-N bot (0 disables).
    pub seek_top_rank: i64,
    /// Bots the account may run at once.
    pub max_bots: usize,
    /// Whether saving is allowed, and why not.
    pub can_write: bool,
    /// Reason saving is refused, when it is.
    pub write_blocked: Option<String>,
    /// Whether a save restarts the fleet automatically (running under `scripts/start.sh`).
    pub supervised: bool,
    /// A saved change is waiting for the restart.
    pub restart_pending: bool,
}

/// One bot in a save request.
#[derive(Clone, Debug, Deserialize)]
pub struct BotInput {
    /// Bot name as registered on openpoker.ai.
    pub name: String,
    /// A new key; `None` keeps the key of `from_slot`.
    #[serde(default)]
    pub key: Option<String>,
    /// Existing slot whose stored key this bot keeps.
    #[serde(default)]
    pub from_slot: Option<usize>,
    /// Plays when the fleet starts.
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// A save request.
#[derive(Clone, Debug, Deserialize)]
pub struct SetupInput {
    /// Bots in the order they should occupy slots.
    pub bots: Vec<BotInput>,
    /// Maximum buy-in.
    pub buy_in: i64,
    /// Seek tables with a top-N bot (0 disables).
    pub seek_top_rank: i64,
}

/// A bot as configured: name, key, enabled.
#[derive(Clone, Debug, PartialEq)]
pub struct Configured {
    /// Bot name.
    pub name: String,
    /// API key.
    pub key: String,
    /// Plays when the fleet starts.
    pub enabled: bool,
}

fn key_vars(slot: usize) -> (String, String) {
    if slot == 0 {
        ("SVANBOT_API_KEY".into(), "SVANBOT_MAIN_NAME".into())
    } else {
        (format!("OPENPOKER_API_KEY_{}", slot + 1), format!("BOT_{}_NAME", slot + 1))
    }
}

/// Values from `.env` text, falling back to `fallback` (the process environment) for keys the file
/// does not set, empty values treated as unset — the same precedence the fleet uses at startup.
pub fn lookup<'a>(env_text: &str, fallback: impl Fn(&str) -> Option<String> + 'a) -> impl Fn(&str) -> Option<String> + 'a {
    let file: HashMap<String, String> = sv10_rt::parse_env(env_text).into_iter().collect();
    move |k: &str| fallback(k).or_else(|| file.get(k).cloned()).filter(|v| !v.trim().is_empty())
}

/// The configured bots and table settings, as `Config::from_env` would read them (without applying
/// `SVANBOT_ONLY` or the fleet size, which become the `enabled` flags).
pub fn current(var: &dyn Fn(&str) -> Option<String>) -> (Vec<Configured>, i64, i64) {
    let mut bots = Vec::new();
    for slot in 0..MAX_SLOTS_READ {
        let (kv, nv) = key_vars(slot);
        if let Some(key) = var(&kv) {
            let name =
                var(&nv).or_else(|| if slot == 0 { var("SVANBOT_BOT_NAME") } else { None }).unwrap_or_else(|| format!("bot{}", slot + 1));
            bots.push(Configured { name, key, enabled: true });
        }
    }
    if let Some(only) = var("SVANBOT_ONLY") {
        let keep: Vec<&str> = only.split(',').map(str::trim).collect();
        for b in &mut bots {
            b.enabled = keep.contains(&b.name.as_str());
        }
    }
    if let Some(n) = var("SVANBOT_RUNTIME__FLEET_SIZE").and_then(|v| v.parse::<usize>().ok()) {
        let mut seen = 0;
        for b in bots.iter_mut().filter(|b| b.enabled) {
            seen += 1;
            b.enabled = seen <= n.max(1);
        }
    }
    let buy_in = var("SVANBOT_BUY_IN").and_then(|v| v.parse().ok()).unwrap_or(5000i64).clamp(1000, 5000);
    let seek = var("SVANBOT_SEEK_TOP_RANK").and_then(|v| v.parse().ok()).unwrap_or(30i64).max(0);
    (bots, buy_in, seek)
}

/// Dashboard view of `bots` (keys reduced to hints).
pub fn slot_views(bots: &[Configured]) -> Vec<SlotView> {
    bots.iter()
        .enumerate()
        .map(|(slot, b)| {
            let tail: String = b.key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            SlotView { slot, name: b.name.clone(), key_hint: format!("…{tail}"), enabled: b.enabled }
        })
        .collect()
}

/// Validate a save request against the current bots and turn it into `.env` updates.
pub fn plan(input: &SetupInput, existing: &[Configured]) -> Result<Vec<(String, Option<String>)>, String> {
    if input.bots.is_empty() {
        return Err("Configure at least one bot.".into());
    }
    if input.bots.len() > MAX_BOTS {
        return Err(format!("At most {MAX_BOTS} bots can play at once (Pro portfolio limit)."));
    }
    if !input.bots.iter().any(|b| b.enabled) {
        return Err("Enable at least one bot.".into());
    }
    if !(1000..=5000).contains(&input.buy_in) {
        return Err("Buy-in must be between 1,000 and 5,000 chips.".into());
    }
    if !(0..=1000).contains(&input.seek_top_rank) {
        return Err("Seek rank must be between 0 (off) and 1,000.".into());
    }
    let mut resolved: Vec<(String, String, bool)> = Vec::new();
    for (i, b) in input.bots.iter().enumerate() {
        let n = i + 1;
        let name = b.name.trim();
        if !(3..=32).contains(&name.len()) || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("Bot {n}: names are 3–32 letters, digits or underscores."));
        }
        let key = match (&b.key, b.from_slot) {
            (Some(k), _) if !k.trim().is_empty() => k.trim().to_string(),
            (_, Some(slot)) => existing
                .get(slot)
                .map(|e| e.key.clone())
                .ok_or_else(|| format!("Bot {n}: its previous key no longer exists; paste the key."))?,
            _ => return Err(format!("Bot {n}: paste its API key.")),
        };
        if !(16..=512).contains(&key.len()) || !sv10_rt::env_value_is_safe(&key) {
            return Err(format!("Bot {n}: that does not look like an API key."));
        }
        if resolved.iter().any(|(rn, _, _)| rn.eq_ignore_ascii_case(name)) {
            return Err(format!("Bot {n}: the name {name} is used twice."));
        }
        if resolved.iter().any(|(_, rk, _)| *rk == key) {
            return Err(format!("Bot {n}: the same key is used twice."));
        }
        resolved.push((name.to_string(), key, b.enabled));
    }
    let mut updates: Vec<(String, Option<String>)> = Vec::new();
    for slot in 0..MAX_SLOTS_READ {
        let (kv, nv) = key_vars(slot);
        match resolved.get(slot) {
            Some((name, key, _)) => {
                updates.push((kv, Some(key.clone())));
                updates.push((nv, Some(name.clone())));
            }
            None => {
                updates.push((kv, None));
                updates.push((nv, None));
            }
        }
    }
    let enabled: Vec<&str> = resolved.iter().filter(|r| r.2).map(|r| r.0.as_str()).collect();
    updates.push(("SVANBOT_ONLY".into(), (enabled.len() < resolved.len()).then(|| enabled.join(","))));
    // The enabled flags replace these older switches.
    updates.push(("SVANBOT_RUNTIME__FLEET_SIZE".into(), None));
    updates.push(("SVANBOT_BOT_NAME".into(), None));
    updates.push(("SVANBOT_BUY_IN".into(), Some(input.buy_in.to_string())));
    updates.push(("SVANBOT_SEEK_TOP_RANK".into(), Some(input.seek_top_rank.to_string())));
    if let Some((k, _)) = updates.iter().find(|(_, v)| v.as_deref().is_some_and(|v| !sv10_rt::env_value_is_safe(v))) {
        return Err(format!("Unsafe value for {k}."));
    }
    Ok(updates)
}

/// Whether a dashboard bound to `host` without an operator token may save setup (loopback only).
pub fn loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "op_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1111";
    const KEY_B: &str = "op_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb2222";
    const KEY_C: &str = "op_cccccccccccccccccccccccccccccccccccc3333";

    fn env(text: &str) -> impl Fn(&str) -> Option<String> {
        lookup(text, |_| None)
    }

    #[test]
    fn current_reads_like_the_fleet_and_hides_keys() {
        let text = format!(
            "SVANBOT_API_KEY={KEY_A}\nSVANBOT_BOT_NAME=Legacy\nOPENPOKER_API_KEY_2={KEY_B}\nBOT_2_NAME=Two\nOPENPOKER_API_KEY_4={KEY_C}\nSVANBOT_ONLY=Legacy,bot4\nSVANBOT_BUY_IN=3000\n"
        );
        let (bots, buy_in, seek) = current(&env(&text));
        assert_eq!(
            bots.iter().map(|b| (b.name.as_str(), b.enabled)).collect::<Vec<_>>(),
            vec![("Legacy", true), ("Two", false), ("bot4", true)]
        );
        assert_eq!((buy_in, seek), (3000, 30));
        let views = slot_views(&bots);
        assert_eq!(views[0].key_hint, "…1111");
        assert!(!serde_json::to_string(&views).unwrap().contains(KEY_A));
    }

    #[test]
    fn a_plan_round_trips_through_env_text_and_keeps_keys_by_slot() {
        let text = format!(
            "# keep me\nSVANBOT_API_KEY={KEY_A}\nSVANBOT_MAIN_NAME=Alpha\nOPENPOKER_API_KEY_2={KEY_B}\nBOT_2_NAME=Beta\nSVANBOT_RUNTIME__FLEET_SIZE=2\nOTHER=1\n"
        );
        let (existing, _, _) = current(&env(&text));
        // Reorder: Beta first (kept key), a new bot, Alpha disabled (kept key).
        let input = SetupInput {
            bots: vec![
                BotInput { name: "Beta".into(), key: None, from_slot: Some(1), enabled: true },
                BotInput { name: "Gamma_3".into(), key: Some(KEY_C.into()), from_slot: None, enabled: true },
                BotInput { name: "Alpha".into(), key: None, from_slot: Some(0), enabled: false },
            ],
            buy_in: 4000,
            seek_top_rank: 0,
        };
        let updates = plan(&input, &existing).unwrap_or_else(|e| panic!("{e}"));
        let refs: Vec<(&str, Option<&str>)> = updates.iter().map(|(k, v)| (k.as_str(), v.as_deref())).collect();
        let out = sv10_rt::update_env_text(&text, &refs);
        assert!(out.starts_with("# keep me\n") && out.contains("OTHER=1") && !out.contains("FLEET_SIZE"));
        let (after, buy_in, seek) = current(&env(&out));
        assert_eq!(
            after,
            vec![
                Configured { name: "Beta".into(), key: KEY_B.into(), enabled: true },
                Configured { name: "Gamma_3".into(), key: KEY_C.into(), enabled: true },
                Configured { name: "Alpha".into(), key: KEY_A.into(), enabled: false },
            ]
        );
        assert_eq!((buy_in, seek), (4000, 0));
    }

    #[test]
    fn invalid_requests_are_refused_with_a_reason() {
        let existing = vec![Configured { name: "Alpha".into(), key: KEY_A.into(), enabled: true }];
        let bot =
            |name: &str, key: Option<&str>| BotInput { name: name.into(), key: key.map(String::from), from_slot: None, enabled: true };
        let req = |bots: Vec<BotInput>| SetupInput { bots, buy_in: 5000, seek_top_rank: 30 };
        let err = |r: SetupInput| plan(&r, &existing).unwrap_err();
        assert!(err(req(vec![])).contains("at least one"));
        assert!(err(req(vec![bot("ab", Some(KEY_B))])).contains("names"));
        assert!(err(req(vec![bot("Evil", Some("short"))])).contains("API key"));
        assert!(err(req(vec![bot("Evil", Some("op_bbbbbbbbbbbbbbbbbbbb\nSVANBOT_WEB__HOST=0.0.0.0"))])).contains("API key"));
        assert!(err(req(vec![bot("Beta", Some(KEY_B)), bot("beta", Some(KEY_C))])).contains("twice"));
        assert!(err(req(vec![bot("Beta", Some(KEY_B)), bot("Gamma", Some(KEY_B))])).contains("same key"));
        assert!(err(req((0..6).map(|i| bot(&format!("Bot_{i}"), Some(&format!("{KEY_B}{i}")))).collect())).contains("At most 5"));
        assert!(err(req(vec![BotInput { name: "Zed".into(), key: None, from_slot: Some(7), enabled: true }])).contains("no longer exists"));
        let mut disabled = req(vec![bot("Beta", Some(KEY_B))]);
        disabled.bots[0].enabled = false;
        assert!(err(disabled).contains("Enable"));
        assert!(err(SetupInput { buy_in: 900, ..req(vec![bot("Beta", Some(KEY_B))]) }).contains("Buy-in"));
        assert!(loopback("127.0.0.1") && !loopback("0.0.0.0"));
    }
}
