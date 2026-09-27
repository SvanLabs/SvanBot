//! The automatic read of the fleet's own play (0273).
//!
//! The project has instruments for every question an operator would ask of the fleet's decisions —
//! `review audit-by` for what the deep re-solve says each choice cost, `review margins` for
//! whether the model is priced right, the tripwires, the nemesis test — and a session had to
//! remember to run them. A finding that only exists until the next session is a finding that costs
//! chips until someone happens to look.
//!
//! So the fleet reads itself. [`scan`] turns the instruments into a list of [`Finding`]s, each with
//! the evidence that produced it and a stable signature, and the caller stores it, shows it, and
//! (under [`write_tickets`]) files a ticket for anything new. A finding that stops reproducing
//! closes its own ticket, so the board cannot fill with stale work.
//!
//! **The rule that keeps this honest**, learned from 0269: a finding must name the *decision* cost
//! or it is a measurement, not a leak. Calibration residuals are reported as measurements and are
//! never a ticket on their own.

use crate::headtohead::HeadToHead;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use sv10_store::store::Store;

/// The decision-loss instrument (0345), split out by responsibility; its items are re-exported here so
/// `sv10_bot::findings::…` is the path a caller (and the store's docs) still uses.
mod decision_loss;
pub use decision_loss::*;

/// The pass's own inputs and outputs, persisted as one deduped row (0356); its items are re-exported
/// for the same reason the decision-loss ones are.
mod snapshot;
pub use snapshot::*;

/// Store key holding the current findings.
pub const FINDINGS_KEY: &str = "findings.v1";

/// One thing the fleet found out about its own play.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Finding {
    /// Stable signature: the same finding must produce the same id on every scan, so a repeat is
    /// recognised as a repeat and does not file a second ticket.
    pub id: String,
    /// What to call it.
    pub title: String,
    /// How much it matters: `P0` for a measured decision loss, `P1` for a risk, `P2` for a
    /// measurement worth watching.
    pub severity: String,
    /// What was measured, with its sample behind it.
    pub evidence: String,
    /// The number a reader should judge it by (bb per decision, chips, or a rate).
    pub value: f64,
    /// When the fleet first saw it (Unix seconds).
    pub since: f64,
    /// The last scan that still saw it.
    pub updated: f64,
    /// The ticket it filed, when it filed one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
}

impl Finding {
    /// A finding as it is first seen.
    pub fn new(id: &str, severity: &str, title: &str, evidence: String, value: f64, now: f64) -> Finding {
        Finding {
            id: id.to_string(),
            title: title.to_string(),
            severity: severity.to_string(),
            evidence,
            value,
            since: now,
            updated: now,
            ticket: None,
        }
    }
}

/// What one pass of the instruments saw.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Scan {
    /// Unix seconds.
    pub at: f64,
    /// Everything found, most severe first.
    pub findings: Vec<Finding>,
    /// The questions the scan could not answer (no samples yet), so an empty scan is not silent.
    pub unanswered: Vec<String>,
    /// Comparable decisions behind each decision-loss class this scan measured, by finding id, so a
    /// finding that clears can say whether it was measured under the floor or no longer measured. The
    /// count is from the window the class was *tested* on (0345), so a class that was measured on the
    /// long window is never reported as unmeasured.
    #[serde(skip)]
    pub sampled: BTreeMap<String, i64>,
    /// Id prefixes (`decision-loss:`, `decision-measurement:`, `calibration:`) of instruments whose
    /// store read failed: their findings are unknown this pass, not gone, so [`merge`] keeps them
    /// instead of clearing them.
    #[serde(skip)]
    pub unreadable: Vec<String>,
    /// What this pass read and emitted, as the payload the snapshot row holds ([`ScanSnapshot`],
    /// 0356). Built from the same values as the fields above and deliberately without the wall clock,
    /// so two passes over an unchanged store produce the same bytes and the store writes a row only
    /// when the scan's view of the world changed. Not part of the stored finding set, like `sampled`.
    #[serde(skip)]
    pub snapshot: String,
}

