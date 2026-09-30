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

/// Our own preflop mix against the six days before it (0332), re-exported for the same reason.
mod style_drift;
pub use style_drift::{DRIFT_POINTS, DRIFT_Z, style_drift, style_drift_legend};

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
    /// What the decision-cost instrument could not count, in the reader's terms (#321): how much of the
    /// window's re-solves carry no live inputs, and how far the best comparable class is from the
    /// floor. The panel leads with these — thirty class rows that cannot be decided are worth less than
    /// the sentence saying why — while [`unanswered`](Self::unanswered) keeps the rest.
    pub coverage: Vec<String>,
    /// Every class the decision-cost instrument measured, as the panel's table (#321). Empty when the
    /// instrument could not read, where the panel falls back to the findings list.
    pub classes: Vec<ClassRow>,
    /// The explanation a family of findings shares, keyed by an id prefix (`decision`, `calibration`,
    /// `style-drift`). A row no longer carries it (#321), so the panel states it once above the family
    /// and a filed ticket adds it back — see [`legend_for`].
    pub legends: BTreeMap<String, String>,
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
/// `bb` is the store's big blind: pricing these at a constant would disagree with the rivals
/// card beside them on the dashboard, which is read at the store's own big blind (#611).
pub fn nemesis(h2h: &std::collections::HashMap<String, HeadToHead>, bb: f64) -> Vec<Finding> {
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
                    h.mean() / bb * 100.0
                ),
                h.mean() / bb * 100.0,
                0.0,
            )
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
            format!("{n} settled decisions at every pot size"),
            *residual,
            0.0,
        ));
    }
    out
}

/// The explanation the calibration rows share (#321), stated once above them: what the residual is
/// measured against, what reads it, and why it is a measurement and not a loss.
pub fn calibration_legend() -> String {
    "Realized minus the uncorrected price, the residual self-calibration is fitted from — not the \
     price live play corrects against, and not the audit's big spots. `review margins` reads it at the \
     decision margin and `review wiring` counts the choices the correction decides. A measurement, not \
     a loss: only the deep re-solve says a choice cost chips (0269)."
        .to_string()
}

