//! `review unrecorded [HOURS]` — hands the server's history export shows our bots playing that
//! this fleet's store never recorded (0151: 2,179 such hands in season 13, -52,391 chips, a
//! strategy folding 40-54% preflop against our ~2%). The fleet-check script flagged this in its
//! window and exited 1; the check lives here now, with both windows (the recent one and the
//! season) and the hot-swap exemption it had.
//!
//! The export arrives with a delay, so the newest minutes can lag behind the store.

use anyhow::Result;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use std::collections::HashMap;
use std::path::Path;
use sv10_store::store::Store;

/// A hand running when the fleet hot-swaps or restarts finishes after the old process exited, so
/// the new one never records it: a swap gap, not a foreign client (0151).
const SWAP_BEFORE: Duration = Duration::minutes(5);
const SWAP_AFTER: Duration = Duration::minutes(1);
/// Log tail read for swap lines: a swap older than this can only matter to a window this long.
const LOG_TAIL: u64 = 4 << 20;

/// One exported hand this store has no record of.
struct Missing {
    bot: String,
    started_at: String,
    profit: i64,
}

/// Report the unrecorded hands of each window; the bool is the exit verdict (the recent window has
/// any), as the fleet check exited 1 when its window was flagged.
pub fn check(store: &Store, root: &Path, hours: f64) -> Result<(String, bool)> {
    let history = root.join("artifacts").join("history.db");
    if !history.exists() {
        return Ok((format!("no history.db at {}: nothing to compare yet\n", history.display()), false));
    }
    let db = crate::history::HistoryDb::open(&history)?;
    let swaps = swap_times(&root.join("artifacts").join("logs").join("svanbot10.log"));
    let since = (Utc::now() - Duration::seconds((hours * 3600.0) as i64)).to_rfc3339_opts(SecondsFormat::Micros, false);
    let mut windows = vec![(format!("window ({hours} h, since {since})"), since)];
    if let Some(season) = season_since(store) {
        windows.push((format!("season (since {season})"), season));
    }
    let mut out = String::from("hands the server exported that this store never recorded (0151)\n");
    let mut flagged = false;
    for (label, since) in windows {
        let (missing, gaps) = scan(store, &db, &since, &swaps)?;
        out.push_str(&block(&label, &missing, &gaps));
        if label.starts_with("window") && !missing.is_empty() {
            flagged = true;
        }
    }
    Ok((out, flagged))
}

/// The unrecorded hands of one window, split into ordinary ones and those a hot swap explains.
fn scan(store: &Store, db: &crate::history::HistoryDb, since: &str, swaps: &[DateTime<Utc>]) -> Result<(Vec<Missing>, Vec<Missing>)> {
    let rows = db.raw_after(since)?;
    let ids: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
    let recorded = store.recorded_hand_ids(&ids)?;
    let (mut missing, mut gaps) = (Vec::new(), Vec::new());
    for (bot, id, started_at, profit) in rows {
        if recorded.contains(&id) {
            continue;
        }
        let m = Missing { bot, started_at, profit };
        if at_swap(&m.started_at, swaps) {
            gaps.push(m);
        } else {
            missing.push(m);
        }
    }
    Ok((missing, gaps))
}

/// One window's lines: the count and chips per bot, then the hands a swap explains.
fn block(label: &str, missing: &[Missing], gaps: &[Missing]) -> String {
    let sum = |rows: &[Missing]| rows.iter().map(|m| m.profit).sum::<i64>();
    let hands = |n: usize| format!("{n} hand{}", if n == 1 { "" } else { "s" });
    let mut out = format!("{label}: {}, {:+} chips", hands(missing.len()), sum(missing));
    if !missing.is_empty() {
        let mut per_bot: HashMap<&str, (usize, i64)> = HashMap::new();
        for m in missing {
            let e = per_bot.entry(m.bot.as_str()).or_insert((0, 0));
            e.0 += 1;
            e.1 += m.profit;
        }
        let mut rows: Vec<(&str, (usize, i64))> = per_bot.into_iter().collect();
        rows.sort_by_key(|(bot, _)| *bot);
        out.push_str(&format!(": {}", rows.iter().map(|(b, (c, p))| format!("{b} {c} ({p:+})")).collect::<Vec<_>>().join(", ")));
    }
    out.push('\n');
    if !gaps.is_empty() {
        out.push_str(&format!("  at a hot swap, not flagged: {}, {:+} chips\n", hands(gaps.len()), sum(gaps)));
    }
    out
}

/// A hand that started in a swap's window belongs to the swap, not to a foreign client.
fn at_swap(started_at: &str, swaps: &[DateTime<Utc>]) -> bool {
    let Some(t) = parse_utc(started_at) else { return false };
    swaps.iter().any(|s| t >= *s - SWAP_BEFORE && t <= *s + SWAP_AFTER)
}

fn parse_utc(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok().map(|n| n.and_utc())
}

