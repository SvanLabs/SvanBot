//! Lazily built tables must be safe to first touch from inside rayon workers: an initializer
//! that itself uses rayon lets a blocked worker steal a task that re-enters the same
//! `OnceLock`, which deadlocks. The race is timing dependent and a table builds only once per
//! process, so the test re-runs itself as fresh child processes.

use rayon::prelude::*;
use std::process::Command;
use std::time::{Duration, Instant};
use sv10_core::cards::Card;
use sv10_core::preflop;

const CHILD: &str = "SV10_COLD_TABLES_CHILD";

/// Staggered outer tasks, like simulated tables reaching their first decision at different times.
fn touch_cold_tables() {
    let (a, b) = (Card::parse("As").unwrap(), Card::parse("Kd").unwrap());
    let total: f64 = (0..16)
        .into_par_iter()
        .with_max_len(1)
        .map(|t| {
            std::thread::sleep(Duration::from_millis(t as u64 * 3));
            let x = if t % 2 == 0 { preflop::percentile(a, b) } else { preflop::class_equity_vs_top(0.1)[0] };
            x as f64
        })
        .sum();
    assert!(total.is_finite() && total > 0.0);
}

#[test]
fn cold_tables_first_used_inside_rayon_do_not_deadlock() {
    if std::env::var_os(CHILD).is_some() {
        return touch_cold_tables();
    }
    let exe = std::env::current_exe().unwrap();
    for run in 0..10 {
        let mut child = Command::new(&exe)
            .args(["--exact", "cold_tables_first_used_inside_rayon_do_not_deadlock", "--nocapture"])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "child run {run} failed");
                break;
            }
            if started.elapsed() > Duration::from_secs(30) {
                let _ = child.kill();
                panic!("child run {run} deadlocked building lazy tables inside rayon");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