/// The nemesis findings: an opponent beating us at 95% family-wise across every opponent tested.
///
/// The population is champion play, and the row says so: the ledger's read leaves the experiment
/// arms' treatment hands out ([`crate::headtohead`], 0361), because the number moves the fleet
/// between tables and a challenger's hands are not the policy that plays there.
///
/// One pass per scan, from the live ledger (0328). The two risk instruments used to share one
/// function, so the scan called it twice and every nemesis was reported — and ticketed — twice.
pub fn nemesis(h2h: &std::collections::HashMap<String, HeadToHead>) -> Vec<Finding> {
    let tested = crate::headtohead::tested(h2h, 150.0);
    h2h.iter()
        .filter(|(_, h)| h.hands >= 150.0 && h.beats_us(150.0, tested))
        .map(|(name, h)| {
            Finding::new(
                &format!("nemesis:{name}"),
                "P1",
                &format!("{name} beats us"),
                format!(
                    "{:.0} shared hands of champion play (experiment-arm hands excluded, 0291), {:+.1} bb/100, 95% family-wise across {tested} opponents",
                    h.hands,
                    h.mean() / 20.0 * 100.0
                ),
                h.mean() / 20.0 * 100.0,
                0.0,
            )
        })
        .collect()
}

/// Smallest change in a first-preflop-action share, in percentage points, that is a style drift.
pub const DRIFT_POINTS: f64 = 5.0;
/// Standard errors a drift must clear: both windows hold thousands of hands, so only a real shift passes.
pub const DRIFT_Z: f64 = 5.0;

/// The style-drift findings (0332): the share of hands whose first preflop decision was each action,
/// the last day against the days before. On 2026-09-26 a self-calibration rule change took first-in
/// raising from 40% of hands to 3% in an hour, and nothing noticed: a shift that size comes from a
/// rule or parameter, not the cards. `P1`: a risk to look at, never a ticket on its own.
pub fn style_drift(recent: &[(String, i64)], baseline: &[(String, i64)]) -> Vec<Finding> {
    let total = |w: &[(String, i64)]| w.iter().map(|r| r.1).sum::<i64>().max(0) as f64;
    let (nr, nb) = (total(recent), total(baseline));
    if nr < 1.0 || nb < 1.0 {
        return Vec::new();
    }
    let share = |w: &[(String, i64)], a: &str| w.iter().filter(|r| r.0 == a).map(|r| r.1).sum::<i64>() as f64;
    let mut actions: Vec<&str> = recent.iter().chain(baseline).map(|r| r.0.as_str()).collect();
    actions.sort_unstable();
    actions.dedup();
    actions
        .into_iter()
        .filter_map(|a| {
            let (pr, pb) = (share(recent, a) / nr, share(baseline, a) / nb);
            let pooled = (share(recent, a) + share(baseline, a)) / (nr + nb);
            let se = (pooled * (1.0 - pooled) * (1.0 / nr + 1.0 / nb)).sqrt();
            let points = (pr - pb) * 100.0;
            (points.abs() >= DRIFT_POINTS && se > 0.0 && ((pr - pb) / se).abs() >= DRIFT_Z).then(|| {
                Finding::new(
                    &format!("style-drift:preflop:{a}"),
                    "P1",
                    &format!("first preflop action {a}: {:.1}% of hands in the last day, {:.1}% the six days before", pr * 100.0, pb * 100.0),
                    format!(
                        "{nr:.0} hands against {nb:.0}: a shift this size comes from a rule, parameter or calibration change, not the cards — check the releases, the champion and `review margins` (0332)"
                    ),
                    points,
                    0.0,
                )
            })
        })
        .collect()
}

