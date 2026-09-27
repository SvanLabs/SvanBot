use anyhow::{Result, bail};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct BotConfig {
    pub name: String,
    pub api_key: String,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bots: Vec<BotConfig>,
    pub ws_url: String,
    pub rest_base: String,
    pub max_buy_in: i64,
    /// Prefer tables with a bot ranked this high or better on the current season
    /// leaderboard (0 disables): tables without one are left and re-queued.
    pub seek_top_rank: i64,
    /// Bank winnings: once our table stack reaches this many big blinds, leave after the hand and
    /// rejoin with a fresh buy-in (`SVANBOT_BANK_STACK_BB`, 0 disables). The score counts chips on
    /// and off the table alike, and past 2,000 bb the per-hand swing tripled with no better win
    /// rate (2026-09-23), so banking protects the rank for free.
    pub bank_stack_bb: i64,
    pub web_host: String,
    pub web_port: u16,
    pub operator_token: Option<String>,
    pub dry_run: bool,
    pub artifacts: PathBuf,
    pub web_dist: PathBuf,
    /// Archive root written by the `archive` binary (`SVANBOT_ARCHIVE_DIR`, default
    /// `artifacts/archive`; on the reference box a second disk); the fleet only reports on it.
    pub archive_dir: PathBuf,
    /// Project root (holds `.env`).
    pub root: PathBuf,
    /// Running under the `scripts/start.sh` supervisor (our parent is `artifacts/supervisor.pid`),
    /// which restarts the fleet when it exits; only then may a saved setup restart it automatically.
    pub supervised: bool,
    /// Deepest hand the server exports per bot (Free tier 20,000; `SVANBOT_EXPORT_CAP`, 0 = unlimited).
    pub export_cap: i64,
    /// Head process of a split fleet (`SVANBOT_HEAD=1`): spawns no bots, runs the background tasks,
    /// owns the canonical models and serves the dashboard from worker heartbeats (0128).
    pub head: bool,
    /// Worker process of a split fleet (`SVANBOT_WORKER=1`, bots selected with `SVANBOT_ONLY`):
    /// plays its bots only — no background tasks, no model checkpoints (the head owns them) —
    /// and reports liveness through `bot.live.<name>` heartbeats.
    pub worker: bool,
    /// Every numeric setting and the value in force after clamping (see [`numbers`]).
    pub(crate) numbers: std::collections::BTreeMap<&'static str, i64>,
}

fn var(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

/// `1`/`true` environment flag, absent or anything else off.
fn flag(k: &str) -> bool {
    var(k).map(|v| v == "true" || v == "1").unwrap_or(false)
}

/// One integer setting: its key, its default and the range its consumers' arithmetic can take.
#[derive(Clone, Copy)]
struct Setting {
    key: &'static str,
    default: i64,
    min: i64,
    max: i64,
}

/// The buy-in the server allows per seat.
const BUY_IN: Setting = Setting { key: "SVANBOT_BUY_IN", default: 5_000, min: 1_000, max: 5_000 };
/// Leaderboard rank to seek a table for (0 disables).
const SEEK_TOP_RANK: Setting = Setting { key: "SVANBOT_SEEK_TOP_RANK", default: 30, min: 0, max: 1_000 };
/// Bank winnings at this many big blinds (0 disables). Bounded so `bank_bb * bb` in the hand loop
/// cannot overflow: an operator's `4000000000000000000` parsed and panicked the frame loop (0250).
const BANK_STACK_BB: Setting = Setting { key: "SVANBOT_BANK_STACK_BB", default: 2_000, min: 0, max: 100_000 };
/// Deepest hand the server exports per bot (0 = unlimited).
const EXPORT_CAP: Setting = Setting { key: "SVANBOT_EXPORT_CAP", default: 20_000, min: 0, max: 10_000_000 };
/// The dashboard's port.
const WEB_PORT: Setting = Setting { key: "SVANBOT_WEB_PORT", default: 5_000, min: 1, max: 65_535 };
/// Bots to play: unset means every configured key, so the default is the most keys we read.
const FLEET_SIZE: Setting = Setting { key: "SVANBOT_RUNTIME__FLEET_SIZE", default: 10, min: 1, max: 10 };

/// Every numeric setting, so the gate can hold each one to the same rule.
const SETTINGS: [Setting; 6] = [BUY_IN, SEEK_TOP_RANK, BANK_STACK_BB, EXPORT_CAP, WEB_PORT, FLEET_SIZE];

/// Every numeric setting's value in force, read once. [`SETTINGS`] is the single list, so a setting
/// cannot be added without a declared range and the test that holds it there.
fn numbers() -> std::collections::BTreeMap<&'static str, i64> {
    SETTINGS.iter().map(|s| (s.key, var(s.key).and_then(|v| v.parse().ok()).unwrap_or(s.default).clamp(s.min, s.max))).collect()
}

