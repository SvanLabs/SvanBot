//! `review wiring [N]` (0316): is every component the dashboard calls active really changing live
//! decisions? The newest N recorded big decisions are re-run with each component switched off in turn;
//! for each component the share of decisions whose action changes and what the change costs under the
//! full model (its EV of its own best action minus its EV of the action the variant picks, in bb).
//!
//! Records made before replay version 3 lack the per-opponent corrections live play used; they are
//! re-run with the corrections installed now (the closest available), and the report says how many
//! replay exactly each way — which is how the missing corrections were found.

use anyhow::Result;
use rayon::prelude::*;
use sv10_core::nn::Mlp;
use sv10_core::policy::{Decision, Params};
use sv10_store::store::Store;

use crate::replay::{Corrections, ReplayRecord, identical, rerun};

/// One way of switching a component off.
struct Variant {
    name: &'static str,
    apply: fn(&mut ReplayRecord, &mut Params, &mut Option<Mlp>),
}

const VARIANTS: [Variant; 11] = [
    Variant { name: "response network", apply: |_, _, nn| *nn = None },
    Variant { name: "per-opponent response correction", apply: |r, _, _| r.corrections.values_mut().for_each(|c| c.response_ratio = None) },
    Variant { name: "per-opponent fold offsets", apply: |r, _, _| r.corrections.values_mut().for_each(|c| c.fold_offset = None) },
    Variant { name: "per-opponent river sizing tells", apply: |r, _, _| r.corrections.values_mut().for_each(|c| c.size_tell = None) },
    Variant { name: "street fold calibration", apply: |_, p, _| p.fold_logit_shift = [0.0; 3] },
    Variant { name: "preflop fold calibration", apply: |_, p, _| p.preflop_fold_logit_shift = 0.0 },
    Variant {
        name: "self-calibration EV corrections",
        apply: |_, p, _| {
            p.ev_bias.clear();
            p.ev_bias_pot_cap.clear();
        },
    },
    Variant { name: "showdown-fitted range model", apply: |_, p, _| p.range = Default::default() },
    Variant {
        name: "all-in call fits (river jam, deep pot, overbet)",
        apply: |_, p, _| {
            p.river_jam_call_shift = 0.0;
            p.deep_call_shift = 0.0;
            p.overbet_call_shift = 0.0;
            p.overbet_call_slope = 0.0;
        },
    },
    Variant { name: "per-player stats (population only)", apply: |r, _, _| r.players.clear() },
    Variant { name: "mixing (temperature 0: always the best EV)", apply: |_, p, _| p.temperature = 0.0 },
];

/// The full model's EV cost, in bb, of playing `variant`'s action instead of the full model's best.
fn cost_bb(full: &Decision, variant: &Decision, bb: f64) -> f64 {
    let best = full.candidates.iter().map(|c| c.ev).fold(f64::MIN, f64::max);
    let picked = full
        .candidates
        .iter()
        .filter(|c| c.action == variant.action_name)
        .min_by_key(|c| (c.amount.unwrap_or(0) - variant.amount.unwrap_or(0)).abs())
        .map_or(best, |c| c.ev);
    ((best - picked) / bb).max(0.0)
}

/// Install `fits`' corrections for the record's players (what live play has installed now).
fn with_current(mut rec: ReplayRecord, fits: &crate::playerfits::PlayerFits) -> ReplayRecord {
    for p in &rec.situation.players {
        let c = Corrections {
            response_ratio: fits.response_ratios.get(&p.name).copied(),
            fold_offset: fits.fold_offsets.get(&p.name).copied(),
            size_tell: fits.size_tells.get(&p.name).copied(),
        };
        if c != Corrections::default() {
            rec.corrections.insert(p.name.clone(), c);
        }
    }
    rec
}

