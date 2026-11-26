//! The child half of the SvanDB crash test (V11-128): appends hands for ever, printing each
//! index only after its append returned. The test kills this process with SIGKILL; the printed
//! indices are the writes the store acknowledged. A separate binary so the test spawns it by a
//! compile-time path (`CARGO_BIN_EXE_…`), and the log is a constant name in the working directory
//! the test chooses, so no path or number is ever read from outside.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]

use std::io::Write;
use std::path::Path;

use serde_json::json;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::{Source, Svandb};

fn main() {
    let db = Svandb::open(Path::new("A.svandb"), "A").expect("open the log");
    let mut at = 0u64;
    loop {
        let id = format!("h-{at}");
        db.append_hand(&hand(&id), Source::Live, i64::try_from(at).unwrap_or(0))
            .expect("append a hand");
        println!("{at}");
        std::io::stdout()
            .flush()
            .expect("flush the acknowledged index");
        at += 1;
    }
}

fn hand(id: &str) -> Hand {
    Hand {
        hand_id: id.into(),
        table_id: "t-1".into(),
        frames: vec![
            json!({"type": "hand_start", "hand_id": id}).to_string(),
            json!({"type": "hand_result", "hand_id": id}).to_string(),
        ],
        traced: Vec::new(),
    }
}
