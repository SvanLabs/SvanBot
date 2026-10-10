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

use crate::replay::{Corrections, ReplayRecord, exact_inputs, identical, rerun};

mod calibration;
use calibration::calibration_from;
pub use calibration::{CALIBRATION_HOURS, CalibrationFlips, calibration_flips};

/// One way of switching a component off, and whether a spot has the component to switch off (#315:
/// "moves nothing" and "was not there to move anything" read the same without it).
struct Variant {
    name: &'static str,
    apply: fn(&mut ReplayRecord, &mut Params, &mut Option<Mlp>),
    installed: fn(&ReplayRecord, &Option<Mlp>) -> bool,
}

const VARIANTS: [Variant; 11] = [
    Variant { name: "response network", apply: |_, _, nn| *nn = None, installed: |_, nn| nn.is_some() },
    Variant {
        name: "per-opponent response correction",
        apply: |r, _, _| r.corrections.values_mut().for_each(|c| c.response_ratio = None),
        installed: |r, _| r.corrections.values().any(|c| c.response_ratio.is_some()),
    },
    Variant {
        name: "per-opponent fold offsets",
        apply: |r, _, _| r.corrections.values_mut().for_each(|c| c.fold_offset = None),
        installed: |r, _| r.corrections.values().any(|c| c.fold_offset.is_some()),
    },
    Variant {
        name: "per-opponent river sizing tells",
        apply: |r, _, _| r.corrections.values_mut().for_each(|c| c.size_tell = None),
        // A tell prices a river bet: on another street the fit is installed but has nothing to read.
        installed: |r, _| r.situation.street.name() == "river" && r.corrections.values().any(|c| c.size_tell.is_some()),
    },
    Variant {
        name: "street fold calibration",
        apply: |_, p, _| p.fold_logit_shift = [0.0; 3],
        // One shift per postflop street; a spot has only its own street's.
        installed: |r, _| {
            ["flop", "turn", "river"]
                .iter()
                .position(|s| *s == r.situation.street.name())
                .is_some_and(|i| r.params.fold_logit_shift[i] != 0.0)
        },
    },
    Variant {
        name: "preflop fold calibration",
        apply: |_, p, _| p.preflop_fold_logit_shift = 0.0,
        installed: |r, _| r.situation.street.name() == "preflop" && r.params.preflop_fold_logit_shift != 0.0,
    },
    Variant {
        name: "self-calibration EV corrections",
        apply: |_, p, _| {
            p.ev_bias.clear();
            p.ev_bias_pot_cap.clear();
        },
        installed: |r, _| !r.params.ev_bias.is_empty(),
    },
    Variant {
        name: "showdown-fitted range model",
        apply: |_, p, _| p.range = Default::default(),
        installed: |r, _| r.params.range != Default::default(),
    },
    Variant {
        name: "all-in call fits (river jam, deep pot, overbet)",
        apply: |_, p, _| {
            p.river_jam_call_shift = 0.0;
            p.deep_call_shift = 0.0;
            p.overbet_call_shift = 0.0;
            p.overbet_call_slope = 0.0;
        },
        installed: |r, _| {
            let p = &r.params;
            [p.river_jam_call_shift, p.deep_call_shift, p.overbet_call_shift, p.overbet_call_slope].iter().any(|x| *x != 0.0)
        },
    },
    Variant { name: "per-player stats (population only)", apply: |r, _, _| r.players.clear(), installed: |r, _| !r.players.is_empty() },
    Variant {
        name: "mixing (temperature 0: always the best EV)",
        apply: |_, p, _| p.temperature = 0.0,
        installed: |r, _| r.params.temperature != 0.0,
    },
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
pub const WIRING_KEY: &str = "wiring.v2";

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
    /// Decisions in the sample that had the component to switch off (#315). `None` in rows stored
    /// before it was counted.
    #[serde(default)]
    pub installed: Option<usize>,
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
    /// The sample's decisions per street, in play order (#315): the big spots are rarely preflop, which
    /// is why a preflop component can read as moving nothing. Empty in rows stored before it.
    #[serde(default)]
    pub streets: Vec<(String, usize)>,
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
            let as_recorded = exact_inputs(rec, &rec.params, nn.as_ref()) && identical(rec, &rerun(rec, &rec.params, nn.as_ref()));
            let had = !rec.corrections.is_empty();
            let fixed = with_current(rec.clone(), fits);
            (
                as_recorded,
                had,
                exact_inputs(&fixed, &fixed.params, nn.as_ref()) && identical(&fixed, &rerun(&fixed, &fixed.params, nn.as_ref())),
                rec.version,
            )
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
                installed: Some(full.iter().filter(|(rec, nn, _)| (v.installed)(rec, nn)).count()),
            }
        })
        .collect();
    let streets = ["preflop", "flop", "turn", "river"]
        .iter()
        .map(|s| (s.to_string(), full.iter().filter(|(rec, _, _)| rec.situation.street.name() == *s).count()))
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
        streets,
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
        store.put_kv(WIRING_KEY, &report_json(&report)?)?;
    }
    Ok(report)
}

/// Whether `args` asks for the JSON render (`review wiring --json`, #723).
pub fn wants_json(args: &[String]) -> bool {
    args.iter().any(|a| a == "--json")
}