fn full_twice(cases: &[(ReplayRecord, Option<Mlp>)]) -> usize {
    cases
        .par_iter()
        .filter(|(rec, nn)| {
            let (a, b) = (rerun(rec, &rec.params, nn.as_ref()), rerun(rec, &rec.params, nn.as_ref()));
            a.candidates.iter().zip(&b.candidates).any(|(x, y)| x.ev.to_bits() != y.ev.to_bits()) || a.action_name != b.action_name
        })
        .count()
}

/// The store row the dashboard reads ([`WIRING_KEY`]).
pub const WIRING_KEY: &str = "wiring.v1";

/// One component's measured value on the sample (0316).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Row {
    /// Component as the dashboard names it.
    pub component: String,
    /// Decisions whose action or size moved with the component switched off.
    pub changed: usize,
    /// Share of the sample that moved, in percent.
    pub share_pct: f64,
    /// Mean EV the moved choices give up under the full model, big blinds per decision.
    pub cost_bb: f64,
    /// Largest single cost in the sample, big blinds.
    pub max_bb: f64,
}

/// What the wiring measurement found, stored so the dashboard can show it ([`WIRING_KEY`]).
///
/// Every figure here is measured, none is a claim about the code: `sample` and `at` say how much
/// and when, so a panel can say what the table is worth instead of presenting it as current truth.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WiringReport {
    /// Unix seconds the sample was measured.
    pub at: f64,
    /// Recorded big decisions re-run.
    pub sample: usize,
    /// Re-runs of the same inputs that disagreed with each other (nondeterminism in the decision).
    pub unstable: usize,
    /// Records that replayed exactly as recorded.
    pub exact: usize,
    /// Records that replayed exactly once today's corrections were installed.
    pub exact_with_current: usize,
    /// Records that carry their own corrections (replay v3+).
    pub carrying_corrections: usize,
    /// Records at replay v3 or above, and how many of those replayed exactly.
    pub v3: usize,
    /// See [`WiringReport::v3`].
    pub v3_exact: usize,
    /// Decisions that picked a candidate below the best EV (the mixing temperature at work).
    pub chosen_not_best: usize,
    /// One row per component, in the order the table prints them.
    pub rows: Vec<Row>,
    /// Self-calibration's reach over *every* decision of the last [`CALIBRATION_HOURS`], by street (0332):
    /// the big recorded decisions above hardly see it, and it was deciding over half of all preflop
    /// choices. Empty in rows stored before it was measured.
    #[serde(default)]
    pub calibration: Vec<CalibrationFlips>,
}

/// Hours of the decision log the calibration count reads.
pub const CALIBRATION_HOURS: i64 = 24;

/// On one street, how many decisions the self-calibration bias decided (0332).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CalibrationFlips {
    /// `preflop`, `flop`, `turn` or `river`.
    pub street: String,
    /// Decisions with two or more priced options.
    pub decisions: usize,
    /// Of those, the ones whose best action changes when each candidate's applied bias is removed.
    pub flipped: usize,
    /// `flipped` as a share of `decisions`, percent.
    pub share_pct: f64,
    /// The commonest change, `without -> with` (e.g. `fold -> call`), and how many.
    pub main: String,
    /// See [`CalibrationFlips::main`].
    pub main_count: usize,
}