/// The season's start as RFC 3339, when the store has one: the boundary stored hands are scoped by.
fn season_since(store: &Store) -> Option<String> {
    let raw = store.get_kv(crate::SEASON_KEY).ok().flatten()?;
    let season: crate::season::CurrentSeason = serde_json::from_str(&raw).ok()?;
    let ms = (season.started_at * 1000.0) as i64;
    Some(DateTime::from_timestamp_millis(ms)?.to_rfc3339_opts(SecondsFormat::Micros, false))
}

/// UTC times of the fleet's hot-swap exits and starts, from the log's tail.
fn swap_times(path: &Path) -> Vec<DateTime<Utc>> {
    let Ok(log) = read_tail(path, LOG_TAIL) else { return Vec::new() };
    log.lines().filter_map(swap_line).collect()
}

fn swap_line(line: &str) -> Option<DateTime<Utc>> {
    let clean = strip_ansi(line);
    if !clean.contains("hot swap: exiting") && !clean.contains("svanbot10 starting") {
        return None;
    }
    let mut it = clean.split_whitespace();
    let (date, time, level) = (it.next()?, it.next()?, it.next()?);
    if level != "INFO" {
        return None;
    }
    DateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M:%S%.f%:z").ok().map(|t| t.with_timezone(&Utc))
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // CSI escape: ESC [ ... final letter.
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}

/// The last `max` bytes of a file, first (partial) line dropped when the tail starts mid-file.
fn read_tail(path: &Path, max: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let from = len.saturating_sub(max);
    f.seek(SeekFrom::Start(from))?;
    let mut buf = Vec::with_capacity((len - from) as usize);
    f.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    Ok(if from == 0 { text } else { text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sv10_store::store::HandRow;

    fn stamp(d: DateTime<Utc>) -> String {
        d.to_rfc3339_opts(SecondsFormat::Micros, false)
    }

    /// A root with a store, a history db holding the given export rows, and the log tail that
    /// records a hot swap at the moment the caller names.
    fn fixture(tag: &str, rows: &[(&str, &str, DateTime<Utc>, i64)], swap_at: Option<DateTime<Utc>>) -> (std::path::PathBuf, Store) {
        let root = std::env::temp_dir().join(format!("sv10-unrecorded-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("artifacts/logs")).unwrap();
        let store = Store::open(&root.join("artifacts/svanbot10.db")).unwrap();
        let now = Utc::now();
        store.put_kv(crate::SEASON_KEY, &json!({"number": 13, "started_at": now.timestamp() as f64 - 3.0 * 3600.0}).to_string()).unwrap();
        store
            .insert_hand(&HandRow {
                bot: "Foo".into(),
                hand_id: "h-recorded".into(),
                ended_at: stamp(now - Duration::minutes(5)),
                ..Default::default()
            })
            .unwrap();
        let db = crate::history::HistoryDb::open(&root.join("artifacts/history.db")).unwrap();
        let pages: Vec<serde_json::Value> = rows
            .iter()
            .map(|(bot, id, started, profit)| json!({"hand_id": id, "bot": bot, "started_at": stamp(*started), "profit": profit}))
            .collect();
        for p in &pages {
            db.insert_page(p["bot"].as_str().unwrap(), std::slice::from_ref(p)).unwrap();
        }
        if let Some(at) = swap_at {
            let line =
                format!("\u{1b}[2m{}\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m svanbot10 starting: 5 bots\n", at.format("%Y-%m-%d %H:%M:%S%.f%:z"));
            std::fs::write(root.join("artifacts/logs/svanbot10.log"), line).unwrap();
        }
        (root, store)
    }

    /// 0151: a hand in the export that the store never recorded is reported per bot with its chips,
    /// and the recent window is the exit verdict — a hand the swap explains is not.
    #[test]
    fn exported_hands_missing_from_the_store_are_reported() {
        let now = Utc::now();
        let swap = now - Duration::minutes(30);
        let (root, store) = fixture(
            "flagged",
            &[
                ("Foo", "h-recorded", now - Duration::minutes(5), 100),
                ("Foo", "h-missing-1", now - Duration::minutes(10), 100),
                ("Foo", "h-missing-2", now - Duration::minutes(20), -600),
                ("Bar", "h-swap", swap, 20),
            ],
            Some(swap),
        );
        let (text, flagged) = check(&store, &root, 2.0).unwrap();
        assert!(flagged, "{text}");
        assert!(text.contains("window (2 h, since "), "{text}");
        assert!(text.contains("2 hands, -500 chips: Foo 2 (-500)"), "{text}");
        assert!(text.contains("at a hot swap, not flagged: 1 hand, +20 chips"), "{text}");
        // The season window starts before every row, so it sees the same hands as the recent one.
        assert!(text.contains("season (since "), "{text}");
    }

    /// A store that recorded everything exports nothing unrecorded, and the check exits 0.
    #[test]
    fn a_recorded_store_reports_nothing() {
        let now = Utc::now();
        let (root, store) = fixture("recorded", &[("Foo", "h-recorded", now - Duration::minutes(5), 100)], None);
        let (text, flagged) = check(&store, &root, 2.0).unwrap();
        assert!(!flagged, "{text}");
        assert!(text.contains("window (2 h, since "), "{text}");
        assert!(text.contains("0 hands, +0 chips"), "{text}");
    }
}
