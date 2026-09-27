//! Results monitor and machine view for the dashboard: the newest alerts and summaries written by
//! `scripts/monitor.py`, pressure stall information, replay records and the last season check.

use super::*;

/// Tail of a log file (at most `max` bytes), as lines.
pub(super) fn tail_lines(path: &std::path::Path, max: u64) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return Vec::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(max)));
    let mut buf = String::new();
    let _ = f.read_to_string(&mut buf);
    let mut lines: Vec<String> = buf.lines().map(str::to_string).collect();
    if len > max && !lines.is_empty() {
        lines.remove(0); // partial first line
    }
    lines
}

/// `avg60` of the `some` (cpu) or `full` (io, memory) line of a pressure file, in percent.
fn pressure(kind: &str) -> Option<f64> {
    let text = std::fs::read_to_string(format!("/proc/pressure/{kind}")).ok()?;
    let want = if kind == "cpu" { "some" } else { "full" };
    let line = text.lines().find(|l| l.starts_with(want))?;
    line.split_whitespace().find_map(|f| f.strip_prefix("avg60=")).and_then(|v| v.parse().ok())
}

pub(super) async fn monitor(State(s): State<Arc<Shared>>) -> Json<Value> {
    let logs = s.config.artifacts.join("logs");
    let monitor_log = logs.join("monitor.log");
    let lines = tail_lines(&monitor_log, 256 * 1024);
    let kind_of = |l: &str| l.split_whitespace().nth(1).unwrap_or("").to_string();
    let entry = |l: &String| {
        let mut parts = l.splitn(3, ' ');
        let time = parts.next().unwrap_or("").to_string();
        let kind = parts.next().unwrap_or("").to_string();
        json!({"time": time, "kind": kind, "text": parts.next().unwrap_or("")})
    };
    let alerts: Vec<Value> = lines
        .iter()
        .filter(|l| matches!(kind_of(l).as_str(), "BIGWIN" | "BIGLOSS" | "NEMESIS" | "STALL" | "ERROR" | "MONITOR"))
        .rev()
        .take(40)
        .map(entry)
        .collect();
    let latest = |kind: &str| lines.iter().rev().find(|l| kind_of(l) == kind).map(entry);
    // The monitor writes a summary every 30 minutes, so a log untouched for 40 means it stopped.
    let age = std::fs::metadata(&monitor_log).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs());
    let running = age.is_some_and(|a| a < 40 * 60);
    // Reported, not defaulted (0326): a failed read published `"recorded": 0` beside a live season
    // check, so an unreadable store looked like a fleet that had recorded nothing.
    let (replays, newest_replay, replays_error) = match s.store.replay_stats() {
        Ok((n, newest)) => (Some(n), newest, None),
        Err(e) => {
            snapshot_warn(&format!("store unreadable (replay stats), dashboard figure left empty: {e}"));
            (None, None, Some(format!("store unreadable (replay stats): {e}")))
        }
    };
    let season_lines = tail_lines(&logs.join("season-check.log"), 64 * 1024);
    let season = season_lines.iter().rev().find(|l| l.starts_with("season check ")).map(|l| {
        let fails: Vec<&String> = season_lines.iter().filter(|x| x.starts_with("FAIL")).collect();
        json!({"result": l, "failures": fails})
    });
    Json(json!({
        "monitor": {"running": running, "log_age_seconds": age, "started": latest("START"), "summary": latest("SUMMARY"), "opponents": latest("OPPONENTS"), "alerts": alerts},
        "pressure": {"cpu": pressure("cpu"), "io": pressure("io"), "memory": pressure("memory")},
        "replays": {"recorded": replays, "newest": newest_replay, "keep_days": crate::replay::KEEP_DAYS, "error": replays_error},
        "season_check": season,
    }))
}
