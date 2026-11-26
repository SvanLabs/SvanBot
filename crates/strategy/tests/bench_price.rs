//! The pricer's latency at the public seam, for the performance ticket's numbers: per-decision
//! p50/p95/p99 at the play sample size. Ignored by default — run by hand with `--run-ignored` and
//! `--no-capture`; it asserts nothing, it measures.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use std::time::Instant;

use svanbot_core::rules::{Legal, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_strategy::action::ActionModel;
use svanbot_strategy::pricer::{Budget, price_lines_at};

fn spot() -> Spot {
    Spot {
        hole: "AhKh".parse().unwrap(),
        board: "QhJh2c3d".parse().unwrap(),
        pot: 400,
        legal: Legal {
            call: Some(80),
            raise: Some(RaiseRange {
                min: 160,
                max: 2_000,
            }),
        },
        opponents: 2,
        stack: 2_000,
        callable_pot: None,
        committed: 0,
        big_blind: 20,
        in_position: false,
        players_behind: None,
        lines: vec![vec![]; 2],
        seats: vec![],
    }
}

#[test]
#[ignore = "a measurement harness, not a gate: run with --run-ignored --no-capture"]
fn priced_decision_latency_percentiles() {
    let at = spot();
    let model = ActionModel::baseline();
    let models = [&model, &model];
    // Warm the evaluator's tables.
    let _ = price_lines_at(
        &at,
        &models,
        Budget {
            samples: 2_000,
            seed: 0,
        },
        1.0,
    );
    let runs = 500u64;
    let mut nanos = Vec::with_capacity(runs as usize);
    for seed in 0..runs {
        let budget = Budget {
            samples: 2_000,
            seed,
        };
        let started = Instant::now();
        let decision = price_lines_at(&at, &models, budget, 1.0);
        nanos.push(started.elapsed().as_nanos() as u64);
        assert!(decision.equity > 0.0);
    }
    nanos.sort_unstable();
    let at_percent = |p: f64| nanos[((nanos.len() - 1) as f64 * p) as usize] as f64 / 1_000.0;
    let total: u64 = nanos.iter().sum();
    let mean = total as f64 / runs as f64 / 1_000.0;
    let samples_per_second = (2_000 * runs) as f64 / (total as f64 / 1e9);
    println!(
        "priced decision ({} samples): p50 {:.1} µs, p95 {:.1} µs, p99 {:.1} µs, mean {:.1} µs; {:.0} deals/s",
        Budget {
            samples: 2_000,
            seed: 0
        }
        .samples,
        at_percent(0.50),
        at_percent(0.95),
        at_percent(0.99),
        mean,
        samples_per_second
    );
}
