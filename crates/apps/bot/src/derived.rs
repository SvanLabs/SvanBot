//! Derived public data: aggregates and schema, never raw opponent hands (#17).
//!
//! The recorded decision publishes derived aggregates and the schema: counts, summaries and the
//! table definitions — no hole cards, no hand histories, no opponent names, no API keys or tokens,
//! no emails, no home or disk paths. The export fails closed: the scrub runs over the exact bytes
//! written, and a hit names the violation *class*, never the value.
//!
//! What is deliberately absent, and why: every `hands` row (hole cards and named players),
//! `decision_details` (per-decision explanations), the nemesis findings (named opponents), and the
//! live databases themselves. An aggregate that names a player is not an aggregate.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use sv10_store::store::Store;

/// Secret values from the operator's `.env`, held for comparison and never printed.
pub struct SecretSet {
    values: Vec<String>,
}

impl SecretSet {
    /// Values worth scrubbing: the right-hand side of `*_KEY*`/`*_TOKEN*` assignments, at least 8
    /// characters (shorter ones collide with ordinary words).
    pub fn from_dotenv(text: &str) -> SecretSet {
        let values = text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                let (key, value) = line.split_once('=')?;
                if !(key.contains("KEY") || key.contains("TOKEN")) {
                    return None;
                }
                let value = value.trim().trim_matches('"').trim_matches('\'').trim().to_string();
                (value.len() >= 8).then_some(value)
            })
            .collect();
        SecretSet { values }
    }

    /// Violation classes found in `text`: `api-key-or-token`, `email`, `home-path`. The returned
    /// strings name classes only — a value is never copied into them.
    pub fn classes_found_in(&self, text: &str, home: &str) -> Vec<String> {
        let mut out = Vec::new();
        if self.values.iter().any(|v| text.contains(v)) {
            out.push("api-key-or-token".to_string());
        }
        if text.split(|c: char| !(c.is_alphanumeric() || matches!(c, '@' | '.' | '-' | '_' | '+'))).any(is_email) {
            out.push("email".to_string());
        }
        if !home.is_empty() && text.contains(home) {
            out.push("home-path".to_string());
        }
        out
    }
}

/// A token shaped like `name@domain.tld`.
fn is_email(token: &str) -> bool {
    let Some(at) = token.find('@') else { return false };
    let (name, domain) = token.split_at(at);
    let domain = &domain[1..];
    !name.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !name.contains('@')
}

/// The public aggregates of one store: counts, summaries and mixes. Every number is over a whole
/// population; no row names a player, a hand or a card.
pub fn build_aggregates(store: &Store) -> Result<serde_json::Value> {
    let epoch = "1970-01-01T00:00:00Z";
    let decisions: Vec<serde_json::Value> = store
        .decision_counts_since(epoch)?
        .into_iter()
        .map(|(street, action, n)| serde_json::json!({"street": street, "action": action, "n": n}))
        .collect();
    let audit = store.audit_summary(epoch).unwrap_or_default();
    let calibration: Vec<serde_json::Value> = store
        .calibration_summary()?
        .into_iter()
        .map(|(category, n, predicted, realized, _)| {
            serde_json::json!({"category": category, "n": n, "predicted_bb": predicted, "realized_bb": realized})
        })
        .collect();
    let preflop_mix: Vec<serde_json::Value> = store
        .first_preflop_actions(epoch, "2999-01-01T00:00:00Z")?
        .into_iter()
        .map(|(action, n)| serde_json::json!({"action": action, "n": n}))
        .collect();
    Ok(serde_json::json!({
        "meta": {
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "app_version": env!("CARGO_PKG_VERSION"),
            "window_since": epoch,
        },
        "decisions": decisions,
        "audit": {
            "decisions": audit.decisions,
            "same_action": audit.same_action,
            "mean_gap_bb": audit.mean_gap_bb,
            "max_gap_bb": audit.max_gap_bb,
            "mean_deep_ms": audit.mean_deep_ms,
            "queued": audit.queued,
        },
        "calibration": calibration,
        "preflop_mix": preflop_mix,
    }))
}

/// `CREATE TABLE` statements of a database, ordered by name: the schema, without a row of data.
fn schema_ddl(path: &Path) -> Result<String> {
    if !path.exists() {
        return Ok(format!("-- {path} absent\n", path = path.display()));
    }
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {} read-only", path.display()))?;
    let mut stmt = conn.prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
    let ddl: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(ddl.join(";\n") + ";\n")
}