/// The calibration findings: a category's realized result against its *uncorrected* price, over
/// every settled decision at every pot size (the pricing population, 0346). The fitted quantity:
/// live play records each sample as `candidate.ev - candidate.bias`, so the residual is what
/// self-calibration is fitted from — not the price live play corrects against, and not the audit's
/// big spots. `review margins` reads the corrected residual at the decision margin and
/// `review wiring` counts the choices the correction decides. A measurement, so `P2`, and never a
/// ticket on its own (0269). Separate from [`nemesis`] so an unreadable calibration summary cannot
/// take the nemesis findings with it.
pub fn mispriced(calibration_off: &[(String, i64, f64)]) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    for (category, n, residual) in calibration_off {
        out.push(Finding::new(
            &format!("calibration:{category}"),
            "P2",
            &format!("{category} realizes {residual:+.1} bb over its uncorrected price"),
            format!("{n} settled decisions at every pot size — realized minus the uncorrected price, the residual self-calibration is fitted from, not the price live play corrects against; `review margins` reads it at the decision margin and `review wiring` counts the choices the correction decides. A measurement, not a loss: only the deep re-solve says a choice cost chips"),
            *residual,
            0.0,
        ));
    }
    out
}

/// Run every instrument over the store and return what the fleet found about itself.
///
/// `h2h` is the live ledger the fleet already keeps; the calibration rows come from the store's
/// summary. Anything that could not be measured is named in `unanswered` rather than silently
/// producing an empty finding list.
pub fn scan(store: &Store, h2h: &std::collections::HashMap<String, HeadToHead>, now: f64) -> Scan {
    let mut unanswered = Vec::new();
    let mut findings = Vec::new();
    let mut sampled = BTreeMap::new();
    let mut unreadable = Vec::new();
    // What the snapshot row records beside the findings (0356): the classes this pass tested and the
    // calibration summary as it was read, both empty when their read failed — which is what happened,
    // and is what the row then says.
    let mut classes = Classes::new();
    let mut calibration_read: Vec<sv10_store::store::CalibrationRow> = Vec::new();

    // The decision-cost instrument, on the verdicts whose records carry the live inputs (0316).
    //
    // One read, two windows (0345): the whole retention-long window comes back once and the short test
    // window is sliced out of it in memory, so the two stats are the same verdicts counted twice over
    // and can never disagree. `ts` is the store's own RFC 3339 text, compared as SQLite compares it.
    let long_since = (chrono::Utc::now() - chrono::Duration::days(DECISION_LOSS_DAYS)).to_rfc3339();
    let short_since = (chrono::Utc::now() - chrono::Duration::days(GAP_WINDOW_DAYS)).to_rfc3339();
    match store.audit_results_since(&long_since) {
        Ok(rows) if !rows.is_empty() => {
            let (long, excluded) = comparable_classes(rows.iter(), DECISION_LOSS_DAYS);
            let (short, _) = comparable_classes(rows.iter().filter(|(ts, _)| ts.as_str() >= short_since.as_str()), GAP_WINDOW_DAYS);
            let tested = tested_classes(&short, &long);
            // The class populations (0355), the denominator every coverage names: read over the same two
            // windows the verdicts were counted over, and `None` — coverage unknown, not zero — if either
            // read fails.
            let counts = |since: &str| store.decision_counts_since(since).ok();
            let populations = counts(&long_since).zip(counts(&short_since)).map(|(l, s)| Populations::new(&l, &s));
            if populations.is_none() {
                unanswered.push("decision loss: the classes' decision counts were unreadable, so coverage is unknown this pass".into());
            }
            // Classes the population holds and the verdicts do not (0351's `preflop:check`) are tested with
            // nothing measured, so they report `never queued` rather than taking no row at all.
            let tested = match &populations {
                Some(p) => p.seeded(tested),
                None => tested,
            };
            let cover = coverages(&tested, populations.as_ref(), &class_pots(rows.iter()), analyst_floor(store));
            if excluded > 0 {
                // Name the evidence that *is* comparable, and how far the best of it is from the floor
                // (0344): the excluded count alone cannot tell a reader whether the instrument is about
                // to speak or is a hundred decisions short of speaking, and this note expires (0342).
                // The counts are the window this read covers (0345): the comparable verdicts are those
                // rows, not the class tallies, which count the pooled all-in family's verdicts twice.
                let comparable = (rows.len() - excluded) as i64;
                let largest = match long.iter().filter(|(k, _)| matches!(k, ClassKey::Spot { .. })).max_by_key(|(_, c)| c.n) {
                    Some((ClassKey::Spot { street, action }, c)) => format!(" (largest {street}:{action}, {})", c.n),
                    _ => String::new(),
                };
                unanswered.push(format!(
                    "decision loss: {excluded} of {} re-solves in the last {DECISION_LOSS_DAYS} days graded records without the live inputs \
                     (before replay v{LIVE_INPUTS_REPLAY_VERSION}) and are not counted; {comparable} comparable verdicts \
                     against the {GAP_MIN_DECISIONS} a finding needs{largest}",
                    rows.len()
                ));
            }
            // The tested window's counts, so a cleared finding's "measured" is what the scan actually
            // tested and a class tested on the long window cannot read as unmeasured.
            sampled = tested.iter().map(|(key, c)| (key.id(), c.n)).collect();
            let filed = decision_losses(&tested, &cover);
            findings.extend(measurements(&tested, &filed, &cover));
            findings.extend(filed);
            classes = tested;
        }
        Ok(_) => unanswered.push(format!("decision loss: no analyst re-solves in the last {DECISION_LOSS_DAYS} days")),
        Err(e) => {
            unanswered.push(format!("decision loss: the analyst's re-solves were unreadable ({e}); last findings kept"));
            // Both families the instrument produces: a failed read leaves its measurements unknown too.
            unreadable.push("decision-loss:".to_string());
            unreadable.push("decision-measurement:".to_string());
        }
    }

    // The nemesis test.
    findings.extend(nemesis(h2h));

    // Style drift (0332): our own preflop mix, the last day against the six before.
    let at = |hours: i64| (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
    match (store.first_preflop_actions(&at(24), &at(0)), store.first_preflop_actions(&at(24 * 7), &at(24))) {
        (Ok(recent), Ok(baseline)) => findings.extend(style_drift(&recent, &baseline)),
        _ => unanswered.push("style drift: our first preflop actions were unreadable".into()),
    }

    // Calibration: measurements, never findings on their own (0269).
    match store.calibration_summary() {
        Ok(rows) => {
            let off: Vec<(String, i64, f64)> = rows
                .iter()
                .filter(|(_, n, predicted, realized, var)| {
                    let se = (var.max(0.0) / (*n).max(1) as f64).sqrt();
                    let resid = realized - predicted;
                    *n >= 200 && resid.abs() > 1.96 * se && resid.abs() > 2.0
                })
                .map(|(cat, n, predicted, realized, _)| (cat.clone(), *n, realized - predicted))
                .collect();
            findings.extend(mispriced(&off));
            // The summary as it was read, for the snapshot row (0356): it is an unbounded live
            // aggregate over every stored sample, so a past finding's read of it is only
            // reconstructible from `calibration_samples_since` if the row says what it saw.
            calibration_read = rows;
        }
        Err(e) => {
            unanswered.push(format!("calibration: the summary was unreadable ({e}); last findings kept"));
            unreadable.push("calibration:".to_string());
        }
    }

    findings.sort_by(|a, b| a.severity.cmp(&b.severity).then(b.value.total_cmp(&a.value)));
    for f in &mut findings {
        f.since = now;
        f.updated = now;
    }
    // What this pass read and emitted, as one payload (0356), assembled in the snapshot module: the
    // correction table it records is overwritten by the next learner cycle, so without it a finding
    // cannot be re-derived from its own ticket.
    let snapshot = scan_payload(store, &calibration_read, &classes, &findings, &mut unanswered);
    Scan { at: now, findings, unanswered, sampled, unreadable, snapshot }
}

/// Merge this scan into the stored set: a finding already known keeps its `since` and its ticket, a
/// new one is `P0` and gets a ticket, and one that has cleared is dropped — which is what closes
/// its ticket.
pub fn merge(previous: &Value, scan: &Scan) -> Value {
    let old: Vec<Finding> = serde_json::from_value(previous.get("findings").cloned().unwrap_or(json!([]))).unwrap_or_default();
    // One entry per signature, whoever produced it: a repeated id would be served twice to the panel
    // and — worse — filed as two tickets, only the first of which the close path could ever find
    // (0328). The instruments now report each id once; this is what keeps that a property of the
    // stored set rather than of each caller.
    let mut merged: Vec<Finding> = Vec::with_capacity(scan.findings.len());
    for f in &scan.findings {
        if merged.iter().any(|m| m.id == f.id) {
            continue;
        }
        merged.push(match old.iter().find(|o| o.id == f.id) {
            Some(o) => Finding { since: o.since, ticket: o.ticket.clone(), ..f.clone() },
            None => f.clone(),
        });
    }
    // An instrument that could not read this pass saw nothing either way: its findings carry over as
    // they were, ticket and all, rather than clearing (which would close their tickets).
    for o in &old {
        if scan.unreadable.iter().any(|p| o.id.starts_with(p.as_str())) && !merged.iter().any(|m| m.id == o.id) {
            merged.push(o.clone());
        }
    }
    let cleared: Vec<Value> = old
        .iter()
        .filter(|o| !merged.iter().any(|m| m.id == o.id))
        .map(|o| json!({"id": o.id, "title": o.title, "ticket": o.ticket, "measured": scan.sampled.get(&o.id)}))
        .collect();
    json!({"at": scan.at, "findings": merged, "cleared": cleared, "unanswered": scan.unanswered})
}

/// The findings the fleet should file tickets for: the new `P0` ones, at most `per_scan` of them.
pub fn to_file(current: &Value, per_scan: usize) -> Vec<Finding> {
    let findings: Vec<Finding> = serde_json::from_value(current.get("findings").cloned().unwrap_or(json!([]))).unwrap_or_default();
    findings.into_iter().filter(|f| f.severity == "P0" && f.ticket.is_none()).take(per_scan).collect()
}

/// The ticket a finding files, in the tracker's own format.
pub fn ticket_text(finding: &Finding, now: f64) -> String {
    let date = chrono::DateTime::from_timestamp(now as i64, 0).map(|t| t.format("%Y-%m-%d").to_string()).unwrap_or_default();
    format!(
        "---\ntype: wayfinder:task\nstatus: open\npriority: {sev}\nassignee: fleet\nlabels: [found-by-fleet, decisions]\ncreated: {date}\nblocks: []\nblocked_by: []\n---\n\n## Question\n\n{title}\n\nFound by the fleet's own instruments on {date} (0273), no session involved: {evidence}.\n\nThe finding's signature is `{id}`, so a repeat is recognised as a repeat.\n\nThe threshold is {GAP_BB_PER_DECISION} bb per decision over at least {GAP_MIN_DECISIONS} decisions —\nthe floor at which a per-decision loss is larger than the instrument's own noise. The class's 95% lower\nbound has to clear it (0345), over the last {GAP_WINDOW_DAYS} days or, for a class that cannot reach the\nfloor there, over the last {DECISION_LOSS_DAYS} days. This finding was filed\nautomatically; it closes itself when the class stops reproducing.\n",
        sev = finding.severity,
        date = date,
        title = finding.title,
        id = finding.id,
        evidence = finding.evidence
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
    let store: fn(&Finding) -> String = |f| f.ticket.clone().unwrap_or_default();
    for finding in to_file(current, MAX_NEW_TICKETS_PER_SCAN) {
        let id = next_ticket_id(&dir);
        let path = dir.join(format!("{id}-{}.md", slug(&finding.id)));
        // Never overwrite: a ticket is a person's (or the fleet's) record.
        if std::fs::write(&path, ticket_text(&finding, now)).is_ok() {
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
    let _ = store;
    (filed, closed)
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

#[cfg(test)]
mod tests;
