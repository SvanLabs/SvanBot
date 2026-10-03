//! `review recent` / `audit-by`: the recent window and the deep re-solve readout (0257).

use anyhow::Result;
use sv10_store::store::Store;

/// The raise-war equity study (0158), printed.
/// What the analyst's deep re-solve says we gave up, by street and by the action we took (0273).
///
/// This is the instrument for "is the residual a real leak?": the calibration residual answers
/// "is the model calibrated?", which is a different question — a decision can be badly mispriced and
/// still have been the best one available. `gap_bb` is the deep search's own opinion of what the
/// live choice cost *given the prices live play used*: the re-solve takes the record's own parameters,
/// self-calibration included, so a price error the correction has absorbed cannot appear here — this
/// grades post-pricing tactics, on the analyst's big spots and no other population (0346,
/// [`audit_population`]).
/// The last `n` hands of one bot: what happened, what it was worth in expectation, and the streak.
///
/// The season aggregate (`review all`) cannot answer "are we losing *now*": at +300 bb/100 with a
/// ±120 interval, a bad afternoon is inside the season's noise, and a good one is too. This is the
/// window the operator actually asks about, and the one the finding loop's thresholds are read
/// against (0273).
pub fn recent(store: &Store, bot: &str, n: usize) -> Result<String> {
    type Hand = (Option<i64>, Option<f64>, bool, String);
    let bb = store.latest_big_blind()?.unwrap_or(crate::live::DEFAULT_BIG_BLIND) as f64;
    let rows = store.bot_ev_results(bot)?;
    // (net, all-in EV net, showdown, ended_at), newest first, then oldest first for the stats.
    let newest: Vec<Hand> = rows.iter().rev().take(n).cloned().collect();
    if newest.is_empty() {
        return Ok(format!("{bot}: no hands stored"));
    }
    let window: Vec<Hand> = newest.iter().rev().cloned().collect();
    let net_of = |r: &Hand| r.0.unwrap_or(0) as f64;
    let ev_of = |r: &Hand| r.1.unwrap_or_else(|| r.0.unwrap_or(0) as f64);
    let stats = |f: &dyn Fn(&Hand) -> f64| {
        let len = window.len() as f64;
        let sum: f64 = window.iter().map(f).sum();
        let sq: f64 = window.iter().map(|r| f(r) * f(r)).sum();
        let mean = sv10_stats::moments::mean(len, sum);
        let half = sv10_stats::moments::half_width(len, sum, sq, 1.96);
        (mean / bb * 100.0, half / bb * 100.0)
    };
    let (net_bb, net_hw) = stats(&net_of);
    let (ev_bb, ev_hw) = stats(&ev_of);
    let luck: f64 = newest.iter().map(|r| net_of(r) - ev_of(r)).sum();
    let losing = newest.first().and_then(|r| r.0).unwrap_or(0) < 0;
    let streak = newest.iter().take_while(|r| (r.0.unwrap_or(0) < 0) == losing).count();
    let chips: i64 = newest.iter().take(streak).map(|r| r.0.unwrap_or(0)).sum();
    Ok(format!(
        "{bot}: the last {} hands{}\n   net {:+.1} bb/100 (95% {:+.0}..{:+.0})   all-in EV {:+.1} bb/100 (95% {:+.0}..{:+.0})\n   luck {:+.0} chips   streak: {streak} {}{} in a row ({chips:+} chips)\n   from {} to {}",
        window.len(),
        if streak > 3 { "  <- read the EV line, not the net line" } else { "" },
        net_bb,
        net_bb - net_hw,
        net_bb + net_hw,
        ev_bb,
        ev_bb - ev_hw,
        ev_bb + ev_hw,
        luck,
        if losing { "loss" } else { "win" },
        if streak == 1 { "" } else { "es" },
        window.first().map(|r| r.3.as_str()).unwrap_or("?"),
        window.last().map(|r| r.3.as_str()).unwrap_or("?"),
    ))
}

/// One (street, action) class of deep re-solves: how many, what the live choices gave up, what the
/// deep search's best candidate was, and — the split that keeps a modal column honest (0346) — how
/// many of the disagreements are another *size* of the action we took rather than another action.
///
/// `Serialize` is what `review audit-by --json` prints (#723), the same class the table row is
/// formatted from.
#[derive(Default, serde::Serialize)]
struct AuditClass {
    /// Verdicts in the class.
    n: usize,
    /// Big blinds the live choice gave up, each verdict floored at zero.
    total: f64,
    /// Every candidate the deep search picked as best, by label (`raise:1605`).
    deep: std::collections::BTreeMap<String, usize>,
    /// Verdicts whose deep best is the action we took with a different amount.
    size_only: usize,
    /// Verdicts whose deep best is a different action.
    other_action: usize,
}