pub fn wiring(store: &Store, args: &[String]) -> Result<()> {
    let json = wants_json(args);
    let n = args.iter().find_map(|a| a.parse::<usize>().ok()).unwrap_or(200);
    let fits = crate::playerfits::PlayerFits::load(store)?;
    let cases = load_cases(store, n)?;
    // Diagnostics are lines of their own and would break the one-object contract of `--json`.
    if !json && args.iter().any(|a| a == "--diag") {
        for (rec, nn) in &cases {
            let d = rerun(rec, &rec.params, nn.as_ref());
            if exact_inputs(rec, &rec.params, nn.as_ref()) && identical(rec, &d) {
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
    // `--json` (#723) prints the same `WiringReport` the dashboard's row carries; the text table is
    // the default.
    if json {
        println!("{}", report_json(&report)?);
    } else {
        print_report(&report);
    }
    let note = if report.sample < MIN_SAMPLE {
        Some(format!("not stored: a sample of {} is under the {MIN_SAMPLE} a stored report needs", report.sample))
    } else if let Err(e) = store.put_kv(WIRING_KEY, &report_json(&report)?) {
        // The measurement stands even when the row cannot be written; say so rather than fail the run.
        Some(format!("stored row not written ({e}): the dashboard panel will keep the previous one"))
    } else {
        None
    };
    if let Some(note) = note {
        if json {
            // stdout is exactly one JSON object; the operator still sees why nothing was stored.
            eprintln!("\n{note}");
        } else {
            println!("\n{note}");
        }
    }
    Ok(())
}

/// The measurement, as the CLI prints it. One function for the text, so a test can pin it whole
/// (the 500-line rule keeps it here, not in a printer).
fn print_report(r: &WiringReport) {
    print!("{}", report_text(r));
}

/// The measurement as one JSON object (#723): the same [`WiringReport`] the dashboard's row carries
/// and the text table is formatted from, so both an agent and a test read the values rather than
/// the layout. One function, so the CLI and the stored row cannot diverge.
pub fn report_json(r: &WiringReport) -> Result<String> {
    Ok(serde_json::to_string(r)?)
}

/// The text table [`print_report`] writes, as a string: the default render, which `--json` leaves
/// untouched (#723).
pub fn report_text(r: &WiringReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "{} of {} re-run twice with identical inputs gave different results", r.unstable, r.sample);
    let _ = writeln!(
        out,
        "{} recorded big decisions: {} replay exactly as recorded; {} with today's per-opponent corrections added; {} recorded their corrections (replay v3)",
        r.sample, r.exact, r.exact_with_current, r.carrying_corrections
    );
    // 0316: the acceptance for the missing-corrections fix is exactness on the records that carry
    // them, over a day of them — the pre-v3 rows above are inexact by construction and say nothing.
    let _ = writeln!(
        out,
        "{}",
        match r.v3 {
            0 => "no replay-v3+ records in this sample yet: exactness on records carrying corrections is not measurable here".to_string(),
            _ if r.v3_exact == r.v3 => {
                format!(
                    "all {} replay-v3+ records (carrying corrections; exactness also requires the table image) replay exactly as recorded",
                    r.v3
                )
            }
            _ => format!(
                "{} of {} replay-v3+ records (carrying corrections; exactness also requires the table image) replay exactly as recorded",
                r.v3_exact, r.v3
            ),
        }
    );
    let _ = writeln!(
        out,
        "{} of {} decisions picked a candidate below the best EV (the mixing temperature at work)",
        r.chosen_not_best, r.sample
    );
    let mix: Vec<String> = r.streets.iter().map(|(s, n)| format!("{s} {n}")).collect();
    let _ = writeln!(out, "sample by street: {}\n", mix.join(", "));
    let _ = writeln!(out, "{:<50} {:>7} {:>9} {:>12} {:>12}", "component switched off", "on", "changed", "cost bb/dec", "max bb");
    for row in &r.rows {
        let on = row.installed.map_or("?".to_string(), |n| n.to_string());
        let _ = writeln!(
            out,
            "{:<50} {:>7} {:>5} {:>3.0}% {:>12.3} {:>12.1}",
            row.component, on, row.changed, row.share_pct, row.cost_bb, row.max_bb
        );
    }
    let _ = writeln!(out, "\n'on': decisions that had the component to switch off — 'changed' 0 of 'on' 0 is not measured, not idle.");
    let _ = writeln!(out, "'changed': decisions whose action or size moves with the component off. 'cost': what the moved");
    let _ = writeln!(out, "choice gives up under the full model (same seed, same samples) — the component's value on these spots.");
    let _ = writeln!(
        out,
        "\nself-calibration over every decision of the last {CALIBRATION_HOURS} h (best action with and without each candidate's bias):"
    );
    for f in &r.calibration {
        let _ = writeln!(
            out,
            "  {:<8} {:>7} decisions  {:>6} flipped ({:>4.1}%)  mostly {} ({})",
            f.street, f.decisions, f.flipped, f.share_pct, f.main, f.main_count
        );
    }
    out
}

#[cfg(test)]
mod tests;
