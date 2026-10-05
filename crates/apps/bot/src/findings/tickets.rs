//! The ticket half of the finding loop (0273), split out of `findings.rs` (the 500-line rule).
//!
//! The fleet writing its own board is the point: a leak that only exists until the next session is a
//! leak that costs chips until someone happens to look. Two guards keep it honest — a file that already
//! exists is never overwritten, and only `P0` findings file. A finding that stops reproducing closes the
//! ticket it filed, so the board cannot fill with work that no longer exists.

use super::{DECISION_LOSS_DAYS, Finding, GAP_BB_PER_DECISION, GAP_MIN_DECISIONS, GAP_WINDOW_DAYS, LIVE_INPUTS_REPLAY_VERSION, legend_for};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The findings the fleet should file tickets for: the new `P0` ones, at most `per_scan` of them.
pub fn to_file(current: &Value, per_scan: usize) -> Vec<Finding> {
    let findings: Vec<Finding> = serde_json::from_value(current.get("findings").cloned().unwrap_or(json!([]))).unwrap_or_default();
    findings.into_iter().filter(|f| f.severity == "P0" && f.ticket.is_none()).take(per_scan).collect()
}

/// The ticket a finding files, in the tracker's own format. `legend` is the explanation the panel
/// states once above the family ([`legend_for`], #321): a ticket has no panel above it, so it carries
/// it, and the row's own evidence stays the numbers it measured.
pub fn ticket_text(finding: &Finding, legend: Option<&str>, now: f64) -> String {
    let date = chrono::DateTime::from_timestamp(now as i64, 0).map(|t| t.format("%Y-%m-%d").to_string()).unwrap_or_default();
    format!(
        "---\ntype: wayfinder:task\nstatus: open\npriority: {sev}\nassignee: fleet\nlabels: [found-by-fleet, decisions]\ncreated: {date}\nblocks: []\nblocked_by: []\n---\n\n## Question\n\n{title}\n\nFound by the fleet's own instruments on {date} (0273), no session involved: {evidence}.\n\n{legend}\n\nThe finding's signature is `{id}`, so a repeat is recognised as a repeat.\n\nThe threshold is {GAP_BB_PER_DECISION} bb per decision over at least {GAP_MIN_DECISIONS} decisions —\nthe floor at which a per-decision loss is larger than the instrument's own noise. The class's 95% lower\nbound has to clear it (0345), over the last {GAP_WINDOW_DAYS} days or, for a class that cannot reach the\nfloor there, over the last {DECISION_LOSS_DAYS} days. This finding was filed\nautomatically; it closes itself when the class stops reproducing.\n",
        sev = finding.severity,
        date = date,
        title = finding.title,
        id = finding.id,
        evidence = finding.evidence,
        legend = legend.unwrap_or("")
    )
}

/// Why a cleared finding cleared, when it is not "measured and under the floor": `measured` is the
/// comparable decisions its class had in the scan that cleared it, over the window that scan tested it
/// on (0345: the short one when that holds [`GAP_MIN_DECISIONS`], else the retention-long one — so
/// "under the floor" means under it in the window the class would have been filed on). Under
/// [`GAP_MIN_DECISIONS`] the class was not measured at all, and saying it "stopped reproducing" would
/// be a claim nobody tested (0316: the evidence behind 0282–0284 was graded on records without the
/// live inputs).
pub fn cleared_reason(measured: Option<i64>) -> Option<String> {
    let n = measured.unwrap_or(0);
    (n < GAP_MIN_DECISIONS).then(|| {
        format!(
            "the class now has {n} decisions graded on records that carry the live inputs (replay v{LIVE_INPUTS_REPLAY_VERSION}+, 0316), \
             under the {GAP_MIN_DECISIONS} a verdict needs: the evidence this finding rested on is no longer counted, or no longer \
             in the window. It was not measured to have stopped; the scan files it again if it reproduces on comparable records"
        )
    })
}