/// Run every instrument over the store and return what the fleet found about itself.
///
/// `h2h` is the live ledger the fleet already keeps; the calibration rows come from the store's
/// summary. Anything that could not be measured is named in `unanswered` rather than silently
/// producing an empty finding list.
pub fn scan(store: &Store, h2h: &std::collections::HashMap<String, HeadToHead>, now: f64) -> Scan {
    let mut unanswered = Vec::new();
    let mut coverage = Vec::new();
    let mut findings = Vec::new();
    let mut sampled = BTreeMap::new();
    let mut unreadable = Vec::new();
    // What the snapshot row records beside the findings (0356): the classes this pass tested and the
    // calibration summary as it was read, both empty when their read failed — which is what happened,
    // and is what the row then says.
    let mut classes = Classes::new();
    let mut rows_out: Vec<ClassRow> = Vec::new();
    let mut calibration_read: Vec<sv10_store::store::CalibrationRow> = Vec::new();
    // The analyst's pot floor, read once: it is what the class table's legend names and what the
    // coverage of every class is measured against.
    let floor = analyst_floor(store);

    // The decision-cost instrument, on the verdicts whose records carry the live inputs (0316).
    //
    // One read, two windows (0345): the whole retention-long window comes back once and the short test
    // window is sliced out of it in memory, so the two stats are the same verdicts counted twice over
    // and can never disagree. `ts` is the store's own RFC 3339 text, compared as SQLite compares it.
    let long_since = (chrono::Utc::now() - chrono::Duration::days(DECISION_LOSS_DAYS)).to_rfc3339();
    let short_since = (chrono::Utc::now() - chrono::Duration::days(GAP_WINDOW_DAYS)).to_rfc3339();
    match store.audit_results_since(&long_since) {
        Ok(rows) if !rows.is_empty() => {
            let (long, long_shapes, excluded) = comparable_classes(rows.iter(), DECISION_LOSS_DAYS);
            let (short, short_shapes, _) =
                comparable_classes(rows.iter().filter(|(ts, _)| ts.as_str() >= short_since.as_str()), GAP_WINDOW_DAYS);
            let tested = tested_classes(&short, &long);
            // The class populations (0355), the denominator every coverage names: read over the same two
            // windows the verdicts were counted over, and `None` — coverage unknown, not zero — if either
            // read fails.
            let counts = |since: &str| store.decision_counts_since(since).ok();
            let populations = counts(&long_since).zip(counts(&short_since)).map(|(l, s)| Populations::new(&l, &s));
            if populations.is_none() {
                coverage.push("the classes' decision counts were unreadable, so coverage is unknown this pass".into());
            }
            // Classes the population holds and the verdicts do not (0351's `preflop:check`) are tested with
            // nothing measured, so they report `never queued` rather than taking no row at all.
            let tested = match &populations {
                Some(p) => p.seeded(tested),
                None => tested,
            };
            let cover = coverages(&tested, populations.as_ref(), &class_pots(rows.iter()), floor);
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
                coverage.push(format!(
                    "{excluded} of {} re-solves in the last {DECISION_LOSS_DAYS} days graded records without the live inputs \
                     (before replay v{LIVE_INPUTS_REPLAY_VERSION}) and are not counted; {comparable} comparable verdicts \
                     against the {GAP_MIN_DECISIONS} a finding needs{largest}",
                    rows.len()
                ));
            }
            // The tested window's counts, so a cleared finding's "measured" is what the scan actually
            // tested and a class tested on the long window cannot read as unmeasured.
            sampled = tested.iter().map(|(key, c)| (key.id(), c.n)).collect();
            let shapes = tested_shapes(&short, &short_shapes, &long_shapes);
            let filed = decision_losses(&tested, &cover, &shapes);
            rows_out = class_rows(&tested, &filed, &cover);
            findings.extend(measurements(&tested, &filed, &cover, &shapes));
            findings.extend(filed);
            classes = tested;
        }
        Ok(_) => coverage.push(format!("no analyst re-solves in the last {DECISION_LOSS_DAYS} days")),
        Err(e) => {
            unanswered.push(format!("decision loss: the analyst's re-solves were unreadable ({e}); last findings kept"));
            // Both families the instrument produces: a failed read leaves its measurements unknown too.
            unreadable.push("decision-loss:".to_string());
            unreadable.push("decision-measurement:".to_string());
        }
    }

    // The nemesis test.
    let bb = store.latest_big_blind().ok().flatten().unwrap_or(crate::live::DEFAULT_BIG_BLIND).max(1) as f64;
    findings.extend(nemesis(h2h, bb));

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
    // The explanations the rows no longer repeat (#321), one per family. They are static descriptions
    // of the instruments, so they are stated even on a pass whose read failed: the findings that carry
    // over from the last pass are the ones that still need explaining.
    let legends = BTreeMap::from([
        ("decision".to_string(), decision_legend(floor)),
        ("calibration".to_string(), calibration_legend()),
        ("style-drift".to_string(), style_drift_legend()),
    ]);
    Scan { at: now, findings, unanswered, coverage, classes: rows_out, legends, sampled, unreadable, snapshot }
}

/// The shared explanation behind a finding (#321), from a scan's own legend map: the longest key that
/// prefixes its id, so `decision-loss:turn:call` and `decision-measurement:turn:call` read the same
/// legend without the scan having to name either family twice.
pub fn legend_for<'a>(legends: &'a BTreeMap<String, String>, id: &str) -> Option<&'a str> {
    legends
        .iter()
        .filter(|(prefix, _)| id.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, text)| text.as_str())
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
    // The table and the coverage notes are what this pass measured, not a set that ages like the
    // findings: a failed read leaves them empty and the panel falls back to the carrying-over findings
    // list, rather than printing a table from before the failure as if it were this pass's.
    json!({"at": scan.at, "findings": merged, "cleared": cleared, "unanswered": scan.unanswered,
           "coverage": scan.coverage, "classes": scan.classes, "legends": scan.legends})
}

/// The ticket path (0273), re-exported for the same reason the instruments above are.
mod tickets;
pub use tickets::*;

#[cfg(test)]
mod tests;