/// What the audit grades, and on which rows (0346). `review::audit` re-runs the record with
/// `deep = Params { samples, deal_chunks, ..rec.params.clone() }` (`bin/analyst.rs`), and `rec.params`
/// carries the self-calibration corrections live play used. The deep search therefore prices every
/// candidate exactly as live play did: its `gap_bb` is post-pricing *tactics* — given these prices,
/// was a better action (or a better size of the same action) available — and a pricing error the
/// correction has absorbed cannot appear in it. The pricing residual is `review margins`, on every
/// settled decision; this instrument sees only the spots the analyst audits (`replay::worth_auditing`).
fn audit_population() -> String {
    let floor = crate::replay::AUDIT_MIN_POT_BB;
    format!(
        "the deep search re-solves with the record's own parameters, self-calibration included, so this grades post-pricing \
         tactics, never the price itself (`review margins` is the pricing residual, over every settled decision); these are the \
         spots the analyst audits: pot >= {floor:.0} bb, a call of at least a quarter of the pot and {:.1} bb, or any all-in",
        floor / 4.0
    )
}

/// `version` keeps only verdicts computed from records of that replay version (`None` = every
/// version the analyst has graded, the whole window).
///
/// The version matters because the record is the model's input list: a record written before replay
/// v3 (2026-09-27) lacks the per-opponent corrections live play used, so its gap compares the live
/// choice with a model that saw less than the live choice did — every finding 0282–0284 came from
/// such rows and is being re-measured on v3 (0316). The header therefore always states the mix, and
/// `version not recorded` rows are excluded by any filter rather than assumed to be old or new.
pub fn audit_by(store: &Store, days: i64, version: Option<u32>, json: bool) -> Result<String> {
    let since = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    let all = store.audit_results_since(&since)?;
    let total = all.len();
    let rows = select_version(all, version);
    // One row per (street, live action), worst total loss first: this is "where do we lose chips",
    // which the margin report cannot answer because it only sees spots we took.
    // The recorded action carries its size ("raise:80"), so group by the family: one row per
    // street and action, not one per bet size.
    let mut rows_out = audit_classes(&rows);
    rows_out.sort_by(|a, b| b.1.total.total_cmp(&a.1.total));
    let (size_only, other): (usize, usize) = rows_out.iter().fold((0, 0), |(s, o), (_, c)| (s + c.size_only, o + c.other_action));
    let all: f64 = rows.iter().map(|(_, r)| r.gap_bb.max(0.0)).sum();
    let worst = worst_by_category(store, &rows);
    if json {
        // #723: the same `AuditResult` verdicts and `AuditClass` rows the table is built from, so an
        // agent reads the numbers rather than the layout.
        let classes: Vec<serde_json::Value> =
            rows_out.iter().map(|((street, action), c)| serde_json::json!({"street": street, "action": action, "class": c})).collect();
        let verdicts: Vec<&sv10_store::store::AuditResult> = rows.iter().map(|(_, r)| r).collect();
        return Ok(serde_json::json!({
            "days": days,
            "version": version,
            "population": audit_population(),
            "total": total,
            "graded": rows.len(),
            "version_mix": version_mix(&rows),
            "mixed_note": mixed_note(&rows, version),
            "classes": classes,
            "size_only": size_only,
            "other_action": other,
            "total_gap_bb": all,
            "verdicts": verdicts,
            "worst_by_category": worst,
        })
        .to_string());
    }
    if rows.is_empty() {
        return Ok(match version {
            Some(v) => format!(
                "no analyst re-solves in the last {days} days were graded from replay v{v} records ({total} of other versions in the window)"
            ),
            None => format!("no analyst re-solves in the last {days} days"),
        });
    }
    let mut out = format!(
        "{} deep re-solves in the last {days} days{}; gap = big blinds the live choice gave up against the deep search — {}\n",
        rows.len(),
        match version {
            Some(v) => format!(", graded from replay v{v} records, of {total} in the window"),
            None => format!(", graded from records: {}", version_mix(&rows)),
        },
        audit_population(),
    );
    out.push_str(&mixed_note(&rows, version));
    out.push_str(&format!(
        "   {:<8} {:<8} {:>6} {:>11} {:>9} {:>9} {:>8}   the deep search's commonest best candidate\n",
        "street", "we took", "n", "given up", "per dec", "new size", "new act"
    ));
    for ((street, action), c) in &rows_out {
        let best = c.deep.iter().max_by_key(|(_, n)| **n).map(|(k, n)| format!("{k} x{n}")).unwrap_or_default();
        out.push_str(&format!(
            "   {street:<8} {action:<8} {:>6} {:>11.1} {:>9.3} {:>9} {:>8}   {best}\n",
            c.n,
            c.total,
            c.total / c.n.max(1) as f64,
            c.size_only,
            c.other_action
        ));
    }
    out.push_str(&format!(
        "   of the {} verdicts whose best candidate differs from the action we took, {size_only} keep the action and change its \
         size and {other} change the action (0346): a modal column read as \"it would have checked\" over-reads the size changes, \
         and `gap_bb` for one is the cost of the size, not of the action\n",
        size_only + other
    ));
    out.push_str(&format!("\n   total {all:.1} bb given up over {} decisions ({:.3} bb each)\n", rows.len(), all / rows.len() as f64));
    out.push_str(&worst.text());
    Ok(out)
}