impl Config {
    /// Every numeric setting with the value in force after clamping, one line at startup: the
    /// answer to "is this setting even taking effect?" without reading `.env` and the dashboard
    /// side by side (0244, 0246).
    pub fn describe(&self) -> String {
        format!("{} bots · {}", self.bots.len(), self.numbers.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", "))
    }

    pub fn from_env(root: &std::path::Path) -> Result<Config> {
        let _ = sv10_rt::load_env_file(&root.join(".env"));
        let n = numbers();
        let mut bots = Vec::new();
        if let Some(key) = var("SVANBOT_API_KEY") {
            let name = var("SVANBOT_MAIN_NAME").or_else(|| var("SVANBOT_BOT_NAME")).unwrap_or_else(|| "bot1".into());
            bots.push(BotConfig { name, api_key: key });
        }
        for i in 2..=10 {
            if let Some(key) = var(&format!("OPENPOKER_API_KEY_{i}")) {
                let name = var(&format!("BOT_{i}_NAME")).unwrap_or_else(|| format!("bot{i}"));
                bots.push(BotConfig { name, api_key: key });
            }
        }
        if let Some(only) = var("SVANBOT_ONLY") {
            let keep: Vec<&str> = only.split(',').map(|s| s.trim()).collect();
            bots.retain(|b| keep.contains(&b.name.as_str()));
        }
        if bots.is_empty() {
            bail!("no bot API keys configured (SVANBOT_API_KEY / OPENPOKER_API_KEY_n)");
        }
        let fleet = (n[FLEET_SIZE.key]).min(bots.len() as i64) as usize;
        bots.truncate(fleet);
        let server = var("SVANBOT_SERVER_URL").unwrap_or_else(|| "wss://openpoker.ai/ws".into());
        Ok(Config {
            bots,
            ws_url: server,
            rest_base: var("SVANBOT_REST_BASE").unwrap_or_else(|| "https://api.openpoker.ai/api".into()),
            max_buy_in: n[BUY_IN.key],
            seek_top_rank: n[SEEK_TOP_RANK.key],
            bank_stack_bb: n[BANK_STACK_BB.key],
            archive_dir: var("SVANBOT_ARCHIVE_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("artifacts").join("archive")),
            export_cap: n[EXPORT_CAP.key],
            web_host: var("SVANBOT_WEB__HOST").unwrap_or_else(|| "127.0.0.1".into()),
            web_port: n[WEB_PORT.key] as u16,
            operator_token: var("SVANBOT_WEB__OPERATOR_TOKEN"),
            dry_run: flag("SVANBOT_RUNTIME__DRY_RUN"),
            head: flag("SVANBOT_HEAD"),
            worker: flag("SVANBOT_WORKER"),
            artifacts: root.join("artifacts"),
            root: root.to_path_buf(),
            numbers: n,
            supervised: supervised(root),
            web_dist: root.join("web").join("dist"),
        })
    }
}

#[cfg(unix)]
fn supervised(root: &std::path::Path) -> bool {
    std::fs::read_to_string(root.join("artifacts").join("supervisor.pid"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .is_some_and(|pid| pid == std::os::unix::process::parent_id())
}

#[cfg(not(unix))]
fn supervised(_root: &std::path::Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0250: one parser, one declared range per setting, so a typo cannot reach a multiplication
    /// in the hand loop. `bank_stack_bb` had only `.max(0)`, and `4e18` parsed.
    #[test]
    fn every_numeric_setting_is_bounded_by_one_parser() {
        for setting in SETTINGS {
            assert!(setting.min <= setting.default && setting.default <= setting.max, "{} default outside its range", setting.key);
            let read = |raw: &str| raw.parse().ok().map(|v: i64| v.clamp(setting.min, setting.max)).unwrap_or(setting.default);
            assert_eq!(read(&setting.default.to_string()), setting.default);
            assert_eq!(read("0"), setting.min.max(0), "0 is the floor where the setting allows it");
            assert_eq!(read("-99999999999999999999"), setting.default, "unparseable keeps the default");
            assert_eq!(read("loud"), setting.default);
            assert_eq!(read("4000000000000000000"), setting.max, "an absurd value is clamped, never wrapped");
        }
        // The bound the frame loop depends on: `bank_bb * bb` at the largest blind we have seen.
        assert!(BANK_STACK_BB.max.checked_mul(10_000).is_some(), "the banking multiplication cannot overflow");
        assert!(EXPORT_CAP.max.checked_mul(1_000_000).is_some(), "nor the export cursor");
        // The loader walks exactly this list, so a setting cannot exist without a bound.
        let read = numbers();
        assert_eq!(read.len(), SETTINGS.len());
        for setting in SETTINGS {
            let v = read[setting.key];
            assert!((setting.min..=setting.max).contains(&v), "{} out of range: {v}", setting.key);
        }
    }

    #[test]
    fn fleet_roles_parse_from_the_env_file_and_default_off() {
        let root = std::env::temp_dir().join(format!("sv10-config-0128-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".env"), "SVANBOT_API_KEY=x\nSVANBOT_HEAD=1\nSVANBOT_WORKER=yes\n").unwrap();
        let c = Config::from_env(&root).unwrap();
        assert!(c.head);
        assert!(!c.worker, "only 1/true enable a role");
        let _ = std::fs::remove_dir_all(&root);
    }
}