/// The note a cleared finding leaves on its ticket; `reason` is [`cleared_reason`] when the class was
/// not measured, `None` for a class measured under the floor.
pub fn cleared_text(finding: &Finding, reason: Option<&str>, now: f64) -> String {
    let date = chrono::DateTime::from_timestamp(now as i64, 0).map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
    let (id, title) = (finding.id.clone(), finding.title.clone());
    match reason {
        Some(why) => format!(
            "\n## Resolution ({date})\n\nThe fleet's own scan no longer measures `{id}` ({title}): {why}. Closed by the finding loop\n(0273), not by a session.\n"
        ),
        None => format!(
            "\n## Resolution ({date})\n\nThe fleet's own scan stopped reproducing `{id}` ({title}): the class is no longer above\n{GAP_BB_PER_DECISION} bb per decision over {GAP_MIN_DECISIONS} decisions. Closed by the finding loop (0273),\nnot by a session.\n"
        ),
    }
}

/// File (or close) the tickets a scan implies, in the wayfinder ticket directory under `root`.
///
/// The fleet writing its own board is the point: a leak that only exists until the next session is a
/// leak that costs chips until someone happens to look. Two guards keep it honest — a file that
/// already exists is never overwritten, and only `P0` findings file.
pub fn write_tickets(root: &std::path::Path, current: &Value, now: f64) -> (Vec<String>, Vec<String>) {
    let dir = root.join(".scratch").join("svanbot10").join("issues");
    if std::fs::create_dir_all(&dir).is_err() {
        return (vec![], vec![]);
    }
    let mut filed = Vec::new();
    let mut closed = Vec::new();
    // The explanations this scan stated once (#321), so a filed ticket carries the one its finding
    // belongs to and reads on its own without the panel above it.
    let legends: BTreeMap<String, String> =
        current.get("legends").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    // The names already on the board. The findings row is saved after the tickets are written, so
    // a save that failed left no record of them and the next scan filed the same findings again.
    let on_board: Vec<String> =
        std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    for finding in to_file(current, MAX_NEW_TICKETS_PER_SCAN) {
        if on_board.iter().any(|name| name.ends_with(&format!("-{}.md", slug(&finding.id))) && open_ticket(&dir.join(name))) {
            continue;
        }
        let id = next_ticket_id(&dir);
        let path = dir.join(format!("{id}-{}.md", slug(&finding.id)));
        // Never overwrite: a ticket is a person's (or the fleet's) record.
        if std::fs::write(&path, ticket_text(&finding, legend_for(&legends, &finding.id), now)).is_ok() {
            filed.push(format!("{id}-{}", slug(&finding.id)));
        }
    }
    // A finding that cleared closes the ticket it filed, with the evidence for closing it.
    for cleared in current.get("cleared").and_then(|c| c.as_array()).into_iter().flatten() {
        let (Some(name), Some(id)) = (cleared["ticket"].as_str(), cleared["id"].as_str()) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let path = dir.join(format!("{name}.md"));
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if text.contains("status: resolved") {
            continue;
        }
        let finding = Finding::new(id, "P0", cleared["title"].as_str().unwrap_or(id), String::new(), 0.0, now);
        let updated = text
            .replace("status: open", "status: resolved")
            .replace("assignee: fleet", "assignee: fleet\nresolved_by: the finding loop (0273)")
            + &cleared_text(&finding, cleared_reason(cleared["measured"].as_i64()).as_deref(), now);
        if std::fs::write(&path, updated).is_ok() {
            closed.push(name.to_string());
        }
    }
    (filed, closed)
}

/// Whether the ticket at `path` is still open: a resolved one does not stand in for a finding that
/// has come back.
fn open_ticket(path: &std::path::Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| !text.contains("status: resolved"))
}

/// At most this many tickets one scan may file, so a noisy instrument cannot flood the board.
pub const MAX_NEW_TICKETS_PER_SCAN: usize = 3;

/// The next free ticket number in the directory.
pub fn next_ticket_id(dir: &std::path::Path) -> String {
    let highest = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter_map(|n| n.get(..4).and_then(|p| p.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("{:04}", highest + 1)
}

/// A file name for a finding id: `decision-loss:turn:raise` becomes `decision-loss-turn-raise`.
pub fn slug(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect()
}
