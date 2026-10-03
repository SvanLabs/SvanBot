//! Self-calibration application from stored residuals (0256).

use crate::live::Shared;
use sv10_store::store;

/// Pure step of self-calibration: correct only the part of each category's mean residual that
/// survives a 95% haircut, shrink that by sample size, and cap it asymmetrically (small upward,
/// larger downward), so a correction appears only where the evidence is consistent and only a
/// correction that makes an action less attractive may grow past a few big blinds.
/// Returns the applied bb corrections per category, each penalty's per-pot bound (the pot-relative
/// residual plus 1.96 se, 0207: a bb penalty fitted on big pots must not tax small ones), and the
/// dashboard table. Pot-relative residuals are not applied as corrections themselves (0079).
/// An upward correction is also bounded by the residual supported at the decision margin
/// (`margin_rows`: samples predicted within [`sv10_stats::margins::MARGIN`], 0222): the category mean is
/// dominated by big-EV spots whose action a few big blinds cannot change, and a flat +3 bb fitted
/// there over-credited marginal big flop bets by 2.5 bb (live 2026-09-26, `review margins`).
pub(super) fn calibration_update(
    rows: Vec<store::CalibrationRow>,
    pot_rows: Vec<(String, i64, f64, f64)>,
    margin_rows: Vec<(String, i64, f64, f64)>,
) -> (std::collections::HashMap<String, f64>, std::collections::HashMap<String, f64>, serde_json::Map<String, serde_json::Value>) {
    const MIN_SAMPLES: i64 = 60;
    const SHRINK: f64 = 250.0;
    /// Standard errors stripped off a residual before it is applied: only the part of the gap
    /// supported at 95% is ever corrected, so a noisy category is left alone instead of nudged.
    const SUPPORT_Z: f64 = 1.96;
    /// Cap on a correction that makes an action *more* attractive. The residual is measured only
    /// where we already took the action, so raising its value pushes into spots the evidence has
    /// never seen; this stays deliberately small.
    const CAP_UP_BB: f64 = 3.0;
    /// Cap on a correction that makes an action *less* attractive. Over-optimism is the failure
    /// mode that spends chips (0021: river calls realized 24 bb below prediction over 865
    /// decisions while the old flat 3 bb cap bound), and correcting it only moves us toward
    /// checking, folding or a smaller size, which the evidence does cover.
    const CAP_DOWN_BB: f64 = 15.0;
    let pot: std::collections::HashMap<String, (i64, f64, f64)> =
        pot_rows.into_iter().map(|(c, n, mean, var)| (c, (n, mean, (var.max(0.0) / n.max(1) as f64).sqrt()))).collect();
    let margin: std::collections::HashMap<String, (i64, f64, f64)> =
        margin_rows.into_iter().map(|(c, n, mean, var)| (c, (n, mean, (var.max(0.0) / n.max(1) as f64).sqrt()))).collect();
    let mut table = serde_json::Map::new();
    let mut bias = std::collections::HashMap::new();
    let mut caps = std::collections::HashMap::new();
    for (cat, n, predicted, realized, var) in rows {
        let resid = realized - predicted;
        let se = (var.max(0.0) / n.max(1) as f64).sqrt();
        let (mn, mmean, mse) = margin.get(&cat).copied().unwrap_or((0, 0.0, 0.0));
        // The largest upward correction the margin supports: its 95%-supported residual, shrunk.
        let up_bound = if mn >= MIN_SAMPLES { (mmean - SUPPORT_Z * mse).max(0.0) * mn as f64 / (mn as f64 + SHRINK) } else { 0.0 };
        // Correct only the part of the gap that survives a 95% haircut, then shrink it by sample
        // size, then cap it asymmetrically, and bound an upward one by its margin evidence.
        let shrunk = (resid.abs() - SUPPORT_Z * se).max(0.0) * resid.signum() * n as f64 / (n as f64 + SHRINK);
        let capped = shrunk.clamp(-CAP_DOWN_BB, CAP_UP_BB);
        let applied = match n >= MIN_SAMPLES {
            true if capped > 0.0 => capped.min(up_bound),
            true => capped,
            false => 0.0,
        };
        // Which rule set the size (0330): a question like "would lifting the +3 bb cap change
        // anything?" is read off the table instead of re-derived.
        let bound_by = match () {
            _ if n < MIN_SAMPLES => "too few samples",
            _ if shrunk == 0.0 => "inside its 95% band",
            _ if shrunk < -CAP_DOWN_BB => "downward cap",
            _ if shrunk > 0.0 && applied < capped => "margin evidence",
            _ if shrunk > CAP_UP_BB => "upward cap",
            _ => "evidence, in full",
        };
        if applied != 0.0 {
            bias.insert(cat.clone(), applied);
        }
        let (pn, pmean, pse) = pot.get(&cat).copied().unwrap_or((0, 0.0, 0.0));
        let pot_cap = (applied < 0.0 && pn >= MIN_SAMPLES).then(|| pmean.abs() + SUPPORT_Z * pse);
        if let Some(cap) = pot_cap {
            caps.insert(cat.clone(), cap);
        }
        let pot_bias =
            if pn >= MIN_SAMPLES && pmean.abs() > pse { (pmean * pn as f64 / (pn as f64 + SHRINK)).clamp(-0.25, 0.25) } else { 0.0 };
        table.insert(cat, serde_json::json!({"n": n, "predicted_bb": predicted, "realized_bb": realized, "residual_bb": resid, "se_bb": se, "bias_bb": applied,
            "pot_n": pn, "residual_pot": pmean, "se_pot": pse, "pot_bias_candidate": pot_bias, "penalty_pot_cap": pot_cap,
            "margin_n": mn, "margin_residual_bb": mmean, "margin_se_bb": mse, "up_bound_bb": up_bound, "bound_by": bound_by}));
    }
    (bias, caps, table)
}

