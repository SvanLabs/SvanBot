//! `svanbot tables preflop [samples] [seed]`: measures the preflop table (V11-39) and prints it, the
//! text of `crates/strategy/src/data/preflop.txt`. Each of the 169 classes against one to five
//! random hands, from `samples` seeded deals each (default 400,000, seed 1). It touches neither
//! the venue nor a store, and the same samples and seed print the same table on any machine.
//!
//! Run it at low priority, and write its output over the data file:
//! `nice -n 15 svanbot tables preflop > crates/strategy/src/data/preflop.txt`.

use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use svanbot_strategy::preflop::{CLASSES, MOST, measure, table_text};

/// Threads the classes are spread over. The box is shared with the live fleet.
const THREADS: usize = 4;

pub(crate) fn run(args: &[String]) -> ExitCode {
    if args.first().map(String::as_str) != Some("preflop") {
        eprintln!("svanbot tables: usage: svanbot tables preflop [samples] [seed]");
        return ExitCode::from(2);
    }
    let number =
        |at: usize, default: u64| args.get(at).map_or(Ok(default), |text| text.parse::<u64>());
    let (Ok(samples), Ok(seed)) = (number(1, 400_000), number(2, 1)) else {
        eprintln!("svanbot tables: the samples and the seed are whole numbers");
        return ExitCode::from(2);
    };
    let next = AtomicUsize::new(0);
    let mut equities = vec![[0.0; MOST]; CLASSES];
    let measured: Vec<(usize, [f64; MOST])> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let class = next.fetch_add(1, Ordering::Relaxed);
                        if class >= CLASSES {
                            return done;
                        }
                        // Each entry has its own seed, so the table is the same however the
                        // classes fall across the threads.
                        let row = std::array::from_fn(|at| {
                            let entry = (class * MOST + at) as u64;
                            measure(class, at + 1, samples, seed ^ (entry << 20))
                        });
                        done.push((class, row));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("a table worker panicked"))
            .collect()
    });
    for (class, row) in measured {
        equities[class] = row;
    }
    print!("{}", table_text(&equities));
    ExitCode::SUCCESS
}
