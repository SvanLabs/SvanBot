//! `review margins [DAYS] [CATEGORY...]` (0222, 0346): the pricing residual at the decision margin,
//! bin by predicted EV, before and after the installed self-calibration correction.
//!
//! The text table is the default; `--json` (#723) prints one JSON object carrying the same
//! [`MarginBin`] structs the table is formatted from, so an agent reads the numbers rather than the
//! layout and the two renders cannot drift apart.

use anyhow::Result;
use sv10_store::store::Store;

/// The report over the window `args` names: an optional DAYS, then optional categories. Flags
/// (`--json`) are not positional.
pub fn report(store: &Store, args: &[String], json: bool) -> Result<String> {
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let days: i64 = positional.first().and_then(|d| d.parse().ok()).unwrap_or(8);
    let since = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    let cats: Vec<&str> = positional.iter().skip(1).map(|s| s.as_str()).collect();
    let samples = store.calibration_samples_since(&since)?;
    let table = store.get_kv(crate::CALIBRATION_KEY)?;
    let (bias, bound) = (sv10_stats::margins::installed_bias(table.as_deref()), sv10_stats::margins::bound_by(table.as_deref()));
    let bins = sv10_stats::margins::margins(&samples, &cats, &bias);
    if json {
        let categories: Vec<serde_json::Value> = bins
            .iter()
            .filter(|(_, b)| !b.is_empty())
            .map(|(cat, b)| {
                serde_json::json!({
                    "category": cat,
                    "live_correction": bias.get(cat).copied().unwrap_or(0.0),
                    "set_by": bound.get(cat).map_or("?", String::as_str),
                    "bins": b,
                })
            })
            .collect();
        return Ok(serde_json::json!({"days": days, "decisions": samples.len(), "categories": categories}).to_string());
    }
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} priced decisions in the last {days} days: every settled decision the bot priced and got a stack result for, at every pot \
         size — the pricing population (the deep audit's is its big spots only, `review audit-by`)",
        samples.len()
    );
    let _ = writeln!(
        out,
        "   residual = realized - predicted (bb) on the UNCORRECTED price, the quantity self-calibration is fitted from; 'after correction' \
         subtracts the installed correction at its largest size, since in play a penalty is capped at its supported per-pot residual (0207); \
         * = off at 95% after that"
    );
    for (cat, bins) in &bins {
        if bins.is_empty() {
            continue;
        }
        let _ = writeln!(
            out,
            "{cat}  (live correction {:+.2} bb, set by {})",
            bias.get(cat).copied().unwrap_or(0.0),
            bound.get(cat).map_or("?", String::as_str)
        );
        for b in bins {
            let _ = writeln!(
                out,
                "  {} pred [{:>5}, {:>5})  n {:6}  residual {:+7.2} ± {:5.2}  after correction {:+7.2}{}",
                if b.at_margin() { "margin" } else { "      " },
                b.lo,
                b.hi,
                b.n,
                b.residual,
                b.half_width,
                b.after_correction,
                if b.miscalibrated() { " *" } else { "" }
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("sv10-margins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Store::open(&dir.join("svanbot10.db")).unwrap()
    }

    /// #723: `--json` is one parseable object carrying the same bins the table prints, and the text
    /// path — what an operator/script saw before the flag existed — is unchanged.
    #[test]
    fn the_json_report_parses_and_the_text_path_is_unchanged() {
        let store = store("json");
        // The default render, byte for byte as the bin printed it before the flag existed: an empty
        // store is the two header lines and no bins.
        let text = report(&store, &[], false).unwrap();
        let expected = concat!(
            "0 priced decisions in the last 8 days: every settled decision the bot priced and got a stack result for, at every pot ",
            "size — the pricing population (the deep audit's is its big spots only, `review audit-by`)\n",
            "   residual = realized - predicted (bb) on the UNCORRECTED price, the quantity self-calibration is fitted from; 'after correction' ",
            "subtracts the installed correction at its largest size, since in play a penalty is capped at its supported per-pot residual (0207); ",
            "* = off at 95% after that\n",
        );
        assert_eq!(text, expected);
        let json: serde_json::Value = serde_json::from_str(&report(&store, &["--json".into()], true).unwrap()).unwrap();
        assert_eq!(json["days"], 8);
        assert_eq!(json["decisions"], 0);
        assert_eq!(json["categories"].as_array().map(Vec::len), Some(0), "no samples, no bins");
        // The flag is not a positional: `--json 30` still reads 30 days and no category.
        let json: serde_json::Value = serde_json::from_str(&report(&store, &["--json".into(), "30".into()], true).unwrap()).unwrap();
        assert_eq!(json["days"], 30);
    }
}