/// Total hands and hands per day (`YYYY-MM-DD`, no names, no cards).
fn hands_totals(main: &Path) -> Result<(i64, Vec<(String, i64)>)> {
    let conn = rusqlite::Connection::open_with_flags(main, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {} read-only", main.display()))?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM hands", [], |r| r.get(0))?;
    let mut stmt = conn.prepare("SELECT substr(ended_at, 1, 10), COUNT(*) FROM hands GROUP BY 1 ORDER BY 1")?;
    let by_day: Vec<(String, i64)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
    Ok((total, by_day))
}

/// Write `schema.sql`, `aggregates.json` and `SHA256SUMS` into `out`, scrubbed, and return the
/// files written. Fails closed on any scrub hit.
pub fn export_derived(main: &Path, history: &Path, out: &Path, secrets: &SecretSet, home: &str) -> Result<Vec<PathBuf>> {
    if !main.exists() {
        bail!("no live database at {}", main.display());
    }
    let store = Store::open(main)?;
    let mut aggregates = build_aggregates(&store)?;
    let (hands_total, hands_by_day) = hands_totals(main)?;
    aggregates["hands"] = serde_json::json!({
        "total": hands_total,
        "by_day": hands_by_day.into_iter().map(|(day, n)| serde_json::json!({"day": day, "n": n})).collect::<Vec<_>>(),
    });
    let schema = format!("-- svanbot10.db\n{}-- history.db\n{}", schema_ddl(main)?, schema_ddl(history)?);
    let aggregates_text = serde_json::to_string_pretty(&aggregates)?;
    for (name, text) in [("schema.sql", &schema), ("aggregates.json", &aggregates_text)] {
        let hits = secrets.classes_found_in(text, home);
        if !hits.is_empty() {
            bail!("refusing the export: {name} trips the scrub ({})", hits.join(", "));
        }
    }
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("schema.sql"), &schema)?;
    std::fs::write(out.join("aggregates.json"), &aggregates_text)?;
    let mut sums = String::new();
    for name in ["schema.sql", "aggregates.json"] {
        sums += &format!("{}  {name}\n", sv10_store::integrity::file_sha256(&out.join(name))?);
    }
    std::fs::write(out.join("SHA256SUMS"), &sums)?;
    Ok(["schema.sql", "aggregates.json", "SHA256SUMS"].iter().map(|n| out.join(n)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scrub names violation classes, never the values themselves.
    #[test]
    fn the_scrub_names_classes_never_values() {
        let secrets = SecretSet::from_dotenv("SVANBOT_API_KEY=tok_live_abc123\nOTHER=1\n");
        let hits = secrets.classes_found_in("key=tok_live_abc123 mail=a@b.com p=/home/op/x", "/home/op");
        assert!(hits.contains(&"api-key-or-token".to_string()), "{hits:?}");
        assert!(hits.contains(&"email".to_string()), "{hits:?}");
        assert!(hits.contains(&"home-path".to_string()), "{hits:?}");
        assert!(!format!("{hits:?}").contains("tok_live_abc123"), "the value leaked into the report");
        assert!(secrets.classes_found_in("nothing here, all quiet", "/home/op").is_empty());
    }

    /// An export over a store with a decision in it holds counts and schema, and nothing that
    /// names a player, a hand or a card.
    #[test]
    fn the_export_holds_aggregates_not_hands() {
        let dir = std::env::temp_dir().join(format!("sv10-derived-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        store.insert_decision("SvanBotV10", "h1", "river", "raise", Some(1375), Some(0.4), 67, 0, 12.0, "{}").unwrap();
        let out = dir.join("out");
        let files = export_derived(&dir.join("svanbot10.db"), &dir.join("history.db"), &out, &SecretSet::from_dotenv(""), "/home/op")
            .expect("an export over a readable store");
        assert_eq!(files.len(), 3, "{files:?}");
        let agg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out.join("aggregates.json")).unwrap()).unwrap();
        for key in ["meta", "decisions", "audit", "calibration", "preflop_mix"] {
            assert!(agg.get(key).is_some(), "no {key} in {agg}");
        }
        let schema = std::fs::read_to_string(out.join("schema.sql")).unwrap();
        assert!(schema.contains("CREATE TABLE"), "no DDL in the schema");
        let text = std::fs::read_to_string(out.join("aggregates.json")).unwrap();
        for forbidden in ["SvanBotV10", "h1", "Ah", "hole"] {
            assert!(!text.contains(forbidden), "{forbidden} reached the export");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