/// Group the verdicts into (street, live action family) classes, counting what the deep search's
/// best candidate was and splitting the disagreements into a new size of the same action and a new
/// action (0346). Pure, so the split is testable without a store.
fn audit_classes(rows: &[Graded]) -> Vec<((String, String), AuditClass)> {
    fn family(a: &str) -> &str {
        a.split(':').next().unwrap_or(a)
    }
    let mut groups: std::collections::BTreeMap<(String, String), AuditClass> = Default::default();
    for (_, r) in rows {
        let c = groups.entry((r.street.clone(), family(&r.live_action).to_string())).or_default();
        c.n += 1;
        c.total += r.gap_bb.max(0.0);
        *c.deep.entry(r.deep_action.clone()).or_default() += 1;
        if r.deep_action == r.live_action {
            continue;
        }
        if family(&r.deep_action) == family(&r.live_action) {
            c.size_only += 1;
        } else {
            c.other_action += 1;
        }
    }
    groups.into_iter().collect()
}

/// A verdict row with its time, as the store returns it.
type Graded = (String, sv10_store::store::AuditResult);

/// The rows graded from records of one replay version (`None` keeps every row): the only honest way
/// to read a version's loss, since a gap from a record without the live inputs is not comparable.
fn select_version(rows: Vec<Graded>, version: Option<u32>) -> Vec<Graded> {
    match version {
        Some(v) => rows.into_iter().filter(|(_, r)| r.replay_version == Some(v)).collect(),
        None => rows,
    }
}

