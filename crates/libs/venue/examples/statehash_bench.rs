//! state_hash verification bench over the captured fixture (0265): frames/second and
//! nanoseconds/frame, so a future change to canonicalization is visible. Not a gate —
//! verification costs microseconds against a ~2 s frame cadence — just a ruler.
//!
//! Run: `cargo run -p sv10-venue --example statehash_bench`

use serde_json::Value;

fn main() {
    let frame: Value = serde_json::from_str(include_str!("../tests/fixtures/table_state_hash.json")).unwrap();
    // Warm up (tables, first touch), then time steady state.
    for _ in 0..1_000 {
        assert_eq!(sv10_venue::statehash::verify(&frame), Some(true));
    }
    let n = 20_000;
    let t0 = std::time::Instant::now();
    for _ in 0..n {
        assert_eq!(sv10_venue::statehash::verify(&frame), Some(true));
    }
    let ns = t0.elapsed().as_nanos() as f64 / n as f64;
    println!("state_hash verify over the captured fixture: {ns:.0} ns/frame ({:.0} frames/s)", 1e9 / ns);
    let mut tampered = frame.clone();
    tampered["pot"] = serde_json::json!(tampered["pot"].as_i64().unwrap() + 1);
    assert_eq!(sv10_venue::statehash::verify(&tampered), Some(false));
    println!("{}", sv10_venue::statehash::mismatch_report(&tampered, tampered["table_seq"].as_i64().unwrap() - 1));
}