/// Count, per street, the decisions whose best action (by family: every raise size is `raise`) differs
/// with and without the calibration bias recorded on each candidate. Pure over `(street, detail JSON)`.
pub fn calibration_flips(rows: &[(String, String)]) -> Vec<CalibrationFlips> {
    // Per street: decisions with a choice, and each (without, with) change of best action.
    type Changes = std::collections::BTreeMap<(String, String), usize>;
    let mut by: std::collections::BTreeMap<&str, (usize, Changes)> = Default::default();
    for (street, detail) in rows {
        let Ok(d) = serde_json::from_str::<serde_json::Value>(detail) else { continue };
        let cands: Vec<(&str, f64, f64)> = d["candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| Some((c["action"].as_str()?, c["ev"].as_f64()?, c["bias"].as_f64().unwrap_or(0.0))))
            .collect();
        if cands.len() < 2 {
            continue;
        }
        let best = |f: &dyn Fn(&(&str, f64, f64)) -> f64| cands.iter().max_by(|a, b| f(a).total_cmp(&f(b))).map(|c| c.0).unwrap_or("");
        let (with, without) = (best(&|c| c.1), best(&|c| c.1 - c.2));
        let entry = by.entry(street.as_str()).or_default();
        entry.0 += 1;
        if with != without {
            *entry.1.entry((without.to_string(), with.to_string())).or_default() += 1;
        }
    }
    let order = |s: &str| ["preflop", "flop", "turn", "river"].iter().position(|x| *x == s).unwrap_or(4);
    let mut out: Vec<CalibrationFlips> = by
        .into_iter()
        .map(|(street, (n, flips))| {
            let flipped = flips.values().sum();
            let (main, main_count) =
                flips.iter().max_by_key(|(_, c)| **c).map(|((a, b), c)| (format!("{a} -> {b}"), *c)).unwrap_or_default();
            CalibrationFlips {
                street: street.to_string(),
                decisions: n,
                flipped,
                share_pct: flipped as f64 * 100.0 / n.max(1) as f64,
                main,
                main_count,
            }
        })
        .collect();
    out.sort_by_key(|f| order(&f.street));
    out
}

/// [`calibration_flips`] over the store's last [`CALIBRATION_HOURS`] of decisions.
fn calibration_from(store: &Store) -> Result<Vec<CalibrationFlips>> {
    let since = (chrono::Utc::now() - chrono::Duration::hours(CALIBRATION_HOURS)).to_rfc3339();
    Ok(calibration_flips(&store.decision_details_since(&since)?))
}

/// Load the newest `n` recorded big decisions with the network each used.
fn load_cases(store: &Store, n: usize) -> Result<Vec<(ReplayRecord, Option<Mlp>)>> {
    let mut cases: Vec<(ReplayRecord, Option<Mlp>)> = Vec::new();
    for row in store.replays(n, None)? {
        let Ok(rec) = serde_json::from_str::<ReplayRecord>(&row.record) else { continue };
        let nn = match &rec.net_digest {
            Some(d) => store.replay_net(d)?.and_then(|j| serde_json::from_str::<Mlp>(&j).ok()),
            None => None,
        };
        cases.push((rec, nn));
    }
    Ok(cases)
}

/// Measure every component on `cases`: the whole computation behind [`WiringReport`], with no
/// printing, so the analyst can refresh the stored row and a test can check the numbers.
pub fn measure(cases: Vec<(ReplayRecord, Option<Mlp>)>, fits: &crate::playerfits::PlayerFits) -> WiringReport {
    // Exactness as recorded, and with today's corrections added to records that lack them.
    let exact: Vec<(bool, bool, bool, u32)> = cases
        .par_iter()
        .map(|(rec, nn)| {
            let as_recorded = identical(rec, &rerun(rec, &rec.params, nn.as_ref()));
            let had = !rec.corrections.is_empty();
            let fixed = with_current(rec.clone(), fits);
            (as_recorded, had, identical(&fixed, &rerun(&fixed, &fixed.params, nn.as_ref())), rec.version)
        })
        .collect();
    let unstable = full_twice(&cases);
    // The full model for every ablation: the record with its corrections (or today's, when it lacks them).
    let full: Vec<(ReplayRecord, Option<Mlp>, Decision)> = cases
        .into_par_iter()
        .map(|(rec, nn)| {
            let rec = if rec.corrections.is_empty() { with_current(rec, fits) } else { rec };
            let d = rerun(&rec, &rec.params, nn.as_ref());
            (rec, nn, d)
        })
        .collect();
    let chosen_not_best =
        full.iter().filter(|(rec, _, d)| cost_bb(d, d, rec.situation.bb.max(1) as f64) > 0.0 && d.candidates.len() > 1).count();
    let rows: Vec<Row> = VARIANTS
        .iter()
        .map(|v| {
            let measured: Vec<(bool, f64)> = full
                .par_iter()
                .map(|(rec, nn, d)| {
                    let (mut r, mut p, mut net) = (rec.clone(), rec.params.clone(), nn.clone());
                    (v.apply)(&mut r, &mut p, &mut net);
                    let dv = rerun(&r, &p, net.as_ref());
                    let changed = (dv.action_name.as_str(), dv.amount) != (d.action_name.as_str(), d.amount);
                    // Only a moved choice costs anything: an unchanged one would otherwise carry the full
                    // model's own mixing loss into every row (0.032 bb/decision on components that moved
                    // nothing, in the first stored table).
                    (changed, if changed { cost_bb(d, &dv, rec.situation.bb.max(1) as f64) } else { 0.0 })
                })
                .collect();
            let changed = measured.iter().filter(|r| r.0).count();
            let n = measured.len().max(1);
            Row {
                component: v.name.to_string(),
                changed,
                share_pct: changed as f64 * 100.0 / n as f64,
                cost_bb: measured.iter().map(|r| r.1).sum::<f64>() / n as f64,
                max_bb: measured.iter().map(|r| r.1).fold(0.0, f64::max),
            }
        })
        .collect();
    WiringReport {
        at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64(),
        sample: exact.len(),
        unstable,
        exact: exact.iter().filter(|e| e.0).count(),
        exact_with_current: exact.iter().filter(|e| e.0 || e.2).count(),
        carrying_corrections: exact.iter().filter(|e| e.1).count(),
        v3: exact.iter().filter(|e| e.3 >= 3).count(),
        v3_exact: exact.iter().filter(|e| e.3 >= 3 && e.0).count(),
        chosen_not_best,
        rows,
        calibration: Vec::new(),
    }
}

/// Fewest recorded decisions a stored report may be built from: below this, one decision's flip
/// moves the share by tens of points and the table says more about the sample than the fleet.
pub const MIN_SAMPLE: usize = 20;

/// Whether the stored report is due for a refresh: nothing stored, unreadable, or older than
/// `max_age_secs`. Restart-safe — the age comes from the row, not from a process timer.
pub fn due(store: &Store, now: f64, max_age_secs: f64) -> bool {
    let stored: Option<WiringReport> = store.get_kv(WIRING_KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok());
    match stored {
        Some(r) => now - r.at >= max_age_secs,
        None => true,
    }
}

/// Measure the newest `n` recorded big decisions and store the report, so the dashboard's wiring
/// panel is a measurement rather than a claim about the code (0316). Returns the report; a sample
/// under [`MIN_SAMPLE`] is measured and returned but **not** stored, so the panel keeps the last
/// report it can trust instead of a fresh one built on nothing.
pub fn refresh(store: &Store, n: usize) -> Result<WiringReport> {
    let cases = load_cases(store, n)?;
    let mut report = measure(cases, &crate::playerfits::PlayerFits::load(store)?);
    report.calibration = calibration_from(store)?;
    if report.sample >= MIN_SAMPLE {
        store.put_kv(WIRING_KEY, &serde_json::to_string(&report)?)?;
    }
    Ok(report)
}

pub fn wiring(store: &Store, args: &[String]) -> Result<()> {
    let n = args.iter().find_map(|a| a.parse::<usize>().ok()).unwrap_or(200);
    let fits = crate::playerfits::PlayerFits::load(store)?;
    let cases = load_cases(store, n)?;
    if args.iter().any(|a| a == "--diag") {
        for (rec, nn) in &cases {
            let d = rerun(rec, &rec.params, nn.as_ref());
            if identical(rec, &d) {
                let live = rec.situation.live_opponents().count();
                println!(
                    "exact: {} live opponents {} to_call {} action {:?}",
                    rec.situation.street.name(),
                    live,
                    rec.situation.call_amount,
                    rec.action
                );
                continue;
            }
            let worst = rec.candidates.iter().zip(&d.candidates).map(|(a, b)| (a.2 - b.ev).abs()).fold(0.0, f64::max);
            println!(
                "inexact: {} live opponents {} to_call {} history {} nn {} corrections {} max |dEV| {:.6} recorded {:?} replay {} {:?} chunks {} samples {}",
                rec.situation.street.name(),
                rec.situation.live_opponents().count(),
                rec.situation.call_amount,
                rec.situation.history.len(),
                rec.net_digest.is_some(),
                rec.corrections.len(),
                worst,
                rec.action,
                d.action_name,
                d.amount,
                rec.params.deal_chunks,
                rec.params.samples
            );
        }
    }
    let mut report = measure(cases, &fits);
    report.calibration = calibration_from(store)?;
    print_report(&report);
    if report.sample < MIN_SAMPLE {
        println!("\nnot stored: a sample of {} is under the {MIN_SAMPLE} a stored report needs", report.sample);
    } else if let Err(e) = store.put_kv(WIRING_KEY, &serde_json::to_string(&report)?) {
        // The measurement stands even when the row cannot be written; say so rather than fail the run.
        println!("\nstored row not written ({e}): the dashboard panel will keep the previous one");
    }
    Ok(())
}

/// The measurement, as the CLI prints it.
fn print_report(r: &WiringReport) {
    println!("{} of {} re-run twice with identical inputs gave different results", r.unstable, r.sample);
    println!(
        "{} recorded big decisions: {} replay exactly as recorded; {} with today's per-opponent corrections added; {} recorded their corrections (replay v3)",
        r.sample, r.exact, r.exact_with_current, r.carrying_corrections
    );
    // 0316: the acceptance for the missing-corrections fix is exactness on the records that carry
    // them, over a day of them — the pre-v3 rows above are inexact by construction and say nothing.
    println!(
        "{}",
        match r.v3 {
            0 => "no replay-v3 records in this sample yet: exactness on the records that carry the live inputs is not measurable here"
                .to_string(),
            _ if r.v3_exact == r.v3 => {
                format!("all {} replay-v3 records (the ones that carry the live inputs) replay exactly as recorded", r.v3)
            }
            _ => format!("{} of {} replay-v3 records (the ones that carry the live inputs) replay exactly as recorded", r.v3_exact, r.v3),
        }
    );
    println!("{} of {} decisions picked a candidate below the best EV (the mixing temperature at work)\n", r.chosen_not_best, r.sample);
    println!("{:<50} {:>9} {:>12} {:>12}", "component switched off", "changed", "cost bb/dec", "max bb");
    for row in &r.rows {
        println!("{:<50} {:>5} {:>3.0}% {:>12.3} {:>12.1}", row.component, row.changed, row.share_pct, row.cost_bb, row.max_bb);
    }
    println!("\n'changed': decisions whose action or size moves with the component off. 'cost': what the moved");
    println!("choice gives up under the full model (same seed, same samples) — the component's value on these spots.");
    println!(
        "\nself-calibration over every decision of the last {CALIBRATION_HOURS} h (best action with and without each candidate's bias):"
    );
    for f in &r.calibration {
        println!(
            "  {:<8} {:>7} decisions  {:>6} flipped ({:>4.1}%)  mostly {} ({})",
            f.street, f.decisions, f.flipped, f.share_pct, f.main, f.main_count
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0316: the dashboard's wiring panel reads this row, so its shape is a contract — and a sample
    /// too small to mean anything never replaces a stored report.
    #[test]
    fn a_stored_report_round_trips_and_a_tiny_sample_is_not_stored() {
        let dir = std::env::temp_dir().join(format!("sv10-wiring-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        // No replays recorded: nothing measurable, and nothing stored for the panel to show.
        let empty = refresh(&store, 50).unwrap();
        assert_eq!(empty.sample, 0);
        assert_eq!(store.get_kv(WIRING_KEY).unwrap(), None, "an empty sample must not overwrite a stored table");
        // Nothing stored is due; a fresh row is not; an old one is again.
        assert!(due(&store, 1_000.0, 86_400.0), "no report yet");
        store
            .put_kv(
                WIRING_KEY,
                r#"{"at": 900.0, "sample": 200, "unstable": 0, "exact": 1, "exact_with_current": 1,
            "carrying_corrections": 1, "v3": 1, "v3_exact": 1, "chosen_not_best": 0, "rows": []}"#,
            )
            .unwrap();
        assert!(!due(&store, 1_000.0, 86_400.0), "measured 100 s ago");
        assert!(due(&store, 90_000.0, 86_400.0), "measured a day ago");
        store.put_kv(WIRING_KEY, "not json").unwrap();
        assert!(due(&store, 1_000.0, 86_400.0), "an unreadable row must be replaced, not trusted");
        // The row the panel parses: every figure survives JSON, and the components keep their order.
        let report = WiringReport {
            at: 1_790_000_000.0,
            sample: 200,
            unstable: 0,
            exact: 131,
            exact_with_current: 132,
            carrying_corrections: 30,
            v3: 30,
            v3_exact: 30,
            chosen_not_best: 1,
            rows: VARIANTS
                .iter()
                .map(|v| Row { component: v.name.to_string(), changed: 3, share_pct: 1.5, cost_bb: 0.25, max_bb: 40.0 })
                .collect(),
            calibration: vec![CalibrationFlips {
                street: "preflop".into(),
                decisions: 3_227,
                flipped: 1_794,
                share_pct: 55.6,
                main: "fold -> call".into(),
                main_count: 1_030,
            }],
        };
        store.put_kv(WIRING_KEY, &serde_json::to_string(&report).unwrap()).unwrap();
        let back: WiringReport = serde_json::from_str(&store.get_kv(WIRING_KEY).unwrap().unwrap()).unwrap();
        assert_eq!(back, report);
        assert_eq!(back.rows.len(), VARIANTS.len());
        assert_eq!(back.rows[0].component, VARIANTS[0].name, "the table prints in measurement order");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 0332: self-calibration decided over half of all preflop choices while the big-decision table
    /// showed it moving 0.7%. A decision counts as flipped when its best action differs with and without
    /// the bias recorded on each candidate; sizes of one action are one family; one option is no choice.
    #[test]
    fn a_decision_counts_as_flipped_when_the_bias_decides_its_best_action() {
        let d = |cands: &str| format!(r#"{{"candidates": [{cands}]}}"#);
        let fold = r#"{"action": "fold", "ev": 0.0, "bias": 0.0}"#;
        let rows = vec![
            // Call −20 without its +50 bias, +30 with it: the bias turned a fold into a call.
            ("preflop".to_string(), d(&format!(r#"{fold}, {{"action": "call", "ev": 30.0, "bias": 50.0}}"#))),
            // Call is best either way.
            ("preflop".to_string(), d(&format!(r#"{fold}, {{"action": "call", "ev": 80.0, "bias": 50.0}}"#))),
            // Two raise sizes trade places under the bias: the same family, no flip.
            (
                "flop".to_string(),
                d(
                    r#"{"action": "raise", "ev": 10.0, "bias": 5.0}, {"action": "raise", "ev": 9.0, "bias": 0.0}, {"action": "check", "ev": 1.0}"#,
                ),
            ),
            // One option is no choice at all, and an unreadable detail is skipped.
            ("river".to_string(), d(fold)),
            ("river".to_string(), "not json".to_string()),
        ];
        let f = calibration_flips(&rows);
        assert_eq!(f.len(), 2, "streets with a choice only: {f:?}");
        assert_eq!((f[0].street.as_str(), f[0].decisions, f[0].flipped, f[0].main.as_str()), ("preflop", 2, 1, "fold -> call"));
        assert_eq!(f[0].share_pct, 50.0);
        assert_eq!((f[1].street.as_str(), f[1].decisions, f[1].flipped), ("flop", 1, 0), "streets print in play order");
    }
}