/// What a window is made of, largest share first: `v3 30, version not recorded 1470`.
fn version_mix(rows: &[Graded]) -> String {
    let mut counts: std::collections::BTreeMap<Option<u32>, usize> = Default::default();
    for (_, r) in rows {
        *counts.entry(r.replay_version).or_default() += 1;
    }
    let mut named: Vec<(Option<u32>, usize)> = counts.into_iter().collect();
    // Largest share first; on a tie the unrecorded rows lead, since those are the ones whose inputs
    // are unknown and a reader should see before any version's count.
    named.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    named
        .iter()
        .map(|(v, n)| match v {
            Some(v) => format!("v{v} {n}"),
            None => format!("version not recorded {n}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A note when the table mixes record versions, so a class read off it is not taken as measured on
/// the records live play made: empty when the rows are all the current version, or one was selected.
fn mixed_note(rows: &[Graded], version: Option<u32>) -> String {
    if version.is_some() {
        return String::new();
    }
    let current = crate::replay::REPLAY_VERSION;
    let other = rows.iter().filter(|(_, r)| r.replay_version != Some(current)).count();
    if other == 0 {
        return String::new();
    }
    format!(
        "   {other} of {} re-solves graded records without the live inputs (pre-v{current}) or without a version; \
         read a class on the current records alone with `review audit-by <days> {current}` before acting on it\n",
        rows.len()
    )
}

/// Categories the attribution table prints: the tail is one or two decisions each and adds rows
/// nobody reads.
const WORST_CATEGORIES: usize = 12;

/// One category's share of the worst disagreements: how many, and the big blinds given up.
#[derive(Default, serde::Serialize)]
struct CategoryLoss {
    category: String,
    n: usize,
    gap_bb: f64,
}

/// The category attribution table: the sample it was read over, how many verdicts had no decision
/// record to attribute, and the rows, biggest loss first.
///
/// `Serialize` is what `review audit-by --json` prints (#723); [`WorstByCategory::text`] prints the
/// same rows, so the two renders cannot drift apart.
#[derive(Default, serde::Serialize)]
struct WorstByCategory {
    /// Verdicts with a positive gap the table was read over (at most 400).
    sample: usize,
    /// Of those, the ones with no decision record to attribute.
    unattributed: usize,
    /// Per category, at most [`WORST_CATEGORIES`], biggest loss first.
    rows: Vec<CategoryLoss>,
}

/// The recorded category of the decisions the deep search disagreed with most: the audit row has no
/// category, so each one is matched to the decision record the bot stored for that hand (the
/// candidate whose action and amount it took). This is the only place the calibration's categories
/// and the analyst's "what it cost" meet, and it is what settles whether a category-level
/// miscalibration is a decision-level loss (0273).
fn worst_by_category(store: &Store, rows: &[Graded]) -> WorstByCategory {
    let mut worst: Vec<&Graded> = rows.iter().filter(|(_, r)| r.gap_bb > 0.0).collect();
    worst.sort_by(|a, b| b.1.gap_bb.total_cmp(&a.1.gap_bb));
    let sample: Vec<&Graded> = worst.iter().take(400).copied().collect();
    let mut by_category: std::collections::BTreeMap<String, (usize, f64)> = Default::default();
    let mut unattributed = 0usize;
    for (_, r) in &sample {
        let Some(cat) = decision_category(store, r) else {
            unattributed += 1;
            continue;
        };
        let c = by_category.entry(cat).or_default();
        c.0 += 1;
        c.1 += r.gap_bb;
    }
    let mut cats: Vec<CategoryLoss> = by_category.into_iter().map(|(category, (n, gap_bb))| CategoryLoss { category, n, gap_bb }).collect();
    cats.sort_by(|a, b| b.gap_bb.total_cmp(&a.gap_bb));
    cats.truncate(WORST_CATEGORIES);
    WorstByCategory { sample: sample.len(), unattributed, rows: cats }
}

impl WorstByCategory {
    /// The table as the text report prints it.
    fn text(&self) -> String {
        if self.rows.is_empty() {
            return format!("   the worst {} disagreements had no recorded decision to attribute them to\n", self.unattributed);
        }
        let mut out = format!(
            "\n   the {} worst disagreements, by the category the bot priced ({} unattributed):\n   {:<24} {:>5} {:>10}\n",
            self.sample, self.unattributed, "category", "n", "given up"
        );
        for c in &self.rows {
            out.push_str(&format!("   {:<24} {:>5} {:>10.1}\n", c.category, c.n, c.gap_bb));
        }
        out
    }
}

/// The category of the candidate a decision took, from the record the bot stored for that hand.
///
/// The audit row records a sized action ("raise:1605") and the decision row keeps the action and
/// the amount apart, so both are matched on the family and the amount; the candidate is then the one
/// with the same pair. Without that, every raise is unattributed — which is most of the decisions
/// that cost anything.
fn decision_category(store: &Store, r: &sv10_store::store::AuditResult) -> Option<String> {
    if r.hand_id.is_empty() {
        return None;
    }
    let family = r.live_action.split(':').next().unwrap_or(&r.live_action);
    let size: Option<i64> = r.live_action.split_once(':').and_then(|(_, n)| n.parse().ok());
    for d in store.decisions_for_hand(&r.bot, &r.hand_id).ok()? {
        if d["street"] != r.street || d["action"] != family {
            continue;
        }
        let amount = d["amount"].as_i64();
        if let (Some(to), Some(a)) = (size, amount)
            && a != to
        {
            continue;
        }
        let candidates = d["detail"].get("candidates")?.as_array()?;
        let exact = candidates.iter().find(|c| c["action"] == family && (c["amount"].as_i64() == amount || amount.is_none()));
        let picked = exact.or_else(|| candidates.iter().find(|c| c["category"].is_string()))?;
        return picked["category"].as_str().map(str::to_string);
    }
    None
}

#[cfg(test)]
mod tests;
