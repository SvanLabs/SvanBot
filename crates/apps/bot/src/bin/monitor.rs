//! `monitor` — the read-only results monitor started and stopped with the fleet (0317).
//!
//! See [`sv10_bot::monitor`] for the line kinds and flags. Runs until stopped, or one pass with
//! `--once`; stdout is the event stream the dashboard reads back by kind.

use anyhow::Result;
use std::path::PathBuf;
use sv10_bot::monitor::{Monitor, Options, unix_now};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = Options::parse(&args)?;
    let root = std::env::var("SVANBOT10_ROOT").map(PathBuf::from).unwrap_or(std::env::current_dir()?);
    let mut monitor = Monitor::open(&root, opts)?;
    println!("{}", monitor.start_line());
    loop {
        if !opts.once {
            std::thread::sleep(std::time::Duration::from_secs(opts.interval));
        }
        for line in monitor.pass(unix_now(), opts.once) {
            println!("{line}");
        }
        if opts.once {
            return Ok(());
        }
    }
}