/// Apply the 95%-supported part of each category's mean residual, shrunk by sample size and
/// capped asymmetrically, so a correction only appears once the evidence is consistent.
pub fn update_calibration(shared: &Shared) {
    update_calibration_with(shared, true);
}

/// [`update_calibration`], storing the table only when `persist` (the head; workers apply the
/// correction locally and leave the table to the head).
pub fn update_calibration_with(shared: &Shared, persist: bool) {
    let rows = match shared.store.calibration_summary() {
        Ok(rows) => rows,
        Err(e) => {
            shared.log("calibration", "warn", format!("calibration summary unavailable: {e}"));
            return;
        }
    };
    // Pot-relative residuals (0079): reported next to the bb correction, not applied, until a
    // controlled fleet split shows they improve results.
    let pot_rows = shared.store.calibration_pot_summary().unwrap_or_else(|e| {
        tracing::warn!("per-pot calibration unavailable, penalties keep their bb size: {e}");
        Vec::new()
    });
    // Without margin evidence no upward correction is applied (the conservative side).
    let (lo, hi) = sv10_stats::margins::MARGIN;
    let margin_rows = shared.store.calibration_margin_summary(lo, hi).unwrap_or_else(|e| {
        tracing::warn!("margin calibration unavailable, no upward corrections this round: {e}");
        Vec::new()
    });
    let (bias, caps, table) = calibration_update(rows, pot_rows, margin_rows);
    if persist {
        let json = serde_json::Value::Object(table).to_string();
        if let Err(e) = shared.store.put_kv(crate::CALIBRATION_KEY, &json) {
            shared.log("calibration", "warn", format!("calibration table not stored (corrections still applied live): {e}"));
            // 0322 pattern (#744): a locked database does not cost the row — we are on the blocking
            // pool, so the blocking twin of the decision-record retry runs here.
            match crate::client::retry_locked_write_blocking(|| shared.store.put_kv(crate::CALIBRATION_KEY, &json)) {
                Ok(()) => shared.log("calibration", "info", "the calibration table stored on retry"),
                Err(e) => shared.log("calibration", "warn", format!("the calibration table lost after retries: {e}")),
            }
        }
    }
    let mut params = shared.params.write();
    params.ev_bias = bias;
    params.ev_bias_pot_cap = caps;
}
