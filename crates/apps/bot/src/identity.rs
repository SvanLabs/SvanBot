//! A bot's identity across renames. Hands are stored under the configured bot name, so renaming a
//! bot in `.env` (2026-09-23: SvanBotV7 -> SvanBotV10, a server-side rename that kept its season
//! score and #1 rank) split its season into two names and the dashboard showed 43 hands for the
//! #1 bot. Each start records the configured name under a fingerprint of the bot's API key, so every
//! name that key ever played under stays linked; `SVANBOT_ALIASES` (`new:old[,new:old]`) seeds names
//! used before this record existed. The key itself is never stored.

use std::collections::HashMap;
use sv10_digest::Sha256;
use sv10_store::store::Store;

use crate::config::BotConfig;

/// Kv key prefix of the per-key name list.
const NAMES_PREFIX: &str = "bot.names.";

/// Stable, non-reversible fingerprint of an API key (first 12 bytes of SHA-256, domain-separated).
pub fn key_fingerprint(api_key: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"svanbot10 bot identity\0");
    h.update(api_key.as_bytes());
    sv10_digest::hex(&h.finalize()[..12])
}

/// Parse `new:old,new:old2` into new -> [old, ...].
fn parse_aliases(spec: &str) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for pair in spec.split(',') {
        if let Some((new, old)) = pair.split_once(':') {
            let (new, old) = (new.trim(), old.trim());
            if !new.is_empty() && !old.is_empty() && new != old {
                out.entry(new.to_string()).or_default().push(old.to_string());
            }
        }
    }
    out
}

/// Every name any key has played under -> that key's current name, from the stored name lists (first
/// entry = current), for readers without the fleet config (`review`, 0246). Unreadable lists are skipped.
pub fn current_names(store: &Store) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (_, list) in store.kv_with_prefix(NAMES_PREFIX).unwrap_or_default() {
        let names: Vec<String> = serde_json::from_str(&list).unwrap_or_default();
        if let Some(current) = names.first() {
            for n in &names {
                out.insert(n.clone(), current.clone());
            }
        }
    }
    out
}

/// Record every configured name under its key and return current name -> all its names (current
/// first, then earlier ones, no duplicates). `aliases` is the `SVANBOT_ALIASES` value.
pub fn resolve(store: &Store, bots: &[BotConfig], aliases: &str) -> HashMap<String, Vec<String>> {
    let seeds = parse_aliases(aliases);
    let mut out = HashMap::new();
    for bot in bots {
        let key = format!("{NAMES_PREFIX}{}", key_fingerprint(&bot.api_key));
        let mut known: Vec<String> = store.get_kv(&key).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
        let before = known.clone();
        for name in std::iter::once(&bot.name).chain(seeds.get(&bot.name).into_iter().flatten()) {
            if !known.contains(name) {
                known.push(name.clone());
            }
        }
        if known != before
            && let Err(e) = store.put_kv(&key, &serde_json::to_string(&known).unwrap_or_default())
        {
            tracing::warn!("bot names for {} not recorded: {e}", bot.name);
        }
        let mut names = vec![bot.name.clone()];
        names.extend(known.into_iter().filter(|n| *n != bot.name));
        out.insert(bot.name.clone(), names);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("sv10-identity-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Store::open(&dir.join("svanbot10.db")).unwrap()
    }

    fn bot(name: &str, key: &str) -> BotConfig {
        BotConfig { name: name.into(), api_key: key.into() }
    }

    /// 0246: readers without the fleet config (`review all`, `status.sh`) merge a renamed bot's hands
    /// under its current name from the stored lists alone; a name nobody recorded maps to nothing.
    #[test]
    fn stored_name_lists_map_every_name_to_the_current_one() {
        let s = store("current");
        resolve(&s, &[bot("SvanBotV10", "key-main"), bot("Svanar", "key-2")], "SvanBotV10:SvanBotV7");
        s.put_kv("bot.names.broken", "not json").unwrap();
        s.put_kv("bot.namesake", r#"["Other"]"#).unwrap();
        let m = current_names(&s);
        assert_eq!(m["SvanBotV7"], "SvanBotV10");
        assert_eq!(m["SvanBotV10"], "SvanBotV10");
        assert_eq!(m["Svanar"], "Svanar");
        assert!(!m.contains_key("Other"), "the prefix is literal: `bot.namesake` is not a name list");
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn a_renamed_bot_keeps_every_name_it_played_under() {
        let s = store("rename");
        // First start after the rename: the old name is seeded once from SVANBOT_ALIASES.
        let m = resolve(&s, &[bot("SvanBotV10", "key-main"), bot("Svanar", "key-2")], "SvanBotV10:SvanBotV7");
        assert_eq!(m["SvanBotV10"], ["SvanBotV10", "SvanBotV7"]);
        assert_eq!(m["Svanar"], ["Svanar"]);
        // Later starts remember it without the setting.
        assert_eq!(resolve(&s, &[bot("SvanBotV10", "key-main")], "")["SvanBotV10"], ["SvanBotV10", "SvanBotV7"]);
        // A future rename in .env needs no setting at all: the key links the names.
        assert_eq!(resolve(&s, &[bot("SvanBotV11", "key-main")], "")["SvanBotV11"], ["SvanBotV11", "SvanBotV10", "SvanBotV7"]);
    }

    #[test]
    fn the_key_is_never_stored_and_aliases_parse_leniently() {
        let s = store("secret");
        resolve(&s, &[bot("A", "super-secret-key")], "");
        let fp = key_fingerprint("super-secret-key");
        assert_eq!(fp.len(), 24);
        assert!(!fp.contains("secret"));
        assert!(s.get_kv(&format!("bot.names.{fp}")).unwrap().is_some());
        assert_eq!(parse_aliases(" A : B , bad, :x, C:C ,A:D"), HashMap::from([("A".to_string(), vec!["B".to_string(), "D".to_string()])]));
    }
}
